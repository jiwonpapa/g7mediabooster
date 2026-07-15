# R2/Lightsail 외부 저장소 검증 인계서

- 예정일: 2026-07-16
- 현재 상태: 구현·로컬 MinIO conformance 완료, 실계정 환경값 미설정
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

R2 통과로 제품이 쓰는 presigned PUT, multipart, HEAD, download, PutObject, idempotent delete는
검증할 수 있습니다. 다만 AWS region redirect, IAM/STS, SSE-KMS, storage class 등 AWS 고유 동작을
검증한 것으로 기록하지 않습니다. AWS 계정이 생기면 동일 하네스로 AWS profile만 추가합니다.

Lightsail 통과는 제품이 사용하는 Amazon 계열 S3 object protocol 전체를 검증한 것으로
인정합니다. 다만 동일 private bucket의 prefix 검증이므로 raw/derivative 두 버킷 IAM 격리는
별도 운영 점검 항목입니다.

## 별도 수동 확인

- raw/derivative bucket public access block
- raw read/write와 derivative write/delete 최소 IAM 분리
- browser origin만 허용한 PUT CORS와 `ETag` expose
- lifecycle rule이 미완료 multipart와 private raw 정책을 침해하지 않는지 확인
- 실제 G7 origin에서 single/multipart PUT 후 PHP/Nginx request body가 증가하지 않는지 확인
