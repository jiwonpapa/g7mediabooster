# ADR-0005: native 자원 게이트의 정본은 Python이다

- 상태: 승인
- 날짜: 2026-07-22

## 상황

25,000px JPEG, 64MP AVIF, 100개 JPEG 부하 스크립트가 프로세스 트리 RSS 계산, timeout,
임시 경로 정리, JSON 증거 생성을 Bash로 각각 복제했습니다. timeout 시 직계 프로세스만 종료해
자식이 남을 수 있었고, 같은 결함을 세 파일에서 따로 고쳐야 했습니다.

## 결정

1. 상태·분기·프로세스·증거 수집의 정본을 dependency-free Python 모듈로 이관합니다.
2. 기존 Bash 경로와 `cargo xtask` 명령은 Python 모듈로 `exec`하는 6줄 래퍼로 유지합니다.
3. child는 새 process group으로 시작하고 timeout·예외 때 그룹 전체를 종료한 뒤 wait합니다.
4. RSS는 전체 자식 트리, 임시 디스크는 격리 runtime만 측정합니다.
5. Python 파일은 300줄 상한을 지키고 unit·Ruff·Mypy strict·실 native gate를 통과해야 합니다.
6. 이관 시 Python 총량 증가는 Bash 감소량보다 작아야 하며 합계 감소 후 세 상한을 재고정합니다.

## 결과

Bash는 2,137줄에서 1,636줄, Python은 2,736줄에서 3,182줄로 바뀌며 합계는 4,873줄에서
4,818줄로 55줄 감소합니다. 세 명령의 출력·환경변수·JSON schema는 유지하면서 공통 자원 제한과
프로세스 그룹 정리를 한 구현에서 회귀 테스트할 수 있습니다.
