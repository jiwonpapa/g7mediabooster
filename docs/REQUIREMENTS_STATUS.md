# 요구사항 1~17 현재 판정

- 기준일: 2026-07-15
- 판정 원칙: `PASS`만 배포 기능으로 게시하고 `PARTIAL`·`PENDING`은 제한 또는 후속 게이트로 표시
- 외부 환경값이 필요한 R2/Lightsail 실계정 검증은 2026-07-16 인계 항목

| 번호 | 판정 | 현재 증거와 공식 범위 | 남은 게이트 |
|---:|---|---|---|
| 1 이미지 업로드 | PASS | 최대 100개 batch, single/multipart 직접 업로드 | 실 R2/Lightsail profile 재검증 |
| 2 S3 호환 | PARTIAL | MinIO에서 필요한 object 작업 conformance PASS | R2·Lightsail 각각 실계정 PASS 후 profile별 공식화 |
| 3 동영상 업로드 | PASS | MP4/H.264 직접 업로드·검사·master·poster | MOV/WebM은 release fixture 전 공식 게시 제외 |
| 4 최신 포맷 | PASS | JPEG/PNG/GIF/WebP/AVIF/HEIC·HEIF runtime decode gate | JPEG XL, 영상 HEVC/AV1은 v1 제외 |
| 5 진짜 파일·보안 | PASS | signature, 실제 decode/ffprobe, digest, hard limit, no-network sandbox | ClamAV·moderation은 선택 hook |
| 6 G7 연동 | PARTIAL | 관리자 설정·HMAC·100개 uploader, form 자동 연결, Ready→native attachment 원자적 bridge, 게시글 권한 private delivery, 보존 삭제 대조와 upstream 계약 patch 구현 | patch 정식 반영, 사용자/권한별 create/update/삭제/복원 browser smoke |
| 7 다중 업로드 | PASS | 1~100개, 파일·part·전체 연결 bounded 병렬 처리 | 실 브라우저/provider 부하 재측정 |
| 8 대용량 streaming | PARTIAL | PHP/Rust body를 거치지 않는 object storage 직접 multipart | 실계정 5GiB 중단·재개·RSS 증거 |
| 9 EXIF 개인정보 | PASS | 이미지 orientation 적용 후 EXIF/GPS/XMP/IPTC 제거 | 영상 metadata 제거는 공식 범위 아님 |
| 10 썸네일 | PARTIAL | eager 1,280px JPEG, 불변 key, 원자적 Ready, owner 기반 private signed GET | public CDN profile, manifest cache·singleflight 운영 구현 |
| 11 초고해상도 | PASS | 25,000×4,000 JPEG, 100MP, heavy lane/RSS gate | hard cap 초과는 별도 offline tier |
| 12 FFmpeg 폴백 | PASS | FFmpeg 부재 시 MP4/H.264 Rust demux+OpenH264 첫 frame | HEVC/AV1·MOV·WebM 폴백 제외 |
| 13 CPU 제한 | PASS | worker semaphore, native thread 제한, Linux cgroup CPU/RSS/PID gate | 배포 서버별 capacity 재산정 |
| 14 큐 | PASS | 모든 변환 SQLite WAL durable queue, lease·retry·dead-letter·backpressure | 멀티노드는 v1 제외 |
| 15 워터마크 | PASS | 자산 SHA-256 pin, 위치·여백·비율·투명도 제한, revision key | 관리자 전용 asset picker browser smoke |
| 16 G7 관리자 설정 | PASS | encrypted secret, signed monotonic policy revision, exact worker revision | 실제 G7 설치 browser smoke |
| 17 운영 기능 | PARTIAL | Rust lifecycle과 G7 soft-delete 보존 대조, multipart abort·tombstone·공정 queue·공급망 gate | provider orphan inventory, tenant byte quota, 백업·복원, 운영 관측 |

## 이번 마감에서 확정한 Ready 계약

이미지는 metadata를 제거하고 sRGB로 정규화한 최대 8,192px JPEG `master`와 1,280px JPEG
`thumbnail`을 생성합니다. 영상은 검증된 MP4 원 container `master`와 1,280px JPEG `thumbnail`
(poster)을 생성합니다. 두 object upload와 두 derivative DB 행이 모두 성공한 경우에만 upload를
`Ready`로 바꿉니다. DB 충돌 시 전체 derivative transaction을 rollback해 부분 Ready를 막습니다.

## 배포 게시 금지 항목

G7 upstream·browser 미검증 상태의 게시글 form 자동 연결·첨부 표시·삭제/복원, 멀티노드/PostgreSQL, 임의 동적 리사이즈, 영상 트랜스코딩·metadata 제거,
MOV/WebM release 지원, S3 관리 기능 전체, 실계정 conformance 전 R2/Lightsail profile은 공식
지원 기능으로 게시하지 않습니다.
