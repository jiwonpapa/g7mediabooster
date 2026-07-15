# 운영 배포 경계

## 배포 설명에 게시할 공식 애플리케이션 기능

배포 페이지와 릴리스 노트에는 아래 구현·자동 검증 범위만 기능으로 게시합니다.

- 이미지·동영상 최대 100개 다중 선택과 bounded 병렬 직접 업로드
- 작은 파일 single PUT, 큰 파일·영상 S3 호환 multipart 업로드와 abort
- 최대 5GiB 정책 상한에서 PHP/Rust body를 우회하는 32MiB direct multipart
- JPEG, PNG, GIF, WebP, AVIF, HEIC/HEIF 실제 decode 기반 검증
- MP4/H.264 실제 runtime 검사와 FFmpeg JPEG poster
- FFmpeg 실행 불가 시 MP4/H.264 한정 Rust demux + OpenH264 poster 폴백
- 이미지 EXIF/GPS/XMP/IPTC 제거, 방향·sRGB 정규화, 최대 8,192px JPEG master
- 이미지 thumbnail·영상 poster 1,280px JPEG 및 digest/revision 불변 object key
- 이미지 master와 thumbnail/poster가 모두 기록된 뒤에만 `Ready`가 되는 원자적 발행
- G7 소유권 검사 뒤 private master/thumbnail을 5분 presigned GET으로 전달하는 no-store redirect
- 썸네일 bytes가 아닌 immutable manifest만 기본 4MiB·60초로 제한하는 LRU/frequency cache와 singleflight
- 위치·여백·비율·투명도가 제한된 revision 고정 워터마크
- SQLite WAL 단일 노드 durable queue, lease 복구, backpressure, lifecycle cleanup
- 전역·tenant retained-source byte quota와 tombstone 완료 후 용량 반환
- `/v1` token bucket·동시 처리 hard limit과 안정된 `429` backpressure
- queue depth·oldest age·dead-letter와 worker 단계별 Prometheus 메트릭
- provider orphan bounded audit/prune, 검증된 SQLite backup·격리 restore rehearsal
- upload·orphan tombstone 기본 365일 보존과 bounded purge
- G7 관리자 설정·HMAC policy 동기화와 실제 브라우저 single/multipart 직접 업로드
- Ready 결과의 G7 native attachment 연결·표시, 게시글 수정 후 첨부 유지
- G7 권한 확인 뒤 private master/thumbnail 302 redirect와 저장소 200 전달
- G7 비밀글·블라인드글·삭제글 첨부 직접 경로의 작성자·권한자 fail-closed 정책
- G7 관리자 current-admin Ready 이미지 선택기와 워터마크 설정 저장·재로드·rollback
- G7 soft-delete 보존 예약, lease 재검증, 복원 취소와 원격 삭제 시작 뒤 복원 차단
- G5 5.6.24 core-free 플러그인과 게시판 쓰기·업로드 권한 및 회원/세션 소유권 확인
- G5 실제 브라우저 single PUT·2-part multipart 동시 업로드와 native 첨부 2개 표시
- G5 권한 확인 뒤 private master/thumbnail 전달과 MyISAM advisory-lock 첨부 연결

다음 항목은 구현·실환경 검증 전 공식 지원 기능으로 게시하지 않습니다.

- upstream patch `0001`~`0005`를 적용하지 않은 Gnuboard7 배포
- 보존 만료 command→실 provider 객체 삭제 종단 증거
- 멀티노드, PostgreSQL queue, 임의 URL query 기반 동적 리사이즈
- MOV/WebM release fixture, 영상 트랜스코딩·metadata 제거, HEVC/AV1 Rust 폴백
- S3의 ACL, Object Lock, replication, inventory, IAM/STS, SSE-KMS 관리 기능
- 실계정 conformance를 아직 통과하지 않은 R2 또는 Lightsail profile

설치·관리자 설정·user/admin form 주입·disabled fail-safe와 MinIO 기반 실제 single/multipart 전송,
Ready→native attachment create/update 유지, private thumbnail 전달은 0.3.0 격리 browser E2E를
통과했습니다. 0.3.1의 추가 변경인 비밀·블라인드·삭제글 권한 계약과 삭제/복원·보존 lease는
standalone module gate와 실제 G7 DB 호스트 게이트를 통과했습니다. 이 범위는 upstream patch
`0001`~`0005` 적용을 전제로 공식 게시합니다. 0.4.0의 관리자 워터마크 자산 선택·rollback과
작성자·다른 회원·비회원·관리자 권한 매트릭스도 실제 G7 브라우저에서 통과했습니다. 실 provider
보존 만료 삭제는 해당 종단 게이트 통과 전 게시하지 않습니다. G7 PHP HMAC 정책 게시,
Rust worker의 digest 고정 워터마크 출력과 정책 해제 후 원본 출력 복원도 MinIO 종단에서
통과했습니다. 정확한 5GiB direct multipart는 로컬 MinIO에서 통과했으며 이를 R2/Lightsail
실계정 profile 검증으로 대체해 게시하지 않습니다.

G5는 5.6.24 코어 무수정 설치, MySQL 8.4·MyISAM host gate와 MinIO 기반 실제 브라우저
single/multipart 전송, Rust 처리, 게시글 저장·첨부 표시, 비로그인 thumbnail `403`까지
통과한 범위만 공식 게시합니다. 다른 G5 버전과 실 R2/Lightsail profile은 각 게이트 통과 전
지원으로 표시하지 않습니다.

## 공식 object storage 지원 범위

배포판은 S3 전체 관리 API가 아니라 다음 runtime 작업만 공식 지원합니다.

- presigned single PUT
- multipart create, part PUT, complete, abort
- HEAD, bounded worker GET, private derivative presigned GET, worker PutObject, idempotent DeleteObject

R2와 Lightsail은 각각 실계정 conformance를 통과한 profile만 지원으로 표시합니다. ACL,
Object Lock, replication, inventory, IAM/STS, SSE-KMS는 검증 전 지원 기능으로 게시하지 않습니다.
Lightsail 단일 bucket access key를 쓸 때는 같은 private bucket 안의 `raw/`, `media/` prefix로
구성할 수 있지만, 이는 두 버킷 IAM 격리를 제공하지 않습니다.

`systemd` 예시는 API와 worker 전체에 CPU, RSS, process 수 상한을 적용합니다. worker 내부도
동시 작업 수와 libvips/FFmpeg thread 수를 별도로 제한하므로 두 제한을 함께 사용해야 합니다.

설치 전 다음 경로와 전용 사용자를 만듭니다.

- 설정: `/etc/g7mediabooster/g7mb.toml` (`root:g7mediabooster`, `0640`)
- 상태·SQLite·임시파일: `/var/lib/g7mediabooster`
- 실행 파일: `/usr/local/bin/g7mb-api`, `/usr/local/bin/g7mb-worker`
- 자격 증명 없는 실행 파일: `/usr/local/libexec/g7mb-sandbox`

`g7mediabooster-cleanup.timer`는 15분마다 최대 설정 batch만 처리합니다. 동일 oneshot unit은
systemd가 중복 실행하지 않으며, SQLite lease가 수동 실행·강제 종료 후 재선점도 보호합니다.
만료 multipart abort와 private raw/derivative 삭제는 S3/R2 작업이 모두 성공한 뒤에만
upload를 `deleted`로 tombstone 처리합니다.

`g7mediabooster-inventory.timer`는 매일 `raw/`·`media/` bounded inventory를 비파괴 audit합니다.
기본 unit은 orphan을 삭제하지 않습니다. 48시간 이상 반복 관측된 후보의 명시적 prune 절차와
삭제 직전 ownership 재검사는 [provider orphan runbook](../docs/ORPHAN_INVENTORY.md)을 따릅니다.

`g7mediabooster-backup.timer`는 매일 network 없이 일관된 SQLite snapshot과 SHA-256 manifest를
생성하고 기본 14개로 회전합니다. 복원은 [backup·restore runbook](../docs/BACKUP_RESTORE.md)의
read-only 검증·격리 rehearsal·offline cutover를 모두 따라야 합니다.

API `/metrics`와 worker `127.0.0.1:9091/metrics`는 내부 Prometheus만 scrape합니다. 공개
reverse proxy에는 연결하지 않으며 metric·초기 alert 기준은 [운영 관측 runbook](../docs/OPERATIONS.md)을
따릅니다.

worker가 S3/R2에 접근해야 하므로 worker service 자체는 네트워크를 사용합니다. Linux
`g7mb-sandbox`는 명령 처리 전에 seccomp-BPF를 모든 runtime thread에 설치하고 socket,
connect, bind, listen, send/recv 계열 syscall을 `EPERM`으로 거부합니다. 필터는 FFmpeg와
FFprobe child에도 상속됩니다. 환경 비우기·local protocol allowlist·worker cgroup과 함께
사용하며 별도 network namespace는 추가 방어층이지 v1 필수 차단선은 아닙니다.
