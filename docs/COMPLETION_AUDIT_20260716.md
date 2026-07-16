# Spec 1.1 요구사항 1~17 완료 감사

- 감사일: 2026-07-16
- 기준: `SPEC.md` 1.1.0과 최초 요구사항 1~17
- 판정 원칙: 실제 구현 범위와 같은 크기의 증거만 PASS로 인정

| 번호 | 판정 | 권위 증거 | 완료를 막는 잔여 조건 |
|---:|---|---|---|
| 1 이미지 업로드 | PASS | `full-stack-smoke`, G5/G7 browser E2E | 없음 |
| 2 S3 호환 업로드 | PARTIAL | pinned MinIO single/multipart/abort/HEAD/GET/PUT/Delete conformance, 외부 credential 하네스 | R2·Lightsail 각각 실계정 profile PASS |
| 3 동영상 업로드 | PASS | 실제 MP4/MOV H.264 multipart→FFprobe/FFmpeg→master/poster→private delivery | WebM은 공식 범위 밖 |
| 4 최신 이미지 포맷 | PASS | runtime capability의 JPEG/PNG/GIF/WebP/AVIF/HEIF decode, 4 output 실제 fixture | JPEG XL은 공식 범위 밖 |
| 5 진짜 파일·보안 | PASS | signature+decoder/FFprobe, 위장 PHP 거부, no-network sandbox, hard limits | 선택 ClamAV/moderation hook |
| 6 G5/G7 연동 | PARTIAL | G5 5.6.24 browser E2E, G7 patch 5개·계약 28/28·browser/DB gate | G7 upstream 정식 반영, 실 provider 보존 삭제 |
| 7 다중 업로드 | PASS | browser 100개, 전체 연결 8개, 파일별 part 4개, 원자 batch 예약 | 공급자별 처리량 재측정 |
| 8 대용량 streaming | PASS | 정확한 5GiB 160-part, API 재기동·재개, complete 2회 멱등, API RSS +416KiB | 공급자별 외부 재실행 |
| 9 EXIF 개인정보 | PASS | private EXIF/GPS/XMP/IPTC fixture 제거 후 재검사 | 영상 metadata 제거는 공식 범위 밖 |
| 10 썸네일 | PASS | eager 1280px, immutable key, atomic Ready, 4MiB/60초 weighted manifest cache·singleflight | 공개 CDN profile은 선택 |
| 11 초고해상도 | PASS | 25,000×4,000 JPEG/RSS, 64MP AVIF/RSS, 200MP AVIF full decode 전 거부 | hard cap 초과 offline tier |
| 12 FFmpeg 폴백 | PASS | FFmpeg spawn 실패 주입 MP4/H.264 Rust demux+OpenH264 first frame | MOV/HEVC/AV1 fallback 제외 |
| 13 CPU 제한 | PASS | semaphore/native thread limit, Linux cgroup CPU 2·memory 2GiB·PID 64 | 서버별 capacity 산정 |
| 14 큐 | PASS | SQLite WAL lease/heartbeat/retry/dead-letter/fair queue/backpressure·crash recovery | 멀티노드/PostgreSQL은 v1 제외 |
| 15 워터마크 | PASS | G7 Ready asset→revision 1→digest-pinned worker output→revision 2 rollback | 운영 변경 감시 설정 |
| 16 G7 관리자 설정 | PASS | encrypted secret, current-admin asset picker, signed monotonic policy, exact worker revision | production secret 주입 |
| 17 운영 기능 | PASS | lifecycle/quota/orphan audit-prune/backup-restore/API admission/metrics/tombstone purge | production Prometheus·alert 연결 |

## 현재 결론

내부 단일 노드 v1 구현과 로컬 인수 게이트는 완료됐습니다. 그러나 전체 목표 완료는 아직
증명되지 않았습니다. 요구사항 2의 R2/Lightsail 실계정 profile과 요구사항 6의 G7 upstream
정식 반영·실 provider 보존 삭제가 남아 있으므로 goal은 계속 활성 상태로 유지합니다.

외부 값이 준비되면 `docs/EXTERNAL_VALIDATION_20260716.md` 순서로 provider별 conformance,
5GiB 중단·재개, 보존 만료 삭제 후 object count 0을 확인하고 이 표의 PARTIAL만 갱신합니다.

## 현재 G7 upstream checkout 상태

현재 개발 checkout은 `main` HEAD `c275b41b`이며 `origin/main`보다 11 commit 앞입니다.
media contract는 28/28과 PHP·JSON parser 검사를 통과하지만, patch 관련 경로가 다른 사용자
작업과 함께 아직 uncommitted 상태입니다. 이 저장소의 검증과 패키징은 read-only로 수행했고
실행 전후 checkout dirty-state hash가 같았습니다.

production DB와 분리한 임시 MySQL 8.4와 일회용 `.env.testing`으로 현재 checkout의 첨부·권한
58 tests/90 assertions, 전체 layout extension 57/193, 첨부 수 동기화 1/2를 통과했습니다.
이 과정에서 `prepend`가 삽입한 노드를 같은 순회에서 다시 처리하던 무한 반복을 발견해 원본
인덱스 역순 처리로 수정했고 전체 layout 회귀로 확인했습니다. 테스트 후 컨테이너와
`.env.testing`은 제거했습니다. 같은 checkout에 module 0.4.0을 임시 bundled 배치해 host
보안·watermark catalog·보존 7 tests/38 assertions도 통과한 뒤 모듈 사본까지 제거했습니다.
정식 upstream 변경으로 commit되기 전까지는 해당 checkout을
공식 지원 대상으로 게시하지 않습니다.

G7 module 0.4.0 배포 압축본은 module commit `b118240`에 고정해 122,811 bytes,
SHA-256 `53b4dc1c…d026`으로 재현했습니다. 배포 설명 자동화는
`deploy/official-features-v1.json`의 `publishable_features`만 사용하며, R2/Lightsail 실계정과
실 provider 보존 삭제는 `withheld_until_verified`에 남겨둡니다.

현재 관리자 설치판 0.4.1은 module commit `f2594aac` 기준 ZIP 149,966 bytes,
SHA-256 `a667232a…a6a3`로 두 번 재현했고, G7 checkout `c275b41b`의 실제
`ZipInstallHelper`와 checkout 무변경 검사를 통과했습니다.

## 설치·비밀값 경로 보강

`g7mbctl setup` 대화형 CUI와 비대화형 file-input 모드를 추가했습니다. R2 Account ID endpoint
파생, 공급자 profile, bucket/CORS bootstrap, bounded single/multipart canary, 설정 충돌 사전검사,
파일별 원자 교체와 재실행 멱등성을 구현했습니다. 일반 TOML은 secret file 경로만 보유하고,
systemd는 세 root-only source를 `LoadCredential=`로 서비스별 격리 경로에 전달합니다.

외부 값이 없는 설치는 `--defer-storage`로 명시하며 `PENDING`을 출력합니다. 이 경우 코드 구현은
완료지만 R2/Lightsail 공식 지원 증거는 아니므로 요구사항 2의 `PARTIAL` 판정은 유지합니다.

실 provider 하네스는 protocol 삭제 뒤 `HEAD NotFound`와 실제 SQLite lifecycle의 Ready 사용자
삭제·rejected 보존 만료→derivative/raw 삭제→tombstone 2건→object count 0까지 자동화했습니다.
현재는 credential-free MinIO 회귀까지만 PASS이며 R2/Lightsail 실계정 판정은 그대로 남습니다.

## 현재 HEAD 내부 재검증

외부 자격증명 없이 실행 가능한 100개 queue, 25,000px JPEG, 64MP AVIF와 200MP 선차단,
Linux cgroup, 정확한 5GiB, G7 watermark policy, backup/restore, setup CUI를 현재 코드로 다시
통과했습니다. 부하 보고서에는 Spec 완료 정의에 맞춰 peak temp disk도 포함합니다. 수치와
하네스 보강 내역은
[`docs/evidence/INTERNAL_REVALIDATION_20260716.md`](evidence/INTERNAL_REVALIDATION_20260716.md)에
고정했습니다. 요구사항 2와 6의 외부 잔여 조건은 바뀌지 않습니다.
