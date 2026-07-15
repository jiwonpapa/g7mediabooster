# G7 module 0.4.0 배포 산출물 증거

- module commit: `b118240bdbdd1b252aaacbf74708555422b646a0`
- artifact: `jiwonpapa-g7mediabooster-0.4.0.tar.gz`
- bytes: `122811`
- SHA-256: `53b4dc1ce48c24e9a0fa12506a9be93c79543a8c96a887ee148e0ad812c5d026`

`cargo xtask g7-module-package`가 같은 module commit을 두 번 archive해 byte-for-byte 일치를
확인한 뒤 압축본과 `.sha256`을 생성했습니다. module subtree를 마지막으로 바꾼 commit 시각을
모든 tar entry에 고정하고 gzip timestamp를 제거하므로 문서만 바뀐 후에도 같은 module은 같은
digest를 냅니다.

산출물에는 `module.json` 0.4.0, PHP source, 세 migration, 설정·layout·route, production JS와
재빌드용 source가 포함됩니다. `vendor`, `node_modules`, tests, `*.test.*`, PHPUnit cache는
포함하지 않습니다. 압축 내부 버전과 `deploy/official-features-v1.json` 버전이 다르거나,
공식 기능과 검증 보류 기능 ID가 겹치거나, R2/Lightsail·실 provider 삭제·멀티노드 보류 항목이
누락되면 패키징은 fail-closed합니다.

같은 실제 patched G7 checkout의 임시 MySQL 8.4 gate에서 core 권한 10 tests/10 assertions와
module host 보안·watermark catalog·보존 7 tests/38 assertions를 통과했습니다. 외부 R2와
Lightsail profile은 실계정 conformance 전까지 manifest의 공식 게시 기능에 포함하지 않습니다.
