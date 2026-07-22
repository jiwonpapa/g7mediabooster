# ADR-0004: 빌드 피드백과 회귀 ratchet을 제품 구조보다 우선한다

- 상태: 승인
- 날짜: 2026-07-22

## 상황

저장소 명령 라우터인 `xtask`가 OpenAPI 생성을 위해 `g7mb-api`를 직접 의존하면서 단순 하네스
명령도 AWS SDK, SQLx, Axum을 포함한 약 369 package graph를 컴파일했습니다. PR quality job은
`check`, Clippy, `cargo test`, nextest와 doctest를 겹쳐 실행했고 adapter와 coverage도 같은 job에
직렬 배치했습니다. 전체 line coverage 80%만으로는 S3 adapter의 낮은 coverage를 숨길 수
있었습니다.

## 결정

1. `xtask`는 제품 crate를 직접 의존하지 않는 얇은 명령 라우터로 유지합니다.
2. OpenAPI 생성은 API package의 전용 binary가 담당하며 해당 명령에서만 제품 graph를 빌드합니다.
3. Clippy와 같은 surface의 `cargo check`, `cargo test`와 같은 test의 nextest 재실행을 제거합니다.
4. adapter와 coverage는 Rust quality job에서 분리해 병렬 실행합니다.
5. aggregate coverage 80%와 함께 component별 현재 통과값을 하한으로 고정합니다.
6. 신규 수기 source 500줄 상한과 기존 대형 파일·Bash·Python 총량의 비증가 ratchet을 둡니다.
7. 파일 줄 수만 줄이기 위한 새 crate/service/package는 만들지 않습니다.

## 결과

일반 `xtask` 명령의 빌드 graph가 제품 graph와 분리되고 CI 중복 컴파일과 직렬 대기가 줄어듭니다.
기존 대형 파일은 즉시 미관상 분할하지 않지만 더 커질 수 없으며, 변경 충돌이나 테스트 격리가
실제로 필요한 순서대로 같은 crate 안에서 모듈화합니다. Coverage 하한은 테스트 보강 없이 낮출 수
없습니다.
