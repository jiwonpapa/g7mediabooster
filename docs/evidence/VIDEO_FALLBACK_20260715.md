# MP4/H.264 OpenH264 폴백 증거 — 2026-07-15

## 결과

- 판정: PASS
- 하네스: `cargo xtask native-smoke`
- 주 경로: FFprobe 검증 후 FFmpeg fast seek, 실패 시 bounded exact seek 1회
- 폴백 진입: FFmpeg 실행 파일이 없어 `spawn` 자체가 실패한 경우만
- 폴백 범위: MP4 컨테이너의 H.264 video track 1개, 첫 유효 frame

## 실제 주입

하네스가 FFmpeg로 320×180 H.264 MP4 fixture를 만든 뒤 썸네일 명령에는 존재하지 않는
`--ffmpeg-bin` 경로를 전달했습니다. Rust `mp4` parser가 sample과 SPS/PPS를 demux하고,
OpenH264가 첫 frame을 decode한 뒤 임시 PPM을 기존 libvips JPEG 경로로 재가공했습니다.

검증 결과:

- JPEG 출력 비어 있지 않음
- 출력 폭 160px 이하
- 출력 3 bands
- 임시 `fallback.ppm` 삭제
- HEVC/AV1, MOV, WebM은 worker allowlist에서 폴백 flag를 받지 않음

## 독립 하드 한도

| 항목 | 한도 |
|---|---:|
| MP4 입력 | 512 MiB |
| 검사 sample | 120개 |
| 개별 sample | 16 MiB |
| decoder 전달 합계 | 32 MiB |
| sample당 NAL | 1,024개 |
| SPS/PPS 합계 | 256 KiB |
| frame 한 변 | 4,096 px |
| frame 총 픽셀 | 16 MP |
| 허용 decode error | 8개 |

OpenH264는 native C codec이므로 Rust 안전성으로 보호된다고 주장하지 않습니다. 폴백도
자격 증명·네트워크가 없는 sandbox, worker process timeout, native thread 제한과 cgroup
CPU/RSS/PID 제한 안에서만 실행합니다.
