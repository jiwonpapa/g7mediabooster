# G7 upstream media contract 검증 증거

- 기준 G7 commit: `e64381ddb5ba02caed60933427fbb86ef72ef94e`
- 대상: `sirsoft-board` 1.2.0 후보 patch
- 원본 G7 사용자 작업 트리: 미수정

## 결과

- patch `0001`~`0004`: 현재 기준에 순차 clean apply
- 정적 계약: 23/23 PASS
- 핵심 PHP: 48 tests, 73 assertions, 실패 0
- core module overlay: 2 tests, 4 assertions, 실패 0
- 모든 동일 overlay target·ID bulk link count 신규 회귀: PASS
- G7 module: PHP 55 tests/137 assertions, TS 17 tests, typecheck/build PASS
- 전체 board suite 기준선: 80 failed, 1102 passed
- patch 적용 후: 80 failed, 1108 passed
- 신규 실패: 0
- 격리 설치·관리자 설정·user/admin form disabled smoke: PASS

## 발견·수정한 결함

이전 patch는 admin `Http/Requests/StorePostRequest.php`와 `UpdatePostRequest.php`만 보강했습니다.
실제 사용자 `PostController`는 `Http/Requests/User/*Request.php`를 사용하므로 과다·비-list
`attachment_ids`가 검증을 통과해 각각 500과 403으로 끝났습니다. 사용자 create/update
FormRequest에도 list, 게시판 최대 수, strict distinct 제한을 적용하고 422 회귀 테스트를
추가했습니다.

실제 gallery 분기에서 동일 uploader ID 중 첫 target만 교체돼 native PHP uploader가 남던 문제는
`0003`에서 모든 일치 target을 병합하도록 고쳤습니다. ID bulk link가 모델 hook을 우회해
`attachments_count=0`이던 문제는 `0004`에서 같은 transaction 재계산으로 고쳤습니다.

전체 board suite의 80개 실패는 patch 전후가 동일하며 설정·권한·메뉴 등 미디어 계약 밖의
기존 기준선입니다. 실제 MinIO single/multipart 전송과 create/update·private thumbnail은 후속
E2E에서 통과했습니다. patch 적용을 전제로 이 통과 범위만 공식 게시하며, 타 사용자·비밀글·
삭제글 권한과 삭제/복원은 아직 게시하지 않습니다. 설치·초기 화면은
[`G7_BROWSER_SMOKE_20260716.md`](G7_BROWSER_SMOKE_20260716.md), 저장소 E2E는
[`G7_STORAGE_E2E_20260716.md`](G7_STORAGE_E2E_20260716.md)에 분리했습니다.
