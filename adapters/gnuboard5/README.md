# Gnuboard 5 adapter

실제 G5 플러그인은 [`jiwonpapa-g7mediabooster`](jiwonpapa-g7mediabooster)에 있습니다.

- G5 5.6.24 hook만 사용하며 core 파일을 수정하지 않음
- 게시판 쓰기·업로드 권한과 회원/비회원 session 소유권 확인
- PHP→Rust HMAC 제어 요청과 secret 비노출
- 브라우저→S3/R2 direct single/multipart 최대 100개 멀티업로드
- Ready 결과를 `g5_board_file`의 remote row로 멱등 연결
- 기존 G5 게시글 열람·다운로드 session 권한 뒤 private derivative redirect
- 게시글/첨부 삭제를 durable local deletion queue로 전환

PHP 요청에는 파일 바이트가 들어오지 않습니다. 설치·계약 검증은 플러그인 README와
`scripts/verify-gnuboard5-media-contract.sh`를 따릅니다. G5 5.6.24·MySQL 8.4·MinIO 격리
browser/storage smoke까지 통과한 범위만 공식 배포 기능으로 게시합니다.
