# G7 module 0.4.1 ZIP 설치 계약 증거

- module commit: `f2594aac76e7ee692074a8ee8f5c51622b2123ca`
- G7 checkout: `c275b41b6d804f4c5b91fecaa9b5652410f1cc7c` (검증 중 무변경)
- 관리자 설치 ZIP: `jiwonpapa-g7mediabooster-0.4.1.zip`
- ZIP bytes: `149966`
- ZIP SHA-256: `a667232a6f57dc0a98fbdd1f19e79ab728e49353e3a4037f49395cdb3be0a6a3`
- 수동 배치 tar.gz bytes: `123435`
- tar.gz SHA-256: `6487d2c78f68bb6cc759db9335b5de01464092390b9278aa949a84f89d197b27`

`cargo xtask g7-module-package`를 서로 다른 실행 시각에 두 번 실행해 ZIP과 tar.gz SHA가 각각
같음을 확인했습니다. ZIP 파일 timestamp는 module commit 시각, 파일 순서는 byte 정렬로
고정합니다. 한 실행 안의 단순 2회 비교뿐 아니라 실행 간 digest도 일치합니다.
현재 G7 checkout의 dirty-state SHA-256도 실행 전후
`da27cb7f224deaa63d698bd0d587e8972ccf5b68daefb9612bb8ca860f0b4e76`로 같았습니다.

`GNUBOARD7_ROOT=/Users/neojins/workspace/gnuboard7 cargo xtask g7-module-package`는 실제 G7의
`App\Extension\Helpers\ZipInstallHelper::extractAndValidate()`를 호출해 다음 결과를 냈습니다.

```text
G7 ZipInstallHelper PASS identifier=jiwonpapa-g7mediabooster version=0.4.1
```

G7 관리자 파일 설치가 요구하는 ZIP, 1단계 내부 `module.json`, identifier, version,
`sirsoft-board >=1.2.0` 계약을 실제 helper로 확인했습니다. 이 검사는 임시 디렉터리만 사용하며
G7 checkout과 DB를 변경하지 않습니다.

Standalone module gate는 PHP 57 tests/155 assertions, TypeScript 21 tests, typecheck와 production
build가 PASS입니다. GitHub release workflow는 `g7-module-v<version>` annotated tag와 manifest
버전이 정확히 같을 때만 ZIP·checksum·계약 patch를 게시합니다. 현재 `release_status=candidate`라
stable release나 tag는 자동 생성하지 않았습니다.
