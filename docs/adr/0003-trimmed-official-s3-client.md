# ADR-0003: 경량화한 공식 S3 작업 클라이언트 유지

- 상태: Accepted
- 날짜: 2026-07-15

## 결정

S3 Signature Version 4와 HTTP protocol을 직접 구현하지 않습니다. `aws-sdk-s3`는 기본 feature를
끄고 HTTPS/Tokio만 활성화해 유지합니다. 반면 환경·profile·IMDS·STS credential chain을 만드는
`aws-config`는 제거하고, 운영 설정의 static access key, region, endpoint를 S3 client builder에
직접 주입합니다.

application의 `ObjectStore` port에는 SDK type을 노출하지 않습니다. 따라서 실제 release 크기와
빌드 시간이 목표를 침해한다는 측정이 쌓이면 호환 adapter만 별도 구현으로 교체할 수 있습니다.

## 이유

- 직접 구현 범위는 canonical path/query/header, payload signing, presigned URL, session token,
  XML 오류, multipart/ETag, retry·timeout·clock skew까지 포함합니다.
- 서명 오류는 단순 기능 결함이 아니라 credential 노출과 요청 변조 방어 실패로 이어질 수 있습니다.
- 제거 전 release 기준 전체 API는 14MB, worker는 13MB였고, 제거 후 각각 12MB로 줄었습니다.
- 실제 불필요했던 것은 S3 작업 client가 아니라 사용하지 않는 `aws-config → aws-sdk-sts` 체인입니다.

## 지원하는 최소 작업 집합

- presigned single PUT
- multipart create, part PUT, complete, abort
- HEAD와 bounded download
- worker PutObject
- idempotent DeleteObject

bucket 생성, ACL, Object Lock, replication, inventory 등 관리 API는 제품 runtime 범위가 아닙니다.

## R2 검증 범위

R2 계정만으로 위 최소 작업 집합과 5GiB multipart, ETag, CORS를 실제 검증할 수 있습니다. 이 결과는
`R2 profile PASS`이며 AWS S3 고유 region redirect, IAM/STS, SSE-KMS, storage class 동작까지
검증했다는 뜻은 아닙니다. AWS profile은 실제 AWS 자격 증명으로 별도 통과할 때까지
`UNVERIFIED`로 유지합니다.

## Lightsail 검증 범위

Amazon Lightsail Object Storage는 제품 최소 작업 집합의 PutObject, HeadObject, GetObject,
DeleteObject와 multipart create/upload/complete/abort를 제공합니다. Lightsail 실버킷을 통과하면
`LIGHTSAIL S3 PROFILE PASS`로 기록하며 제품의 Amazon 계열 S3 protocol gate로 인정합니다.
Lightsail 자체가 의도적으로 최소 기능 제품이므로 AWS S3의 모든 관리 기능을 인증한 것으로
확대 해석하지 않습니다.

Lightsail bucket access key는 버킷 단위입니다. 단일 키로 검증할 때 raw와 derivative 환경값에
같은 private bucket을 넣고 서로 다른 `raw/`, `media/` prefix를 사용합니다. 이는 모든 runtime
operation을 검증하지만 두 버킷의 IAM 격리까지 증명하지는 않습니다. raw와 derivative의 공개
정책이 다르면 운영에서는 같은 버킷을 사용하지 않습니다.

## 재검토 조건

- strip된 release binary 또는 container가 운영 크기 예산을 넘음
- clean build 시간이 CI SLO를 지속적으로 위반함
- 필요한 provider가 공식 client로 구현할 수 없는 protocol 차이를 가짐
- 직접 adapter에 대해 AWS/R2 golden SigV4 corpus와 fault-injection conformance를 먼저 확보함
