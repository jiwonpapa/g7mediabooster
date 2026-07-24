# 썸네일 서버·Thumbor 비교·보안 검토

- 검토일: 2026-07-24
- 기준: `5c00662` 이후 2026-07-24 공개 썸네일 개선분
- 범위: G5/G7 PHP bridge, Rust API·worker·sandbox, private derivative delivery,
  향후 공개 URL delivery와 얼굴 중심 crop

## 결론

기존 기본 경로는 **PHP 권한 확인 → loopback Rust HMAC 요청 → 5분 private object URL → 302**
방식입니다. 추가로 별도 loopback listener의 signed exact-thumbnail endpoint를 구현했지만 기본
비활성이며 G7 URL producer·실 Nginx ingress 검증 전에는 공식 지원으로 게시하지 않습니다.
이미지 bytes와 변환은 PHP가 처리하지 않으며, 업로드 후 Rust worker가 master와 thumbnail을
미리 생성합니다. 얼굴 검출은 아직 구현되지 않았습니다.

최종 구조는 Rust 서버 하나가 다음 두 표면을 분리해 제공하는 것이 적절합니다.

1. `127.0.0.1:8088` control plane: PHP 전용 HMAC API
2. 별도 public delivery plane: Nginx 뒤의 서명된 immutable preset URL

PHP는 G5/G7 게시글 권한 확인과 짧은 access token 발급만 담당하고, preset 검증·cache·queue·가공·
storage redirect는 Rust가 담당해야 합니다. Nginx는 TLS 종료와 경로 제한만 맡습니다.

## 현재 구현

```text
Browser
  -> G5/G7 same-origin attachment URL
  -> PHP 게시글·첨부 소유권/열람 권한 확인
  -> HMAC + timestamp + nonce로 127.0.0.1:8088 호출
  -> Rust가 tenant, Ready, deletion 상태, master|thumbnail을 재검사
  -> 5분 S3/R2 GET URL 발급
  -> PHP가 private/no-store/no-referrer 302
```

- Rust API 기본 bind와 제한: `config/g7mb.example.toml:1-6`
- HMAC body hash·timestamp·nonce 검증: `apps/g7mb-api/src/lib.rs:1209-1255`
- delivery variant와 Ready/deletion 재검사: `crates/g7mb-application/src/delivery.rs:137-200`
- bounded metadata cache: `crates/g7mb-application/src/delivery.rs:17-52`, `107-127`
- G7 게시글 권한과 exact derivative 검증:
  `adapters/gnuboard7/jiwonpapa-g7mediabooster/src/Http/Controllers/User/AttachmentDeliveryController.php:37-84`
- G5 게시글 열람·attachment mapping 검증:
  `adapters/gnuboard5/jiwonpapa-g7mediabooster/plugin/g7mediabooster/src/DeliveryEndpoint.php:13-49`
  `adapters/gnuboard5/jiwonpapa-g7mediabooster/plugin/g7mediabooster/src/RemoteDelivery.php:17-52`

현재 URL은 stable G5/G7 attachment URL이지만, Rust가 직접 받는 Thumbor식 변환 URL은 아닙니다.
허용 variant도 `master|thumbnail` 두 개로 고정되어 있습니다.

## Thumbor가 처리하는 방식

Thumbor 7.8은 Python Tornado 서버가 URL을 받아 loader로 원본을 읽고, 요청 경로의 crop·resize·
filter를 적용한 뒤 result storage에 결과를 저장합니다. 현재 기본 engine은 PIL입니다.
운영 문서는 여러 Thumbor process를 Nginx 같은 load balancer 뒤에 두도록 권장합니다.

- URL 변조와 무제한 size 조합을 막기 위해 `SECURITY_KEY`로 URL을 서명합니다.
- production에서는 `/unsafe`를 꺼야 합니다.
- HTTP loader를 쓸 때 source allowlist와 source byte 상한이 필요합니다.
- face/feature detector는 OpenCV를 사용하며 기본 비활성입니다.
- 이 기능은 사람의 신원을 식별하는 얼굴 인식이 아니라, smart crop용 얼굴 위치 검출입니다.
- 공식 문서도 얼굴 검출을 동기 요청에서 수행하는 것은 비싸므로 Redis queued lazy detection을
  권장합니다.
- result storage hit는 재가공을 피하지만 cold miss는 원본 fetch, decode, 얼굴/feature 검출,
  resize, encode가 한 HTTP 요청에 모두 걸립니다.

따라서 과거 Thumbor가 느렸던 주원인은 Python 라우팅 자체보다는 **cold request에 source fetch와
PIL decode/encode, OpenCV smart detection을 묶고, cache miss와 URL variant 수만큼 같은 비용을
반복하는 구조**였을 가능성이 큽니다. process 수·result storage·lazy detection 설정이 부족하면
더 크게 드러납니다.

공식 참고:

- https://thumbor.readthedocs.io/en/stable/running.html
- https://thumbor.readthedocs.io/en/stable/hosting.html
- https://thumbor.readthedocs.io/en/7.7.7/configuration.html
- https://thumbor.readthedocs.io/en/latest/detection_algorithms.html
- https://thumbor.readthedocs.io/en/latest/lazy_detection.html
- https://thumbor.readthedocs.io/en/7.5.2/result_storage.html

## 권장 URL 서버 계약

```text
GET /media/v1/{tenant}/{upload_id}/{preset}/{revision}.{ext}?token={signature}
```

서명 대상에는 method, canonical path, tenant, upload ID, preset, revision, expiration을 포함합니다.
다음은 받지 않습니다.

```text
?url=https://arbitrary.example/image.jpg
?width=99999&height=99999&filter=...
```

원본은 서버가 이미 소유한 `upload_id`로만 찾고 preset은 관리자 allowlist만 허용합니다. 공개
preset은 upload 직후 미리 생성하며, 드문 preset만 durable queue로 lazy 생성합니다. cache miss
HTTP 요청 안에서 full decode나 얼굴 검출을 실행하지 않습니다.

비공개 게시물은 두 방식을 선택할 수 있습니다.

1. 현재 방식: 매 요청 PHP 권한 확인. 즉시 권한 회수에 강하지만 PHP request가 한 번 필요합니다.
2. 짧은 token 방식: 페이지 렌더 때 PHP가 1~5분 access token 발급, Rust가 직접 검증. 빠르지만
   token 만료 전까지 권한 회수가 지연됩니다.

기본은 비공개 attachment에 현재 방식을 유지하고, 공개 이미지와 목록 썸네일에 Rust public
delivery URL을 사용하는 것이 안전합니다.

## 얼굴 중심 crop

현재 저장소에는 OpenCV, ONNX, face detector, focal-point 구현이 없습니다. 지원으로 표시하면
안 됩니다.

추가할 경우 얼굴 검출은 URL 요청 시점이 아니라 업로드 queue의 선택 작업으로 수행합니다.

1. credential 없는 no-network sandbox에서 축소 preview만 detector에 입력
2. model/cascade 파일 SHA-256 고정
3. timeout, pixel, RSS, CPU, process 동시성 제한
4. 얼굴 원본이나 embedding은 저장하지 않고 crop용 bounding box/focal point만 저장
5. 검출 실패 시 center crop으로 결정적으로 fallback
6. focal-point revision을 derivative key에 포함
7. 실제 crop·encode는 libvips가 수행

이 방식은 Thumbor의 smart crop 장점을 유지하면서 request latency와 detector 폭주를 제거합니다.

## 보안 판정

### Critical / High

확인된 항목이 없습니다.

### MED-01 — 공개 reverse proxy 계약 부재

- 상태: **코드·reference config 해결, 실 Nginx 종단 검증 대기**
- 증거: 공개 listener는 control router와 분리됐고 `/media/v1/` exact route만 가집니다.
  `deploy/nginx/g7mediabooster-public.conf`는 다른 모든 경로를 `404`로 닫습니다.
- 영향: 운영자가 `8088` 전체를 임의 proxy하면 `/metrics`와 health가 외부에 노출되고 control API
  공격 표면이 불필요하게 증가합니다. `/v1`은 HMAC·rate limit으로 보호되지만 public ingress
  계약으로 충분하지 않습니다.
- 조치: public delivery를 control API와 다른 listener/router로 분리하고 Nginx는 그 경로만
  허용해야 합니다. `/v1`, `/metrics`, worker metrics는 public listener에 존재하지 않게 합니다.

### MED-02 — 얼굴 detector 보안 경계 부재

- 상태: 구현 전 기능입니다.
- 영향: detector를 worker나 HTTP handler에 단순 추가하면 CPU/RSS 폭주, native parser 공격,
  model 변조, 얼굴 좌표 개인정보 잔존 위험이 생깁니다.
- 조치: 위의 async sandbox 계약과 자원 gate가 구현되기 전에는 얼굴 인식을 공식 지원으로
  게시하지 않습니다.

### LOW-01 — provider redirect host allowlist 없음

- 상태: **Rust provider boundary 해결**
- 증거: storage provider·endpoint·bucket·region에서 exact redirect authority를 도출하고,
  presign 결과의 scheme·authority가 일치하지 않으면 `UntrustedRedirect`로 폐기합니다.
- 잔여 방어: G5/G7 PHP validator의 독립 host pin은 Rust 프로세스 자체 침해까지 방어하려는
  추가 계층으로 남아 있습니다.

### INFO-01 — native codec 보안은 Cargo 감사 범위 밖

- `cargo audit --deny warnings`: Rust vulnerability 0
- libvips, FFmpeg, libheif와 향후 OpenCV/ONNX는 Cargo advisory가 검사하지 않습니다.
- 배포 시 OS 보안 업데이트, native SBOM, capability fixture와 버전 정책을 별도 gate로 유지해야
  합니다.

## 이미 확인된 방어

- arbitrary remote source URL이 없으므로 Thumbor HTTP loader형 SSRF가 없습니다.
- API body 1MiB, 15초 timeout, token bucket과 동시 요청 상한이 있습니다.
- HMAC-SHA256이 method/path/body hash/timestamp/durable nonce를 묶습니다.
- source key는 서버 생성 `ObjectKey`이고 원본 파일명을 사용하지 않습니다.
- magic byte와 실제 libvips/FFprobe 검사를 함께 사용합니다.
- FFmpeg/FFprobe protocol은 `file,crypto,data`로 제한하고 shell을 사용하지 않습니다.
- sandbox는 환경을 비우고 timeout·kill-on-drop·출력 상한을 적용합니다.
- Linux sandbox는 socket 계열 syscall을 seccomp로 차단합니다.
- systemd는 `NoNewPrivileges`, read-only system, capability drop, CPU/RSS/PID/file-size 상한을
  적용합니다.
- signed object URL은 기본 5분이며 bytes와 signed URL은 메모리 cache에 저장하지 않습니다.
- TypeScript source의 `innerHTML` 사용은 정적 component template에 한정되며 사용자 파일명과
  서버 메시지는 `textContent`로 출력합니다. 동적 code execution과 arbitrary navigation sink는
  발견되지 않았습니다.

## 현재 재검증 결과

- `cargo xtask quick`: Rust 130 tests PASS, 외부 provider·실 fixture 5 tests 명시적 ignored
- public router 종단 회귀: signed exact URL `302`, control·metrics·tamper·extra query·HEAD `404`
- `cargo xtask harness-governance`: Python governance 25 tests PASS
- workspace clippy `-D warnings`, rustdoc, `git diff --check` PASS
- RustSec: 378 dependencies, vulnerabilities 0

## 구현 우선순위

1. ~~control listener와 public delivery listener 분리~~
2. ~~signed immutable preset URL과 provider authority pin~~
3. ~~Nginx 최소 route·rate limit reference와 router 회귀 테스트~~
4. G7 public thumbnail URL producer와 실제 Nginx ingress 종단 검증
5. 공개/list thumbnail은 PHP 우회, private attachment는 현재 PHP gate 유지
6. async face focal-point sandbox를 선택 기능으로 추가
7. 같은 fixture로 center crop 대비 latency·RSS·crop 품질 측정 후 기본 활성화 여부 결정
