# Gnuboard 7.1 모듈 배포와 설치

0.5.1 core-free 어댑터는 원본 호스트 회귀와 g7devops.com ZIP 업데이트·브라우저·실 R2 검증을 마쳤습니다.
새 stock 사이트 첫 설치와 전체 인수 게이트 전까지 `gnuboard7_module.publishable=false`이며 ZIP은 preview입니다.
코어·게시판·기본 템플릿 patch는 설치 산출물에 넣지 않습니다.

2026-10-13 첨부 UI·보존 정책·보안 의존성 개선은 소스 검증 단계이며 운영 배포·재 E2E 전입니다.
[최신 검증 보고서](evidence/G7_E2E_FIXES_20261013.json)를 별도로 확인하십시오.

## 두 구성의 설치 경계

| 구성 | 설치 담당·방법 | 설치하지 않는 것 |
|---|---|---|
| Rust 백엔드 | 서버 관리자, Linux 서버 Release 번들 | G7 모듈 자동 등록·활성화 |
| G7 연동 모듈 | G7 관리자, 전용 모듈 ZIP | Rust 데몬·libvips·FFmpeg·systemd |

현재 이미지 처리에는 libvips가 필수이며 Rust 이미지 폴백은 미구현입니다.
서버 관리자가 런타임을 먼저 설치하는 경로를 권장하며, 선택적 설치 위임과 정확한 패키지 목록은
[Linux 서버 설치 안내](SERVER_INSTALL.md)를 따릅니다. PHP에 root/sudo 권한을 주지 않습니다.

## 사용자 경로

1. 서버 관리자가 libvips·FFmpeg를 설치하고 Rust 서버 Release 번들을
   `sudo ./bin/g7mbctl install --skip-dependency-install`로 설치합니다.
2. `g7mbctl setup`에서 R2/S3 자격증명과 서버 설정을 입력합니다. 운영 중 재실행할 수 있습니다.
3. 전용 모듈 ZIP과 `.zip.sha256`을 받은 뒤 checksum을 확인합니다.
4. 설치된 G7와 게시판의 공개 계약을 검증합니다. 현재 검증 기준은 G7 7.1.0과 게시판 1.2.0이며,
   후속 버전은 버전 문자열만으로 지원을 보장하지 않습니다.
5. G7 관리자 **모듈 관리 → 파일에서 설치**로 ZIP 설치·활성화합니다.
6. 관리자 **미디어 부스터**에서 endpoint·HMAC key ID/secret을 맞추고 업로드 사용을 켭니다.

```bash
sha256sum -c jiwonpapa-g7mediabooster-0.5.1.zip.sha256
# macOS: shasum -a 256 -c jiwonpapa-g7mediabooster-0.5.1.zip.sha256
scripts/verify-gnuboard7-media-contract.sh /path/to/gnuboard7
```

통합 서버 번들의 모듈 ZIP은 `/usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip`입니다.
같은 위치의 `jiwonpapa-g7mediabooster.zip.sha256`도 함께 복사하여 파일명에 맞는 checksum을 검사합니다.
같은 서버 제어 endpoint는 `http://127.0.0.1:8088`, 별도 서버는 HTTPS origin입니다.
PHP에는 R2/S3 자격증명을 입력하지 않습니다.
모노레포 URL로 G7 **GitHub에서 설치**를 사용하지 마십시오. 전용 ZIP을 사용합니다.

자동 설치 하네스는 모듈 디렉터리만 설치하고 코어·게시판·템플릿 원본을 해시로 대조합니다.
원본 변경이 감지되면 실패하며 원본을 덮어써서 복원하지 않습니다.
설치 실패 rollback도 자체 모듈만 제거합니다.

## 적용·해제

기존 서버 하네스의 대상 확인을 반드시 명시합니다. 기본 SSH 별칭은 `g7devops`입니다.

```bash
scripts/g7-live-control.sh preflight
scripts/g7-live-control.sh status
scripts/g7-live-control.sh apply --confirm g7devops
scripts/g7-live-control.sh disable --confirm g7devops
```

`apply`는 서버 ready 후 모듈 설정의 업로드 사용을 켜고 실제 활성 템플릿의 form/partial mount를 확인합니다.
`disable`은 `enabled=false`로 **신규 MediaBooster 업로드만 해제**합니다. 원본 uploader가
복원되고 기존 원격 첨부를 전달하기 위해 모듈과 Rust API는 유지합니다. 자격증명·DB·객체는 삭제하지 않습니다.

전체 모듈 제거·비활성화는 원격 첨부를 먼저 이관하거나 정상 삭제한 뒤 가능합니다.
기존 첨부가 남아 있으면 차단합니다. 새 `receipt.json` 기반 rollback만 지원하며 과거
core-patch 배포의 native 파일 복원은 이 하네스에서 수행하지 않습니다.
다른 서버는 `G7MB_SSH_HOST`, `G7MB_G7_ROOT`, `G7MB_G7_USER`를 지정합니다.

## 유지관리자 게이트

PHP/TypeScript 변경 검증은 Rust 전체 재빌드 없이 수행할 수 있습니다.

```bash
cd adapters/gnuboard7/jiwonpapa-g7mediabooster
npm ci
npm run typecheck
npm test
npm run build
G7MB_TEST_HOST_ROOT=/path/to/stock-g7 /path/to/stock-g7/vendor/bin/phpunit -c phpunit.xml --do-not-cache-result
```

저장소 루트에서 별도 `_testing` DB를 준비한 원본 호스트 게이트를 실행합니다.

```bash
scripts/g7-host-security-gate.sh /path/to/stock-g7
python3 -m tools.harness.g7mb_harness feature-status
python3 -m tools.harness.g7mb_harness release-policy
```

GitHub CI는 공식 7.1.0 SHA를 고정해 위 테스트와 원본 source diff를 검사합니다.
정식 module tag 및 사용자 배포는 남은 인수 게이트와 `publishable=true` 전환 뒤에만 허용합니다.
g7devops.com 승인 canary 적용·해제와 기존 R2 재사용 검증은 완료했습니다.
[배포·검증 보고서](evidence/G7DEVOPS_CORE_FREE_R2_E2E_20261012.md)의 남은 게이트를 따릅니다.
