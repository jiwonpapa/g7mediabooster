# G7 upstream media contract 검증 증거

- 기준 G7 commit: `e64381ddb5ba02caed60933427fbb86ef72ef94e`
- 대상: `sirsoft-board` 1.2.0 후보 patch
- 원본 G7 사용자 작업 트리: 미수정

## 결과

- patch: 현재 기준에 clean apply
- 정적 계약: 21/21 PASS
- 핵심 PHP: 48 tests, 73 assertions, 실패 0
- 전체 board suite 기준선: 80 failed, 1102 passed
- patch 적용 후: 80 failed, 1108 passed
- 신규 실패: 0

## 발견·수정한 결함

이전 patch는 admin `Http/Requests/StorePostRequest.php`와 `UpdatePostRequest.php`만 보강했습니다.
실제 사용자 `PostController`는 `Http/Requests/User/*Request.php`를 사용하므로 과다·비-list
`attachment_ids`가 검증을 통과해 각각 500과 403으로 끝났습니다. 사용자 create/update
FormRequest에도 list, 게시판 최대 수, strict distinct 제한을 적용하고 422 회귀 테스트를
추가했습니다.

전체 board suite의 80개 실패는 patch 전후가 동일하며 설정·권한·메뉴 등 미디어 계약 밖의
기존 기준선입니다. 실제 G7 브라우저 create/update·비밀글·삭제/복원 smoke 전에는 게시물
자동 첨부 연동을 공식 배포 기능으로 게시하지 않습니다.
