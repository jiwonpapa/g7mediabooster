# Gnuboard 7 adapter

실제 G7 모듈은 [`jiwonpapa-g7mediabooster`](jiwonpapa-g7mediabooster)에 있습니다.

- G7 세션·게시판 권한 검증
- PHP→Rust HMAC 서명과 secret 비노출
- 사용자·게시판별 upload ID 소유권 저장
- 브라우저→S3/R2 direct PUT·multipart 멀티업로드
- 사용자·게시판 소유권을 확인한 비동기 삭제 예약
- 관리자 설정 화면과 bounded 동시성

G7 코어에 필요한 owner-scoped attachment·URL filter·layout target 계약은
[`upstream-contract`](upstream-contract)에 patch와 검증기로 분리했습니다.

PHP 요청에는 파일 바이트가 들어오지 않습니다. 현재 G7 본체에는 외부 미디어 URL을 기존
`attachment_ids`로 연결하는 공개 resolver가 없으므로 게시물 첨부 표시까지 완료됐다고
간주하지 않습니다. 상세 설치와 남은 경계는 모듈 README를 따릅니다.
