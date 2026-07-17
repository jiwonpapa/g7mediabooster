# Python 하네스 전환과 G7 실제 레이아웃 복구 증거

- 일시: 2026-07-17 KST
- 대상 저장소: `jiwonpapa/g7mediabooster`
- 운영 대상: SSH `g7devops`, `/home/g7devops/public_html`

## 결론

대형 Bash 하네스 4개를 Python 정본으로 전환하고, CI에 타입·보안·크기 ratchet을 추가했습니다.
운영 G7은 서비스와 모듈이 활성 상태였지만 DB-resolved uploader overlay가 0건이었습니다. 새
fail-closed `apply`가 upstream/template/module layout을 갱신한 뒤 사용자·관리자 mount를 실제
DB 결과로 확인했습니다.

## 운영 전후 증거

적용 전 `preflight`:

```text
service=active api=ready module_active=true layout_applied=false
user_mount=0 user_handler=0 admin_mount=0 admin_handler=0
```

적용 후 독립 `status`:

```text
service=active api=ready module_active=true layout_applied=true
user_mount=6 user_handler=3 admin_mount=2 admin_handler=1
```

적용 명령은 `module:refresh-layout sirsoft-board`, `template:refresh-layout sirsoft-basic`,
`module:refresh-layout jiwonpapa-g7mediabooster`, cache clear 순으로 실행했습니다. mount 검증 실패
시 모듈 비활성화와 product target 중지를 수행하도록 구성했습니다.

## 검증 결과

```text
harness governance: PASS, Python 20 files/2575 LOC, Bash 25 files/2137 LOC
Ruff: PASS
Mypy strict: PASS
Pytest: 9 PASS
cargo xtask ci: PASS
full-stack-smoke: PASS, ready=3, derivatives=6, multipart_parts=2
G7 public baseline source contract: PASS 28/28 + PHP/JSON parser + activation
installed zipapp simulation: PASS 28/28 + PHP/JSON parser + activation
g7mbctl installer tests: 7 PASS
```

외부 R2/Lightsail 자격증명을 새로 사용하지 않았습니다. Linux 서버 bundle 실제 설치는 GitHub
CI의 격리 Ubuntu job이 검증하며, 로컬에서는 동일 Python source의 결정적 ZIP application 두
개가 byte-for-byte 동일하고 설치 위치 형태에서도 실행되는 것을 확인했습니다.
