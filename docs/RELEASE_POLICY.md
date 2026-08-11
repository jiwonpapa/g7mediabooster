# 릴리스·버전 정책

## 기준

G7MediaBooster는 [Keep a Changelog 1.1.0](https://keepachangelog.com/ko/1.1.0/)과
[Semantic Versioning 2.0.0](https://semver.org/lang/ko/)을 따릅니다. 커밋 로그나 GitHub 자동 생성
노트는 변경 이력의 정본이 아닙니다. 사람이 작성한 제품별 `CHANGELOG.md`가 정본이며 GitHub
Release 노트도 해당 버전 섹션에서 생성합니다.

## 독립 버전 제품

| 제품 | 버전 정본 | 변경 이력 | 태그 |
|---|---|---|---|
| Rust 서버·설치 번들 | `Cargo.toml`의 `workspace.package.version` | `/CHANGELOG.md` | `server-vX.Y.Z` |
| Gnuboard 7 모듈 | `module.json` | 모듈 디렉터리의 `CHANGELOG.md` | `g7-module-vX.Y.Z` |

G7 모듈의 `package.json`과 `deploy/official-features-v1.json` 버전은 `module.json`과 같아야
합니다. Gnuboard 5 어댑터는 현재 서버 번들에 포함되며 별도 공개 릴리스 계열이나 태그를 갖지
않습니다. 별도 배포를 시작할 때 독립 CHANGELOG·manifest·tag workflow를 먼저 추가합니다.

태그의 `v`는 Git tag 접두사일 뿐 SemVer 문자열 일부가 아닙니다. 배포한 버전의 내용은
수정하지 않고 모든 변경을 새 버전으로 배포합니다.

## 공개 API

SemVer 판정 대상 공개 API는 다음과 같습니다.

- 생성 OpenAPI의 endpoint, schema, 오류 코드와 인증 계약
- `g7mbctl` 명령·옵션, TOML 환경 설정, 설치 경로와 운영 상태 계약
- G5/G7 HMAC·attachment·module capability 계약
- 공식 지원으로 게시한 입력 포맷, 파생물, storage provider 동작

workspace 내부 Rust crate API, 비공개 migration 구현, 테스트 helper는 외부 공개 API가
아닙니다. 다만 저장 데이터나 운영 업그레이드 호환성을 깨뜨리면 공개 API 변경과 같은 수준으로
판정합니다.

## 버전 상승

| 변경 | 1.0.0 이후 | 현재 0.y.z 단계 |
|---|---|---|
| 호환되는 버그·보안 수정 | PATCH | PATCH |
| 호환되는 기능 추가·기능 폐기 예고 | MINOR | MINOR |
| 공개 API 비호환 변경·기능 제거 | MAJOR | MINOR, 변경점과 migration 필수 |

정식 배포 전 버전은 `X.Y.Z-rc.1` 형태로 쓰며 빌드 메타데이터는 재현 증거에만 사용합니다.
버전 상승이 애매하면 더 큰 영향을 나타내는 쪽을 선택합니다.

## CHANGELOG 규칙

- 가장 위에 날짜 없는 `[Unreleased]`를 둡니다.
- 릴리스는 최신순으로 `## [X.Y.Z] - YYYY-MM-DD` 형식을 사용합니다.
- 사용자에게 중요한 항목만 `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`로
  묶습니다. 빈 분류는 만들지 않습니다.
- 모든 릴리스와 `[Unreleased]`에 비교 또는 tag/commit 링크를 둡니다.
- 철회한 릴리스는 삭제하지 않고 제목 끝에 `[YANKED]`를 붙입니다.

## 배포 순서

1. 개발 중 주목할 변경을 해당 제품의 `[Unreleased]`에 기록합니다.
2. 공개 API 영향을 기준으로 다음 SemVer를 결정합니다.
3. manifest 버전을 맞추고 `[Unreleased]` 항목을 새 버전·ISO 날짜 아래로 이동합니다.
4. `python3 -m tools.harness.g7mb_harness release-policy`와 제품 전체 게이트를 통과시킵니다.
5. `server-vX.Y.Z` 또는 `g7-module-vX.Y.Z` annotated tag를 생성합니다.
6. 태그를 push하면 workflow가 버전·CHANGELOG·annotated tag를 재검증하고 CHANGELOG에서
   GitHub Release 노트를 추출합니다.

릴리스 검증만 실행하거나 노트를 미리 확인할 수 있습니다.

```bash
python3 -m tools.harness.g7mb_harness release-policy
python3 -m tools.harness.g7mb_harness release-policy --tag server-v0.1.1
python3 -m tools.harness.g7mb_harness release-notes server-v0.1.1
```
