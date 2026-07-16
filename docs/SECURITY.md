# 보안 모델

## 보호 자산

- PHP 사용자 세션과 게시판 권한
- S3/R2 access key와 presigned URL
- 비공개 원본 및 공개 전 파생물
- 작업 상태와 tenant 경계
- 서버 CPU, 메모리, 디스크, process table

## 공격자 능력

공격자는 업로드 파일명, 바이트, MIME, 크기, 요청 순서와 반복 횟수를 통제할 수 있다고
가정합니다. 손상 이미지, polyglot, 압축 폭탄, 과다 프레임, 비정상 MP4, replay, 중복 완료,
고의 timeout과 저장소 오류를 만들 수 있습니다.

## 주요 위협과 통제

| 위협 | 기본 통제 |
|---|---|
| PHP 우회 직접 호출 | 앱별 HMAC, timestamp, durable nonce, tenant scope |
| 같은 tenant 사용자 간 upload ID 탈취 | G7 DB의 `upload_id + user_id + board_slug` 소유권 검사 |
| 업로드 key 덮어쓰기 | 서버 생성 불변 key, 짧은 presign, content length 조건 |
| MIME/확장자 위장 | magic byte + decoder probe + allowlist |
| 픽셀/프레임 폭탄 | bytes/dimension/pixel/frame/duration 다중 한도 |
| native decoder 취약점 | credential 없는 sandbox, local-path 입력, timeout, cgroup/rlimit |
| FFmpeg SSRF/프로토콜 악용 | 외부 URL 금지, protocol allowlist, shell 미사용 |
| OpenH264 폴백 남용 | MP4/H.264만 허용, FFmpeg spawn 실패만 진입, sample/byte/NAL/pixel hard cap |
| path traversal | 원본명 미사용, 검증된 `ObjectKey` newtype |
| replay/중복 작업 | nonce 원자 소비, idempotency key, DB unique constraint |
| secret 유출 | `secrecy`, sensitive header, 구조화 로그 redaction |
| 저장소 혼동 | raw와 derivative bucket/IAM 분리, HEAD 검증 |
| 삭제 권한 우회·경합 | HMAC tenant scope, G7 소유권, 처리 중 충돌, policy asset pin |
| 삭제 중 장애·중복 | SQLite lease/CAS, idempotent abort/delete, 성공 뒤 tombstone |
| 공급망 취약점 | lockfile, audit/deny, pinned CI, native SBOM |

## HMAC 계약

canonical payload는 UTF-8로 다음 순서를 사용합니다.

```text
G7MB-HMAC-SHA256
{key_id}
{unix_timestamp}
{nonce}
{HTTP_METHOD}
{path_and_canonical_query}
{lowercase_hex_sha256_body}
```

- 서명은 base64url no-padding으로 전송합니다.
- timestamp 허용 오차 기본값은 300초입니다.
- nonce는 128-bit 이상 난수이며 `(key_id, nonce)` unique입니다.
- body를 읽기 전에 body size 상한을 적용합니다.
- signature 검증과 nonce 소비가 성공한 뒤에만 비즈니스 처리를 시작합니다.

## 네이티브 런타임

`cargo-audit`는 libvips, GLib, FFmpeg, libheif, OpenH264 native code와 codec library를
검사하지 않습니다. 배포
이미지는 패키지 목록과 `vips --version`, `ffmpeg -buildconf`를 SBOM에 포함해야 합니다.
버전 문자열 성공만으로 capability를 승인하지 않습니다. sandbox `capabilities` 명령이
내장 fixture로 필수 이미지 decode/encode와 MP4/MOV H.264 추출을 실행하고, API는 결과가 v1
전체 조건을 충족하지 않으면 시작을 거부합니다. capability HTTP 응답도 HMAC·nonce 인증을
요구합니다. 별도의 더 큰 native fixture release gate도 실행합니다.

## 운영 기본값

- TLS는 reverse proxy에서 종료합니다.
- API는 `127.0.0.1` 또는 private interface에만 bind합니다.
- `/metrics`는 public ingress에서 차단합니다.
- raw bucket public access와 listing을 모두 차단합니다.
- sandbox root filesystem read-only, `PrivateTmp`, `NoNewPrivileges`, capability drop을 적용합니다.
- production secret은 환경변수 평문 배포보다 secret file/manager를 우선합니다.
- G7 HMAC secret은 `sensitive` 설정으로 Laravel Crypt 암호화하며 관리자 조회와 browser
  configuration 응답에는 원문을 포함하지 않습니다.
- direct upload는 HTTPS 또는 literal loopback HTTP만 허용하고 cross-origin credential을
  보내지 않습니다. multipart CORS는 `ETag`를 명시적으로 expose해야 합니다.
- 워터마크는 브라우저 경로·임의 옵션을 받지 않습니다. worker에 등록된 16MiB 이하 자산을
  작업 디렉터리로 복사해 SHA-256 pin을 확인하고 revision+digest를 불변 결과 key에 넣습니다.
  자산 부재·digest 불일치는 원본 거부로 오인하지 않고 worker가 fail-closed로 중지합니다.

## 현재 구현과 남은 차단선

- 구현: API는 presign·multipart 제어·HEAD·private delivery에, worker/maintenance는 raw GET과
  derivative PUT·삭제에 S3/R2 자격 증명을 사용합니다. 기본 systemd 설치는 같은 root-only
  source를 서비스별 credential directory에 복사하지만 PHP·브라우저에는 전달하지 않습니다.
  sandbox child는 `env_clear` 후 로컬 경로만 받아 저장소 자격 증명을 소유하지 않습니다.
- 구현: `[storage].provider`를 필수 정본으로 저장하고 endpoint·region·path-style·Lightsail
  단일 bucket 형태를 API/worker와 S3 adapter 진입 전에 fail-closed로 검증합니다.
- 구현: FFmpeg/FFprobe protocol을 `file,crypto,data`로 제한하고 shell을 사용하지 않습니다.
- 구현: OpenH264 폴백은 worker가 이미 검증한 MP4/H.264에만 허용하고 FFmpeg `spawn`
  실패에서만 진입합니다. 입력 512MiB, sample 120개/16MiB, decode 32MiB, NAL 1,024개,
  parameter set 256KiB, 4,096px/16MP 한도를 적용하며 같은 no-network sandbox와 cgroup 안에서
  실행합니다. 임시 PPM은 libvips로 재인코딩한 뒤 삭제합니다.
- 구현: subprocess timeout·kill, native thread 수, 출력 JSON 크기, systemd
  `CPUQuota`/`MemoryMax`/`TasksMax`를 제한합니다.
- 구현: 16,384px 초과 또는 100MP 초과 image와 모든 video의 full-pixel 구간은 일반 worker
  slot 외에 별도 semaphore를 획득하며 기본 동시성은 각각 1입니다.
- 구현: 활성 upload 예약은 global/tenant hard cap을 preflight와 원자적 저장에서 검사하고,
  초과 요청은 provider presign 전에 안정적인 429로 차단합니다.
- 구현: 삭제 요청은 tenant와 G7 사용자 소유권을 확인해 durable 예약하며, 처리 중 upload와
  site-policy 참조 자산을 거부합니다. 삭제 대기 상태에서는 파생 URL을 반환하지 않습니다.
- 구현: 썸네일 bytes와 presigned URL은 캐시하지 않습니다. immutable manifest만 기본
  4MiB·60초로 제한하고, delivery마다 SQLite의 Ready/deletion guard를 다시 확인합니다.
  durable 삭제 요청 성공 시 해당 manifest를 즉시 invalidate합니다.
- 구현: 만료 created multipart abort와 rejected/failed 원본 보존 정리를 최대 100개 batch,
  5분 lease, 1분 retry, 기본 10회 attempt 상한으로 실행하고 derivative→raw 성공 뒤에만
  tombstone 처리합니다.
- 구현: Linux sandbox 시작 시 seccomp-BPF를 모든 runtime thread에 설치해 socket/connect/
  bind/listen/send/recv 계열 syscall을 `EPERM`으로 차단합니다. FFmpeg/FFprobe child가 필터를
  상속하며 Linux 컨테이너 테스트가 새 socket 생성 거부를 확인합니다.
- 구현: Docker cgroup에서 CPU 2 core, memory 2GiB, PID 64, network none을 강제한 채 API와
  실제 100개 JPEG worker 부하를 함께 실행해 health 실패 0과 cgroup peak 상한 준수를 확인합니다.
- 구현: worker는 프로세스당 기본 12GiB 임시 디스크를 1MiB permit으로 관리하고, 각 작업이
  `원본 + 이미지 파생물 2개 + 워터마크 16MiB`를 다운로드 전에 선예약합니다. 파생 파일은
  이미지 hard cap 초과 시 업로드 전에 거부하며 systemd `LimitFSIZE=6G`가 파일별 안전망입니다.
- 운영 확인: 기본 unit은 `ProtectSystem=strict`와 제한된 `ReadWritePaths`를 사용합니다. 실제
  배포 filesystem의 여유 공간은 12GiB 예약 상한보다 크게 잡고 별도 volume quota를 권장합니다.
