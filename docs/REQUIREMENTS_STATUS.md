# 요구사항 1~17 현재 판정

- 기준일: 2026-07-16
- 판정 원칙: `PASS`만 배포 기능으로 게시하고 `PARTIAL`·`PENDING`은 제한 또는 후속 게이트로 표시
- 외부 환경값이 필요한 R2/Lightsail 실계정 검증은 2026-07-16 인계 항목

| 번호 | 판정 | 현재 증거와 공식 범위 | 남은 게이트 |
|---:|---|---|---|
| 1 이미지 업로드 | PASS | 최대 100개 batch, single/multipart 직접 업로드 | 실 R2/Lightsail profile 재검증 |
| 2 S3 호환 | PARTIAL | MinIO object 작업·multipart client 재생성 재개 PASS, R2/Lightsail profile shape fail-closed 하네스 | R2·Lightsail 각각 실계정 PASS 후 profile별 공식화 |
| 3 동영상 업로드 | PASS | MP4/MOV H.264 직접 multipart·실제 FFprobe/FFmpeg 검사·원 container master·JPEG poster·private delivery 종단 PASS | WebM은 release fixture 전 공식 게시 제외 |
| 4 최신 포맷 | PASS | JPEG/PNG/GIF/WebP/AVIF/HEIC·HEIF runtime decode gate | JPEG XL, 영상 HEVC/AV1은 v1 제외 |
| 5 진짜 파일·보안 | PASS | signature, 실제 decode/ffprobe, digest, hard limit, no-network sandbox | ClamAV·moderation은 선택 hook |
| 6 G5/G7 연동 | PARTIAL | G5 5.6.24 core-free browser E2E PASS. G7 0.4.3 patch는 공개 main `fcaacad`에 clean apply·29/29+parser+실제 activation fail-closed, 기존 DB·browser·보존 gate PASS | G7 patch 정식 upstream commit과 실 provider 보존 만료 삭제 |
| 7 다중 업로드 | PASS | 1~100개 bounded 병렬 처리, 실제 G7 브라우저에서 single PUT와 2-part multipart 동시 첨부 PASS | 실 provider 부하 재측정 |
| 8 대용량 streaming | PASS | 정확히 5GiB를 32MiB 160-part로 직접 전송, 80-part 뒤 API 재기동·재개, complete 2회 멱등, API RSS 증가 416KiB, Quarantined 진입 | 공급자별 처리량·재개 수치는 해당 실계정 profile 검증에서 기록 |
| 9 EXIF 개인정보 | PASS | 이미지 orientation 적용 후 EXIF/GPS/XMP/IPTC 제거 | 영상 metadata 제거는 공식 범위 아님 |
| 10 썸네일 | PASS | eager 1,280px JPEG, 불변 key, 원자적 Ready, private signed GET, 4MiB/60초 weighted manifest cache·singleflight·삭제 guard | 공개 CDN profile은 선택 기능이며 v1 필수 범위 아님 |
| 11 초고해상도 | PASS | 25,000×4,000 JPEG, 100MP, heavy lane/RSS gate | hard cap 초과는 별도 offline tier |
| 12 FFmpeg 폴백 | PASS | FFmpeg 부재 시 MP4/H.264 Rust demux+OpenH264 첫 frame | HEVC/AV1·MOV·WebM 폴백 제외 |
| 13 CPU 제한 | PASS | worker semaphore, native thread 제한, Linux cgroup CPU/RSS/PID gate | 배포 서버별 capacity 재산정 |
| 14 큐 | PASS | 모든 변환 SQLite WAL durable queue, lease·retry·dead-letter·backpressure | 멀티노드는 v1 제외 |
| 15 워터마크 | PASS | 자산 SHA-256 pin, 위치·여백·비율·투명도 제한, revision key, current-admin Ready 자산 선택과 실제 PHP→Rust→worker 출력·rollback 종단 PASS | 배포별 자산·정책 변경 감시는 운영 설정 |
| 16 G7 관리자 설정 | PASS | encrypted secret, signed monotonic policy revision, exact worker revision, 실제 G7 설치·설정 화면과 PHP HMAC PUT/GET→worker 적용 종단 PASS | production secret 주입은 배포 설정 |
| 17 운영 기능 | PASS | lifecycle, 365일 bounded tombstone 보존·purge, byte quota, orphan audit/prune, verified backup·restore, API rate/concurrency limit, queue·worker 단계별 metrics | 실제 배포 Prometheus·alert route 연결은 운영 설정 |

## 이번 마감에서 확정한 Ready 계약

이미지는 metadata를 제거하고 sRGB로 정규화한 최대 8,192px JPEG `master`와 1,280px JPEG
`thumbnail`을 생성합니다. 영상은 검증된 MP4/MOV 원 container `master`와 1,280px JPEG `thumbnail`
(poster)을 생성합니다. 두 object upload와 두 derivative DB 행이 모두 성공한 경우에만 upload를
`Ready`로 바꿉니다. DB 충돌 시 전체 derivative transaction을 rollback해 부분 Ready를 막습니다.

## 배포 게시 금지 항목

upstream patch 미적용 G7, 실 provider 보존 삭제, 멀티노드/PostgreSQL, 임의 동적 리사이즈, 영상 트랜스코딩·metadata 제거,
WebM release 지원, S3 관리 기능 전체, 실계정 conformance 전 R2/Lightsail profile은 공식
지원 기능으로 게시하지 않습니다.
