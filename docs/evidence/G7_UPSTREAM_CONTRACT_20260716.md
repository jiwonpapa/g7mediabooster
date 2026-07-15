# G7 upstream media contract 검증 증거

- 기준 G7 commit: `e64381ddb5ba02caed60933427fbb86ef72ef94e`
- 대상: `sirsoft-board` 1.2.0 후보 patch
- 현재 개발 checkout: patch `0001`~`0005` 반영, patch 외 파일 보존

## 결과

- patch `0001`~`0005`: 현재 checkout 반영, canonical patch 역검사 PASS
- 정적 계약: 28/28 + patch 전체 PHP/JSON parser PASS
- 현재 checkout 첨부·권한 PHP: 58 tests/90 assertions, 실패 0
- 현재 checkout 전체 layout extension: 57 tests/193 assertions, 실패 0
- 현재 checkout ID bulk link count: 1 test/2 assertions, 실패 0
- G7 module 0.4.0: PHP 57 tests/153 assertions, TS 21 tests, typecheck/build PASS
- 이전 clean worktree 전체 board suite: patch 전 80 failed/1102 passed, patch 후 80 failed/1108 passed, 신규 실패 0
- 격리 설치·관리자 설정·user/admin form disabled smoke: PASS

## 발견·수정한 결함

이전 patch는 admin `Http/Requests/StorePostRequest.php`와 `UpdatePostRequest.php`만 보강했습니다.
실제 사용자 `PostController`는 `Http/Requests/User/*Request.php`를 사용하므로 과다·비-list
`attachment_ids`가 검증을 통과해 각각 500과 403으로 끝났습니다. 사용자 create/update
FormRequest에도 list, 게시판 최대 수, strict distinct 제한을 적용하고 422 회귀 테스트를
추가했습니다.

실제 gallery 분기에서 동일 uploader ID 중 첫 target만 교체돼 native PHP uploader가 남던 문제는
`0003`에서 모든 일치 target을 병합하도록 고쳤습니다. 실제 MySQL 회귀에서 `prepend`가 순회 중
추가한 형제를 다시 처리해 무한 반복하는 문제도 발견해, 원본 형제 인덱스를 역순으로 처리하고
삽입된 노드는 같은 overlay에서 재처리하지 않도록 수정했습니다. ID bulk link가 모델 hook을 우회해
`attachments_count=0`이던 문제는 `0004`에서 같은 transaction 재계산으로 고쳤습니다.
비밀·블라인드 게시글의 첨부 직접 경로가 본문 열람 정책을 우회하던 문제는 `0005`에서
작성자·manager·read-secret 정책과 삭제글 fail-closed 조회를 추가해 고쳤습니다.

전체 board suite의 80개 실패는 patch 전후가 동일하며 설정·권한·메뉴 등 미디어 계약 밖의
기존 기준선입니다. 실제 MinIO single/multipart 전송과 create/update·private thumbnail은 후속
E2E에서 통과했습니다. 권한 10 tests/10 assertions와 보존 lease host 5 tests/27 assertions도
통과했습니다. patch 적용을 전제로 이 통과 범위만 공식 게시하며 실 provider 만료 삭제는 아직
게시하지 않습니다. 설치·초기 화면은
[`G7_BROWSER_SMOKE_20260716.md`](G7_BROWSER_SMOKE_20260716.md), 저장소 E2E는
[`G7_STORAGE_E2E_20260716.md`](G7_STORAGE_E2E_20260716.md)에 분리했습니다.
권한·보존 DB 증거는 [`G7_SECURITY_RETENTION_GATE_20260716.md`](G7_SECURITY_RETENTION_GATE_20260716.md)에 있습니다.
후속 실브라우저 거부 매트릭스는
[`G7_WATERMARK_PICKER_AUTH_MATRIX_20260716.md`](G7_WATERMARK_PICKER_AUTH_MATRIX_20260716.md)에서 통과했습니다.
