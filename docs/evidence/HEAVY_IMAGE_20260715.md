# 25,000px heavy-image 증거

- 실행일: 2026-07-15
- 결과: PASS
- 명령: `cargo xtask heavy-image`
- 장비: Apple M4 Pro, 48 GiB RAM, macOS 26.5.2 arm64
- 런타임: Rust 1.96.0, libvips 8.18.0

## 정책과 부하 조건

- 실제 25,000×4,000 JPEG, 100,000,000 pixels, 1,563,799 bytes
- fixture SHA-256: `c21969afc41fcd1da1a34dd96c1094fca41d15244edcd1e928686c5b374134fe`
- 16,384px 초과 또는 100MP 초과 이미지는 `heavy` class
- 일반 worker slot 안에서 heavy-image semaphore를 추가 획득하며 기본 동시성은 1
- libvips native thread 1, sequential decode와 shrink-on-load, 1,280px JPEG 출력
- RSS는 shell runner와 descendant sandbox process 합계를 50ms 간격으로 측정
- 상한: process tree peak RSS 1,048,576 KiB, wall time 60초

## 결과

| 지표 | 결과 |
|---|---:|
| header probe | 25,000×4,000 확인 |
| resource class | heavy |
| derivative | 1,280px, 3-band JPEG |
| 총 시간 | 386ms |
| process tree peak RSS | 24,368 KiB |

별도 worker 단위 테스트는 25,000×4,000 probe 두 건을 동시에 실행해도 full-pixel thumbnail
구간의 최대 동시 실행이 1임을 검증합니다. 영상 변환도 별도 semaphore 기본값 1을 사용합니다.

## 증거 경계

검정색 JPEG는 압축과 shrink-on-load에 유리하므로 이 RSS를 최악 입력의 대표값으로 사용하지
않습니다. 이 결과는 25,000px 축 지원, hard pixel 검사, native thread 1, heavy lane 실행 상한을
증명합니다. AVIF decoder 경계는 별도 실측에서 64MP 처리와 200MP full-decode 전 거부로
고정했습니다. 노이즈가 큰 실사진과 혼합 heavy queue는 계속 별도 운영 게이트입니다. Linux
cgroup CPU·memory·PID 상한은 별도 100개 JPEG 부하로 검증했습니다.
