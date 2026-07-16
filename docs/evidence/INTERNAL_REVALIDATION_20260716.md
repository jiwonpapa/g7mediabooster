# Spec 1.1 내부 인수 게이트 재검증

- 실행일: 2026-07-16
- 기준: 이 문서를 포함하는 `main` commit의 실행 가능한 코드와 하네스
- 결과: 외부 자격증명이 필요 없는 요구사항 1~17 인수 게이트 PASS
- 제외: 실제 R2/Lightsail, 실 provider 보존 삭제, G7 upstream 정식 commit

## 실행 결과

| 게이트 | 현재 결과 |
|---|---|
| `cargo xtask load100` | 100/100 Ready, lease 10/10 복구, dead-letter 0, 13.12 jobs/s, p50/p95/p99 279/352/785ms, peak RSS 582,976KiB, peak temp disk 9,628KiB |
| `cargo xtask heavy-image` | 25,000×4,000 JPEG PASS, 460ms, peak RSS 11,968KiB |
| `cargo xtask heavy-avif` | 8,000×8,000 64MP AVIF PASS, 200MP AVIF full decode 전 거부, 372ms, peak RSS 1,219,872KiB |
| `cargo xtask cgroup-smoke` | CPU 2 core, memory 2GiB, PID 64, network none, API health 665/665, 100 jobs PASS, cgroup peak 1,782,890,496 bytes, worker peak temp disk 8,588KiB |
| `cargo xtask large-multipart-smoke` | 정확한 5GiB, 32MiB 160 part, 80 part 뒤 API 재기동·재개, duplicate complete 멱등, API RSS +416KiB, Quarantined PASS |
| `cargo xtask g7-policy-smoke` | G7 PHP HMAC revision 1 적용, digest-pinned worker output, revision 2 rollback PASS |
| `cargo xtask database-recovery` | online snapshot, hash, retention, 격리 restore rehearsal PASS |
| `cargo xtask setup-smoke` | offline/deferred storage CUI, secret 분리·멱등 설정 PASS |
| `cargo xtask ci` / `cargo xtask rustdoc` | fmt, check, Clippy `-D warnings`, 전체 Rust test, 문서 경고 0, OpenAPI, setup/preflight/package, bench compile PASS |
| `cargo xtask coverage` | 7,629/9,404 lines, 81.13%, 80% ratchet PASS |
| `cargo xtask supply-chain` | 378 dependency, RustSec 1,160 advisories scan, bans/licenses/sources PASS |
| `scripts/live-storage-preflight-smoke.sh` | 필수값 7개, secret redaction, HTTPS·provider label·R2/Lightsail profile shape guard PASS |
| G7 media contract | 공개 `gnuboard/g7@fcaacad` + patch 6개, 29/29 + parser + 실제 activation, MySQL 115 tests/283 assertions PASS |
| G7 module package | 0.4.3 ZIP 153,244 bytes, SHA-256 `1137edfb…ecac`, 실제 ZipInstallHelper·checkout 무변경 PASS |

`reports/*.json`은 로컬 실행 산출물이므로 Git에 넣지 않습니다. 표에는 secret, bucket 이름,
presigned URL을 기록하지 않았습니다.

## 이번 재검증에서 닫은 공백

1. API startup capability가 cgroup 컨테이너에서 실제 sandbox binary를 찾도록 하네스가
   `G7MB__WORKER__SANDBOX_BINARY`를 절대 경로로 주입합니다.
2. Debian cgroup 이미지에도 AOM decode/encode, libde265, x265 HEIF plugin을 설치합니다.
   필수 AVIF/HEIF capability가 없으면 API는 계속 fail-closed합니다.
3. 100개 부하 하네스가 process-tree RSS뿐 아니라 격리 runtime의 peak temp disk와 1GiB
   상한을 함께 기록합니다. 삭제 중인 job directory와 `du` 샘플 경합은 다음 샘플로 복구합니다.
4. cgroup 보고서에도 제한 안에서 측정한 worker peak temp disk를 포함합니다.
5. worker 설정과 코드 정본에 프로세스당 기본 12GiB 임시 디스크 예약을 추가했습니다.
   작업별 최악치 예약이 불가능한 설정은 시작 시 거부하고, 파생 파일이 이미지 hard cap을
   넘으면 provider 업로드 전에 거부합니다. 최종 수치는 위 재검증 결과에 반영했습니다.

## 완료 경계

내부 단일 노드 v1과 로컬 S3 호환 경로는 현재 코드로 재검증됐습니다. 전체 goal은 아직
완료가 아닙니다. 다음 세 증거가 추가될 때까지 공식 기능 목록의 withheld 상태를 유지합니다.

- R2와 Lightsail 각각의 실계정 protocol conformance
- 실 provider 보존 만료 삭제 후 관리 prefix object count 0
- 현재 G7 media contract 변경의 upstream 정식 commit/배포 기준 확정
