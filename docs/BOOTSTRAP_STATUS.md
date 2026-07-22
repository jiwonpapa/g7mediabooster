# 부트스트랩 완료 보고서

- 기준일: 2026-07-16
- 범위: Git, Rust 워크스페이스, 기술 스펙, 개발 헌법, 필수 크레이트와 품질 하네스
- 결론: 내부 v1, 로컬 정확한 5GiB 직접 multipart, G5/G7 격리 브라우저와 G7 정책 종단 완료. 외부 R2/Lightsail, G7 upstream과 실 provider 삭제 게이트만 남았습니다.

## 완료 항목

| 항목 | 결과 | 증거 |
|---|---|---|
| Git | PASS | `main`, `origin=https://github.com/jiwonpapa/g7mediabooster.git` |
| Rust | PASS | Rust 2024, toolchain 1.96.0, resolver 3 |
| 워크스페이스 | PASS | API, worker, sandbox, domain/application/adapters, xtask 구성 |
| 스펙·헌법 | PASS | `SPEC.md`, `DEVELOPMENT_CONSTITUTION.md`, ADR·보안·개발 문서 |
| 기본·전체 feature 빌드 | PASS | `cargo xtask ci` |
| fmt/clippy/rustdoc | PASS | 경고 0 |
| Linux 통합 설치 | PASS | payload SHA-256, Ubuntu native runtime, service user·경로, 단일 systemd target, setup, API ready, MinIO storage doctor를 CI에서 실제 실행 |
| 테스트 | PASS | 공통 Rust 101개 + Linux seccomp 1개 통과, 실패 0 |
| OpenAPI drift | PASS | 생성 계약과 저장본 일치 |
| 커버리지 | PASS | 7,350/8,622 lines, 85.25%, 하한 80% |
| 공급망 | PASS | RustSec 취약점 0, deny advisories/bans/licenses/sources 통과 |
| API 스모크 | PASS | 실제 프로세스 live/ready, HMAC capabilities, security headers 확인 |
| S3 호환 스모크 | PASS | pinned MinIO presigned PUT·multipart complete/abort·HEAD·download·derivative PUT·private signed GET |
| worker 수직 경로 | PASS | bounded 원본 stream, 진위 검사, digest, master+thumbnail/poster 원자적 Ready 게시 |
| FFmpeg MP4/MOV 썸네일 | PASS | 실제 fixture probe·원 container master·프레임 추출, fast seek 실패 시 bounded exact seek 재시도 |
| OpenH264 영상 폴백 | PASS | FFmpeg 실행 파일 부재 주입, MP4/H.264 첫 frame→libvips JPEG, 160px 제한·임시 PPM 제거 |
| AVIF/HEIF native runtime | PASS | AVIF encode/decode, HEIC signature·decoder probe·JPEG 파생 실제 smoke |
| EXIF/GPS 제거 | PASS | 개인정보 fixture를 libvips로 가공 후 metadata 부재 확인 |
| 이미지·영상 poster 워터마크 | PASS | bounded 합성, SHA-256 pin, revision+digest key, fail-closed worker와 실제 MP4 경로 |
| sandbox egress | PASS | Linux seccomp socket 계열 차단, native child 상속, 컨테이너 EPERM 테스트 |
| G7 제어 업로더 | PASS | PHP 57 tests/153 assertions, TS 21 tests, 100개 bounded 전송 후 Ready polling·native attachment materialization, MP4/MOV 계약, form state 연결, capability·삭제·private delivery proxy·관리자 asset picker, typecheck·Vite build 통과 |
| G7 module 배포 산출물 | PASS | 0.4.3 ZIP 153,244 bytes, SHA-256 `1137edfb…ecac`; 공개 upstream activation·재현성·실제 G7 ZipInstallHelper·dev dependency/test 제외·manifest fail-closed |
| 런타임 capability | PASS | 필수 image 6 input/4 output, MP4/MOV H.264 poster, OpenH264 폴백 보고와 API startup fail-closed |
| G7 site policy | PASS | HMAC PUT/GET, Ready asset pin, 단조 revision, job 고정·worker exact revision 적용 |
| G7 policy 종단 | PASS | 실제 PHP HMAC client→Rust API revision 1→worker 워터마크 출력→revision 2 해제·원본 출력 복원 |
| 정확한 5GiB 직접 multipart | PASS | 32MiB 160-part, 80-part 뒤 API 재기동·재개, complete 2회 멱등, HEAD 길이·Quarantined 진입, 현재 재검증 API RSS 증가 416KiB |
| lifecycle 삭제·보존 | PASS | HMAC/G7 소유권 삭제 예약, 만료 multipart abort, derivative/raw 정리, SQLite lease·retry·tombstone |
| 운영 hardening | PASS | `/v1` rate·동시 처리 제한, O(1) queue/upload/orphan counter, worker 단계별 metrics, 기본 365일 bounded tombstone purge |
| 저장 용량 quota | PASS | 전역·tenant retained-source byte quota, presign 전 차단, SQLite 원자 재검사, tombstone 후 반환 |
| orphan inventory | PASS | bounded ListObjectsV2, durable cursor, 48시간 grace, audit 기본, ownership 재검사 prune |
| SQLite 복구 | PASS | online snapshot, SHA-256 manifest, 14개 회전, read-only 검증, 격리 writable restore rehearsal |
| 썸네일 manifest cache | PASS | Moka frequency admission/LRU, 기본 4MiB·60초 TTL, same-upload singleflight, mutable 삭제 guard·invalidation, hit/miss·weight metric |
| 100개 worker 부하 | PASS | 4000×3000 JPEG, 동시성 4, Ready 100·master+thumbnail 200, 14.17 jobs/s, p95 274ms, peak RSS 577,584 KiB, lease 10/10 복구 |
| 25,000px heavy image | PASS | heavy semaphore 1, native thread 1, 25,000×4,000 JPEG 481ms, peak RSS 43,472 KiB |
| AVIF decoder memory | PASS | 64MP AVIF peak RSS 1,221,776 KiB, 200MP AVIF full decode 전 정책 거부 |
| tenant fair queue·backpressure | PASS | 영속 round-robin claim, global 1,000/tenant 200 활성 cap, presign 전 차단, 429 계약 |
| Linux cgroup 부하 | PASS | CPU 2 core, memory 2GiB, PID 64, network none, API health 665/665, worker 100/100, peak 1,782,890,496 bytes |
| G7 게시물 첨부 표시 | PARTIAL | 공개 G7 `fcaacad` 기준 patch 6개 clean apply·29/29+parser+activation, 기존 개발 checkout DB·browser·권한·asset picker·보존 gate PASS; upstream commit·실 provider 삭제 필요 |
| G5 게시물 첨부 표시 | PASS | G5 5.6.24 계약 21/21, PHP 17/31, TS 5, MySQL 8.4·MyISAM 11/11, 실제 browser single PUT+2-part multipart→첨부 2개·private thumbnail PASS |

## 준비된 하네스

- 빠른 게이트: `cargo xtask quick`
- 전체 게이트: `cargo xtask ci`
- 공급망: `cargo xtask supply-chain`
- 커버리지: `cargo xtask coverage`
- API·네이티브 스모크: `cargo xtask api-smoke`, `cargo xtask native-smoke`
- G7 어댑터: `cargo xtask g7-adapter`
- G7 관리자 설치 ZIP·수동 tar.gz·각 SHA-256: `cargo xtask g7-module-package`
- G5 어댑터: `cargo xtask g5-adapter`, `cargo xtask g5-host-smoke`
- G7 권한·삭제/복원·보존 DB: `scripts/g7-host-security-gate.sh /path/to/gnuboard7`
- 100개 실제 JPEG/RSS/crash 복구: `cargo xtask load100`
- 25,000px heavy-image/RSS: `cargo xtask heavy-image`
- 64MP AVIF/200MP 거부 경계: `cargo xtask heavy-avif`
- Linux cgroup/API 생존/100개 worker: `cargo xtask cgroup-smoke`
- S3 호환 실연결: `cargo xtask storage-conformance`
- 외부 S3 환경·HTTPS·secret redaction 사전검사: `bash scripts/live-storage-preflight-smoke.sh`
- G7 정책 종단: `cargo xtask g7-policy-smoke`
- 로컬 정확한 5GiB/API RSS: `cargo xtask large-multipart-smoke`
- 성능·강건성: `cargo xtask bench`, `cargo xtask fuzz`, `cargo xtask miri`
- 재현성: OpenAPI drift, SBOM, native inventory, 고정 Rust toolchain과 lockfile

CI는 동일 Rust test surface를 한 번만 실행하고 adapter·coverage를 병렬화하며 CycloneDX 도구를
실행합니다. 배포판 libvips 8.15+
빌드 기준 위에서 codec fixture를 검증합니다. 운영 참조 이미지는 libvips 8.18.x와 실제
AVIF/HEIF round-trip을 함께 고정합니다.

## 로컬 네이티브 환경 복구

Homebrew `x265` ABI 216과 구형 `vips 8.18.0_2`의 ABI 215 참조 불일치를 확인해
`vips 8.18.3_1` 패키지 세대로 갱신했습니다. 이후 loader warning 0, AVIF/HEIF 실제
round-trip과 전체 `cargo xtask native-smoke`를 다시 통과했습니다.

## 범위 경계

현재 코드는 batch intent, multipart part/complete/abort, SQLite lease/heartbeat, worker 실행
loop, 원본 검사·master+thumbnail/poster, lifecycle cleanup과 G5/G7 브라우저 직접 업로더·첨부 표시까지 구현됐습니다. 실제 R2/Lightsail
conformance, G7 upstream 반영과 실 provider 보존 삭제는 구현 완료로 표시하지 않습니다.
정확한 로컬 5GiB 직접 multipart와 API RSS 독립성은 통과했지만 이 결과를 R2/Lightsail
profile 실계정 증거로 대체하지 않습니다. 멀티노드는 v1 범위가
아니며 각 기능은 `SPEC.md` 완료 정의를 만족한 뒤에만 완료 처리합니다.
