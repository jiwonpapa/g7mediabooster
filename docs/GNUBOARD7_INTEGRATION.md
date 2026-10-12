# Gnuboard 7.1 core-free 연동

- 모듈: 0.5.1, `g7mb.gnuboard7-core-free` adapter contract 1.0.0
- 검증 기준: 공식 G7 7.1.0 `dbe88420c3219f42db1ac16a7e6bb17a438e144d`, 번들 `sirsoft-board` 1.2.0
- 코어·게시판·기본 템플릿 원본 수정: 금지
- 현재 상태: 원본 호스트 회귀 및 g7devops.com ZIP 업데이트·브라우저·실 R2 canary 완료, 정식 인수 게이트 대기

## 연결 방법

1. 공개 `core.layout_extension.after_apply` filter에서 응답의 원본 FileUploader만 교체합니다.
   별도 요청되는 사용자·관리자 partial도 처리하고 기존 조건·disabled 상태를 보존합니다.
2. 기존 `upload:board_attachments` 이벤트를 async listener로 기다립니다. Ready 첨부 ID를
   원본 `setState` 동작으로 저장 흐름에 전달하며 실패 시 게시글 저장을 중단합니다.
3. 공식 `getMiddleware()`로 게시글 저장 요청을 감쌉니다. 최대 100개 ID의 소유자·게시판·Ready·
   유효기간·삭제 경합·개수를 lock으로 확인합니다. 원본 인증·권한·FormRequest·PostService를
   그대로 실행하고 같은 DB transaction에서 attachment 연결과 개수를 완성합니다.
4. 기존 `/api/modules/sirsoft-board/boards/{slug}/attachment/{hash}` 및 `/preview`를 유지합니다.
   매핑된 원격 첨부에만 요청의 마지막 실행 함수를 바꾸고, 모든 원본·타 모듈 middleware를
   실행합니다. 응답·예외 후 route action을 복원하며 원본 route 파일은 수정하지 않습니다.
5. 원본 AttachmentService에 I/O 차단 storage를 주입해 게시판 scope·비밀·블라인드·삭제 정책을
   재사용합니다. 통과한 파일만 Rust에서 5분 presigned URL을 받아 no-store 302로 전달합니다.

파일 바이트는 브라우저 → S3/R2로 직접 전송되고 PHP는 제어 요청만 처리합니다.
PHP의 업로드 크기 제한은 우회하지만 저장소·서비스 정책 상한과 CORS 설정은 필요합니다.

## 적용 해제와 제거

관리자 `미디어 부스터`의 업로드 사용을 끄면 `enabled=false`가 됩니다.
원본 uploader가 복원되고 신규 MediaBooster 업로드는 거부되지만 기존 원격 미디어 열람은 유지됩니다.
Rust API와 모듈을 함께 끄면 기존 첨부가 깨지므로 해제 하네스는 둘을 중지하지 않습니다.
원격 첨부가 남아 있는 동안 전체 모듈 비활성화·제거를 차단합니다. 먼저 미디어를 이관하거나
정상 삭제 절차를 완료해야 합니다. 자동 로컬 이관은 이번 구현 범위가 아닙니다.

## 버전 판정

7.1.0과 번들 1.2.0은 실제 공개 코드에서 확인한 기준이며 가상의 미래 버전이 아닙니다.
버전만 믿지 않고 필수 공개 PHP 메서드 시그니처도 확인합니다. 과거 7.0.x용 외부 첨부
patch·`external-media-v1.json`·URL filter·고정 layout ID는 필요하지 않습니다.
7.0.x와 임의 커스텀 템플릿을 검증 없이 지원한다고 표시하지 않습니다.

## 남은 인수 게이트

- 공식 stock 사이트의 ZIP 설치·활성화·설정 저장·적용/해제 전 과정
- 사용자/관리자 브라우저의 전체 작성·수정 경로 (g7devops.com 사용자 작성 single/multipart·표시는 통과)
- 비로그인/타인/관리자의 실제 HTTP 권한 매트릭스 및 캐시·언어 전환
- 실 provider 보존 만료 삭제 종단 (새 어댑터의 R2 업로드·전달·명시적 테스트 삭제는 통과)
- 영상 poster의 게시판 목록/본문 표시 및 커스텀 템플릿별 동작

과거 patched-host 브라우저 증거를 새 어댑터 검증으로 재사용하지 않습니다.
정본 구현은 모듈 코드, 공개 판정은 `deploy/official-features-v1.json`입니다.
상세 [초기 stock 검증](evidence/G7_CORE_FREE_710_20261012.md),
[실서버 배포·R2 검증](evidence/G7DEVOPS_CORE_FREE_R2_E2E_20261012.md), [설치 절차](GNUBOARD7_RELEASE.md)를 참고하십시오.
