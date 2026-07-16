# G7 module 0.4.2 activation 호환성 계약 증거

## 판정

- 깨끗한 G7 기준 `e64381dd`에 patch `0001`~`0006` 순차 `git apply --check`: PASS
- 정적 계약 28항목과 module activation capability 1항목: **29/29 PASS**
- patch가 만지는 PHP/JSON parser 검사: PASS
- patch `0006`이 없는 현재 개발 checkout: `G7MB_G7_CONTRACT_FILE_MISSING`으로 예상대로 FAIL
- 모듈 standalone PHPUnit: 62 tests, 165 assertions PASS

## 활성화 시 검사하는 정본

`Gnuboard7MediaContract::assertCompatible()`는 다음을 모두 검사합니다.

1. `sirsoft-board` 1.2.x
2. `sirsoft-board.secure-external-attachments` 1.x capability와 필수 13개 feature
3. delivery·visibility·owner-bound linking PHP 메서드의 public 시그니처
4. 사용자 3개·관리자 2개 layout target ID

하나라도 없거나 중복·손상되면 `Module::activate()`가 예외를 발생시켜 레이아웃과 route가
활성 상태가 되기 전에 차단합니다. 버전 문자열만 올린 호스트는 통과하지 못합니다.

## 남은 외부 게이트

이 검사는 upstream 미반영을 완료로 바꾸지 않습니다. G7 정식 upstream commit과 실 provider
보존 삭제는 여전히 요구사항 6의 외부 잔여 게이트입니다.
