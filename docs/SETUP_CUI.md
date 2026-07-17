# `g7mbctl` 설치·저장소 설정

`g7mbctl`만 사용자 입력을 받습니다. `g7mb-api`와 `g7mb-worker`는 무인 데몬이며 시작 중
질문하거나 비밀값을 표준입력으로 받지 않습니다.

일반 사용자는 서버 Release에서 다음 한 명령만 실행합니다. service user·경로·systemd target
등록 후 이 문서의 대화형 setup을 자동으로 이어서 실행하고 API ready까지 확인합니다.

```bash
sudo ./bin/g7mbctl install
```

설정과 기동을 분리하는 자동화 환경에서만 `install --skip-setup --skip-start`와 아래 setup
명령을 따로 사용합니다. 개별 API·worker·timer unit을 직접 enable하지 않습니다.

## 대화형 설치

```bash
sudo /usr/local/bin/g7mbctl setup
```

CUI가 공급자, 버킷, G5/G7 origin, Access Key ID와 Secret Access Key를 순서대로 받습니다.
두 자격증명은 echo 없이 입력합니다. R2는 32자리 Account ID에서 endpoint를 만들고 region을
`auto`로 고정합니다. MediaBooster에는 S3 client용 Access Key ID와 Secret Access Key를 넣으며
Cloudflare REST API의 Bearer token value나 클라이언트 인증서는 받지 않습니다. 현재 R2 S3
호환 API는 `CreateBucket`과 bucket CORS를 지원하므로 권한이 있는 S3 credential이면 별도
REST token 입력 없이 설치 과정에서 처리할 수 있습니다.

선택한 공급자는 일반 TOML의 `[storage].provider`에 `r2`, `aws-s3`, `lightsail`, `generic` 중
하나로 저장합니다. API·worker·`g7mbctl storage`와 S3 adapter는 네트워크 요청 전에 같은 계약을
검사합니다. R2의 canonical account endpoint/`auto`, AWS의 기본 endpoint/실 region,
Lightsail의 단일 bucket, generic의 명시 endpoint가 맞지 않으면 시작을 거부합니다.

기본 설치는 private 버킷 하나를 `raw/`와 `media/` prefix로 함께 사용합니다. 별도 derivative
버킷을 입력하면 두 버킷을 각각 검사합니다. 사용자가 동의하면 다음 순서로 설정 저장 전에
실제 공급자를 검증합니다.

1. `HeadBucket`, 선택 시 `CreateBucket`
2. 기존 CORS 규칙을 보존하고 `g7mediabooster-browser-v1` 규칙만 병합·재검증
3. canary `PUT → HEAD → GET → LIST → DELETE`
4. multipart `create → upload part → complete` 및 별도 `create → abort`
5. canary object 삭제 확인 후 설정을 원자적으로 저장

Lightsail bucket access key는 기존 단일 버킷에 대한 전체 object 접근 키이며 버킷 생성용 AWS
계정 credential이 아닙니다. CORS도 S3 `PutBucketCors`가 아니라 Lightsail `UpdateBucket`으로
관리합니다. 따라서 Lightsail에서는 버킷과 exact origin의 `PUT/GET/HEAD`, `ETag` 노출 CORS를
Lightsail 콘솔/API에서 먼저 설정한 뒤 `--skip-cors`를 사용해야 합니다. `--create-buckets`나
S3 CORS 변경 요청은 네트워크 전에 거부합니다. 범용 S3는 관리 API 차이 때문에 기본값만 기존
버킷 검사이며 명시적으로 요청하면 해당 공급자에서 검증합니다.

## 비밀값 저장

일반 설정에는 비밀값 대신 절대 파일 경로만 기록합니다.

```text
/etc/g7mediabooster/g7mb.toml                         root:g7mediabooster 0640
/etc/g7mediabooster/credentials/                     root:root           0700
  storage-access-key-id                              root:root           0600
  storage-secret-access-key                          root:root           0600
  g7-hmac-secret                                     root:root           0600
```

systemd unit은 `LoadCredential=`로 세 파일을 서비스별 격리 credential directory에 복사하고
`%d/...` 경로를 설정 override로 전달합니다. 따라서 PHP, 브라우저, 프로세스 인자와 일반 TOML에
R2/S3 자격증명이 들어가지 않습니다. 기존 inline TOML secret은 개발·이전 설치 호환용으로만
남아 있으며 `_file`과 동시에 설정하면 시작을 거부합니다.

생성된 HMAC secret만 G7 관리자 Media Booster 설정의 HMAC 필드에 한 번 입력합니다. R2/S3
자격증명은 G7 관리자나 PHP 설정에 넣지 않습니다.

설치 후 운영자는 내부 unit 목록 대신 다음 제품 명령을 사용합니다.

```bash
sudo g7mbctl status
sudo g7mbctl doctor
sudo g7mbctl doctor --skip-storage
```

## 외부 계정 없이 먼저 설치

공급자 값을 준비했지만 지금 네트워크 검증을 하지 않을 때만 명시적으로 연기합니다.

```bash
sudo g7mbctl setup --defer-storage

# 외부 계정 사용이 가능한 시점
sudo g7mbctl storage bootstrap \
  --config /etc/g7mediabooster/g7mb.toml \
  --create-missing \
  --origin https://example.com
sudo g7mbctl storage doctor --config /etc/g7mediabooster/g7mb.toml
sudo g7mb-worker --config /etc/g7mediabooster/g7mb.toml doctor
```

Lightsail은 위 bootstrap 대신 사전 버킷/CORS 설정 후 다음처럼 검사만 수행합니다.

```bash
sudo g7mbctl storage bootstrap \
  --config /etc/g7mediabooster/g7mb.toml \
  --skip-cors
sudo g7mbctl storage doctor --config /etc/g7mediabooster/g7mb.toml
```

`storage doctor`와 worker doctor는 `raw/g7mb-canary/` 또는 `media/g7mb-canary/` 아래에만 임시
객체를 만들고 삭제합니다. `g7mb-worker doctor --offline`은 설정·client·SQLite·native tool만
검사하며 공급자 지원 증거로 인정하지 않습니다.

## 비대화형 자동화

비밀값을 명령행 인자로 넘기지 않습니다. root-only 입력 파일을 사용합니다.

```bash
sudo g7mbctl setup \
  --non-interactive \
  --provider r2 \
  --account-id ACCOUNT_ID \
  --bucket g7mediabooster-private \
  --origin https://example.com \
  --access-key-id-file /root/r2-access-key-id \
  --secret-access-key-file /root/r2-secret-access-key \
  --create-buckets
```

기존 파일의 내용이 다르면 기본적으로 중단합니다. `--force`는 각 credential을 같은 디렉터리의
임시 파일에서 원자 교체하고 일반 설정을 마지막에 교체합니다. HMAC도 회전하므로 G7 관리자 값을
함께 갱신할 때만 사용합니다. 같은 입력으로 재실행하면 기존 HMAC을 유지하고 변경 없이 통과합니다.

## 지원 경계

bootstrap은 MediaBooster에 필요한 bucket 존재성과 CORS만 관리합니다. ACL, 공개 bucket,
Object Lock, replication, inventory, IAM/STS, SSE-KMS, website 설정은 변경하지 않습니다.
실 R2/Lightsail 공식 지원 승격은 별도 provider conformance와 5GiB gate까지 통과한 뒤 결정합니다.

공급자 관리 경계는 [Cloudflare R2 S3 호환표](https://developers.cloudflare.com/r2/api/s3/api/),
[Lightsail bucket access key](https://docs.aws.amazon.com/lightsail/latest/userguide/amazon-lightsail-creating-bucket-access-keys.html),
[Lightsail CORS 설정](https://docs.aws.amazon.com/lightsail/latest/userguide/cors-configuration-cli.html)을
기준으로 합니다.

0.1 이전 개발 설정을 재사용할 때 `[storage]`에 `provider`가 없으면 의도적으로 시작하지
않습니다. 수동 추측 대신 `sudo g7mbctl setup --force`로 재생성하고 G7 HMAC 회전까지 함께
적용하거나, 기존 endpoint·region·bucket을 확인한 뒤 정확한 provider 값을 명시해야 합니다.
