# 운영 배포 경계

## 공식 object storage 지원 범위

배포판은 S3 전체 관리 API가 아니라 다음 runtime 작업만 공식 지원합니다.

- presigned single PUT
- multipart create, part PUT, complete, abort
- HEAD, bounded GET, worker PutObject, idempotent DeleteObject

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
