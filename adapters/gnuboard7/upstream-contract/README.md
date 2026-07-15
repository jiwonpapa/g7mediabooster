# Gnuboard7 원격 미디어 첨부 계약

이 디렉터리는 G7MediaBooster가 게시물 첨부까지 연결되기 전에 Gnuboard7 코어에 필요한
최소 공개 계약을 보관합니다. 패치가 실제 G7에 반영되고 회귀 테스트를 통과하기 전에는
게시물 본문·미리보기·다운로드 연동을 공식 지원 기능으로 게시하지 않습니다.

## 기준과 범위

- 기준 Gnuboard7 commit: `e64381ddb5ba02caed60933427fbb86ef72ef94e`
- 대상 모듈: `sirsoft-board` 1.2.0
- 실제 Gnuboard7 작업 트리는 수정하지 않고 별도 patch만 제공합니다.
- G7MediaBooster module은 이 계약을 검증한 뒤에만 attachment materialization을 활성화해야 합니다.

패치는 다음을 제공합니다.

1. `attachment_ids` list·게시판 최대 수 제한과 현재 사용자 `created_by` 조건·전건 일치 검사
2. 사용자 게시글 create/update에서 검증된 `attachment_ids`의 실제 전달
3. PHP가 원격 파일 바이트를 읽지 않는 `AttachmentService::authorizeDelivery()`
4. download/preview URL filter, 영상 poster URL, 사용자·관리자 form의 안정적인 layout extension ID
5. 모듈 접두사가 붙은 다른 모듈 레이아웃에도 overlay를 적용하는 G7 core 계약
6. 저장소·URL filter·module overlay 회귀 테스트와 `sirsoft-board` 버전/변경 이력 동기화

기존 patch가 admin FormRequest만 보강하고 실제 사용자 PostController가 쓰는 `User/*Request`를
놓친 문제를 현재 기준에서 수정했습니다. 계약 검증기는 admin·user 경로를 각각 검사합니다.

## 적용 순서

```bash
cd /path/to/gnuboard7
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0001-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0001-*.patch
git apply --check /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0002-*.patch
git apply /path/to/G7MediaBooster/adapters/gnuboard7/upstream-contract/0002-*.patch

/path/to/G7MediaBooster/scripts/verify-gnuboard7-media-contract.sh "$PWD"
php artisan test \
  modules/_bundled/sirsoft-board/tests/Feature/FormRequestValidationTest.php \
  modules/_bundled/sirsoft-board/tests/Unit/AttachmentRepositoryTest.php \
  modules/_bundled/sirsoft-board/tests/Unit/AttachmentServiceTest.php \
  modules/_bundled/sirsoft-board/tests/Unit/AttachmentUrlFilterTest.php
php artisan test tests/Unit/Services/LayoutExtensionServiceTest.php \
  --filter='module_prefixed_requested_layout|applies_overlay_at_correct_position'
```

dirty 작업 트리에는 바로 적용하지 않습니다. 먼저 별도 브랜치나 깨끗한 worktree에서
`git apply --check`와 테스트를 통과시킨 뒤 G7 정식 변경으로 병합합니다.

## 활성화 게이트

다음 네 항목이 모두 PASS일 때만 G7MediaBooster module의 게시물 첨부 bridge를 켭니다.

- 계약 검증기 PASS
- 위 4개 PHP 회귀 테스트 PASS
- G7 사용자 create/update 실제 화면 smoke PASS
- 비소유 첨부 ID, 삭제 글, 비밀 글, 권한 없는 viewer의 전달 요청 차단 PASS
