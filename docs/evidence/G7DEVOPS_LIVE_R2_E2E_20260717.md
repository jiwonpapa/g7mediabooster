# g7devops Gnuboard7 + R2 실서버 E2E

- 검증일: 2026-07-17 KST
- SSH/G7: `g7devops` / `/home/g7devops/public_html`
- 서버 릴리스: `server-v0.1.1`, commit `60e3e1c371265046913a79096b264957e2548003`
- 릴리스 gate: GitHub Actions `29553995065` PASS
- G7 모듈: `jiwonpapa-g7mediabooster` `0.4.3` 활성

## 설치와 저장소

- GitHub Release의 Linux x86_64 archive SHA-256과 내부 `MANIFEST.sha256` 검증 PASS
- `g7mbctl install --skip-setup --skip-start`로 binary·sandbox·systemd target 설치
- `g7mbctl setup`의 root-only 입력 파일 자동화 경로로 실제 R2 연결
- Cloudflare R2 private bucket: `g7mediabooster-private`
- credential: 해당 bucket의 개체 읽기·쓰기·나열만 허용, 브라우저 direct PUT 때문에 IP 필터 없음
- CORS: origin `https://www.g7devops.com`, methods `GET/PUT/HEAD`, header `*`, ETag 노출
- setup/doctor: bucket 확인, single PUT·HEAD·GET·LIST·DELETE, multipart complete·abort PASS
- 동일 setup 재실행: provider canary PASS, HMAC hash 유지, 설정 멱등 PASS

R2/S3 비밀값은 문서·명령행·G7 DB에 기록하지 않았습니다. Rust 설정에는 root-only credential
파일의 절대 경로만 있으며 G7에는 Laravel Crypt 대상 HMAC만 저장했습니다. 자동화 staging 파일과
임시 Sanctum 관리자 토큰은 검증 직후 삭제·폐기했습니다.

## 런타임

- `g7mbctl 0.1.1`
- target/API/worker/timer 3개 active, API ready
- API bind: `127.0.0.1:8088` 전용, public reverse proxy 없음
- libvips `8.15.1`, FFmpeg/ffprobe `6.1.1`
- G7 capabilities: image input `avif/gif/heif/jpeg/png/webp`, output `avif/jpeg/png/webp`
- video input `mov/mp4`, MP4 thumbnail 및 H.264 fallback `true`
- 2 vCPU 호스트 override: worker `CPUQuota=100%`, `MemoryMax=1280M`, swap `0`, tasks `64`
- API 기본 상한: `CPUQuota=50%`, `MemoryMax=256M`, tasks `32`

## G7 실제 업로드

G7 `gallery` 게시판의 인증·권한 middleware와 PHP HMAC client를 통과해 한 batch에서 두 파일을
예약했습니다.

| 입력 | 전송 | 결과 | 파생물 |
|---|---|---|---|
| 1920x1080 JPEG 114 KiB | presigned single PUT | `ready`, 실제 MIME `image/jpeg` | master JPEG 73,883 B, thumbnail JPEG 43,016 B |
| 1280x720 H.264 MP4 746 KiB | presigned multipart | `ready`, 실제 MIME `video/mp4` | master MP4 764,131 B, thumbnail JPEG 57,630 B |
| ASCII 20 B를 `.jpg`로 위장 | presigned single PUT 후 격리 | `rejected`, 실제 MIME `null` | 0개 |

- 두 PUT 모두 R2 `200`, `Access-Control-Allow-Origin: https://www.g7devops.com`
- image/video thumbnail 모두 G7 private delivery `302` 후 R2 `200 image/jpeg`
- 두 thumbnail 모두 실제 `1280x720`
- Ready image/video를 G7 native attachment ID `16`, `17`로 materialize PASS
- 게시글에 연결되지 않은 attachment URL은 `404`로 fail-closed
- 세 test upload 모두 삭제 예약 후 cleanup service `claimed=3 completed=3 failed=0`
- 최종 상태 세 건 모두 `deleted`, `deletion_pending=false`
- disable/apply 실왕복에서 root-only config 검사가 일반 사용자 권한으로 실패하는 하네스 결함 발견
- config 확인을 `sudo test`로 고치고 API ready 최대 30초 대기를 추가한 뒤 실왕복 PASS

## 최종 상태와 즉시 해제

```text
STATUS service=active api=ready module=0.4.3 active
PASS doctor storage single_object=true multipart=true
```

즉시 해제는 데이터와 설정을 보존하며 module과 product target을 함께 중지합니다.

```bash
scripts/g7-live-control.sh disable --confirm g7devops
scripts/g7-live-control.sh apply --confirm g7devops
```

G7 파일까지 이전 상태로 돌릴 때만 기존 receipt를 사용합니다.

```bash
scripts/g7-live-control.sh rollback \
  --deployment-id g7mb-20260717T024913Z \
  --confirm g7devops
```
