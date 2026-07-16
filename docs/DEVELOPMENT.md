# 개발과 검증

## 요구 환경

- Rust 1.96.0 (`rust-toolchain.toml`이 자동 고정)
- pkg-config, libvips 8.15+ (운영 참조 이미지 8.18.x), FFmpeg 8 계열
- sandbox의 `native-vips` feature는 Rust `mp4` parser와 source-built OpenH264 폴백도
  함께 고정
- 선택 도구: nextest, llvm-cov, cargo-audit, cargo-deny, cargo-fuzz

## 일상 명령

```bash
cargo xtask quick          # fmt, check, clippy, test, doc
cargo xtask rustdoc        # 공개 문서 누락, 링크, 코드블록, HTML 등 rustdoc lint 강제
cargo xtask ci             # quick + OpenAPI drift + bench compile
cargo xtask supply-chain   # advisory, license, source 검사
cargo xtask native-smoke   # AVIF/HEIF, MP4/MOV, FFmpeg 부재 OpenH264 폴백
cargo xtask api-smoke      # 실제 binary health/header smoke
cargo xtask full-stack-smoke # MinIO direct upload -> 실제 worker -> private derivative GET
cargo xtask g7-policy-smoke # G7 PHP HMAC policy -> Rust worker watermark -> rollback
cargo xtask large-multipart-smoke # 로컬 정확한 5GiB direct multipart + API RSS gate
cargo xtask g7-adapter     # G7 PHP/TypeScript unit, typecheck, production build
cargo xtask g7-module-package # 재현 가능한 G7 설치 ZIP·tar.gz + 각각의 SHA-256
cargo xtask load100        # 실제 JPEG 100건, RSS 상한, 만료 lease 복구
cargo xtask heavy-image    # 25,000px JPEG, heavy lane, RSS 상한
cargo xtask heavy-avif     # 64MP AVIF 처리, 200MP full-decode 전 거부, RSS 상한
cargo xtask live-storage-conformance # 환경값이 있는 실제 R2/Lightsail/AWS S3만; PREFLIGHT_ONLY 지원
cargo xtask database-recovery # online backup·hash·retention·격리 restore rehearsal
cargo xtask cgroup-smoke   # Linux CPU/memory/PID 한도와 API 생존
cargo xtask coverage       # line coverage 80% 하한
cargo xtask bench --no-run # 벤치 하네스 컴파일
cargo xtask fuzz --seconds 60
cargo xtask miri
cargo xtask sbom           # CycloneDX JSON
cargo xtask native-inventory
cargo xtask openapi check  # 계약 drift 확인
cargo xtask openapi write  # 의도적 계약 갱신
```

## 정본 우선순위

실행 가능한 소스 코드와 코드에서 생성한 계약이 최종 정본입니다. Rust 공개 API의 의미와
제약은 소스 안의 rustdoc, HTTP는 생성 OpenAPI, DB는 migration, G5/G7 연동은 어댑터 소스와
contract test를 따릅니다. Markdown 문서는 설명용 snapshot이며 충돌 시 코드에 맞춰 같은
변경에서 고칩니다.

모든 workspace crate는 `[lints] workspace = true`를 선언해야 합니다. `cargo xtask rustdoc`은
이 상속 여부부터 검사한 뒤 `missing_docs = deny`, `rustdoc::all = deny`, 전체 feature 문서
빌드를 실행합니다. `cargo xtask quick`과 CI도 이 게이트를 반드시 통과합니다.

PR CI는 고정 버전 `cargo-nextest`를 설치한 뒤 `cargo xtask nextest`를 실행합니다. doctest는
nextest 대상이 아니므로 하네스가 별도 `cargo test --doc`도 함께 실행합니다.

## 테스트 계층

- domain: 상태 전이, object key, limit 경계와 proptest
- auth: canonical string, tamper, stale timestamp
- persistence: migration, unique idempotency, lease 복구
- API: health, body limit, security headers, OpenAPI snapshot
- storage: S3/R2 custom endpoint와 presign 계약
- native: JPEG/WebP/AVIF/HEIF 실제 왕복, MP4/MOV FFmpeg와 FFmpeg 부재 OpenH264 frame 추출
- failure: timeout kill/reap, 손상 입력, disk full, 중복 complete

## Coverage

활성 control-plane library(`api`, `auth`, `config`, `domain`) line 80%를 초기 기준으로
삼습니다. 아직 실행 유스케이스가 없는 조립 binary와 adapter를 숫자에 섞어 기준을 왜곡하지
않습니다. 기능이 구현되는 crate는 같은 변경에서 coverage 대상에 편입합니다. 생성 OpenAPI와
외부 binding만 제외하며 first-party native wrapper는 제외하지 않습니다. 기준은 낮출 수 없고
검증 가능한 코드가 늘면 상향합니다.

## 벤치마크 기록

벤치 결과에는 다음을 같이 남깁니다.

- git commit, Rust/libvips/FFmpeg/codec version
- CPU, RAM, OS/container digest
- fixture SHA-256와 preset version
- concurrency와 native thread 값
- p50/p95/p99, throughput, peak RSS, temp disk

언어 또는 알고리즘이 빠르다는 결론은 같은 native build와 fixture를 사용한 비교만 허용합니다.

## 새 dependency 체크리스트

1. 표준 라이브러리나 기존 dependency로 해결 가능한지 확인
2. 최소 feature만 선택
3. MSRV, maintenance, advisory, license, transitive 수 확인
4. 비밀/네트워크/native 권한 증가 여부 확인
5. `cargo deny`, `cargo audit`, `cargo tree -e features` 결과 첨부
