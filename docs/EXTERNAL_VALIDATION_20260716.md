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
- presigned multipart part PUT → 중간 client 재생성 → 같은 upload ID로 재개 → ETag 수집 → complete → HEAD → DELETE
- 빈 multipart create → idempotent ABORT
- derivative PutObject → DELETE
- derivative presigned GET bytes 일치 → DELETE
- `raw/`·`media/` bounded ListObjectsV2와 생성 key 존재 확인
- 모든 생성 key의 삭제 후 `HEAD`가 정확히 `NotFound`이고 run object count가 0인지 확인
- 임시 SQLite에서 Ready 사용자 삭제 요청과 8일 지난 rejected 원본을 함께 lease해
  derivative→raw 삭제, tombstone 2건, provider object count 0 확인
- 선택적으로 sparse 32MiB part를 반복 전송하는 정확한 5GiB multipart
- multipart part/complete 실패 시 알려진 upload ID를 즉시 abort해 미완료 세션을 남기지 않음

bucket 생성·IAM 변경·CORS 변경은 하지 않습니다. object key는
`raw/conformance/{uuid}`, `media/conformance/{uuid}`, `raw/live-retention/{uuid}`와
`media/live-retention/{uuid}` 아래에서만 생성합니다. 테스트 실패 시에도 lifecycle fixture의
알려진 key는 idempotent DELETE로 정리하고 생성된 multipart session은 abort를 시도합니다.

설치 경로의 `g7mbctl storage bootstrap`은 별도로 구현됐습니다. 이 명령은 권한이 있을 때
bucket을 생성하고 기존 CORS를 보존한 채 MediaBooster 관리 규칙을 병합합니다. 이어서
`g7mbctl storage doctor`가 bounded single/multipart canary를 실행합니다. 공식 provider 승격은
이 간편검사만이 아니라 아래 전체 presigned conformance까지 통과해야 합니다.

## 공통 환경값

```bash
export G7MB_LIVE_S3_PROFILE='r2' # r2 | lightsail | aws-s3 | generic
export G7MB_LIVE_S3_LABEL='safe-local-label'
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
선언 profile과 이 형태가 다르면 네트워크 요청 전에 종료하므로 임의 label만으로 R2 또는
Lightsail PASS를 만들 수 없습니다.

## 내일 실행 순서

```bash
# 설치 CUI에서 --defer-storage를 선택했다면 bucket/CORS와 bounded canary부터 완료
sudo g7mbctl storage bootstrap \
  --config /etc/g7mediabooster/g7mb.toml \
  --create-missing \
  --origin https://실제-G7-origin
sudo g7mbctl storage doctor --config /etc/g7mediabooster/g7mb.toml

# 0. secret을 출력하지 않는 환경값·HTTPS·도구·크기 preflight
export G7MB_LIVE_S3_PREFLIGHT_ONLY=true
cargo xtask live-storage-conformance
unset G7MB_LIVE_S3_PREFLIGHT_ONLY

# 1. R2 기본 protocol gate
export G7MB_LIVE_S3_PROFILE=r2
cargo xtask live-storage-conformance

# 2. Lightsail 환경값으로 교체하고 endpoint를 제거한 뒤 protocol gate
export G7MB_LIVE_S3_PROFILE=lightsail
unset G7MB_LIVE_S3_ENDPOINT
cargo xtask live-storage-conformance

# 3. 비용·시간 확인 후 선택한 provider의 5GiB gate
export G7MB_LIVE_S3_LARGE_BYTES=5368709120
cargo xtask live-storage-conformance
unset G7MB_LIVE_S3_LARGE_BYTES
```

한 번의 `live-storage-conformance`는 protocol test와 SQLite lifecycle retention test를
직렬 실행합니다. 성공 출력에는 bucket이나 key 대신 검증된 `profile`,
`multipart_reconnect=true`, `object_count=0`, `user_delete=1`, `retention_expired=1`,
`tombstones=2`만 남습니다.

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

프로필 형태는 [Cloudflare R2 S3 호환 문서](https://developers.cloudflare.com/r2/api/s3/api/)의
계정 endpoint와 `auto` region 계약, [Amazon Lightsail bucket access key 문서](https://docs.aws.amazon.com/lightsail/latest/userguide/amazon-lightsail-creating-bucket-access-keys.html)의
bucket 단위 자격증명 계약을 기준으로 고정합니다.

## 별도 수동 확인

- raw/derivative bucket public access block
- raw read/write와 derivative write/delete 최소 IAM 분리
- inventory identity의 prefix-scoped `ListBucket` 권한
- browser origin만 허용한 PUT CORS와 `ETag` expose
- lifecycle rule이 미완료 multipart와 private raw 정책을 침해하지 않는지 확인
- 실제 G7 origin에서 single/multipart PUT 후 PHP/Nginx request body가 증가하지 않는지 확인

## G7 운영 승격 게이트

공개 `gnuboard/g7@fcaacad`의 깨끗한 clone에 정식 patch 6개를 순서대로 적용하는 CI를
고정했습니다. 모듈 활성화는 버전 문자열만 믿지 않고 host capability JSON·PHP 메서드
signature·layout target을 실제 검사하며 누락 시 fail-closed합니다.

1. patch 6개 `git apply --check` 및 순차 적용 — PASS
2. `scripts/verify-gnuboard7-media-contract.sh` 29/29 + PHP/JSON parser + 실제 activation — PASS
3. 공개 G7 + MySQL 8.4 첨부·권한·layout 회귀 115 tests/283 assertions — PASS
4. module 0.4.3 `cargo xtask g7-adapter` — PASS: PHP 63/168, TS 21, typecheck/build
5. module 0.4.3 재현 ZIP 153,244 bytes, SHA-256 `1137edfb…ecac` — PASS
6. 격리 설치·활성화·migration·관리자 설정 — PASS
7. 실제 browser single PUT + 2-part multipart, Ready attachment 2개 create/update 유지 — PASS
8. private thumbnail G7 302 → MinIO 200, JPEG 505/240,658 bytes — PASS
9. 공개/비밀/블라인드/삭제글 첨부 권한과 실브라우저 403 매트릭스 — PASS
10. host 보안·watermark catalog/picker·보존 경로와 rollback — PASS
11. 보존 만료 command→실 provider 객체 삭제 확인 — 하네스 구현 완료, credential 실행 대기

배포 설명에는 1~10에서 실제 통과한 범위만 게시하며 11은 통과 전 게시하지 않습니다. patch가
G7 공식 upstream에 반영되기 전에는 patch 6개가 함께 적용된 host만 지원 대상으로 표시합니다.

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
