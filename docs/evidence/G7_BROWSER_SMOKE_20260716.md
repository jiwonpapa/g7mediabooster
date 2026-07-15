# G7 설치·브라우저 smoke 증거

- 기준 G7: `e64381ddb5ba02caed60933427fbb86ef72ef94e`
- G7MediaBooster module: `0.3.0`
- 적용 patch: upstream contract `0001`, module-prefixed overlay core fix `0002`
- 격리 환경: 별도 worktree, MySQL 8.4, PHP 8.5.3, G7 7.0.4
- 원본 G7 작업 트리: 미수정

## 결과

- `0001` → `0002` 순차 `git apply --check`: PASS
- G7 module 설치·활성화와 3개 migration: PASS
- `sirsoft-board` 1.2.0, user/admin template 활성화: PASS
- G7 module route와 user/admin overlay 등록: PASS
- 관리자 로그인·대시보드·미디어 부스터 설정 화면: PASS
- 사용자 `/board/free/write` 업로더 주입: PASS
- 관리자 `/admin/board/free/create` 업로더 주입: PASS
- 서비스 disabled 상태의 선택·전송 버튼 차단과 안내 문구: PASS
- module gate: PHP 55 tests/137 assertions, TS 17 tests, typecheck/build PASS
- core overlay gate: 2 tests/4 assertions PASS

## 실제 설치에서 발견·수정한 결함

1. 최초 활성화 시 아직 설정 행이 없어 `control_endpoint is invalid`로 실패했습니다. endpoint와
   key ID는 loopback·disabled 안전 기본값으로 읽고 secret은 빈 값으로 유지하도록 수정했습니다.
2. G7은 다른 모듈 레이아웃을 DB·route에서 `sirsoft-board.*`로 접두사 처리하지만 파일 내부
   `layout_name`은 비접두사 값을 유지합니다. 요청된 레이아웃명도 overlay 후보에 포함하는
   upstream core patch와 회귀 테스트를 추가해 관리자 업로더 주입을 복구했습니다.

## 화면 증거

- [관리자 설정](assets/g7-media-booster-settings-20260716.png)
- [사용자 글쓰기 disabled 상태](assets/g7-user-board-uploader-disabled-20260716.png)
- [관리자 글쓰기 disabled 상태](assets/g7-admin-board-uploader-disabled-20260716.png)

## 아직 승격하지 않는 범위

이번 smoke는 설정과 user/admin form 주입, disabled fail-safe까지 검증했습니다. Rust API와
실제 object storage를 연결한 전송, Ready→native attachment create/update, 비밀글·삭제글
viewer 차단, soft-delete·복원·보존 만료 삭제는 아직 실브라우저 PASS가 아닙니다. 따라서
이 초기 smoke만으로는 G7 게시물 자동 첨부 연동을 공식 지원 기능으로 게시하지 않습니다.

이 초기 disabled smoke 뒤 실제 저장소 연결 결과는
[G7 저장소 browser E2E](G7_STORAGE_E2E_20260716.md)에서 이어집니다.
