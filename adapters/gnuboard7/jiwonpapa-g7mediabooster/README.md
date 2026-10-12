# G7 Media Booster for Gnuboard 7

0.5.0은 공식 G7 7.1.0 / 번들 `sirsoft-board` 1.2.0 공개 확장 계약을 사용하는 core-free 모듈입니다.
코어·게시판·기본 템플릿을 수정하지 않습니다. 새 어댑터의 브라우저·실 저장소·설치 종단 전에는 preview입니다.

## 구현 기능

- 이미지·MP4/MOV H.264 최대 100개 직접 single/multipart 업로드
- 기본 파일 8개, 파일별 part 4개 동시성 제한, 진행률·취소·재시도·abort
- Ready master·thumbnail/poster 검증 후 native attachment 원자적·멱등 생성
- 사용자·관리자 폼 응답의 업로더 교체와 기존 저장 이벤트/첨부 ID 연결
- 원본 게시판 권한 검사를 재사용한 기존 URL의 private no-store 302 전달
- 로그인 사용자·게시판별 upload 소유권, 처리 상태·개수·삭제 경합 검증
- 보존 lease·삭제/복원 경합 보호, 워터마크 정책 revision 동기화
- 관리자 `미디어 부스터` 설정, HMAC secret 암호화 및 브라우저 비노출
- 업로드 사용 해제 시 기존 업로더 복원, 기존 원격 첨부 열람 유지

서버의 이미지·영상 처리 지원과 새 G7 어댑터의 종단 검증은 구분합니다.
특히 영상 poster의 기본 게시판 표시와 임의 커스텀 템플릿은 아직 인수 검증 전입니다.

## 설치와 설정

PHP 8.2+, `mbstring`, 정상 설치된 G7 7.1.0 및 게시판 모듈이 필요합니다.
checksum을 확인한 전용 모듈 ZIP을 G7 관리자 **모듈 관리 → 파일에서 설치**에 넣습니다.
모노레포 URL을 **GitHub에서 설치**에 직접 입력하지 않습니다.

모듈 설치·활성화 후 관리자 **미디어 부스터**에서 제어 endpoint, HMAC key ID·secret을 설정합니다.
같은 서버 endpoint는 `http://127.0.0.1:8088`이며 별도 서버는 HTTPS origin을 사용합니다.
R2/S3 비밀값은 Rust 서버의 `g7mbctl setup`으로 설정합니다. PHP·브라우저에는 넣지 않습니다.

버전과 실제 공개 메서드 계약이 불일치하면 활성화를 차단합니다.
원격 첨부가 남아 있으면 전체 모듈 제거·비활성화를 차단합니다. 단순 사용 해제는 관리자에서
업로드 사용을 끄십시오. Rust API를 중지하면 기존 원격 첨부도 열람할 수 없습니다.

## 저장소 CORS

실제 게시판 origin만 허용합니다. direct-upload URL은 HTTPS이며 multipart는 `ETag` 노출이 필수입니다.

```json
[{"AllowedOrigins":["https://board.example.com"],"AllowedMethods":["PUT"],
  "AllowedHeaders":["content-type","x-amz-*"],"ExposeHeaders":["ETag"],"MaxAgeSeconds":3600}]
```

## 개발 검증

```bash
npm ci
npm run typecheck
npm test
npm run build
G7MB_TEST_HOST_ROOT=/path/to/stock-g7 /path/to/stock-g7/vendor/bin/phpunit -c phpunit.xml --do-not-cache-result
/path/to/G7MediaBooster/scripts/g7-host-security-gate.sh /path/to/stock-g7
```

호스트 테스트는 이름에 `_testing`이 포함된 별도 DB에서만 실행해야 합니다.
설치·연동·남은 게이트는 저장소의 `docs/GNUBOARD7_INTEGRATION.md`, `docs/GNUBOARD7_RELEASE.md`,
`docs/evidence/G7_CORE_FREE_710_20261012.md`에 기록합니다. 과거 patch는 현재 설치에 사용하지 않습니다.
