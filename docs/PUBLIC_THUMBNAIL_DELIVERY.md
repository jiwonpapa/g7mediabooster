# 서명된 공개 썸네일 전달

## 상태

공개 썸네일 listener는 구현되어 있지만 기본 비활성입니다. 기존 G5/G7 private attachment 경로는
변하지 않습니다. 새 데몬은 추가하지 않으며 `g7mb-api` 프로세스가 다음 두 loopback listener를
분리합니다.

| Listener | 용도 | 공개 proxy |
|---|---|---|
| `127.0.0.1:8088` | PHP HMAC control, health, metrics | 금지 |
| `127.0.0.1:8089` | 서명된 exact thumbnail redirect | `/media/v1/`만 허용 |

## URL 계약

```text
GET /media/v1/{tenant_id}/{upload_id}/{preset_id}/thumbnail.jpg
    ?expires={unix_seconds}
    &signature={base64url_hmac_sha256}
```

서명 payload:

```text
G7MB-MEDIA-HMAC-SHA256
GET
{canonical_path}
{expires}
```

- `tenant_id`는 실행 설정의 단일 tenant와 정확히 같아야 합니다.
- `upload_id`는 UUID이고 `preset_id`는 저장된 immutable preset과 정확히 같아야 합니다.
- `file`은 `thumbnail.jpg` 하나만 허용합니다.
- 추가 query, 중복 query, percent-encoded 우회, 임의 크기·필터·원격 URL은 거부합니다.
- 만료 상한 기본값은 5분이며 최대 1시간을 넘길 수 없습니다.
- 실패 응답은 자산 존재 여부를 숨기기 위해 `404`로 수렴합니다.

## 활성화

`g7mbctl setup`이 생성한 설정에는 listener와 secret-file 경로가 들어가지만 비활성 상태입니다.
먼저 [Nginx reference config](../deploy/nginx/g7mediabooster-public.conf)를 설치하고 TLS·도메인을
실제 값으로 바꾼 다음 아래 값만 활성화합니다.

```toml
[delivery]
public_enabled = true
public_bind_addr = "127.0.0.1:8089"
public_signing_secret_file = "/etc/g7mediabooster/credentials/g7-hmac-secret"
public_token_max_ttl_seconds = 300
public_rate_limit_requests_per_second = 100
public_rate_limit_burst = 200
public_max_in_flight_requests = 128
```

기본 설치는 HMAC canonical domain을 분리한 기존 root-only secret을 재사용합니다. 별도 secret을
원하면 새 root-only 파일을 만들고 systemd `LoadCredential` drop-in과 위 경로를 함께 변경해야
합니다.

```bash
sudo systemctl restart g7mediabooster-api.service
curl -i https://media.example.com/metrics
curl -i https://media.example.com/v1/capabilities
```

두 요청은 모두 `404`여야 합니다. 서명이 없는 `/media/v1/...`도 `404`여야 합니다.

## Provider redirect pin

Rust는 presign 결과의 scheme과 authority를 다시 검증합니다.

- R2·generic endpoint: 설정된 endpoint authority와 derivative bucket virtual-host authority
- AWS S3·Lightsail: 설정된 derivative bucket과 region의 exact S3 authority
- 평문 HTTP: literal loopback endpoint에만 허용

일치하지 않으면 provider URL을 PHP나 브라우저에 전달하지 않고 `503`으로 종료합니다.

## 성능 경계

공개 listener는 libvips·FFmpeg·얼굴 검출을 실행하지 않습니다. 업로드 worker가 미리 생성한
`thumbnail.jpg`만 전달합니다. manifest cache hit와 provider presign은 worker의 이미지 처리
semaphore를 공유하지 않으며, cache miss는 동일 upload 단위로 singleflight 됩니다.
