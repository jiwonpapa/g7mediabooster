# ADR 0006: 릴리스와 기능 상태 거버넌스를 Python으로 검증한다

- 상태: Accepted
- 날짜: 2026-08-18

## 결정

서버와 G7 모듈의 Keep a Changelog 1.1.0·Semantic Versioning 2.0.0 계약과 기능별 구현·검증·
공개 상태를 dependency-free Python 하네스로 검증합니다. `deploy/official-features-v1.json`이
기능 상태의 기계 판독 정본이며 CI, package, release workflow가 같은 검증기를 호출합니다.

상태는 `IMPLEMENTED`, `VERIFIED_LOCAL`, `VERIFIED_PROVIDER`, `RELEASED`, `BLOCKED`,
`EXCLUDED`만 허용합니다. 실제 공개 가능한 상태는 검증 증거가 있는 `VERIFIED_LOCAL`,
`VERIFIED_PROVIDER`, `RELEASED`뿐입니다. manifest 버전·호환 조건과 evidence 경로가 어긋나면
릴리스를 fail-closed 합니다.

Python 하네스는 3,566줄, Bash는 1,593줄, 합계 5,159줄로 즉시 다시 고정합니다. 신규 Python
모듈은 300줄 상한을 유지하며 Rust 제품 build graph에는 의존성을 추가하지 않습니다.

## 이유

- 구현 완료와 실제 공급자 검증, 정식 배포는 서로 다른 증거입니다.
- 문서별 수기 목록은 R2 검증 상태나 G7 호환 버전이 쉽게 어긋납니다.
- 표준 라이브러리만 사용하는 Python 검증은 Rust 재빌드 없이 빠르게 실패합니다.

## 영향

- 보안: 검증되지 않은 기능과 호환되지 않는 G7 모듈 배포를 차단합니다.
- 성능: CI에서 작은 JSON·manifest·증거 경로만 읽습니다.
- 호환성: 서버와 모듈은 독립 SemVer를 유지합니다.
- 운영: 과거 날짜 증거는 수정하지 않고 현재 판정만 기능 상태 정본에서 갱신합니다.

## 철회 조건

검증기가 실제 릴리스 정책보다 복잡해지거나 개발 속도를 낮추면 동일 fixture를 유지한 더 작은
구현으로 교체하고 ratchet을 낮춥니다.
