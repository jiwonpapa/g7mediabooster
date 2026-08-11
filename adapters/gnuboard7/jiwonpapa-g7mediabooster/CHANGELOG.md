# Changelog

Gnuboard 7 모듈의 모든 주목할 만한 변경 사항을 이 파일에 기록합니다.

형식은 [Keep a Changelog 1.1.0](https://keepachangelog.com/ko/1.1.0/)을 따르며,
모듈 버전은 [Semantic Versioning 2.0.0](https://semver.org/lang/ko/)을 따릅니다.

## [Unreleased]

### Changed

- 모듈 릴리스 이력을 Keep a Changelog 형식으로 정본화하고 GitHub Release 노트와 연결했습니다.

## [0.4.3] - 2026-07-16

### Changed

- upstream patch 기준을 공개 Gnuboard7 main `fcaacad`와 `sirsoft-board` 1.0.2로 옮겼습니다.

### Fixed

- patch 적용 후 `sirsoft-board` 1.1.0 capability를 검사해 로컬 미공개 선행 commit 의존성을 제거했습니다.

## [0.4.2] - 2026-07-16

### Security

- G7 활성화 시 versioned 보안 첨부 capability, PHP 메서드 시그니처와 사용자·관리자 layout target을 검증합니다.
- patch `0006`이 게시하는 `sirsoft-board.secure-external-attachments` 계약이 없거나 불완전하면 활성화를 fail-closed 합니다.

## [0.4.1] - 2026-07-16

### Fixed

- 설치 직후 G7 제어 endpoint 기본값을 Rust API/CUI의 `http://127.0.0.1:8088`과 일치시켰습니다.
- G7 관리자 파일 설치가 받는 재현 가능한 ZIP과 SHA-256을 정식 산출물로 추가했습니다.
- 모노레포 전체를 모듈로 오인하는 GitHub 자동업데이트 URL은 전용 배포 저장소가 생길 때까지 제거했습니다.

## [0.4.0] - 2026-07-16

### Added

- MP4에 더해 MOV/H.264를 실제 runtime·worker·private delivery fixture로 검증하고 업로더에 공개했습니다.
- 관리자 설정의 수동 UUID 입력을 본인 소유 Ready JPEG·PNG·WebP 자산 선택기로 교체했습니다.

### Changed

- capability 응답의 `video_inputs`로 MP4/MOV 런타임 가용성을 관리자 화면에 보고합니다.

### Security

- 워터마크 후보를 최근 7일·16MiB 이하로 제한하고 session·native attachment 소유권과 collection 메타데이터를 다시 검증합니다.
- 타인 소유·AVIF·과대·삭제·동영상·잘못된 collection 자산 제외를 실제 G7 DB에서 검증합니다.
- 관리자 설정 API도 catalog 밖 UUID를 `422`로 거부해 UI 우회를 차단합니다.

### Fixed

- 실제 G7 관리자 브라우저에서 워터마크 선택·저장·재로드·rollback을 확인했습니다.

## [0.3.1] - 2026-07-16

### Security

- 비밀글·블라인드글·삭제글 첨부의 직접 전달 경로가 게시글 공개 정책을 우회하지 못하도록 강화했습니다.
- G7 upstream 보안 패치 `0005`와 28항목 계약 검증기를 추가했습니다.
- 게시글 삭제·복원·보존기간 만료·lease 재확인 경로를 실제 G7 DB에서 검증합니다.

## [0.3.0] - 2026-07-15

### Added

- 사용자·관리자 게시글 폼에 업로더를 주입하고 완료된 native attachment ID를 기존 저장 계약에 연결합니다.
- G7 soft delete와 복원을 고려한 보존기간·lease 기반 원격 삭제 대조를 추가했습니다.

### Changed

- 전송 중 submit을 차단하고 최대 100개 attachment ID를 중복 없이 병합합니다.
- upstream 첨부 계약을 `sirsoft-board >=1.2.0` 기준으로 재배치하고 사용자·관리자 FormRequest를 분리 검증합니다.

### Security

- 원격 삭제 시작 전 복원은 예약을 취소하고 시작 후 복원은 fail-closed 합니다.
- 최초 활성화 전에 저장 설정이 없어도 안전한 disabled 기본값으로 부팅합니다.
- 관리자 업로더 확장을 G7 모듈 접두사가 붙은 게시판 layout에만 등록합니다.

## [0.2.0] - 2026-07-15

### Added

- Ready master·thumbnail 전건 검증과 DB lock 기반 native attachment 멱등 생성을 추가했습니다.
- G7 게시글 권한을 재사용하는 private master·thumbnail/poster redirect를 추가했습니다.

### Changed

- 100개 direct upload 전송과 Ready 확인을 분리하고 control 요청을 초당 8개로 제한했습니다.
- MOV/WebM은 release 검증 전 사용자 업로더 지원 형식에서 제외했습니다.

### Security

- 원본 파일명은 G7 내부에만 보관하며 보안 첨부 계약이 없으면 설치·runtime을 fail-closed 합니다.

## [0.1.0] - 2026-07-15

### Added

- G7 관리자 설정, HMAC control client, upload ownership, 100개 direct single/multipart uploader를 추가했습니다.

[Unreleased]: https://github.com/jiwonpapa/g7mediabooster/compare/7df24322045806f9b3d6eac002d74fe2b2006126...HEAD
[0.4.3]: https://github.com/jiwonpapa/g7mediabooster/commit/7df24322045806f9b3d6eac002d74fe2b2006126
[0.4.2]: https://github.com/jiwonpapa/g7mediabooster/commit/72e381796d9cebb6864673d71aa145767729a81d
[0.4.1]: https://github.com/jiwonpapa/g7mediabooster/commit/f5ac953fe881f1f038094217280c3aaeb884612d
[0.4.0]: https://github.com/jiwonpapa/g7mediabooster/commit/a8967a97c99c8c469a545e8e50c6b199c9e27f34
[0.3.1]: https://github.com/jiwonpapa/g7mediabooster/commit/b0b642983063de8a52d5c77d471cbc84c6607886
[0.3.0]: https://github.com/jiwonpapa/g7mediabooster/commit/4e225145d5c85aedb8e049dca6efce6a1410094d
[0.2.0]: https://github.com/jiwonpapa/g7mediabooster/commit/b88b63cd41a5932f3ee1d1a9cd535543cc4c866a
[0.1.0]: https://github.com/jiwonpapa/g7mediabooster/commit/4d9d506
