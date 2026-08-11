# ADR 0004: CHANGELOG·SemVer 릴리스 거버넌스를 Python으로 검증한다

- 상태: Accepted
- 날짜: 2026-08-11

## 결정

서버와 G7 모듈의 Keep a Changelog 1.1.0·Semantic Versioning 2.0.0 계약을 dependency-free
Python 모듈로 검증합니다. 기존 Python-first 하네스 CLI와 CI가 이를 호출하고 tag workflow는
동일 코드로 manifest·tag를 검사하고 release note를 추출합니다.

Python 하네스는 3,182줄에서 3,425줄로 243줄 증가하며 Bash는 1,636줄로 유지합니다. 합계
ratchet은 5,061줄로 즉시 다시 고정합니다. 신규 모듈은 300줄 상한 안에 두고 기존 배포
workflow의 중복된 버전 검사는 다음 변경에서 실제 중복이 확인될 때만 제거합니다.

## 이유

- CHANGELOG 문법, ISO 날짜, SemVer precedence, 두 제품 manifest drift는 문자열 몇 줄 검사로
  안전하게 다룰 수 없고 fixture 단위 회귀가 필요합니다.
- Bash 또는 workflow inline script를 늘리면 이미 정한 Python-first 소유권과 어긋납니다.
- 새 외부 dependency 없이 Python 표준 라이브러리만 사용해 제품 Rust build graph를 늘리지
  않습니다.

## 영향

- 보안: 릴리스 출처와 변경 고지를 일치시키며 secret·network 권한은 추가하지 않습니다.
- 성능: CI에서 수 밀리초의 파일 파싱만 추가하고 Rust 재빌드는 추가하지 않습니다.
- 호환성: 기존 서버·G7 버전과 이력을 보존하며 앞으로의 drift만 fail-closed 합니다.
- 대체안: 문서만 추가하거나 GitHub 자동 release note를 유지하는 안은 정본과 자동 검증이 없어
  기각했습니다. 새 Rust crate/의존성을 추가하는 안은 빌드 속도와 책임 경계 때문에 기각했습니다.

## 철회 조건

Python 검증이 표준 형식을 과도하게 확장하거나 release workflow와 중복되어 개발 속도를
떨어뜨리면, 같은 fixture를 유지한 더 작은 구현으로 교체하고 ratchet을 낮춥니다.
