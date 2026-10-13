# ADR: 공식 소스로 고정하는 테스트용 MinIO

## 결정

공개 MinIO registry image의 digest와 release tag가 HTTP 401을 반환하고 공식 바이너리
다운로드가 HTTP 410을 반환하므로, 세 저장소 검증 경로에서 같은 Dockerfile을 빌드합니다.
공식 upstream commit, 다운로드 SHA-256, 공식 Go base image digest를 고정합니다.
멀티 아키텍처 base를 사용하며 빌드 병렬도는 2로 제한합니다.

## 범위와 대안

제품 runtime dependency가 아니라 기존 MinIO 테스트 fixture의 공급 경로 변경입니다.
새 모니터나 중복 E2E를 추가하지 않으며 세 기존 gate와 assertion을 그대로 유지합니다.
불명확한 third-party mirror, mutable latest tag, 저장소 테스트 생략은 선택하지 않았습니다.
upstream은 2026-04-25 archive되었으므로 운영 스토리지로 추천하거나 배포하지 않습니다.

## 비용과 안전

최초 실행은 Go build 비용이 추가되지만 후속 실행은 Docker layer cache를 재사용합니다.
CI는 GitHub-hosted 일회성 runner에서 실행하며 테스트 port는 loopback에만 노출합니다.
AGPL license와 upstream revision을 이미지에 포함합니다. native 운영 파일·키에는 접근하지 않습니다.
