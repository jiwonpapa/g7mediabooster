# 부트스트랩 완료 보고서

- 기준일: 2026-07-15
- 범위: Git, Rust 워크스페이스, 기술 스펙, 개발 헌법, 필수 크레이트와 품질 하네스
- 결론: 기반 구성 완료. 업로드부터 검증·기본 썸네일 게시까지 worker 수직 경로를 구현했습니다.

## 완료 항목

| 항목 | 결과 | 증거 |
|---|---|---|
| Git | PASS | `main`, `origin=https://github.com/jiwonpapa/g7mediabooster.git` |
| Rust | PASS | Rust 2024, toolchain 1.96.0, resolver 3 |
| 워크스페이스 | PASS | API, worker, sandbox, domain/application/adapters, xtask 구성 |
| 스펙·헌법 | PASS | `SPEC.md`, `DEVELOPMENT_CONSTITUTION.md`, ADR·보안·개발 문서 |
| 기본·전체 feature 빌드 | PASS | `cargo xtask ci` |
| fmt/clippy/rustdoc | PASS | 경고 0 |
| 테스트 | PASS | 공통 Rust 79개 + Linux seccomp 1개 통과, 실패 0 |
| OpenAPI drift | PASS | 생성 계약과 저장본 일치 |
| 커버리지 | PASS | 5,791/6,842 lines, 84.64%, 하한 80% |
| 공급망 | PASS | RustSec 취약점 0, deny advisories/bans/licenses/sources 통과 |
| API 스모크 | PASS | 실제 프로세스 live/ready, HMAC capabilities, security headers 확인 |
| S3 호환 스모크 | PASS | pinned MinIO presigned PUT·multipart complete/abort·HEAD·download·derivative PUT·private signed GET |
| worker 수직 경로 | PASS | bounded 원본 stream, 진위 검사, digest, master+thumbnail/poster 원자적 Ready 게시 |
| FFmpeg MP4 썸네일 | PASS | 실제 fixture probe·프레임 추출, fast seek 실패 시 bounded exact seek 재시도 |
| OpenH264 영상 폴백 | PASS | FFmpeg 실행 파일 부재 주입, MP4/H.264 첫 frame→libvips JPEG, 160px 제한·임시 PPM 제거 |
| AVIF/HEIF native runtime | PASS | AVIF encode/decode, HEIC signature·decoder probe·JPEG 파생 실제 smoke |
| EXIF/GPS 제거 | PASS | 개인정보 fixture를 libvips로 가공 후 metadata 부재 확인 |
| 이미지·영상 poster 워터마크 | PASS | bounded 합성, SHA-256 pin, revision+digest key, fail-closed worker와 실제 MP4 경로 |
| sandbox egress | PASS | Linux seccomp socket 계열 차단, native child 상속, 컨테이너 EPERM 테스트 |
| G7 제어 업로더 | PASS | PHP 54 tests, TS 17 tests, 100개 bounded 전송 후 Ready polling·native attachment materialization, form state 연결, capability·삭제·private delivery proxy, typecheck·Vite build 통과 |
| 런타임 capability | PASS | 필수 image 6 input/4 output, MP4/H.264 poster, OpenH264 폴백 보고와 API startup fail-closed |
| G7 site policy | PASS | HMAC PUT/GET, Ready asset pin, 단조 revision, job 고정·worker exact revision 적용 |
| lifecycle 삭제·보존 | PASS | HMAC/G7 소유권 삭제 예약, 만료 multipart abort, derivative/raw 정리, SQLite lease·retry·tombstone |
| 100개 worker 부하 | PASS | 4000×3000 JPEG, 동시성 4, Ready 100·master+thumbnail 200, 14.17 jobs/s, p95 274ms, peak RSS 577,584 KiB, lease 10/10 복구 |
| 25,000px heavy image | PASS | heavy semaphore 1, native thread 1, 25,000×4,000 JPEG 481ms, peak RSS 43,472 KiB |
| AVIF decoder memory | PASS | 64MP AVIF peak RSS 1,221,776 KiB, 200MP AVIF full decode 전 정책 거부 |
| tenant fair queue·backpressure | PASS | 영속 round-robin claim, global 1,000/tenant 200 활성 cap, presign 전 차단, 429 계약 |
| Linux cgroup 부하 | PASS | CPU 2 core, memory 2GiB, PID 64, network none, API health 267/267, worker 100/100 |
| G7 게시물 첨부 표시 | PARTIAL | form 자동 연결·module bridge·viewer redirect·보존 삭제 대조·upstream patch 17/17 구현; upstream merge·browser smoke 필요 |

## 준비된 하네스

- 빠른 게이트: `cargo xtask quick`
- 전체 게이트: `cargo xtask ci`
- 공급망: `cargo xtask supply-chain`
- 커버리지: `cargo xtask coverage`
- API·네이티브 스모크: `cargo xtask api-smoke`, `cargo xtask native-smoke`
- G7 어댑터: `cargo xtask g7-adapter`
- 100개 실제 JPEG/RSS/crash 복구: `cargo xtask load100`
- 25,000px heavy-image/RSS: `cargo xtask heavy-image`
- 64MP AVIF/200MP 거부 경계: `cargo xtask heavy-avif`
- Linux cgroup/API 생존/100개 worker: `cargo xtask cgroup-smoke`
- S3 호환 실연결: `cargo xtask storage-conformance`
- 성능·강건성: `cargo xtask bench`, `cargo xtask fuzz`, `cargo xtask miri`
- 재현성: OpenAPI drift, SBOM, native inventory, 고정 Rust toolchain과 lockfile

CI는 고정 버전 nextest와 CycloneDX 도구를 설치해 실제 실행하고, 배포판 libvips 8.15+
빌드 기준 위에서 codec fixture를 검증합니다. 운영 참조 이미지는 libvips 8.18.x와 실제
AVIF/HEIF round-trip을 함께 고정합니다.

## 로컬 네이티브 환경 복구

Homebrew `x265` ABI 216과 구형 `vips 8.18.0_2`의 ABI 215 참조 불일치를 확인해
`vips 8.18.3_1` 패키지 세대로 갱신했습니다. 이후 loader warning 0, AVIF/HEIF 실제
round-trip과 전체 `cargo xtask native-smoke`를 다시 통과했습니다.

## 범위 경계

현재 코드는 batch intent, multipart part/complete/abort, SQLite lease/heartbeat, worker 실행
loop, 원본 검사·master+thumbnail/poster, lifecycle cleanup과 G7 관리자/브라우저 제어 업로더까지 구현됐습니다. 실제 R2/Lightsail
conformance, G7 게시물 첨부 표시, G5, G7 관리자 전용 asset picker browser smoke,
실제 S3/R2·5GiB와
filesystem quota 증거는 구현 완료로 표시하지 않습니다. 멀티노드는 v1 범위가
아니며 각 기능은 `SPEC.md` 완료 정의를 만족한 뒤에만 완료 처리합니다.
