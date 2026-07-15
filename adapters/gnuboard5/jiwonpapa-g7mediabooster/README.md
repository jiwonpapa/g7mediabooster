# G7 Media Booster for Gnuboard 5

Gnuboard 5.6.24용 core-free 어댑터입니다. PHP는 G5 세션·게시판 권한과 HMAC 제어 요청만
담당합니다. 파일 바이트는 브라우저에서 S3/R2 quarantine bucket으로 직접 전송됩니다.

## 제공 기능

- 최대 100개 파일의 bounded 동시 업로드
- single PUT, multipart, part 재시도·중단
- 회원 ID 또는 비회원 session HMAC에 묶인 upload 소유권
- 원본 파일명은 G5 DB에만 저장하고 Rust/S3 object key에는 전달하지 않는 경계
- Ready master·thumbnail 검증 뒤 `g5_board_file` remote row 연결
- MyISAM 게시판 테이블용 advisory lock과 부분 실패 멱등 복구
- 기존 G5 게시글 열람·다운로드 권한을 재사용하는 private redirect
- 게시글·첨부 삭제 후 보존기간이 지난 객체를 재시도 가능한 로컬 queue로 전달
- PHP 요청 본문 1 MiB, 응답 1 MiB, timeout·동시성·재시도 hard bound

G5 core 파일은 수정하지 않습니다. `extend`, hook, plugin endpoint만 사용합니다.

## 요구 환경

- Gnuboard 5.6.24
- PHP 8.1 이상: `curl`, `mbstring`, `mysqli`
- G7MediaBooster Rust API/worker/sandbox
- 브라우저 origin의 PUT과 `ETag` 노출을 허용한 S3/R2 CORS

## 설치

이 디렉터리에서 다음 두 경로를 G5에 복사합니다.

```text
extend/g7mediabooster.extend.php -> <G5_ROOT>/extend/g7mediabooster.extend.php
plugin/g7mediabooster/           -> <G5_ROOT>/plugin/g7mediabooster/
```

Composer `vendor`와 `node_modules`는 운영 복사 대상이 아닙니다. production IIFE는
`plugin/g7mediabooster/assets/uploader.iife.js`에 포함돼 있습니다.

환경변수를 PHP-FPM과 CLI reconciler에 동일하게 주입합니다.

```dotenv
G7MB_G5_ENABLED=true
G7MB_G5_ENDPOINT=https://media-api.example.com
G7MB_G5_KEY_ID=g5-primary
G7MB_G5_HMAC_SECRET=<32-256 byte secret>
G7MB_G5_CONNECT_TIMEOUT_SECONDS=2
G7MB_G5_TIMEOUT_SECONDS=10
G7MB_G5_MAX_PARALLEL_FILES=8
G7MB_G5_MAX_PARALLEL_PARTS=4
G7MB_G5_MAX_PART_RETRIES=3
G7MB_G5_STATUS_POLL_INTERVAL_MS=1500
G7MB_G5_DELETE_RETENTION_DAYS=7
```

`G7MB_G5_ENDPOINT`는 HTTPS만 허용합니다. 개발 중 literal loopback host에 한해 HTTP를 허용합니다.
HMAC `key_id`와 secret은 Rust API 설정과 정확히 같아야 합니다.

G5 전용 InnoDB session/삭제 queue 테이블을 한 번 설치합니다.

```bash
php <G5_ROOT>/plugin/g7mediabooster/bin/install.php <G5_ROOT>
```

삭제 대조는 single-node `flock`으로 중복 실행을 막습니다. 매분 실행할 수 있습니다.

```cron
* * * * * php <G5_ROOT>/plugin/g7mediabooster/bin/reconcile-deletions.php <G5_ROOT>
```

## Object storage CORS

브라우저 origin만 허용하고 multipart의 `ETag`를 노출해야 합니다.

```json
[
  {
    "AllowedOrigins": ["https://board.example.com"],
    "AllowedMethods": ["PUT"],
    "AllowedHeaders": ["content-type", "x-amz-*"],
    "ExposeHeaders": ["ETag"],
    "MaxAgeSeconds": 3600
  }
]
```

## 검증

```bash
cargo xtask g5-adapter
cargo xtask g5-host-smoke
./scripts/verify-gnuboard5-media-contract.sh /absolute/path/to/gnuboard5
```

`g5-adapter`는 PHP unit, TypeScript typecheck/test, production build를 실행합니다.
`g5-host-smoke`는 실제 MySQL 8.4에서 G5 MyISAM row 연결, advisory lock, 부분 실패 복구,
attachment limit, 삭제 retry queue를 검증합니다.

## 공식 지원 경계

G5 5.6.24 코어 훅 계약, PHP/TypeScript gate, 실제 MySQL 8.4·MyISAM 연결과 격리 브라우저
MinIO direct single/multipart → Rust 처리 → 게시글 첨부 2개 표시 → private thumbnail 전달을
통과했습니다. 이 범위는 배포판 공식 기능으로 게시할 수 있습니다. 로컬 정확한 5GiB direct
multipart는 공통 서버 gate를 통과했습니다. 실 R2/Lightsail profile의 5GiB·중단 재개와
provider 보존 만료 삭제 종단은 계정별 별도 운영 게이트이며 통과 전 지원으로 게시하지 않습니다.
