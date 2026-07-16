# G7MediaBooster

Gnuboard 5/7용 고성능 미디어 업로드·가공 서버입니다. Rust가 제어 계층을 맡고,
이미지는 libvips, MP4/MOV 썸네일은 FFmpeg가 별도 샌드박스 프로세스에서 처리합니다. FFmpeg를
시작할 수 없는 MP4/H.264만 Rust `mp4` + OpenH264 첫 프레임 폴백을 사용합니다.
S3와 Cloudflare R2는 같은 S3 호환 포트로 구현하며, 공급자별 실계정 conformance를 통과한
profile만 공식 지원으로 게시합니다.

현재 저장소는 **v0.1 구현 진행 단계**입니다. batch 생성, S3/R2 single·multipart 제어,
HMAC 인증, SQLite lease queue, 검증된 master+thumbnail/poster 원자적 발행,
4MiB/60초 bounded manifest cache·singleflight, digest-pinned 이미지 워터마크,
sandbox runtime capability, 삭제·보존 cleanup과 G7 0.3
form/Ready attachment bridge·보존 삭제 대조까지 연결됐습니다. G7 격리 설치·관리자 설정과 MinIO 기반 실제 browser single/multipart 전송·create/update·private thumbnail 전달, 비밀·블라인드·삭제글 첨부 권한 및 보존 lease 실제 DB 게이트도 통과했습니다. MP4/MOV H.264의 실제 worker master·poster, API 재기동을 포함한 로컬 5GiB 직접 multipart 재개와 G7 PHP 정책→Rust worker 워터마크→rollback 종단도 통과했습니다. 실제 R2/Lightsail profile과 G7 upstream 정식 반영 등 남은 게이트는 구현
계획에 따라 진행합니다. 배포 시에는 [검증된 공식 기능 범위](deploy/README.md)만 게시합니다.

## 확정 기술 스택

- Rust 2024 Edition / Rust 1.96.0 고정
- Axum + Tokio
- libvips 네이티브 바인딩(격리 프로세스 내부)
- FFmpeg CLI(쉘 미사용, 고정 인자, timeout/kill)
- 경량화한 공식 AWS S3 작업 client 기반 S3/R2 (`aws-config`/STS chain 제외)
- SQLite WAL 단일 노드 durable queue
- OpenAPI, HMAC-SHA256, Prometheus 형식 메트릭

## 공식 저장소 지원 범위

배포판이 보장하는 범위는 S3 전체 관리 API가 아니라 G7MediaBooster가 사용하는 object 작업입니다.

- presigned single PUT
- multipart create, part PUT, complete, abort
- HEAD, bounded worker GET, private derivative presigned GET, worker PutObject, idempotent DeleteObject

MinIO는 로컬 protocol gate, R2와 Lightsail은 각각 별도 실계정 profile로 판정합니다. 검증하지 않은
ACL, Object Lock, replication, inventory, IAM/STS, SSE-KMS는 공식 지원 기능으로 게시하지 않습니다.

## 빠른 확인

실행 가능한 코드와 코드에서 생성한 계약이 최종 정본입니다. Rust 공개 API의 의미와 제한은
소스 안의 rustdoc에 두며, 수기 Markdown은 설명용 snapshot으로만 유지합니다.

```bash
cargo xtask quick
cargo xtask rustdoc
cargo xtask ci
cargo xtask native-smoke
cargo xtask api-smoke
cargo xtask g7-adapter
cargo xtask storage-conformance
cargo xtask full-stack-smoke
cargo xtask g7-policy-smoke
cargo xtask large-multipart-smoke
cargo xtask heavy-avif
# 실 R2/Lightsail 환경값 설정 후
cargo xtask live-storage-conformance
cargo xtask coverage
cargo xtask supply-chain
```

운영 설치의 설정과 비밀값은 직접 TOML에 입력하지 않고 CUI로 생성합니다.

```bash
sudo /usr/local/bin/g7mbctl setup
```

외부 계정 연결을 미룬 설치, R2/AWS S3/Lightsail/범용 S3 profile, 비대화형 자동화는
[설치·저장소 설정 문서](docs/SETUP_CUI.md)를 따릅니다.

공급망 도구가 없다면 먼저 다음을 실행합니다.

```bash
./scripts/install-dev-tools.sh
cargo xtask supply-chain
```

네이티브 스모크와 sandbox startup capability는 버전 문자열만 확인하지 않고 필수 이미지
6개 입력·4개 출력과 MP4/MOV H.264 썸네일 추출을 실제로 수행합니다.

## 문서

- [제품 스펙](SPEC.md)
- [개발 헌법](DEVELOPMENT_CONSTITUTION.md)
- [아키텍처](docs/ARCHITECTURE.md)
- [보안 모델](docs/SECURITY.md)
- [개발·검증 방법](docs/DEVELOPMENT.md)
- [부트스트랩 완료 보고서](docs/BOOTSTRAP_STATUS.md)
- [요구사항 1~17 구현 계획](docs/IMPLEMENTATION_PLAN.md)
- [요구사항 1~17 현재 판정표](docs/REQUIREMENTS_STATUS.md)
- [멀티업로드와 lease queue 설명](docs/QUEUE_MODEL.md)
- [Gnuboard 7 연동 계약](docs/GNUBOARD7_INTEGRATION.md)
- [Gnuboard 7 모듈 배포·설치](docs/GNUBOARD7_RELEASE.md)
- [Gnuboard 7 upstream 첨부 계약 patch](adapters/gnuboard7/upstream-contract/README.md)
- [Gnuboard 7 실제 저장소 browser E2E](docs/evidence/G7_STORAGE_E2E_20260716.md)
- [정확한 5GiB·G7 정책 종단 증거](docs/evidence/LARGE_MULTIPART_AND_G7_POLICY_20260716.md)
- [CUI 설정·storage bootstrap 구현 증거](docs/evidence/SETUP_CUI_AND_STORAGE_BOOTSTRAP_20260716.md)
- [MOV/H.264 runtime·worker 종단 증거](docs/evidence/MOV_H264_E2E_20260716.md)
- [G7 module 0.4.0 재현 배포 산출물](docs/evidence/G7_MODULE_PACKAGE_20260716.md)
- [G7 module 0.4.3 공개 upstream activation 계약](docs/evidence/G7_ACTIVATION_COMPATIBILITY_GATE_20260716.md)
- [Spec 1.1 요구사항 1~17 완료 감사](docs/COMPLETION_AUDIT_20260716.md)
- [Spec 1.1 내부 인수 게이트 재검증](docs/evidence/INTERNAL_REVALIDATION_20260716.md)
- [실 provider lifecycle 삭제 하네스](docs/evidence/LIVE_PROVIDER_LIFECYCLE_HARNESS_20260716.md)
- [워터마크 계약](docs/WATERMARK.md)
- [미디어 수명주기·삭제](docs/LIFECYCLE.md)
- [Provider orphan inventory](docs/ORPHAN_INVENTORY.md)
- [SQLite backup·restore](docs/BACKUP_RESTORE.md)
- [운영 관측·API 보호](docs/OPERATIONS.md)
- [`g7mbctl` 설치·저장소 설정](docs/SETUP_CUI.md)
- [R2/Lightsail 외부 검증 인계서](docs/EXTERNAL_VALIDATION_20260716.md)
- [핵심 기술 결정](docs/adr/0001-rust-libvips-ffmpeg.md)
- [멀티업로드와 단일 서버 queue 결정](docs/adr/0002-multi-upload-single-server-queue.md)
- [공식 S3 client 경량화 결정](docs/adr/0003-trimmed-official-s3-client.md)

## 저장소 경계

```text
apps/       API, worker, native sandbox 실행 파일
crates/     domain, application ports, contracts, adapters
adapters/   Gnuboard 5/7용 얇은 PHP 연동 계층
scripts/    재현 가능한 로컬/CI 하네스
xtask/      Rust 기반 품질 게이트 오케스트레이터
```

라이선스는 Apache-2.0입니다. libvips, FFmpeg와 각 코덱의 배포 라이선스는 별도로
검토해야 합니다.
