# g7devops Gnuboard7 라이브 모듈 검증

- 검증일: 2026-07-17 KST
- SSH 대상: `g7devops`
- G7 root/user: `/home/g7devops/public_html` / `g7devops`
- 배포 receipt: `g7mb-20260717T024913Z`
- 원복 백업: `/var/backups/g7mediabooster/g7mb-20260717T024913Z`

## 결과

- Ubuntu 24.04에 libvips `8.15.1`, FFmpeg/ffprobe `6.1.1` 설치 완료
- 활성·bundled 게시판/템플릿 원본 일치 확인 후 secure external attachment 계약 반영
- G7 media contract `29/29 + parser + activation` PASS
- 공식 ZIP SHA-256 `1137edfb94f7ddedab425b1adfeca1908d205e1340d6c701175132055717ecac` 확인
- G7 ZipInstallHelper 검증 PASS
- `jiwonpapa-g7mediabooster` `0.4.3` 설치·활성화 PASS
- 권한 4개, 관리자 메뉴 1개, 관리자 레이아웃 1개 등록
- route/cache 재생성 후 모듈 API route 15개 등록 확인
- 비로그인 설정 API `401`, 관리자 페이지 `200`
- 로그인한 실제 관리자 화면 렌더링 및 브라우저 console error 0건 확인

## 관리자 위치

- 왼쪽 관리자 사이드바의 독립 메뉴 **미디어 부스터**
- URL: `/admin/media-booster/settings`
- DB menu: `id=31`, `slug=jiwonpapa-g7mediabooster`, `order=45`, `is_active=true`

설정 화면에는 워터마크 정책, 서비스 활성화, Rust 제어 API origin, HMAC key/secret,
요청 timeout, 멀티업로드 동시성·재시도·조회 간격·첨부 보존일 설정이 표시됩니다.

## 현재 경계

Rust server bundle, R2 credential, HMAC pairing은 아직 적용하지 않았습니다. 따라서 G7 모듈은
활성 상태지만 기본 설정 `enabled=false`이며 실제 업로드 예약은 열리지 않습니다.

## 제어·원복

```bash
scripts/g7-live-control.sh preflight
scripts/g7-live-control.sh status
scripts/g7-live-control.sh disable --confirm g7devops
scripts/g7-live-control.sh rollback \
  --deployment-id g7mb-20260717T024913Z \
  --confirm g7devops
```

`disable`은 설정·DB·미디어 객체를 보존합니다. `rollback`은 모듈을 제거하고 receipt의 G7
파일을 복원하지만 안전을 위해 모듈 데이터 테이블과 설정은 삭제하지 않습니다.
