# Changelog

## 0.3.0 - 2026-07-15

- 사용자·관리자 게시글 폼에 업로더를 안전하게 주입하고 완료된 native attachment ID를 기존 저장 계약에 자동 연결합니다.
- 전송 중에는 submit을 차단하고 최대 100개 ID를 중복 없이 병합합니다.
- G7 soft delete를 즉시 원격 삭제로 바꾸지 않고 설정된 보존기간 뒤 lease 기반으로 대조·삭제합니다.
- 원격 삭제 시작 전 복원은 예약을 취소하고, 원격 삭제가 시작된 뒤 복원은 fail-closed 합니다.
- upstream 첨부 계약을 현재 G7 기준 `sirsoft-board >=1.2.0`으로 재배치하고 사용자·관리자 FormRequest를 분리 검증하는 21항목으로 확장했습니다.
- 최초 활성화 전에 저장 설정이 없어도 안전한 disabled 기본값으로 부팅하도록 수정했습니다.
- 관리자 업로더 확장을 G7 모듈 접두사가 붙은 게시판 레이아웃에 등록하도록 수정했습니다.

## 0.2.0 - 2026-07-15

- 100개 direct upload 전송 단계와 Ready 확인 단계를 분리하고 control 요청을 초당 8개로 제한했습니다.
- Ready master·thumbnail 전건 검증과 DB lock 기반 native attachment 멱등 생성을 추가했습니다.
- G7 게시글 권한·삭제글 정책을 재사용하는 private master·thumbnail/poster redirect를 추가했습니다.
- 원본 파일명은 G7 내부에만 보관하고 Rust upload intent에서는 제거합니다.
- 보안 첨부 계약이 없으면 설치·runtime을 fail-closed 합니다.
- MOV/WebM은 release 검증 전 사용자 업로더 지원 형식에서 제외했습니다.

## 0.1.0 - 2026-07-15

- G7 관리자 설정, HMAC control client, upload ownership, 100개 direct single/multipart uploader를 추가했습니다.
