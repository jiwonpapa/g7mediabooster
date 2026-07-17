# Linux 서버 통합 설치

G7MediaBooster의 내부 API·worker·예약 작업을 사용자가 각각 설치하지 않도록 서버 Release는
하나의 번들로 배포합니다. 상시 프로세스는 API와 worker 두 개이지만 systemd에서는
`g7mediabooster.target` 하나로 관리합니다. sandbox는 작업별 자식 프로세스이며 cleanup,
inventory, backup은 target에 딸린 timer입니다.

## 사용자 설치

Release의 Linux architecture용 `.tar.gz`와 `.sha256`을 같은 디렉터리에 받은 뒤 검증합니다.

```bash
sha256sum -c g7mediabooster-server-<version>-linux-<architecture>.tar.gz.sha256
tar -xzf g7mediabooster-server-<version>-linux-<architecture>.tar.gz
cd g7mediabooster-server-<version>-linux-<architecture>
sudo ./bin/g7mbctl install
```

`install` 한 번이 다음을 fail-closed 순서로 수행합니다.

1. systemd Linux, root와 payload 일반 파일·크기·정확한 SHA-256 manifest를 검사
2. libvips·FFmpeg가 없으면 각각의 용도를 안내하고 `설치할까요? [Y/n]` 확인 후 Ubuntu
   24.04의 검증된 최소 runtime package를 설치하며, 번들 sandbox의 실제 native
   doctor·capability fixture 실행
3. `g7mediabooster` system user/group과 제한된 디렉터리 생성
4. API·worker·sandbox·CLI와 hardened unit을 원자적으로 설치
5. `g7mbctl setup` CUI로 R2/S3·exact G7 origin과 root-only 비밀값 설정
6. `g7mediabooster.target` 하나를 활성화하고 API ready를 최대 30초 확인
7. G7 관리자에서 올릴 module ZIP의 설치 경로 출력

설정과 기동을 분리해야 하는 자동화 환경만 다음 옵션을 사용합니다.

```bash
sudo ./bin/g7mbctl install --skip-setup --skip-start
sudo /usr/local/bin/g7mbctl setup --non-interactive <필수 file-input 옵션>
sudo systemctl enable --now g7mediabooster.target
```

TTY가 없는 자동 배포에서는 묻지 않고 설치하지 않습니다. OS package 설치를 허용할 때만
명시적으로 다음 옵션을 추가합니다.

```bash
sudo ./bin/g7mbctl install --install-dependencies --skip-setup --skip-start
```

OS package를 이미지 빌드 단계에서 관리하는 환경은 `--skip-dependency-install`을 추가합니다.
이 옵션에서 libvips·FFmpeg가 없으면 설치기는 변경 없이 실패합니다. 다른 Linux 배포판은 필요한
runtime을 먼저 설치하고 이 옵션으로 설치하며, 공식 서버 Release 기준 환경은 Ubuntu 24.04입니다.

기존 파일과 내용이 다르면 설치기는 덮어쓰지 않습니다. 검증된 새 Release로 업데이트할 때만
`--force`를 사용하며 설정과 credential은 교체하지 않습니다.

## 단일 관리 표면

```bash
sudo g7mbctl status
sudo g7mbctl doctor
sudo g7mbctl doctor --skip-storage
```

`status`는 target, API, worker, timer 세 개와 `/health/ready`를 한 번에 검사합니다. `doctor`는
여기에 native sandbox fixture와 실제 저장소 single/multipart canary를 더합니다. 출력에는
secret, presigned URL, 원본 파일명을 포함하지 않습니다.

G7 관리자에 업로드할 ZIP은 다음 위치에 설치됩니다.

```text
/usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip
```

현재 공개 Gnuboard7에 secure external attachment 계약이 병합되기 전에는 함께 설치된
`0001.patch`~`0006.patch`를 깨끗한 G7 배포 브랜치에서 먼저 검증·반영해야 합니다. 설치기가
사용자 G7 core에 patch를 자동 적용하지는 않습니다.

## 설치 경계

- 일반 설정: `/etc/g7mediabooster/g7mb.toml` (`0640`, root:g7mediabooster)
- root-only 비밀값: `/etc/g7mediabooster/credentials/` (`0700`, 파일 `0600`)
- SQLite·임시 파일·backup: `/var/lib/g7mediabooster`
- API: loopback `127.0.0.1:8088`; 공개 reverse proxy 불필요
- 브라우저 파일 본문: PHP/Rust를 거치지 않고 private R2/S3로 직접 전송
