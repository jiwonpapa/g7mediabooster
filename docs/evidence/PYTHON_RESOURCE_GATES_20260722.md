# Python native 자원 게이트 이관 증거

- 판정: PASS
- 날짜: 2026-07-22
- 기준 환경: macOS arm64, Rust 1.96.0, Python 3.14.6, libvips 8.18.3, FFmpeg 8.1.2

## 변경 결과

| 항목 | 이전 | 이후 |
|---|---:|---:|
| `heavy-image.sh` | 154줄 Bash 정본 | 6줄 Python 호환 래퍼 |
| `heavy-avif.sh` | 180줄 Bash 정본 | 6줄 Python 호환 래퍼 |
| `load-100.sh` | 185줄 Bash 정본 | 6줄 Python 호환 래퍼 |
| Bash 전체 | 2,137줄 | 1,636줄 |
| Python 전체 | 2,736줄 | 3,182줄 |
| Bash+Python | 4,873줄 | 4,818줄 |

공통 Python 실행기는 shell mode 없이 인자 배열로 child를 실행합니다. 각 child를 별도 process
group으로 만들고 timeout·예외 때 그룹 전체를 종료한 뒤 wait합니다. RSS는 전체 자식 트리,
100개 부하의 임시 디스크는 격리 runtime 경로만 측정합니다.

## 실제 native 검증

| 명령 | 결과 | 실측 |
|---|---|---|
| `cargo xtask heavy-image` | PASS | 25,000×4,000 JPEG, 750ms, peak RSS 10,848KiB |
| `cargo xtask heavy-avif` | PASS | 64MP AVIF 503ms, peak RSS 1,215,424KiB, 200MP 사전 거부 |
| `cargo xtask load100` | PASS | 100 jobs, 9,078ms, 11.01/s, p95 470ms, RSS 576,192KiB, temp 8,506KiB |

보고서 schema와 기존 명령·환경변수는 유지했습니다. Python 이관 중 발견한 sandbox probe의 중첩
`probe.width`/`probe.height` 계약은 unit regression test로 고정했습니다.

## 정적·회귀 게이트

- Ruff: PASS
- Mypy strict: PASS
- Pytest/Unittest: PASS
- Bash parse/ShellCheck: PASS
- 하네스 언어별·합계 LOC ratchet: PASS
