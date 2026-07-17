# Gnuboard7 모듈 배포와 설치

## 사용자 설치 경로

서버 통합 Release를 설치한 경우 모듈 ZIP은 이미 다음 경로에 있습니다.

```text
/usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip
```

별도 모듈 Release를 쓰는 경우 `g7-module-v<version>`에서 다음 두 파일을 받습니다.

1. 다음 두 파일을 같은 디렉터리에 둡니다.
   - `jiwonpapa-g7mediabooster-<version>.zip`
   - `jiwonpapa-g7mediabooster-<version>.zip.sha256`
2. 같은 디렉터리에서 checksum을 검증합니다.

```bash
sha256sum -c jiwonpapa-g7mediabooster-<version>.zip.sha256
```

macOS는 다음 명령을 사용합니다.

```bash
shasum -a 256 -c jiwonpapa-g7mediabooster-<version>.zip.sha256
```

3. Release에 함께 첨부된 `0001`~`0006` 계약이 반영된 G7인지
   `verify-gnuboard7-media-contract.sh /path/to/gnuboard7`로 확인합니다.
4. G7 관리자에서 **모듈 관리 → 파일에서 설치**를 열고 검증한 ZIP을 업로드합니다.
5. 모듈을 활성화한 뒤 **미디어 부스터** 설정에서 HMAC key ID·secret을 Rust 서비스와 맞춥니다.
6. 같은 서버의 기본 제어 endpoint는 `http://127.0.0.1:8088`입니다. 별도 서버는 HTTPS origin을
   입력합니다.

서버의 API·worker·timer 등록은 G7 관리자가 수행하지 않습니다. 서버 관리자가 Release
번들에서 `sudo ./bin/g7mbctl install`을 한 번 실행하고, G7 관리자는 출력된 ZIP 설치와 HMAC
연결만 수행합니다.

G7의 파일 설치 API는 ZIP만 허용합니다. `tar.gz`는 관리자 설치용이 아니라 서버 수동 배치용입니다.
현재 저장소는 Rust와 PHP를 함께 보관하는 모노레포이므로 G7의 **GitHub에서 설치**에 이 저장소
URL을 직접 넣으면 안 됩니다. 전용 모듈 저장소가 생기기 전에는 Release ZIP을 사용합니다.

## 유지관리자 배포 경로

모듈·공식 기능 manifest 버전을 먼저 일치시키고 전체 게이트를 통과시킵니다.

```bash
cargo xtask g7-adapter
cargo xtask g7-module-package
cargo xtask ci
```

실제 G7 checkout의 관리자 ZIP helper까지 검증하려면 다음처럼 실행합니다. 대상 G7은 변경하지
않고 임시 디렉터리에서 manifest를 읽습니다.

```bash
GNUBOARD7_ROOT=/path/to/gnuboard7 cargo xtask g7-module-package
```

배포는 명시적인 annotated tag만 허용합니다.

```bash
git tag -a g7-module-v0.4.3 -m "Gnuboard7 module 0.4.3"
git push origin g7-module-v0.4.3
```

태그가 `module.json` 버전과 다르면 workflow가 실패합니다. `release_status=candidate`인 동안에는
GitHub prerelease로만 게시하며, 외부 R2/Lightsail과 실 provider 보존 삭제 게이트가 남아 있는
현재 상태에서 stable release로 승격하지 않습니다.
