# R2/Lightsail 외부 저장소 검증 인계서

- 예정일: 2026-07-16
- 현재 상태: 구현·로컬 MinIO conformance와 API 재기동을 포함한 정확한 5GiB/API RSS gate 완료, `G7MB_LIVE_S3_*` 실계정 환경값 미설정
- 원칙: secret과 bucket 값은 문서·Git·명령 이력에 저장하지 않습니다.
- 보유 계정 기준: 2026-07-16에는 R2와 Lightsail profile을 실행합니다. 일반 AWS S3 profile은
  별도 자격 증명이 생길 때까지 `UNVERIFIED`로 남깁니다.

## 구현된 외부 하네스

`cargo xtask live-storage-conformance`는 이미 존재하는 private raw/derivative bucket을 대상으로
다음을 실제 수행하고 생성 객체를 삭제합니다.

- presigned single PUT → HEAD → idempotent DELETE
- credentialed bounded GET → exact byte 비교
- presigned multipart part PUT → ETag 수집 → complete → HEAD → DELETE
- 빈 multipart create → idempotent ABORT
- derivative PutObject → DELETE
- derivative presigned GET bytes 일치 → DELETE
- `raw/`·`media/` bounded ListObjectsV2와 생성 key 존재 확인
- 선택적으로 sparse 32MiB part를 반복 전송하는 정확한 5GiB multipart

bucket 생성·IAM 변경·CORS 변경은 하지 않습니다. object key는
`raw/conformance/{uuid}`와 `media/conformance/{uuid}` 아래에서만 생성합니다.

## 공통 환경값

```bash
export G7MB_LIVE_S3_LABEL='aws-s3-or-r2'
export G7MB_LIVE_S3_REGION='provider-region'
export G7MB_LIVE_S3_RAW_BUCKET='existing-private-raw-bucket'
export G7MB_LIVE_S3_DERIVATIVE_BUCKET='existing-private-derivative-bucket'
export G7MB_LIVE_S3_ACCESS_KEY='set-locally'
export G7MB_LIVE_S3_SECRET_KEY='set-locally'
export G7MB_LIVE_S3_FORCE_PATH_STYLE='false'
```

AWS S3는 `G7MB_LIVE_S3_ENDPOINT`를 unset하고 실제 region을 사용합니다. R2는 region을
`auto`로 두고 다음 endpoint를 로컬 환경에만 설정합니다.

```bash
export G7MB_LIVE_S3_ENDPOINT='https://ACCOUNT_ID.r2.cloudflarestorage.com'
```

Lightsail은 `G7MB_LIVE_S3_ENDPOINT`를 unset하고 버킷의 실제 AWS region을 사용합니다.
Lightsail bucket access key는 버킷 단위이므로 단일 버킷 검증에서는 raw와 derivative bucket
환경값에 같은 private bucket을 넣습니다. 하네스는 `raw/`, `media/` prefix를 분리합니다.

## 내일 실행 순서

```bash
# 0. secret을 출력하지 않는 환경값·HTTPS·도구·크기 preflight
export G7MB_LIVE_S3_PREFLIGHT_ONLY=true
cargo xtask live-storage-conformance
unset G7MB_LIVE_S3_PREFLIGHT_ONLY

# 1. R2 기본 protocol gate
cargo xtask live-storage-conformance

# 2. Lightsail 환경값으로 교체하고 endpoint를 제거한 뒤 protocol gate
unset G7MB_LIVE_S3_ENDPOINT
cargo xtask live-storage-conformance

# 3. 비용·시간 확인 후 선택한 provider의 5GiB gate
export G7MB_LIVE_S3_LARGE_BYTES=5368709120
cargo xtask live-storage-conformance
unset G7MB_LIVE_S3_LARGE_BYTES
```

5GiB 실행은 실제로 5GiB를 전송·저장하므로 비용과 실행시간을 확인한 뒤 수행합니다. 완료 후
provider, region, 실행시각, elapsed, object count 0, 5GiB HEAD 길이만 보고서에 기록하고
credential·presigned URL·bucket 실명은 남기지 않습니다.

R2 통과로 제품이 쓰는 presigned PUT, multipart, HEAD, bounded download, PutObject,
private derivative presigned GET, idempotent delete를 검증할 수 있습니다. 다만 AWS region
redirect, IAM/STS, SSE-KMS, storage class 등 AWS 고유 동작을 검증한 것으로 기록하지 않습니다.
AWS 계정이 생기면 동일 하네스로 AWS profile만 추가합니다.

Lightsail 통과는 제품이 사용하는 Amazon 계열 S3 object protocol 전체를 검증한 것으로
인정합니다. 다만 동일 private bucket의 prefix 검증이므로 raw/derivative 두 버킷 IAM 격리는
별도 운영 점검 항목입니다.

## 별도 수동 확인

- raw/derivative bucket public access block
- raw read/write와 derivative write/delete 최소 IAM 분리
- inventory identity의 prefix-scoped `ListBucket` 권한
- browser origin만 허용한 PUT CORS와 `ETag` expose
- lifecycle rule이 미완료 multipart와 private raw 정책을 침해하지 않는지 확인
- 실제 G7 origin에서 single/multipart PUT 후 PHP/Nginx request body가 증가하지 않는지 확인

## G7 운영 승격 게이트

현재 G7 개발 checkout `e64381dd`에는 upstream patch `0001`~`0005`가 모두 반영돼 정적 계약
28/28과 PHP·JSON 문법 검사를 통과했습니다. patch 범위 밖 사용자 파일은 수정하지 않았습니다.
다만 현재 checkout에는 `.env.testing`이 없어 아래 DB 회귀의 재실행은 production DB 보호
가드에서 중단됐습니다. 기존 별도 clean worktree에서는 patch 적용, 핵심 회귀, 격리 설치와
MinIO 기반 실제 전송·create/update·private thumbnail 전달까지 통과했습니다.

1. `scripts/verify-gnuboard7-media-contract.sh` 28/28 — PASS
2. board PHP 핵심 4개 파일 48 tests/73 assertions — PASS
3. 전체 board suite 기준선 비교 — patch 전 80 failed/1102 passed, patch 후 80 failed/1108 passed, 신규 실패 0
4. module 0.4.0 `cargo xtask g7-adapter` — PASS: PHP 57/153, TS 21, typecheck/build
5. module 0.3.0 설치·활성화·세 migration·설정 및 user/admin form disabled smoke — PASS
6. 실제 browser single PUT + 2-part multipart, Ready attachment 2개 create/update 유지 — PASS
7. private thumbnail G7 302 → MinIO 200, JPEG 505/240,658 bytes — PASS
8. 공개/비밀/블라인드/삭제글 첨부 권한 10 tests/10 assertions — PASS
9. soft-delete 복원 취소, lease 재검증, 요청 시작 뒤 복원 차단 5 tests/27 assertions — PASS
10. 관리자 watermark asset picker 선택·저장·재로드·rollback — PASS
11. 작성자·다른 회원·비회원·관리자 실브라우저 403 매트릭스 — PASS
12. 보존 만료 command→실 provider 객체 삭제 확인

배포 설명에는 1~11에서 실제 통과한 범위만 게시하며 12는 통과 전 게시하지 않습니다.

## G5 운영 승격 게이트

G5 5.6.24는 core-free adapter를 실제 MySQL 8.4·MyISAM과 MinIO 격리 브라우저에서 검증했습니다.

1. G5 source contract 21/21 — PASS
2. PHP 17 tests/31 assertions, TypeScript 5 tests, typecheck/build — PASS
3. MyISAM attachment advisory lock·부분 실패 복구·삭제 retry 11/11 — PASS
4. 실제 browser single PUT + 2-part multipart 동시 업로드, Rust Ready 2/2 — PASS
5. G5 게시글 `wr_file=2`, remote attachment row 2개, thumbnail decode 2/2 — PASS
6. 비로그인 private thumbnail HTTP 403 — PASS

따라서 G5 5.6.24의 위 범위는 공식 게시할 수 있습니다. 로컬 정확한 5GiB direct multipart는
별도 gate를 통과했습니다. 실 R2/Lightsail profile의 5GiB·중단 재개,
provider 보존 만료 삭제는 공통 외부 게이트 통과 전 게시하지 않습니다.
