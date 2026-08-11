# Changelog

이 프로젝트의 모든 주목할 만한 변경 사항을 이 파일에 기록합니다.

형식은 [Keep a Changelog 1.1.0](https://keepachangelog.com/ko/1.1.0/)을 따르며,
서버 버전은 [Semantic Versioning 2.0.0](https://semver.org/lang/ko/)을 따릅니다.

## [Unreleased]

### Added

- Keep a Changelog와 Semantic Versioning 릴리스 정책 및 자동 검증 게이트를 도입했습니다.
- 제어 API와 분리된 서명형 공개 썸네일 listener와 Nginx 경로 제한 설정을 추가했습니다.

### Changed

- 인프라 하네스의 주 구현을 Python으로 옮기고 빌드·coverage 산출물 정리를 자동화했습니다.

### Security

- 공개 썸네일의 immutable preset HMAC 검증과 R2·AWS S3·Lightsail redirect authority 고정을 추가했습니다.

## [0.1.1] - 2026-07-17

### Fixed

- Ubuntu 서버 Release workflow에 누락된 libvips·FFmpeg·archive 전제 패키지를 추가했습니다.
- 서버 `0.1.1`과 일치하도록 생성 OpenAPI 계약을 갱신했습니다.

## [0.1.0] - 2026-07-17

### Added

- Rust API·worker·sandbox와 SQLite lease queue를 포함한 최초 서버 후보 버전을 배포했습니다.
- S3 호환 single/multipart 직접 업로드, libvips 이미지 처리, FFmpeg 및 Rust MP4 썸네일 폴백을 추가했습니다.
- G5·G7 HMAC 제어 연동, 재현 가능한 Linux 설치 번들, 보안·자원 하네스를 제공했습니다.

[Unreleased]: https://github.com/jiwonpapa/g7mediabooster/compare/server-v0.1.1...HEAD
[0.1.1]: https://github.com/jiwonpapa/g7mediabooster/compare/server-v0.1.0...server-v0.1.1
[0.1.0]: https://github.com/jiwonpapa/g7mediabooster/releases/tag/server-v0.1.0
