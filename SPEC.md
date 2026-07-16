# G7MediaBooster 제품 스펙

- 상태: Accepted
- 스펙 버전: 1.1.0
- 제품 버전: 0.1.0
- 기준일: 2026-07-15

> 이 문서는 설명용 snapshot입니다. 최종 정본은 실행 가능한 소스 코드와 코드에서 생성한
> OpenAPI, DB migration, 어댑터 contract test입니다. 충돌은 코드 기준으로 같은 변경에서
> 바로잡습니다.

## 1. 목적

Gnuboard 5와 7의 PHP 요청 경로에서 대용량 업로드, 이미지 디코딩·리사이즈,
영상 썸네일 추출을 분리합니다. PHP 앱은 권한과 게시물 연결만 담당하고,
Rust 서비스가 업로드 의도, 저장소, 작업 상태, 보안 정책을 일관되게 집행합니다.

우선순위는 다음 순서로 고정합니다.

1. 공격자가 제공한 미디어로부터 PHP/API 프로세스를 보호한다.
2. 무제한 메모리·CPU·디스크·동시성을 없앤다.
3. 브라우저에서 S3/R2로 직접 업로드해 PHP 병목을 제거한다.
4. 결과가 같은 요청은 같은 결과 키를 사용하는 멱등 처리로 만든다.
5. 성능 주장은 고정 fixture와 p95/RSS 실측으로만 승인한다.

### 용어와 v1 범위

- **멀티업로드**: 한 사용자가 여러 파일을 한 번에 선택하고 bounded 병렬로 전송하는 기능
- **multipart upload**: 큰 파일 하나를 여러 part로 나눠 재시도·재개 가능하게 전송하는 기능
- **멀티노드**: API/worker 서버 여러 대가 같은 queue를 공유하는 운영 방식

v1의 필수 범위는 앞의 두 가지이며 멀티노드는 아닙니다. 기본 배포는 단일 서버와 SQLite
WAL durable queue입니다. PostgreSQL 기반 수평 확장은 실제 운영 수요가 확인된 뒤 별도
ADR로만 도입합니다.

## 2. 확정 기술 결정

| 영역 | 결정 |
|---|---|
| 언어 | Rust 2024, toolchain 1.96.0 |
| HTTP | Axum 0.8 + Tokio 1.x |
| 이미지 | libvips 8.15+ 빌드 기준, 운영 참조 이미지 8.18.x 고정, sandbox 프로세스 내부 |
| MP4/MOV 썸네일 | FFmpeg 8 계열 CLI 주 경로 + MP4/H.264 한정 Rust `mp4`/OpenH264 폴백 |
| 저장소 | AWS SDK for Rust, AWS S3와 Cloudflare R2 custom endpoint |
| 작업 저장소 | SQLite WAL 단일 서버 durable queue. 멀티노드는 별도 ADR 후 교체 |
| 인증 | PHP 앱별 HMAC-SHA256 + timestamp + nonce + body hash |
| 계약 | OpenAPI가 HTTP 계약의 단일 기준 |
| 관측 | 구조화 tracing + Prometheus 메트릭 |

SQLite 선택은 G5 서버에도 추가 데몬 없이 설치하고 자원을 적게 쓰기 위한 v1 결정입니다.
공유 파일시스템과 다중 API/worker 노드는 공식 지원하지 않습니다.

## 3. 구성 요소

- `g7mb-api`: 업로드 의도, 완료 확인, 상태 조회, 삭제, 내부 health/metrics
- `g7mb-worker`: durable queue lease, 원본 다운로드, sandbox 실행, 결과 게시
- `g7mb-sandbox`: libvips/FFmpeg/OpenH264를 실행하는 자격 증명 없는 네이티브 경계
- `g7mb-domain`: 상태와 불변 정책
- `g7mb-application`: 저장소·큐·미디어 처리 port와 유스케이스
- 인프라 crate: S3/R2, SQLite, HMAC, telemetry
- `adapters/gnuboard5`, `adapters/gnuboard7`: 앱별 권한 확인과 결과 연결

## 4. 업로드 처리 흐름

1. G7 PHP 모듈이 로그인 사용자와 게시판 업로드 권한을 확인합니다.
2. PHP가 HMAC 서명으로 최대 100개 파일의 `POST /v1/upload-batches`를 호출합니다.
3. API는 서버 생성 object key와 파일별 single PUT 또는 multipart session을 반환합니다.
4. 브라우저는 전체 연결 수와 파일별 part 수를 제한한 채 private quarantine 버킷으로 직접
   병렬 업로드합니다.
5. 실패한 multipart part만 재전송하고 완료 또는 abort를 반드시 호출합니다.
6. PHP/브라우저가 완료 API를 호출하면 API는 `HEAD`와 저장소 응답으로 key, 길이, 상태를
   확인합니다.
7. SQLite 트랜잭션으로 업로드 상태와 durable 작업을 함께 기록합니다.
8. worker가 원본을 제한된 임시 디스크에 스트리밍하고 바이트 시그니처를 검사합니다.
9. 자격 증명이 없는 sandbox가 libvips 또는 FFmpeg를 제한 시간 안에 실행하며, FFmpeg를
   시작할 수 없는 MP4/H.264만 제한된 OpenH264 첫 프레임 경로로 처리합니다.
10. worker가 정규화 master와 파생물을 불변 key로 업로드한 뒤 상태를 `READY`로 전환합니다.
11. PHP 모듈은 서명된 웹훅 또는 상태 조회로 게시물의 `attachment_ids` 흐름에 연결합니다.

API/PHP 프로세스는 원본 전체를 메모리에 적재하지 않습니다. 호환성 프록시 업로드는
별도 feature이며 기본 비활성화합니다.

### 멀티업로드 기본값

- batch 최대 파일 수: 100개
- 브라우저 전체 업로드 연결: 8개
- 파일별 동시 multipart part: 4개
- multipart 전환 기준: 100 MiB 또는 영상
- part 기본 크기: 32 MiB, 공급자 하한·상한 안에서 조정
- bulk lane: 10개 초과 또는 batch 합계 256 MiB 초과
- 활성 upload 예약: global 1,000개, tenant별 200개
- 보존 source 예약량: global 1TiB, tenant별 100GiB

숫자는 hard cap이 아니라 안전한 초기 기본값이며 실제 p95/RSS 측정으로 조정합니다. 연결,
task, channel, retry는 모두 bounded여야 합니다.

## 5. 상태 모델

```text
CREATED -> UPLOADED -> QUARANTINED -> PROCESSING -> READY
                                      |             |
                                      +-> REJECTED  +-> DELETED
                                      +-> FAILED
```

- 상태 전이는 compare-and-set으로만 수행합니다.
- 작업 전달은 at-least-once이며 모든 완료 동작은 멱등입니다.
- 동일 `upload_id + preset_version`은 하나의 활성 작업만 가집니다.
- lease 만료 작업은 재시도하고 최대 횟수 초과 시 dead-letter 상태로 둡니다.
- tenant별 마지막 claim sequence를 영속화해 backlog가 큰 tenant의 독점을 막습니다.
- batch 생성 전과 원자적 DB 저장 시 활성 개수와 보존 source byte capacity를 검사하며 초과 시 presign 없이
  `429 UPLOAD_CAPACITY_EXHAUSTED`를 반환합니다.
- byte quota는 `created`부터 `ready`·`rejected`·`failed`·삭제 대기까지 예약 크기를 유지하고,
  원격 객체 정리가 끝나 `deleted` tombstone이 저장된 뒤에만 반환합니다.

## 6. 포맷 계약

### 이미지 입력

- 필수: JPEG, PNG, WebP, GIF, AVIF, HEIC/HEIF
- 애니메이션은 프레임 수·총 픽셀 예산을 통과한 경우만 첫 프레임 또는 명시 preset 처리
- MIME과 확장자는 힌트일 뿐이며 magic bytes와 실제 decoder 결과가 기준

### 이미지 출력

- JPEG, WebP, AVIF, 필요 시 PNG
- EXIF/GPS/XMP/IPTC는 기본 제거
- orientation 적용 후 sRGB로 정규화
- 크기·품질·crop은 사용자 임의 값이 아니라 버전 관리되는 preset ID로만 선택

### 영상

- 필수 입력 컨테이너는 MP4와 MOV, WebM은 runtime capability로 활성화
- v1 기능은 안전한 원본 업로드와 지정 시점 정지 이미지 추출이며 영상 transcoding은 하지 않음
- FFprobe로 container, codec, duration, stream count를 먼저 제한
- FFmpeg protocol allowlist는 로컬 파일 계열만 허용하며 네트워크 입력은 금지
- FFmpeg fast seek 실패 시 정확 seek와 중앙 timestamp로 한 번만 재시도
- FFmpeg **프로세스를 시작할 수 없을 때만** MP4/H.264 한정 Rust `mp4` demux + OpenH264
  첫 유효 frame fallback 실행. FFmpeg decode 실패를 임의로 우회하지 않음
- fallback은 입력 512MiB, sample 120개, encoded decode 32MiB, 개별 sample 16MiB,
  한 변 4,096px, 16MP로 독립 제한하고 임시 PPM은 정상 libvips strip/resize 경로로 재가공

### v1 제외

- 외부 URL 가져오기, SVG, PDF, PS/EPS, ImageMagick/GD fallback, 임의 변환 URL,
  사용자 제공 FFmpeg/libvips 옵션, 영상 transcoding, JPEG XL 기본 활성화

## 7. 기본 자원 한도

| 항목 | 기본값 | 하드 상한 |
|---|---:|---:|
| 이미지 원본 | 64 MiB | 128 MiB |
| 영상 원본 | 2 GiB | 5 GiB |
| 이미지 한 변 | 32,768 px | heavy tier 65,535 px |
| 총 픽셀 | 일반 raster 200 MP, AVIF/HEIF 64 MP | heavy tier는 decoder 실측 범위 안에서만 상향 |
| 애니메이션 프레임 | 300 | 500 |
| MP4/MOV 길이 | 2시간 | 6시간 |
| 요청 JSON body | 1 MiB | 1 MiB |
| API 요청률 | 50 req/s, burst 100 | 10,000 req/s, burst 100,000 |
| API 동시 처리 | 64 | 1,024 |
| sandbox 작업 시간 | 이미지 30초 / 영상 45초 | 120초 |
| sandbox RSS | standard 512 MiB | heavy AVIF gate 1.5 GiB, worker cgroup 2 GiB |
| worker 임시 디스크 예약 | 프로세스당 12 GiB | 설정 1 TiB, 작업 1개 최악치보다 작으면 시작 거부 |

모든 동시성은 bounded semaphore/queue로 제한합니다. worker 프로세스 수 `N`과 libvips
내부 thread 수 `M`의 곱은 할당 CPU core 수를 넘지 않는 값에서 시작합니다.
작업별로 `원본 + 이미지 파생물 2개 + 워터마크 16MiB`를 1MiB 단위로 선예약하며,
예약이 부족하면 다운로드 전에 대기합니다. 파생 파일은 이미지 원본 hard cap을 넘으면
provider 업로드 전에 거부하고 systemd `LimitFSIZE=6G`를 파일별 최종 안전망으로 둡니다.

heavy tier는 별도 queue와 기본 동시성 1을 사용합니다. 한 변이 20,000px를 넘더라도
format별 총 픽셀 예산 안이면 지원합니다. 200MP AVIF 실측은 3.46GiB peak RSS로 기본
2GiB worker cgroup을 초과했으므로 AVIF/HEIF는 64MP에서 fail-closed합니다. JPEG 25,000px
panorama 지원과 AVIF/HEIF 최신 포맷 지원은 유지하되 decoder별 안전 한도를 분리합니다.

## 8. 저장소 규칙

- `[storage].provider`는 `r2`, `aws-s3`, `lightsail`, `generic` 중 하나를 반드시 명시하며
  endpoint·region·path-style·bucket 형태가 선언과 다르면 모든 네트워크 요청 전에 거부합니다.
- R2는 권한이 있는 S3 access key로 `CreateBucket`과 CORS bootstrap을 수행할 수 있습니다.
  Lightsail bucket access key는 기존 단일 버킷의 object 작업 전용이므로 버킷 생성과 CORS는
  Lightsail API/콘솔에서 먼저 설정하며 S3 bootstrap이 이를 시도하면 네트워크 전에 거부합니다.
- raw/quarantine 버킷은 private이며 CDN 공개를 금지합니다.
- 파생물 버킷은 별도 권한과 lifecycle을 사용합니다.
- 기본 파생물 버킷도 private입니다. G7 사용자 소유권을 확인한 뒤 5분짜리 presigned GET만
  반환하며 PHP는 미디어 bytes를 proxy하지 않습니다.
- 원본 파일명은 object key에 넣지 않습니다.
- 예시 원본 key: `raw/{tenant}/{yyyy}/{mm}/{upload_id}/source`
- 예시 결과 key: `media/{tenant}/{upload_id}/{source_sha256}/{preset_version}/{variant}.{ext}`
- ETag는 SHA-256으로 간주하지 않습니다. 서비스가 별도 digest를 저장합니다.
- 미완료 multipart 24시간, rejected/failed 원본 7일 정리 정책을 기본으로 합니다.

썸네일 URL은 `/media/{tenant}/{asset_id}/{source_rev}/{preset_rev}/{variant}.{ext}` 형태의
불변 versioned key를 사용합니다. v1 preset은 업로드 처리 중 eager 생성하며 임의
`width/quality` query와 요청 시 변환은 지원하지 않습니다. 실제 썸네일은 private object
storage에 저장하고 메모리는 Moka의 frequency admission/LRU eviction을 쓰는 byte-weighted
manifest cache로만 제한합니다. 기본 상한은 4MiB·TTL 60초이며 같은 upload의 동시 miss는
singleflight로 합칩니다. thumbnail bytes, presigned URL, mutable 삭제 상태는 캐시하지 않습니다.

워터마크는 관리자 등록 asset, 위치, 여백, 최대 비율, 투명도 preset만 허용합니다. 결과
key에는 preset revision과 watermark digest를 포함합니다.

## 9. HTTP API v1

- `GET /health/live`: 프로세스 생존
- `GET /health/ready`: 필수 내부 의존성 준비 상태
- `GET /metrics`: 내부망 전용 Prometheus 형식
- `GET /v1/capabilities`: 런타임 decoder/encoder와 한도
- `POST /v1/upload-batches`: 최대 100개 업로드 의도 일괄 생성
- batch 생성 응답에서 파일 정책에 따라 multipart session을 함께 생성
- `POST /v1/uploads/{id}/parts/{part}/presign`: 제한된 part URL 생성
- `POST /v1/uploads/{id}/multipart/complete`: part 목록 검증과 object 완료
- `GET /v1/uploads/{id}/derivatives/{variant}/delivery`: HMAC tenant 확인 후 Ready private
  derivative의 단기 GET URL 발급
- `DELETE /v1/uploads/{id}/multipart`: 미완료 multipart abort
- `POST /v1/uploads/{id}/complete`: 저장소 확인과 작업 enqueue
- `GET /v1/uploads/{id}`: 상태와 파생물 조회
- `DELETE /v1/uploads/{id}`: 삭제 예약

삭제 요청은 `202`로 durable state에 기록됩니다. cleanup worker가 derivative→raw 순으로
삭제하고 모든 저장소 작업 성공 뒤 `deleted` tombstone을 남깁니다. 미완료 `created`
multipart는 자동 abort하며, 기본 TTL 24시간·rejected/failed 원본 보존 7일·batch 100·최대
10회 재시도를 Rust 운영 hard cap으로 둡니다. 삭제 tombstone은 기본 365일 보존하고 한 번에
최대 100개만 물리 삭제합니다. 처리 중 upload와 site-policy 참조 자산은 경합·무결성 보호를
위해 삭제를 거부합니다.

health/metrics를 제외한 모든 API는 인증과 tenant scope가 필요합니다. 오류 응답은
안정된 `code`, 사용자 안전 `message`, `request_id`를 가집니다.
`/v1`은 token bucket과 동시 처리 semaphore를 함께 적용하고 초과 시 `Retry-After`가 있는
안정된 `429`를 반환합니다. health와 metrics는 과부하 판단을 위해 limiter를 거치지 않습니다.

## 10. PHP 어댑터 계약

- G5와 G7 어댑터는 별도 디렉터리로 유지합니다.
- G7은 플러그인이 아니라 `jiwonpapa-g7mediabooster` 전용 모듈로 구현합니다.
- PHP가 소유하는 것: 세션, 사용자/게시판 권한, 게시물 attachment 연결, UI 문구
- Rust가 소유하는 것: object key, 업로드 한도, MIME 판정, 작업 상태, 파생 preset
- 서명 canonical string은 method, path+query, timestamp, nonce, body SHA-256을 포함합니다.
- 허용 시계 오차는 기본 300초이며 nonce는 만료까지 원자적으로 한 번만 사용합니다.
- G7 관리자 설정은 revision이 있는 policy snapshot으로 서명 전송합니다. Rust가 G7 DB를
  직접 읽지 않으며 G7 설정은 Rust hard cap을 높일 수 없습니다.
- S3/R2 credential, SQLite 경로, sandbox hard cap은 Rust 운영 설정에만 둡니다.

## 11. 보안 경계

- API에는 네이티브 decoder를 링크하지 않습니다.
- sandbox에는 S3/R2 자격 증명과 외부 네트워크를 주지 않습니다.
- worker만 raw read와 derivative write 권한을 가집니다.
- child process는 shell 없이 실행하고 timeout 시 kill 후 reap합니다.
- 원본 파일명, presigned URL, 인증 헤더, secret은 로그에 남기지 않습니다.
- reverse proxy만 TLS를 종료하며 API는 기본적으로 loopback/private network에 bind합니다.
- libvips thread, FFmpeg thread, sandbox process 수를 각각 제한하고 systemd/cgroup
  `CPUQuota`, `MemoryMax`, `TasksMax`를 배포 기본값으로 제공합니다.

상세 위협과 통제는 `docs/SECURITY.md`를 따릅니다.

## 12. 성능 승인 기준

초기 목표이며 기준 장비/fixture의 첫 측정 뒤 ADR 없이 낮출 수 없습니다.

- 업로드 의도 API p95 100ms 이하(스토리지 presign 포함)
- 완료 API p95 500ms 이하(HEAD와 DB commit 포함)
- 정상 부하에서 queue-to-start p95 2초 이하
- API idle RSS 128 MiB 이하
- 일반 JPEG/WebP 파생물 p95 1.5초 이하
- AVIF 파생물 p95 5초 이하
- 1080p H.264 MP4 썸네일 p95 2.5초 이하
- 승인 기준 대비 p95 또는 peak RSS 10% 초과 회귀는 release blocker

## 13. 완료 정의

기능은 다음 증거가 모두 있어야 완료입니다.

- 단위·계약·통합·native fixture 테스트 통과
- fmt, clippy, rustdoc, nextest, coverage 게이트 통과
- cargo-audit, cargo-deny, Rust CycloneDX SBOM과 시스템 라이브러리 inventory 검토
- 고정 fixture 기준 p50/p95/p99, 처리량, peak RSS, 임시 디스크 기록
- G5와 G7 각각 브라우저 직접 업로드부터 게시물 표시까지 smoke 통과
- 100개 batch와 multipart 중단·재개·abort에서 메모리 상한과 멱등성 검증
- EXIF/GPS/XMP/IPTC 제거, watermark revision, thumbnail cache eviction 검증
- 25,000px 이상 panorama와 heavy tier 경계에서 peak RSS/CPU 제한 검증
- 실패·timeout·중복 complete·worker 강제 종료 복구 검증

## 14. 구현 현황

2026-07-16 기준으로 다음 제어 계층이 구현됐습니다.

- 최대 100개 batch 정책과 single PUT/multipart 자동 선택
- S3/R2 multipart create, part presign, complete, abort adapter
- digest-pinned MinIO 컨테이너에서 실제 presigned PUT, 2-part complete, HEAD, bounded download,
  abort session 소멸, derivative PUT·private presigned GET S3 호환 conformance
- HMAC timestamp, nonce replay 차단, body SHA-256 검증
- SQLite batch·upload·multipart session 영속화와 lease queue
- multipart part 번호·정확한 길이·완료 순서 검증
- 완료 후 object storage `HEAD` 실제 크기 검증과 멱등 완료
- 원본 object를 64KiB chunk로 private 임시 디스크에 내려받는 bounded streaming worker
- JPEG/PNG/GIF/WebP/AVIF/HEIF/MP4/MOV/WebM magic-byte 판정과 선언 종류 교차 검증
- libvips 이미지 header probe, FFprobe container·codec·duration·stream 제한 검사
- 자격 증명을 전달하지 않는 sandbox subprocess, timeout·thread·출력 크기 제한
- Linux sandbox의 전체 runtime thread seccomp-BPF socket/connect/bind/listen/send/recv
  syscall 차단과 native child 상속, 실제 컨테이너 `EPERM` 검증
- 이미지 metadata를 제거한 JPEG 기본 썸네일과 영상 FFmpeg 썸네일 생성
- FFmpeg 실행 파일 부재를 실제로 주입한 MP4/H.264에서 Rust `mp4` demux + OpenH264 첫
  frame을 추출하고 libvips JPEG로 재가공하며 HEVC/AV1·MOV·WebM은 폴백하지 않는 계약
- 실제 AVIF encode/decode와 HEIC/HEIF signature·decoder probe·JPEG 파생 runtime smoke
- 실제 MP4/MOV H.264를 FFprobe·FFmpeg로 검사해 원 container master와 JPEG poster를
  원자적으로 Ready 발행하고 private delivery에서 container MIME을 보존하는 종단 smoke
- source/derivative SHA-256 기반 불변 key와 멱등 게시, 상태 조회 API
- worker lease heartbeat·재선점·bounded retry·dead-letter와 systemd 자원 제한 기본값
- 프로세스당 12GiB 임시 디스크 예약 semaphore, 작업별 최악치 선예약, 파생 파일 크기
  사후검사와 systemd `LimitFSIZE=6G` 안전망
- 위장 PHP 선차단, EXIF/GPS 제거, 실제 JPEG/MP4 probe·thumbnail fixture 하네스
- `jiwonpapa-g7mediabooster` G7 모듈의 관리자 설정, HMAC 제어 client와 사용자·게시판별
  upload ID 소유권 저장
- 브라우저→S3/R2 direct single/multipart 멀티업로더, 전체 연결·파일·part bounded 동시성,
  진행률·취소·transient retry·ETag/abort 하네스
- 브라우저 100개 선택을 제어 요청 1회·전체 연결 최대 8개로 처리하고, Rust가 100개
  예약을 저장 1회로 커밋하는 경계 테스트
- 이미지·영상 poster 파생물의 위치·비율·투명도 제한 워터마크, 작업별 16MiB
  복사·SHA-256 pin, preset revision+전체 digest 기반 불변 key와 fail-closed 설정 오류 처리
- G7 관리자가 Ready 이미지 upload ID를 선택해 HMAC 서명된 site policy revision을 발행하고,
  Rust가 tenant·상태·형식·크기·digest를 재검증한 뒤 작업 enqueue 시 정확한 revision 고정
- G7 0.4.0 관리자 asset picker가 current-admin·최근 7일·Ready PNG/WebP/JPEG·16MiB 경계를
  적용하고 실제 브라우저에서 선택·저장·재로드·rollback을 통과
- 실제 4000×3000 JPEG 100개를 worker 동시성 4·native thread 1로 처리하고, 의도적으로
  유실한 lease 10개를 만료 후 재선점해 Ready·파생본 100개, dead-letter 0을 확인한
  `cargo xtask load100` 운영 하네스. 현재 기준 장비 재검증에서 13.12 jobs/s, p95 352ms,
  process tree peak RSS 582,976KiB, peak temp disk 9,628KiB
- 16,384px 또는 100MP 경계로 heavy image를 분류하고 full-pixel 구간을 별도 semaphore
  기본 1개로 제한. 실제 25,000×4,000 JPEG를 native thread 1로 1,280px 파생 처리해
  386ms, process tree peak RSS 24,368 KiB 확인
- SQLite tenant round-robin claim sequence와 global 1,000/tenant 200 활성 예약 hard cap,
  provider presign 전 preflight·원자적 저장 재검사·안정적인 429 backpressure
- Linux 컨테이너에서 CPU 2 core, memory 2GiB, PID 64, network none cgroup 아래 실제 API와
  100개 worker 부하를 함께 실행해 API health 665/665, 작업 100/100 완료와 cgroup peak
  1,782,890,496 bytes, worker peak temp disk 8,588KiB 확인
- sandbox가 내장 fixture로 JPEG/PNG/GIF/WebP/AVIF/HEIF decode,
  JPEG/PNG/WebP/AVIF encode, FFprobe+FFmpeg MP4/MOV H.264 poster를 실제 실행하고 OpenH264
  폴백 compile 경계를 보고합니다. API는 이 전체 v1 capability가 없으면 시작을 거부하며
  HMAC 인증된 `/v1/capabilities`만 검증 snapshot을 반환합니다.
- G7 관리자 전용 capability proxy가 Rust 응답을 길이·형식 제한 후 반환합니다.
- HMAC tenant-scoped 삭제 예약, G7 소유권 프록시, SQLite cleanup lease/retry/attempt 상한,
  만료 multipart abort, derivative/raw idempotent 삭제와 systemd timer를 구현했습니다.
- G7 module 0.3.0에 form 자동 연결, Ready master·thumbnail 전건 검증, DB lock 기반 native
  attachment 멱등 materialization, 게시글 scope·삭제글 정책을 재사용하는 private viewer redirect,
  soft-delete 보존 대조를 구현하고 공개 G7 `fcaacad` 기준 `sirsoft-board` 1.0.2→1.1.0 upstream patch 6개와 29항목·activation 검증기를 준비했습니다.
- G5 5.6.24 core-free adapter에 PHP 8-safe hook, HMAC control proxy, browser direct single/multipart,
  MyISAM advisory-lock attachment 연결과 private delivery를 구현하고 실제 MySQL 8.4·MinIO 브라우저에서
  2개 동시 업로드→Rust 처리→게시글 첨부 표시와 비로그인 `403`을 검증했습니다.

- 정확히 5GiB를 32MiB 160-part로 직접 전송하면서 80번째 part 뒤 API를 재기동해 같은
  SQLite/provider session으로 재개하고, complete 2회 멱등 처리·HEAD 길이·Quarantined 상태와
  API RSS 증가 416KiB를 확인했습니다.

실제 R2/Lightsail credential별 conformance, G7 upstream merge와 실 provider 보존 삭제는
외부 운영 게이트로 남아 있으며 `docs/IMPLEMENTATION_PLAN.md` 순서로 추진합니다.
