# CUI 설정·storage bootstrap 구현 증거

- 기준일: 2026-07-16
- 외부 credential: 사용하지 않음
- 판정: 코드·로컬 protocol PASS, R2/Lightsail 실계정 PENDING

## 구현

- `g7mbctl setup`: TTY 대화형 hidden credential 입력
- `--non-interactive`: command-line secret 금지, root-only input file만 허용
- R2 Account ID → HTTPS endpoint, region `auto` 자동 파생
- 선택 provider를 TOML에 영속화하고 API·worker·S3 adapter가 같은 shape를 시작 전에 검증
- 일반 TOML과 S3/HMAC credential 파일 분리
- symlink·상대 secret path·과도한 권한·inline/file 동시 설정 거부
- systemd `LoadCredential=`와 `%d` 서비스 격리 경로
- R2/AWS/generic bucket HEAD/create, 기존 CORS 규칙 보존 병합·readback 검증
- Lightsail bucket access key의 CreateBucket·S3 CORS 시도를 네트워크 전에 거부하고
  Lightsail API/콘솔 사전 설정 + `--skip-cors` 경계 강제
- single PUT/HEAD/GET/LIST/DELETE, 2-part complete/HEAD/delete, 별도 abort canary
- 변경 충돌 사전검사, 파일별 same-directory atomic rename, config-last 저장
- 같은 입력 재실행 시 HMAC 유지, 변경 입력은 `--force` 없으면 거부
- provider 누락, R2 region/endpoint 불일치, Lightsail 다중 bucket, 원격 평문 HTTP endpoint 거부

## 실행 증거

```text
cargo xtask quick                 PASS
cargo xtask setup-smoke           PASS
cargo xtask storage-conformance   PASS (2/2 ignored live tests executed against pinned MinIO)
cargo xtask ci                    PASS
```

MinIO는 `PutBucketCors` 관리 API를 제공하지 않아 로컬 provider CORS 종단에는 사용하지 않습니다.
대신 관리 규칙의 allowed origin/method/header/ETag와 기존 규칙 보존·멱등 교체를 단위시험으로
검증했습니다. 실제 R2에서는 bucket/CORS bootstrap readback과 전체 presigned provider
conformance를 실행하고, Lightsail은 Lightsail API/콘솔 사전 설정 뒤 같은 conformance의 실제
browser OPTIONS·PUT CORS 응답을 통과해야 공식 지원으로 승격합니다.

## 외부 값 준비 후 남은 명령

```bash
# R2/AWS S3
sudo g7mbctl storage bootstrap \
  --config /etc/g7mediabooster/g7mb.toml \
  --create-missing \
  --origin https://실제-G7-origin
sudo g7mbctl storage doctor --config /etc/g7mediabooster/g7mb.toml

# Lightsail: bucket/CORS는 Lightsail API/콘솔에서 먼저 설정
sudo g7mbctl storage bootstrap \
  --config /etc/g7mediabooster/g7mb.toml \
  --skip-cors
sudo g7mbctl storage doctor --config /etc/g7mediabooster/g7mb.toml

export G7MB_LIVE_S3_ORIGIN=https://실제-G7-origin
cargo xtask live-storage-conformance
```

실계정 실행 결과에는 credential, bucket 실명, presigned URL을 기록하지 않습니다.
