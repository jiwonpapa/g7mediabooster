# G7 module 0.4.3 공개 upstream activation 계약 증거

## 판정

- 공개 G7 main `fcaacad8d16d47a8b5bcee65990869992de0a0d8`에 patch `0001`~`0006` 순차 `git apply --check`: PASS
- 정적 계약 28항목과 module activation capability 1항목: **29/29 PASS**
- patch가 만지는 PHP/JSON parser 검사: PASS
- 공개 baseline MySQL 8.4 attachment·layout regression: 115 tests, 283 assertions PASS
- patch `0006`이 없는 현재 개발 checkout: `G7MB_G7_CONTRACT_FILE_MISSING`으로 예상대로 FAIL
- 모듈 standalone PHPUnit: 63 tests, 168 assertions PASS

## 이전 0.4.2 관리자 설치 산출물

- module commit: `72e381796d9cebb6864673d71aa145767729a81d`
- ZIP: `jiwonpapa-g7mediabooster-0.4.2.zip`, 153,123 bytes
- ZIP SHA-256: `a8778321e6fa139609f95171bba24a0a7cb16cbafb74f03ed8d9b4604c7bb57c`
- tar.gz: 126,138 bytes
- tar.gz SHA-256: `807090c2ce49a8fce8d3c7120e9d067cff1cbd400cf16ff7ee4e2feef6d49ce2`
- 실제 G7 `ZipInstallHelper`: identifier·version·activation capability manifest PASS

## 활성화 시 검사하는 정본

`Gnuboard7MediaContract::assertCompatible()`는 다음을 모두 검사합니다.

1. `sirsoft-board` 1.1.x
2. `sirsoft-board.secure-external-attachments` 1.x capability와 필수 13개 feature
3. delivery·visibility·owner-bound linking PHP 메서드의 public 시그니처
4. 사용자 3개·관리자 2개 layout target ID

하나라도 없거나 중복·손상되면 `Module::activate()`가 예외를 발생시켜 레이아웃과 route가
활성 상태가 되기 전에 차단합니다. 버전 문자열만 올린 호스트는 통과하지 못합니다.

## 남은 외부 게이트

이 검사는 upstream 미반영을 완료로 바꾸지 않습니다. G7 정식 upstream commit과 실 provider
보존 삭제는 여전히 요구사항 6의 외부 잔여 게이트입니다.
