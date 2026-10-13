# Linux 서버 통합 설치

G7MediaBooster의 내부 API·worker·예약 작업을 사용자가 각각 설치하지 않도록 서버 Release는
하나의 번들로 배포합니다. 상시 프로세스는 API와 worker 두 개이지만 systemd에서는
`g7mediabooster.target` 하나로 관리합니다. sandbox는 작업별 자식 프로세스이며 cleanup,
inventory, backup은 target에 딸린 timer입니다.

## 설치 전에 확인할 사항

서버 백엔드와 Gnuboard 7 연동 모듈은 별도 구성입니다. **G7 관리자에서 모듈 ZIP만
설치해도 Rust 서버나 OS 라이브러리가 설치되는 것은 아닙니다.** 서버 관리자가 백엔드를
먼저 설치하고 G7 관리자가 모듈을 연결합니다. 모듈에 root·패키지 관리 권한을 주지 않습니다.

- 공식 서버 번들 기준: Ubuntu 24.04, systemd, root/sudo 권한
- architecture: `uname -m`이 `x86_64`이면 `linux-x86_64`, `aarch64`/`arm64`이면 `linux-aarch64`
- **현재 필수 런타임: libvips와 FFmpeg/ffprobe**, AVIF·HEIC용 codec package
- G7 모듈: PHP 8.2+, `mbstring`, 설치된 G7와 게시판의 공개 확장 계약
- 준비할 값: private bucket, 정확한 게시판 HTTPS origin, S3 Access Key ID/Secret Access Key
  (R2는 Account ID도 필요). 비밀값은 채팅·명령행 인자·Git에 넣지 않습니다.

현재 이미지 엔진은 libvips이며 **libvips 미설치 시 Rust 이미지 처리로 자동 전환하지
않습니다.** FFmpeg 실행 불가 시 MP4/H.264 첫 프레임 폴백은 별도 영상 기능이고,
그 결과 이미지 가공에도 현재 libvips가 필요합니다.

### 1. 서버 관리자가 런타임 설치 — 권장 경로

Ubuntu 24.04에서 다음을 실행합니다. 사용자의 OS package 관리 정책을 우선합니다.

```bash
sudo apt-get update
sudo apt-get install --no-install-recommends \
  libvips-tools ffmpeg \
  libheif-plugin-aomdec libheif-plugin-aomenc \
  libheif-plugin-libde265 libheif-plugin-x265
vips --version
ffmpeg -version
ffprobe -version
```

버전 명령 성공만으로 모든 포맷이 지원되는 것은 아닙니다. 설치기의 실제 sandbox
doctor·capability fixture까지 통과해야 합니다. 다른 Linux 배포판은 해당 패키지 관리자로
런타임을 설치하되, 공식 Ubuntu 번들과의 ABI·codec 호환성은 별도로 검증해야 합니다.

### 2. Rust 서버 번들 설치

Release의 Linux architecture용 `.tar.gz`와 `.sha256`을 같은 디렉터리에 받은 뒤 검증합니다.

```bash
sha256sum -c g7mediabooster-server-<version>-linux-<architecture>.tar.gz.sha256
tar -xzf g7mediabooster-server-<version>-linux-<architecture>.tar.gz
cd g7mediabooster-server-<version>-linux-<architecture>
sudo ./bin/g7mbctl install --skip-dependency-install
```

`install` 한 번이 다음을 fail-closed 순서로 수행합니다.

1. systemd Linux, root, 관리 명령과 libvips·FFmpeg 설치 여부 검사
2. payload 일반 파일·크기·정확한 SHA-256 manifest와 실제 sandbox doctor·capability fixture 검사
3. `g7mediabooster` system user/group과 제한된 디렉터리 생성
4. API·worker·sandbox·CLI와 hardened unit을 원자적으로 설치
5. 새 설치에서 `g7mbctl setup` CUI로 R2/S3·exact G7 origin과 root-only 비밀값 설정
6. `g7mediabooster.target` 하나를 활성화하고 API ready를 최대 30초 확인
7. G7 관리자에서 올릴 module ZIP의 설치 경로 출력

`--skip-dependency-install`은 OS package를 변경하지 않습니다. 런타임이 없으면 실패하며
자동 폴백으로 설치를 계속하지 않습니다.

### 선택: 설치기에 OS package 설치 위임

런타임을 직접 설치하지 않은 Ubuntu 24.04 서버에서 `sudo ./bin/g7mbctl install`을 실행하면
이미지·영상 처리 용도를 안내하고 `설치할까요? [Y/n]`를 묻습니다. 명시적 동의 없이
패키지를 설치하지 않으며 거부하면 종료합니다. 이 경로에서도 root/sudo 권한은 필요합니다.

설정과 기동을 분리해야 하는 자동화 환경만 다음 옵션을 사용합니다.

```bash
sudo ./bin/g7mbctl install --skip-dependency-install --skip-setup --skip-start
sudo /usr/local/bin/g7mbctl setup --non-interactive <필수 file-input 옵션>
sudo systemctl enable --now g7mediabooster.target
```

TTY가 없는 자동 배포에서는 묻지 않고 설치하지 않습니다. OS package 설치를 허용할 때만
명시적으로 다음 옵션을 추가합니다.

```bash
sudo ./bin/g7mbctl install --install-dependencies --skip-setup --skip-start
```

OS package를 이미지 빌드 단계에서 관리하는 환경도 권장 경로와 같은
`--skip-dependency-install`을 사용합니다. 다른 Linux에서는 자동 package 설치를 지원하지 않습니다.

기존 파일과 내용이 다르면 설치기는 덮어쓰지 않습니다. 검증된 새 Release로 업데이트할 때만
`--force`를 사용하며 설정과 credential은 교체하지 않습니다.

### 3. Gnuboard 7 모듈 설치·연결

서버 설치기는 모듈 ZIP과 checksum을 다음 디렉터리에 함께 배치합니다. ZIP을 G7에
자동 등록·활성화하지는 않습니다.

```text
/usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip
/usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip.sha256
```

1. ZIP과 checksum을 관리자 PC로 복사하고 checksum을 확인합니다.
2. G7 관리자 **모듈 관리 → 파일에서 설치**에서 ZIP을 설치·활성화합니다.
3. 관리자 **미디어 부스터**에서 endpoint와 서버 setup이 생성한 HMAC key ID/secret을 맞춥니다.
   같은 서버 endpoint는 `http://127.0.0.1:8088`입니다.
4. 아래 완료 점검 후 업로드 사용을 켭니다. R2/S3 자격증명은 PHP·브라우저에 입력하지 않습니다.

원본 G7 7.1 공개 확장 계약을 사용하는 현재 모듈에는 **코어·게시판·기본 템플릿 patch가
필요 없습니다.** 과거 `0001.patch`~`0006.patch`를 적용하지 마십시오. 모듈 ZIP을
모노레포 URL로 대체해 **GitHub에서 설치**하지도 않습니다.

계약 검사, 활성화·해제, 기존 원격 첨부가 있을 때의 제거 제한은
[G7 모듈 설치 안내](GNUBOARD7_RELEASE.md)를 따릅니다.

## 설치 완료 점검과 운영

```bash
sudo g7mbctl status
sudo g7mbctl doctor
sudo g7mbctl doctor --skip-storage
```

`status`는 target, API, worker, timer 세 개와 `/health/ready`를 한 번에 검사합니다. `doctor`는
여기에 native sandbox fixture와 실제 저장소 single/multipart canary를 더합니다. 출력에는
secret, presigned URL, 원본 파일명을 포함하지 않습니다.

`doctor`는 저장소에 테스트용 객체를 생성·삭제합니다. 외부 연결 없이 native 검사만 할 때는
`doctor --skip-storage`를 사용하며 이것을 저장소 연결 성공으로 판정하지 않습니다.

설치 완료 뒤 테스트 게시물에서 이미지·영상 업로드 → 처리 완료 → 저장 → 새로고침/첨부
열람 → 삭제를 확인합니다. 여러 파일과 기존 첨부 순서 변경도 확인하십시오. 모듈 활성화나
CLI storage canary 성공만으로 브라우저 CORS와 전체 미디어 E2E를 통과한 것은 아닙니다.

Lightsail은 버킷과 CORS를 Lightsail 콘솔/API에서 먼저 설정해야 합니다. bucket S3 key만으로
해당 관리 설정을 변경할 수 없습니다. 공급자별 입력·비밀값 저장·운영 중 교체는
[CUI 설정 안내](SETUP_CUI.md)를 따릅니다.

신규 업로드를 중지할 때는 G7 관리자에서 **업로드 사용**을 끕니다. 기존 원격 첨부 열람을
유지하려면 모듈과 Rust API를 유지해야 합니다. 원격 첨부를 이관·삭제하기 전에 모듈이나
스토리지 자격증명을 제거하지 마십시오.

## 예정 정책과 현재 지원 범위

다음은 합의한 개선 방향이며 **현재 Release에는 미구현**입니다.

- libvips가 설치되어 실제 capability 검사를 통과하면 우선 사용
- libvips가 없으면 순수 Rust 기본 이미지 엔진으로 제한된 포맷·작업 지원
- libvips 선택 처리기를 분리하여 라이브러리 부재가 기본 엔진 실행을 막지 않도록 구성
- 엔진별 지원 포맷·최대 픽셀·메모리 한도를 진단/관리자에 표시
- 손상 파일·보안 거부·자원 한도 초과는 다른 엔진으로 재시도하지 않음

폴백 구현과 codec·EXIF 제거·워터마크·대형 이미지 회귀 검증 전에는
“libvips 없이 동작”, “최신 포맷 모두 지원”으로 배포 안내하지 않습니다.

## 설치 경계

- 일반 설정: `/etc/g7mediabooster/g7mb.toml` (`0640`, root:g7mediabooster)
- root-only 비밀값: `/etc/g7mediabooster/credentials/` (`0700`, 파일 `0600`)
- SQLite·임시 파일·backup: `/var/lib/g7mediabooster`
- control API: loopback `127.0.0.1:8088`; 공개 reverse proxy 금지
- 선택형 signed thumbnail: loopback `127.0.0.1:8089`; 기본 비활성, `/media/v1/`만 proxy
- 브라우저 파일 본문: PHP/Rust를 거치지 않고 private R2/S3로 직접 전송
