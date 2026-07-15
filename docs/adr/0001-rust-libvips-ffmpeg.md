# ADR-0001: Rust 제어 계층, libvips 이미지, FFmpeg 영상

- 상태: Accepted
- 날짜: 2026-07-15

## 결정

G7MediaBooster의 API, 상태, 저장소, 인증, 작업 orchestration은 Rust 2024로 구현합니다.
이미지 decode/resize/encode는 libvips, MP4/MOV frame 추출은 FFmpeg CLI가 주로 담당합니다.
FFmpeg 프로세스를 시작할 수 없는 MP4/H.264는 Rust `mp4` demux와 OpenH264 첫 frame
decode로만 제한 폴백합니다. 네이티브 엔진은 API가 아니라 자격 증명 없는 별도 sandbox
프로세스에서 실행합니다.

## 이유

- Rust는 GC/runtime 없이 API와 worker의 메모리·동시성 오류를 컴파일 단계에서 줄입니다.
- 실제 미디어 hot path는 이미 최적화된 native codec과 libvips/FFmpeg가 담당합니다.
- libvips는 demand-driven 처리로 일반적인 전체 이미지 메모리 적재를 줄입니다.
- FFmpeg CLI는 Rust FFI surface를 추가하지 않고 codec/container 지원을 격리할 수 있습니다.
- OpenH264 폴백은 비상 H.264 첫 frame만 담당해 FFmpeg 전체 대체보다 표면이 좁습니다.
- Rust AWS SDK는 S3와 R2 custom endpoint를 같은 port 뒤에서 처리할 수 있습니다.

## 거부한 대안

- Zig 전체 서버: C 연동은 좋지만 hot path가 동일 native engine이므로 전체 이득이 작고,
  API/스토리지 생태계와 안전성 이점이 Rust보다 작습니다.
- Go 전체 서버: 운영은 단순하지만 native FFI와 GC tail-latency 관리 이점이 적습니다.
- PHP 내부 처리: 요청 worker 점유, 메모리 한도, timeout, 장애 격리가 목표와 충돌합니다.
- ImageMagick/GD fallback: 공격 표면과 메모리 비용을 늘려 v1 기본 경로에서 제외합니다.
- FFmpeg Rust FFI: 초기에는 필요 이상의 unsafe와 codec feature surface를 만듭니다.
- 순수 Rust 범용 영상 decoder: v1의 H.264 첫 frame 비상 경로보다 범위와 검증 부담이 큽니다.

## 결과

- Rust끼리의 미세 최적화보다 copy, process spawn, codec preset, concurrency를 먼저 계측합니다.
- native library 취약점은 Rust 안전성으로 해결됐다고 주장하지 않습니다.
- OpenH264는 native C codec이므로 기존 no-network sandbox, timeout/cgroup과 독립 byte·pixel
  한도를 그대로 적용합니다.
- sandbox 배포와 실제 포맷 smoke가 release 필수 조건입니다.
- first-party unsafe가 필요하면 별도 ADR과 헌법 개정이 필요합니다.
