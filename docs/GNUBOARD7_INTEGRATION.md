# Gnuboard 7 연동 계약

- 상태: form 자동 연결·Ready attachment bridge·private viewer·삭제 보존 대조·관리자 watermark asset picker 구현, 격리 browser 전송/create/update/private thumbnail·권한 매트릭스와 삭제/복원·보존 lease G7 DB gate PASS, upstream 정식 반영·실 provider 삭제는 보류
- 기준: Gnuboard 7.0.4 / Laravel 12 / PHP 8.2+
- 모듈: `adapters/gnuboard7/jiwonpapa-g7mediabooster`
- 정식 설치: GitHub Release의 checksum 검증 ZIP을 G7 관리자 파일 설치로 적용

## 책임 경계

| G7 PHP 모듈 | Rust 서비스 | 브라우저 |
|---|---|---|
| 로그인·게시판 권한 | object key·hard cap | 파일 선택·진행률 |
| upload ID 사용자 소유권 | presign·HEAD·queue | S3/R2 직접 PUT |
| HMAC secret 암호화 | nonce replay 차단 | bounded 병렬·취소 |
| 관리자 soft setting | 진짜 포맷·가공 상태 | multipart ETag 수집 |

HMAC secret과 S3/R2 credential은 브라우저에 전달하지 않습니다. PHP는 JSON 제어 요청만
보내며 file body, presigned URL, 인증 헤더를 로그에 남기지 않습니다.

## 구현된 경로

```text
G7 user -> /api/modules/jiwonpapa-g7mediabooster/boards/{slug}/uploads/*
        -> HMAC -> G7MediaBooster /v1/*
Browser -> presigned HTTPS PUT -> private S3/R2 quarantine
G7 owner -> HMAC delivery request -> 5분 private S3/R2 GET redirect
G7 form bridge -> uploader 자동 주입 -> Ready native attachment ID -> 게시글 attachment_ids
G7 viewer -> 공개/비밀/블라인드/삭제글 검사 -> 5분 private master/poster redirect
G7 retention -> soft delete 보존 -> native row 대조 -> Rust 멱등 삭제 예약
```

G7 module DB의 `g7mb_upload_sessions`가 `upload_id + user_id + board_slug`를 보관합니다.
Rust의 tenant scope만으로 같은 사이트 사용자 간 권한을 구분하지 않고, PHP 소유권 확인을
통과한 upload ID만 part presign·complete·abort·status 요청에 사용할 수 있습니다.

Ready 상태 응답은 각 `master`·`thumbnail`에 stable G7 `delivery_url`을 추가합니다. 해당 URL은
세션 소유권을 다시 확인한 뒤 Rust HMAC endpoint에서 5분짜리 provider GET URL을 받아
`Cache-Control: private, no-store`, `Referrer-Policy: no-referrer` redirect를 반환합니다.
PHP는 파생물 bytes와 provider credential을 읽지 않습니다.

## 관리자 설정

G7 관리자는 endpoint, key ID, HMAC secret, timeout, 파일/part 병렬도, retry와 첨부 삭제 보존일을 설정합니다.
secret은 G7 settings schema의 `sensitive: true`로 Laravel Crypt 암호화되며 조회 API는 값 대신
설정 여부만 반환합니다. S3/R2 credential, bucket, SQLite, cgroup hard cap은 Rust 운영 설정에만
둡니다.

관리자 전용 `GET /api/modules/jiwonpapa-g7mediabooster/admin/capabilities`는 HMAC으로
`GET /v1/capabilities`를 호출합니다. 응답의 포맷명·boolean·native version 길이와 형태를
다시 제한한 뒤 반환하며, Rust API는 sandbox의 내장 fixture가 실패하면 시작 자체를
거부합니다. capability API는 브라우저에서 Rust로 직접 호출하지 않습니다.

관리자 워터마크 선택기는 현재 관리자의 session·native attachment 소유권이 모두 일치하고,
최근 7일 안에 materialize된 Ready PNG·WebP·JPEG이며 source가 16MiB 이하인 자산만 최대 50개
표시합니다. 수동 UUID 입력과 인증 없는 파생물 `<img>` 요청은 사용하지 않습니다.

## 구현된 attachment bridge

모듈 0.4.2는 원본 파일명을 G7 내부에만 저장하고 Rust에는 기존 좁은 upload intent만 보냅니다.
Rust status가 같은 upload ID, `ready`, `deletion_pending=false`, 동일 preset의 master·thumbnail
각 1개임을 증명할 때만 `board_attachments`의 byte-free 레코드를 만듭니다. DB row lock과
nullable unique `attachment_id`로 중복·동시 호출은 같은 ID에 수렴합니다.

viewer 경로는 native attachment ID와 `g7mb_upload_sessions` 연결을 다시 확인하고
`AttachmentService::authorizeDelivery()`로 공개/비밀/블라인드/삭제글 scope를 검사한 뒤에만 Rust의
5분 presigned GET으로 redirect합니다. provider URL은 응답 본문·로그·DB에 저장하지 않습니다.
사용자·관리자 form extension은 native PHP uploader를 MediaBooster로 교체하고 전송 중 submit을
막습니다. 완료 이벤트의 native attachment ID를 기존 `attachment_ids`에 최대 100개까지 중복 없이
병합합니다. G7 state/action bridge가 없거나 target ID가 다르면 fail-closed 합니다.

G7 soft delete는 Rust object를 즉시 지우지 않습니다. 설정된 보존기간이 끝난 row만 15분 schedule이
lease로 선점하고, native attachment disk·collection·upload ID를 다시 대조한 뒤 Rust의 멱등 삭제를
요청합니다. 원격 요청 시작 전 복원은 예약을 취소하고, 시작 뒤 복원은 데이터 유실 방지를 위해
`G7_MEDIA_RETENTION_ALREADY_STARTED`로 차단합니다. 이 DB 흐름은 G7 host gate를 통과했으며,
보존 만료 command가 실 provider 객체를 삭제하는 종단 증거는 별도 운영 게이트입니다.

## upstream 계약과 남은 운영 게이트

`sirsoft-board`는 현재 업로드 파일을 PHP `UploadedFile`로 받고 로컬 storage URL을 생성합니다.
실제 Gnuboard 7.0.4를 대조한 결과는 다음과 같습니다.

- 게시글 저장 API는 `attachment_ids`와 `temp_key`를 이미 지원합니다.
- `templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json`의
  `FileUploader`와 그 부모에는 extension overlay가 겨냥할 `id`가 없습니다.
- layout extension은 존재하지만 이 상태에서는 첨부 UI 하나만 안전하게 `replace`할 수
  없고, 게시글 form 전체 교체는 템플릿 계약을 복제하는 취약한 우회입니다.
- `sirsoft-board`의 `Attachment::download_url`, `preview_url`과
  `AttachmentResource`는 보드 모듈의 download/preview 경로를 직접 사용하며 원격 CDN URL
  resolver/filter가 없습니다.
- 기존 `AttachmentService::upload()`는 PHP `file_get_contents()` 경로이므로 대용량 직접
  업로드 목표에 재사용하지 않습니다.
- `linkAttachmentsByIds()`는 현재 임시 attachment의 `created_by`를 확인하지 않아 순차 ID를
  아는 다른 사용자가 첨부를 가로채 연결할 수 있습니다. G7MediaBooster는 이 계약이 보완되기
  전 원격 attachment ID materialize를 활성화하지 않습니다.

별도 upstream patch에는 다음 공개 계약이 구현돼 있습니다.

1. 게시글 폼의 uploader와 submit을 overlay할 안정적인 layout extension ID
2. 원격 immutable URL을 반환하는 download/preview filter와 byte-free 권한 검사
3. `attachment_ids` 연결 시 현재 사용자 owner와 전건 일치를 원자적으로 확인하는 계약
4. module-prefixed 레이아웃 요청명까지 overlay 후보에 포함하는 core 계약
5. 조건부 form의 모든 동일 target에 overlay를 적용하는 core 계약
6. `attachment_ids` bulk link 뒤 `attachments_count`를 같은 transaction에서 동기화하는 계약
7. 비밀·블라인드·삭제글 첨부 직접 URL을 본문 열람 정책과 동일하게 제한하는 계약

모듈은 이 일곱 계약이 없으면 manifest와 runtime reflection 검사에서 fail-closed 합니다.
실제 브라우저 single PUT·2-part multipart, Ready attachment 2개 생성·표시, 게시글 수정 후 유지와
private thumbnail 302→MinIO 200은 통과했습니다. 타 사용자·비밀글·블라인드·삭제글 권한과
삭제/복원·보존 lease는 G7 host gate에서 통과했습니다. 작성자·다른 회원·비회원·관리자의
실브라우저 거부 매트릭스도 통과했으며 실 provider 보존 만료 삭제만 운영 게이트로 유지합니다.

Gnuboard7 본체 저장소는 사용자 변경이 많은 별도 작업영역이므로 여기서 코어를 수정하지
않습니다. 현재 기준 commit `e64381ddb5ba02caed60933427fbb86ef72ef94e`에 적용 가능한
[upstream 계약 patch](../adapters/gnuboard7/upstream-contract/README.md)와 읽기 전용 검증기를
준비했습니다. 별도 깨끗한 worktree에서 patch 6개 clean apply, 계약 검사 29/29와 activation gate, 첨부 전체 PHP
39/39와 권한·보존 host 15/15를 확인했으며 기존 patch 기준 board suite 회귀도 유지했습니다.
module 설치·설정·user/admin form과 실제 저장소 E2E도 통과했고 실제
G7 사용자 작업 트리는 수정하지 않았습니다. patch 적용 배포에는 통과 범위만 공식 게시하며,
upstream 미반영 배포와 실 provider 삭제 종단은 해당 게이트 전 지원으로 게시하지 않습니다.
