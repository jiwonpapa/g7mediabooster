# Gnuboard 7 adapter

실제 G7 모듈은 [`jiwonpapa-g7mediabooster`](jiwonpapa-g7mediabooster)에 있습니다.

- G7 세션·게시판 권한 검증
- PHP→Rust HMAC 서명과 secret 비노출
- 사용자·게시판별 upload ID 소유권 저장
- 브라우저→S3/R2 direct PUT·multipart 멀티업로드
- 사용자·게시판 소유권을 확인한 비동기 삭제 예약
- 비밀·블라인드·삭제글 첨부 직접 경로의 권한 차단
- soft-delete 보존 lease와 복원 경합 차단
- 관리자 설정 화면과 bounded 동시성

G7 코어에 필요한 owner-scoped attachment·URL filter·layout target 계약은
[`upstream-contract`](upstream-contract)에 patch와 검증기로 분리했습니다.

PHP 요청에는 파일 바이트가 들어오지 않습니다. upstream patch `0001`~`0005` 적용 환경에서
Ready 결과의 native `attachment_ids` 연결·표시와 private delivery를 지원합니다. patch가 없거나
권한-aware 계약이 빠진 G7에서는 fail-closed 합니다. 상세 설치와 남은 경계는 모듈 README를 따릅니다.
