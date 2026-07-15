# Changelog

## 0.2.0 - 2026-07-15

- 100개 direct upload 전송 단계와 Ready 확인 단계를 분리하고 control 요청을 초당 8개로 제한했습니다.
- Ready master·thumbnail 전건 검증과 DB lock 기반 native attachment 멱등 생성을 추가했습니다.
- G7 게시글 권한·삭제글 정책을 재사용하는 private master·thumbnail/poster redirect를 추가했습니다.
- 원본 파일명은 G7 내부에만 보관하고 Rust upload intent에서는 제거합니다.
- `sirsoft-board >=1.1.0` 보안 첨부 계약이 없으면 설치·runtime을 fail-closed 합니다.
- MOV/WebM은 release 검증 전 사용자 업로더 지원 형식에서 제외했습니다.

## 0.1.0 - 2026-07-15

- G7 관리자 설정, HMAC control client, upload ownership, 100개 direct single/multipart uploader를 추가했습니다.
