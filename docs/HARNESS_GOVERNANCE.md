# 하네스 언어 거버넌스

## 결론

인프라·배포·통합 테스트의 상태 관리와 분기, SSH, 증거 수집은 Python 3.11+가 담당합니다.
Rust `xtask`는 개발자가 사용하는 단일 명령 표면과 Rust 품질 게이트만 담당합니다. Bash는 운영체제
진입점이나 기존 명령 호환용 얇은 래퍼로 제한합니다.

`xtask`는 제품 crate를 직접 의존하지 않습니다. OpenAPI처럼 제품 타입이 필요한 작업은 해당
package의 전용 binary를 필요한 순간에만 실행합니다. 따라서 하네스 거버넌스·native inventory 같은
가벼운 명령이 AWS SDK, SQLx, API 전체 graph를 먼저 빌드하지 않습니다.

## 소유권

| 영역 | 정본 언어 | 허용 범위 |
|---|---|---|
| 제품 API·worker·sandbox | Rust | 제품 기능, 타입 계약, 성능·보안 경계 |
| 저장소 개발 명령 | Rust `xtask` | 명령 라우팅, Rust 품질·생성 게이트 |
| 배포·SSH·프로세스·통합 시나리오 | Python | 상태 전이, rollback, timeout, 구조화 증거 |
| G5/G7 host 계약 | PHP | 실제 PHP 앱과 framework binding 검증 |
| 브라우저 어댑터 | TypeScript | UI 계약과 브라우저 동작 |
| OS 진입점 | Bash | `exec` 기반 호환 래퍼와 최소 bootstrap |

Python 하네스는 `subprocess` 인자 배열만 사용하고 shell mode를 금지합니다. 비밀값은 명령 인자나
증거 JSON에 기록하지 않습니다. 원격 변경 명령은 명시적 확인값, 실행 전 검증, rollback을 가져야
합니다.

## 강제 게이트

```bash
python3 -m pip install -e './tools/harness[dev]'
cargo xtask harness-governance --require-tools
```

게이트는 Ruff, Mypy strict, Pytest, Python compile, ShellCheck, Bash parse와 파일 크기 상한을 함께
검사합니다. 일반 Python 하네스 파일은 300줄, 신규 Bash 파일은 100줄을 넘길 수 없습니다.
원격에서 단독 실행되는 트랜잭션 파일과 아직 이관되지 않은 기존 Bash만 명시적 ratchet 예외이며,
예외 상한은 자동으로 늘어나지 않습니다.

신규 Rust/PHP/TypeScript 수기 source는 500줄을 상한으로 하고 기존 초과 파일은 현재 줄 수를
상한으로 고정합니다. 이 수치는 성장 경보이며 새 crate를 만들거나 응집된 코드를 기계적으로
분할하라는 기준이 아닙니다. Python과 Bash 전체 줄 수도 현재 통과값에서 증가할 수 없습니다.

현재 Python 정본으로 전환된 주요 경로는 full-stack media smoke, G7 운영 apply/disable/rollback,
G7 원격 설치, G7 source·DB-resolved layout 계약입니다. 기존 `.sh` 경로는 사용자 명령 호환을 위해
Python 모듈로 `exec`하는 얇은 래퍼로 유지합니다.

서버 Release에는 같은 Python 소스에서 결정적으로 만든 `g7mb-harness.pyz`를 포함합니다. 따라서
설치 후 저장소 checkout 없이도 다음 검증 명령을 실행할 수 있습니다.

```bash
/usr/local/share/g7mediabooster/gnuboard7/verify-gnuboard7-media-contract.sh \
  /path/to/gnuboard7 --runtime
```
