# G7MediaBooster 개발 헌법

- 헌법 버전: 1.3.0
- 발효일: 2026-07-22
- 적용 범위: Rust, PHP 어댑터, CI, 배포 이미지, 운영 스크립트 전체

이 문서의 `MUST`, `MUST NOT`은 권고가 아니라 병합 조건입니다. 예외는 만료일이 있는
ADR, 위험 설명, 보완 통제, 책임자 승인을 모두 가져야 합니다.

## 제1조. 제품 경계를 먼저 지킨다

1. domain은 순수 정책만 소유하며 HTTP, SQL, AWS SDK, PHP에 의존하지 않습니다.
2. application은 유스케이스와 port를 소유합니다.
3. adapter와 app은 port를 구현하고 조립할 뿐 비즈니스 규칙을 복제하지 않습니다.
4. 의존 방향은 `domain <- application <- adapters/apps` 한 방향입니다.
5. G5/G7 차이는 얇은 별도 PHP 어댑터에서만 처리합니다.

## 제2조. Safe Rust가 기본이다

1. 첫 번째 코드 전체는 `unsafe_code = forbid`를 유지합니다.
2. `unwrap`, `expect`, `panic`, `todo`, `unimplemented`를 제품과 테스트 코드에서 금지합니다.
3. library는 `thiserror` 기반 typed error를 사용하고 `anyhow`는 binary 조립 경계에서만 씁니다.
4. 오류를 무시하거나 문자열 성공값으로 바꾸지 않습니다.
5. first-party unsafe FFI가 꼭 필요해지면 별도 감사 crate와 헌법 개정 ADR이 선행되어야 합니다.

커뮤니티 libvips binding 내부 unsafe는 sandbox 프로세스 안에만 존재하며 Rust의 안전 보증
범위로 주장하지 않습니다.

## 제3조. 작고 명시적인 의존성을 사용한다

1. workspace가 버전과 MSRV를 한 곳에서 관리하고 `Cargo.lock`을 커밋합니다.
2. 기본 feature와 `full` feature는 필요 근거 없이 켜지 않습니다.
3. 새 runtime dependency는 유지보수 상태, MSRV, 라이선스, advisory, 대체재를 ADR/PR에 기록합니다.
4. wildcard와 출처 불명 registry/git dependency를 금지합니다.
5. 순수 Rust로 충분한 구간에 새로운 C/C++ dependency를 추가하지 않습니다.
6. 저장소 명령 라우터는 제품 crate에 직접 의존하지 않습니다. 코드 생성처럼 제품 타입이 필요한
   명령만 해당 제품의 전용 binary에 위임합니다.

## 제4조. 비동기와 자원은 반드시 bounded다

1. 무제한 channel, queue, spawn, retry, buffer를 금지합니다.
2. 모든 외부 I/O는 timeout과 cancellation 경로를 가집니다.
3. retry는 멱등 호출에만 제한 횟수, exponential backoff, jitter로 적용합니다.
4. 파일 전체를 메모리에 읽지 않고 스트리밍하거나 임시 파일을 사용합니다.
5. Tokio blocking pool과 libvips 내부 pool의 중첩 병렬화를 예산에 포함합니다.
6. timeout된 child는 kill한 뒤 wait하여 zombie와 고아 프로세스가 없음을 테스트합니다.

## 제5조. 공격자 입력은 격리 후 신뢰한다

1. 확장자와 `Content-Type`만으로 포맷을 승인하지 않습니다.
2. 크기, dimensions, 총 픽셀, 프레임, duration, stream 수를 decode 전에 가능한 만큼 제한합니다.
3. API 프로세스는 libvips/FFmpeg/ImageMagick을 직접 실행하지 않습니다.
4. sandbox는 object credential, 외부 네트워크, 영구 쓰기 경로를 갖지 않습니다.
5. 외부 URL fetch, SVG/PDF/PS, 임의 native option을 기본 거부합니다.
6. metadata는 최소 공개 원칙으로 제거하고 필요한 색상 profile만 새로 씁니다.

## 제6조. 인증·비밀·권한은 fail-closed다

1. PHP 요청은 key ID, timestamp, nonce, method, path, body hash를 HMAC-SHA256으로 서명합니다.
2. signature는 constant-time 검증하고 nonce는 durable store에서 원자적으로 소비합니다.
3. object key와 callback destination은 서버가 생성하거나 관리자 allowlist에서만 선택합니다.
4. worker IAM은 raw read와 derivative write 최소 권한만 가집니다.
5. secret, 인증 헤더, cookie, presigned URL, 원본 파일명을 로그·panic·metric label에 넣지 않습니다.
6. 운영 설정 누락은 안전한 fallback이 없으면 시작 실패로 처리합니다.

## 제7조. 코드와 코드에서 생성된 계약이 최종 정본이다

1. 실행 가능한 소스 코드와 코드에서 생성·검증된 계약이 최종 정본입니다.
2. Rust 공개 API의 의미, 불변조건, 오류, 보안·자원 제한은 해당 항목의 rustdoc에 기록합니다.
3. 모든 first-party crate는 workspace lint를 상속하며 `missing_docs`와 `rustdoc::all`을 deny합니다.
4. 공개 항목 이름을 반복하는 빈 문서나 `TODO` 문서는 문서화로 인정하지 않습니다.
5. HTTP wire 계약은 Rust 코드에서 생성한 OpenAPI, 영속 상태는 migration, G5/G7 연동은
   어댑터 소스와 contract test가 정본입니다.
6. Markdown 스펙·도식·보고서는 설명용 snapshot이며 코드와 충돌할 때 코드를 따르고 같은
   변경에서 문서 drift를 제거합니다. 문서가 코드 동작을 덮어쓸 수 없습니다.
7. 상태 전이는 명시적 state machine과 compare-and-set으로만 구현합니다.
8. 새 endpoint, 오류 code, 상태, preset 변경은 G5/G7 contract test와 함께 제출합니다.
9. backward-incompatible 변경은 versioned endpoint 또는 migration 기간을 가집니다.
10. provider 구현만 바꾸고 consumer 어댑터를 추측으로 남기지 않습니다.

## 제8조. 테스트는 계층별 실제 위험을 다룬다

1. domain 경계값은 unit/property test로 검증합니다.
2. HTTP와 OpenAPI는 in-process contract test로 검증합니다.
3. S3/R2는 mock과 실제 선택형 smoke를 모두 둡니다.
4. libvips/FFmpeg는 실제 JPEG/WebP/AVIF/MP4 fixture 왕복으로 검증합니다.
5. 손상·절단·폭탄성 corpus, 중복 finalize, timeout, worker crash, lease expiry를 테스트합니다.
6. parser는 fuzz, 순수 Rust 정책은 가능한 범위에서 Miri 대상입니다.
7. 불안정 테스트를 retry로 숨기지 않습니다.

## 제9조. 관측 없이 최적화하지 않는다

1. request/job/upload ID를 연결하되 tenant/user/파일명은 metric label로 쓰지 않습니다.
2. queue wait, download, decode, transform, upload 시간을 분리 계측합니다.
3. 성공률, reject code, retry, timeout, peak RSS, temp disk, child 잔존 수를 기록합니다.
4. 성능 주장은 기준 장비, native 버전, fixture hash, 명령, p50/p95/p99와 함께 남깁니다.
5. 직전 승인 기준 대비 p95 또는 RSS 10% 초과 회귀는 release blocker입니다.

## 제10조. 변경은 작고 되돌릴 수 있어야 한다

1. 한 PR은 하나의 응집된 변경과 검증 증거를 가집니다.
2. schema 변경은 forward-compatible migration과 rollback/roll-forward 계획을 가집니다.
3. feature flag는 소유자, 기본값, 제거 조건, 만료일을 가집니다.
4. 운영 위험이 있는 변경은 canary와 kill switch를 가집니다.
5. 생성물과 문서는 코드와 같은 PR에서 갱신합니다.
6. 줄 수만 줄이기 위한 crate, service, package 분할을 금지합니다. 새 경계는 보안 격리 또는
   측정된 컴파일·캐시·변경 독립성 이득이 있어야 합니다.
7. 신규 수기 source 파일은 500줄 이하로 유지하고 기존 초과 파일은 현재 크기를 상한으로
   ratchet합니다. 초과 자체는 자동 분할 사유가 아니며 응집도와 변경 빈도를 함께 봅니다.
8. Bash와 Python의 파일별 상한은 현재 통과값에서 자동으로 늘릴 수 없습니다. Bash 정본을
   Python으로 이관할 때만 같은 변경에서 Bash 감소량이 Python 증가량보다 크고 두 언어 합계가
   감소해야 합니다. 이관 직후 언어별·합계 상한을 새 통과값으로 다시 고정합니다.

## 제11조. 품질 게이트를 우회하지 않는다

병합 전 필수 게이트는 다음과 같습니다.

같은 feature·target·test surface를 한 workflow에서 중복 실행하지 않습니다. `clippy`와 동일한
surface의 별도 `check`, `cargo test` 뒤의 동일 nextest 재실행을 금지합니다. coverage 계측,
native fixture, provider contract처럼 서로 다른 증거는 별도 병렬 job으로 실행할 수 있습니다.

- `cargo fmt --check`
- default와 전체 feature의 `cargo clippy -D warnings` 타입·lint 검사
- unit/integration/doc/contract tests
- `cargo xtask rustdoc`: 공개 문서 누락과 모든 rustdoc lint 0
- OpenAPI drift 0
- cargo-audit와 cargo-deny
- native capability/fixture smoke
- benchmark compile과 coverage 기준

게이트 자체를 녹색으로 만든 사실은 제품 기능이 완료됐다는 증거가 아닙니다. 실제 업로드,
native 처리, 저장소, PHP 연결 smoke를 별도로 남깁니다.

## 제12조. 헌법 변경

헌법 변경은 다음을 모두 필요로 합니다.

1. 변경 이유와 버려지는 원칙을 설명한 ADR
2. 보안·성능·호환성 영향
3. 대체 통제와 migration
4. 헌법 semantic version 변경
5. CI/하네스의 동시 갱신
