# 실 provider lifecycle 삭제 하네스 증거

- 실행일: 2026-07-16
- 실제 R2/Lightsail credential: 미설정
- 판정: 하네스 구현과 credential-free 회귀 PASS, 실계정 승격은 대기

## 구현 계약

`cargo xtask live-storage-conformance`는 provider별 protocol conformance 뒤 다음 두 lifecycle
후보를 같은 임시 SQLite에 생성하고 직렬 처리합니다.

1. Ready upload에 인증된 사용자 삭제 요청 기록
2. 기본 7일 보존을 넘긴 8일 전 rejected upload

실제 `LifecycleService`와 `SqliteStore`가 두 후보를 lease하고 derivative→raw 순으로 지운 뒤
두 DB 행을 `deleted` tombstone으로 전환합니다. 생성한 세 object key는 모두 provider `HEAD`가
`ObjectStoreError::NotFound`인지 확인합니다. 성공 로그는 safe provider label과
`profile`, `user_delete=1 retention_expired=1 tombstones=2 object_count=0`만 출력합니다.

테스트가 중간 실패해도 UUID로 만든 알려진 fixture key 세 개에 idempotent DELETE를 시도합니다.
protocol CORS·byte·inventory 검증 실패도 알려진 raw/derivative key 네 개를 전부 정리하며,
multipart part/complete 실패는 알려진 upload ID로 abort합니다. bucket 생성·IAM·CORS 수정이나
임의 prefix 삭제는 하지 않지만, 실제 G7 origin의 OPTIONS preflight와 PUT 응답 CORS/ETag
노출을 검증합니다.

## 현재 실행 증거

- live integration test 2개 compile 및 credential 없을 때 ignore 확인
- pinned MinIO single/multipart/client 재생성 재개/abort/GET/PUT/Delete conformance PASS
- MinIO 삭제 뒤 HEAD와 GET이 정확히 `NotFound`, multipart fixture 잔존 0 PASS
- `scripts/live-storage-preflight-smoke.sh`: 필수값 8개, secret redaction, HTTPS endpoint·exact
  browser origin·label·R2/Lightsail profile shape guard PASS
- `cargo xtask ci`: format, check, clippy, test, rustdoc, package smoke PASS
- `cargo xtask coverage`: line coverage 81.13% (`7629/9404`) PASS
- `cargo xtask supply-chain`: advisories, bans, licenses, sources PASS

실 R2와 Lightsail에서 같은 명령이 PASS하기 전에는 `cloudflare_r2_profile`,
`lightsail_object_storage_profile`, `live_provider_retention_delete`를 공식 기능으로 게시하지 않습니다.
