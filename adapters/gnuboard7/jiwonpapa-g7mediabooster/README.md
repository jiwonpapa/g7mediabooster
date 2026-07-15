# G7 Media Booster for Gnuboard 7

Gnuboard 7.0.4+와 `sirsoft-board` 1.1.0 보안 첨부 계약용 전용 모듈입니다. PHP는 로그인·게시판 권한·HMAC 제어 요청만 담당하고,
파일 바이트는 브라우저에서 S3/R2 quarantine bucket으로 직접 전송합니다.

## 제공 기능

- 최대 100개 batch intent와 게시판별 파일 수·크기 제한
- single PUT 및 multipart, 파일 8개·파일별 part 4개 기본 병렬 제한
- 진행률, 전체 취소, transient part 재시도, multipart abort
- multipart `ETag` 검증과 완료 순서 고정
- HMAC secret을 Laravel Crypt로 암호화하고 브라우저 응답에서 제거
- upload ID를 로그인 사용자와 게시판에 묶어 수평 권한 상승 차단
- 소유 upload의 HMAC 삭제 예약과 `deletion_pending` 상태 프록시
- 소유 Ready master/thumbnail의 5분 private GET no-store redirect
- Ready 상태를 검증한 뒤 G7 native attachment를 원자적·멱등 생성
- 게시글 권한·삭제 상태를 재사용하는 master/thumbnail·영상 poster private redirect
- Ready 이미지 upload ID 기반 워터마크 정책과 HMAC 서명 revision 동기화
- G7 관리자 설정 레이아웃과 PHP/TypeScript 테스트 하네스

## 설치

PHP 8.2 이상과 `mbstring` 확장이 필요합니다.

이 디렉터리를 G7의 다음 위치에 둡니다.

```text
modules/jiwonpapa-g7mediabooster
```

먼저 `adapters/gnuboard7/upstream-contract`의 계약이 정식 반영된 `sirsoft-board >=1.1.0`인지
검증한 뒤 G7의 표준 모듈 설치·활성화 절차를 사용합니다. 설치 시
`g7mb_upload_sessions`와 attachment bridge migration이 실행되고 관리자 메뉴에 `미디어 부스터`가 추가됩니다.
Rust API의 `key_id`, HMAC secret, tenant 설정은 G7 관리자 값과 정확히 맞아야 합니다.

## Object storage CORS

브라우저 origin만 허용하고 최소한 다음 계약을 만족해야 합니다.

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

`ETag`를 expose하지 않으면 multipart 완료를 거부하고 abort합니다. Presign에 포함된
`Content-Length`는 브라우저가 실제 Blob 크기로 자동 전송하므로 JavaScript가 금지 헤더를
직접 설정하지 않습니다. 운영 direct-upload URL은 HTTPS만 허용합니다.

## 사용

게시판 레이아웃 또는 향후 `attachment_uploader` 확장점에서 다음 custom element를 둡니다.

```html
<g7-media-uploader board-slug="free"></g7-media-uploader>
```

전체 직접 전송 후 Ready 확인과 native attachment 생성까지 끝나면 `g7mb:complete` 이벤트가
발생합니다. `detail.files[*].attachment.id`를 게시글 `attachment_ids`에 넣을 수 있습니다.
JavaScript에서 직접 사용하려면 전역 factory를 사용할 수 있습니다.

```js
const uploader = window.__G7MediaBooster.createUploader('free');
const result = await uploader.upload(files, { signal: abortController.signal });
```

## 검증

```bash
npm ci
npm run typecheck
npm test
npm run build

# G7 vendor의 PHPUnit 사용
/path/to/gnuboard7/vendor/bin/phpunit -c phpunit.xml
```

현재 검증 범위는 PHP HMAC/config/client·삭제·Ready materialization·URL resolver 계약,
TypeScript 100개 bounded scheduling·multipart·ETag·Ready polling·native attachment 생성,
site policy PUT/GET, Vite production build와 G7 upstream 계약 검사입니다.
워터마크 자산은 현재 관리자 화면에서 Ready upload UUID로 지정합니다. 실제 S3/R2
credential과 G7 설치 후 브라우저 smoke는 별도 운영 게이트입니다.

## 아직 공식 지원으로 게시하지 않는 연동

모듈에는 Ready→native attachment bridge와 권한 기반 private delivery가 구현돼 있습니다.
다만 G7 upstream patch 정식 반영, 게시글 form 자동 주입, create/update·비밀글·삭제글 실제
브라우저 smoke 전에는 이를 배포 설명의 공식 지원 기능으로 게시하지 않습니다. patch가 없는
`sirsoft-board`에서는 manifest와 runtime 계약 검사가 설치·실행을 fail-closed 합니다.
