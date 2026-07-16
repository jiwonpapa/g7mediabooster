# Gnuboard7 원격 미디어 첨부 계약

이 디렉터리는 G7MediaBooster가 게시물 첨부까지 연결되기 전에 Gnuboard7 코어에 필요한
최소 공개 계약을 보관합니다. patch `0001`~`0006`과 회귀·호스트 게이트를 통과한 배포에서만
게시물 본문·미리보기·다운로드 연동을 공식 지원 기능으로 게시합니다.

## 기준과 범위

- 공개 기준 Gnuboard7 commit: `fcaacad8d16d47a8b5bcee65990869992de0a0d8`
- 기준 `sirsoft-board` 1.0.2, patch 적용 후 계약 버전 1.1.0
- 배포 산출물은 Gnuboard7 코어를 덮어쓰지 않고 별도 patch로 제공합니다.
- G7MediaBooster module은 이 계약을 검증한 뒤에만 attachment materialization을 활성화해야 합니다.

패치는 다음을 제공합니다.

1. `attachment_ids` list·게시판 최대 수 제한과 현재 사용자 `created_by` 조건·전건 일치 검사
2. 사용자 게시글 create/update에서 검증된 `attachment_ids`의 실제 전달
3. PHP가 원격 파일 바이트를 읽지 않는 `AttachmentService::authorizeDelivery()`
4. download/preview URL filter, 영상 poster URL, 사용자·관리자 form의 안정적인 layout extension ID
5. 모듈 접두사가 붙은 다른 모듈 레이아웃에도 overlay를 적용하는 G7 core 계약
6. 조건부 partial이 같은 안정 ID를 재사용할 때 모든 일치 분기에 overlay를 적용하는 계약
7. ID 일괄 연결 뒤 `attachments_count`를 같은 트랜잭션에서 재계산하는 계약
8. 비밀글·블라인드글·삭제글 첨부 직접 URL을 본문 열람 정책과 동일하게 차단하는 계약
9. 저장소·URL filter·module overlay 회귀 테스트와 `sirsoft-board` 버전/변경 이력 동기화
10. 모듈이 활성화 전에 검사할 versioned `secure-external-attachments` capability 문서

기존 patch가 admin FormRequest만 보강하고 실제 사용자 PostController가 쓰는 `User/*Request`를
놓친 문제를 현재 기준에서 수정했습니다. 계약 검증기는 admin·user 경로를 각각 검사합니다.

## 적용 순서

```bash
cd /path/to/gnuboard7
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0001-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0001-*.patch
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0002-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0002-*.patch
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0003-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0003-*.patch
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0004-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0004-*.patch
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0005-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0005-*.patch
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0006-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0006-*.patch

/path/to/G7MediaBooster/scripts/verify-gnuboard7-media-contract.sh "$PWD"
php artisan test \
  modules/_bundled/sirsoft-board/tests/Feature/FormRequestValidationTest.php \
  modules/_bundled/sirsoft-board/tests/Unit/AttachmentRepositoryTest.php \
  modules/_bundled/sirsoft-board/tests/Unit/AttachmentServiceTest.php \
  modules/_bundled/sirsoft-board/tests/Unit/AttachmentUrlFilterTest.php
php artisan test tests/Unit/Services/LayoutExtensionServiceTest.php \
  --filter='module_prefixed_requested_layout|every_matching_target'
php artisan test modules/_bundled/sirsoft-board/tests/Feature/CountSyncIntegrationTest.php \
  --filter='post_create_with_attachment_ids_syncs_attachments_count'
```

dirty 작업 트리에는 바로 적용하지 않습니다. 먼저 별도 브랜치나 깨끗한 worktree에서
`git apply --check`와 테스트를 통과시킨 뒤 G7 정식 변경으로 병합합니다.

## 활성화 게이트

다음 다섯 항목이 모두 PASS일 때만 G7MediaBooster module의 게시물 첨부 bridge를 켭니다.

- 계약 검증기 29/29와 module activation capability PASS
- 위 4개 PHP 회귀 테스트 PASS
- G7 사용자 create/update 실제 화면 smoke PASS
- 비소유 첨부 ID, 삭제·비밀·블라인드 글, 권한 없는 viewer의 전달 요청 차단 PASS
- `scripts/g7-host-security-gate.sh`의 권한·삭제/복원·보존 lease 실제 DB PASS
