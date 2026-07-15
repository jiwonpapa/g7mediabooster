# AVIF decoder memory boundary evidence

- 실행일: 2026-07-15
- 결과: PASS
- 명령: `cargo xtask heavy-avif`
- 장비: Apple M4 Pro, 48 GiB RAM, macOS 26.5.2 arm64
- 런타임: Rust 1.96.0, libvips 8.18.3, native thread 1

## 발견과 정책 결정

초기 16,000×12,500 AVIF(정확히 200MP) thumbnail 실측은 process tree peak RSS
3,635,216 KiB로 2GiB worker cgroup을 초과했습니다. AVIF/HEIF decoder는 JPEG의
shrink-on-load 경로와 달리 큰 full frame을 materialize하므로, 같은 200MP 한도를 계속
허용하면 정상적인 입력도 OOM kill 또는 전체 worker 장애를 일으킵니다.

따라서 v1 hard policy를 다음처럼 확정했습니다.

- JPEG/PNG/GIF/WebP: 기본 최대 200MP 정책 유지
- AVIF/HEIF: decoder-aware 최대 64MP
- AVIF/HEIF 64MP 초과: libvips full-pixel transform 전에 header probe 단계에서 거부
- 큰 축이 필요한 사진: 25,000×4,000 JPEG(100MP) heavy lane 증거를 별도로 유지

최신 포맷을 무제한 허용하는 것보다 서버 생존과 예측 가능한 자원 상한을 우선한 결정입니다.

## 최종 게이트 결과

| 항목 | 결과 |
|---|---:|
| 허용 fixture | 8,000×8,000 AVIF, 64,000,000 pixels |
| 파생물 | 1,280×1,280 metadata-stripped JPEG |
| transform elapsed | 384ms |
| process tree peak RSS | 1,221,776 KiB |
| gate RSS 상한 | 1,572,864 KiB |
| 거부 fixture | 16,000×12,500 AVIF, 200,000,000 pixels |
| 거부 지점 | header probe 뒤, full-frame transform 전 |

실행 결과는 `reports/heavy-avif.json`에도 machine-readable 형식으로 저장됩니다. 검정색
synthetic fixture이므로 압축률이나 처리시간의 최악값을 대표하지 않지만, decoder 메모리
경계와 fail-closed 정책은 실제 AVIF encode/decode로 검증합니다.
