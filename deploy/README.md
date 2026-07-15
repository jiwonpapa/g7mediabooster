# 운영 배포 경계

## 배포 설명에 게시할 공식 애플리케이션 기능

배포 페이지와 릴리스 노트에는 아래 구현·자동 검증 범위만 기능으로 게시합니다.

- 이미지·동영상 최대 100개 다중 선택과 bounded 병렬 직접 업로드
- 작은 파일 single PUT, 큰 파일·영상 S3 호환 multipart 업로드와 abort
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
- G7 관리자 설정·HMAC policy 동기화, 소유권 적용 제어 API와 programmatic browser uploader

다음 항목은 구현·실환경 검증 전 공식 지원 기능으로 게시하지 않습니다.

- 실제 G7 브라우저에서 아직 검증하지 않은 게시글 form 자동 연결·첨부 표시·삭제/복원 연동
- 멀티노드, PostgreSQL queue, 임의 URL query 기반 동적 리사이즈
- MOV/WebM release fixture, 영상 트랜스코딩·metadata 제거, HEVC/AV1 Rust 폴백
- S3의 ACL, Object Lock, replication, inventory, IAM/STS, SSE-KMS 관리 기능
- 실계정 conformance를 아직 통과하지 않은 R2 또는 Lightsail profile

G7 module 0.3.0에는 form 자동 연결, Ready→native attachment, 권한 기반 viewer redirect와
보존기간 삭제 대조 후보 코드가 포함되지만, upstream `sirsoft-board` 1.1.0 계약 반영과 실제 browser smoke 전에는 위 첫 항목을
공식 지원으로 승격하지 않습니다.

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

worker가 S3/R2에 접근해야 하므로 worker service 자체는 네트워크를 사용합니다. Linux
`g7mb-sandbox`는 명령 처리 전에 seccomp-BPF를 모든 runtime thread에 설치하고 socket,
connect, bind, listen, send/recv 계열 syscall을 `EPERM`으로 거부합니다. 필터는 FFmpeg와
FFprobe child에도 상속됩니다. 환경 비우기·local protocol allowlist·worker cgroup과 함께
사용하며 별도 network namespace는 추가 방어층이지 v1 필수 차단선은 아닙니다.
