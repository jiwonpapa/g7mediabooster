# 워터마크 계약

- 상태: 이미지·영상 poster 렌더링과 G7→Rust 정책 revision 연동 구현,
  G7 전용 asset picker 브라우저 smoke 대기
- 기준일: 2026-07-15

## 보안·멱등 원칙

- 브라우저와 PHP 요청은 파일 경로, 위치, 투명도, 크기를 직접 넘길 수 없습니다.
- 운영자가 등록한 로컬 자산만 사용하며 worker 시작 설정에 소문자 SHA-256을 고정합니다.
- worker는 자산을 작업별 private 임시 디렉터리로 최대 16MiB만 복사하고 digest를 다시
  검사한 뒤 credential 없는 sandbox에 전달합니다.
- digest 불일치·자산 부재는 업로드 원본의 잘못으로 기록하지 않습니다. lease를 완료하지
  않고 worker를 fail-closed로 중지해 운영자가 설정을 복구하게 합니다.
- 결과 preset ID와 object key는 `preset_revision + watermark_sha256`을 모두 포함합니다.
  같은 원본·프리셋·워터마크는 같은 불변 key를 사용하고 자산이 바뀌면 새 key가 됩니다.

## 허용 프리셋

| 항목 | 허용 범위 |
|---|---|
| 위치 | center, top-left, top-right, bottom-left, bottom-right |
| 최대 너비 | 결과 이미지의 1~50% |
| 투명도 | 1~100% |
| 여백 | 0~1,024px |
| 등록 이미지 | 최대 4,096×4,096, 16MP, encoded 16MiB |

libvips가 결과와 워터마크를 sRGB로 정규화하고 alpha를 적용해 합성합니다. JPEG 출력은
흰색 배경으로 flatten하며 metadata는 저장하지 않습니다. worker는 이미지 파생물과 FFmpeg로
뽑은 영상 poster에 같은 digest-pinned 정책을 적용합니다. 영상은 전체 timeout의 절반을
FFmpeg의 최대 2회 seek에 배정하고 남은 시간에 libvips 합성을 수행하며 중간 frame은 항상
삭제합니다.

G7 관리자는 같은 tenant에서 안전 검사를 통과한 Ready PNG·WebP·JPEG upload ID만 선택할 수
있습니다. PHP가 `PUT /v1/site-policy` 요청 전체를 HMAC 서명하면 Rust는 소유권, Ready 상태,
16MiB 상한, 실제 MIME과 source SHA-256을 다시 확인합니다. revision은 1부터 단조 증가하며
같은 revision·같은 settings hash 재전송만 멱등 허용합니다. 업로드가 quarantine에 들어갈 때
현재 revision을 SQLite job에 고정하므로 정책이 변경돼도 재시도 결과가 바뀌지 않습니다.
worker는 고정된 object key를 S3/R2에서 exact-length download하고 digest를 다시 확인합니다.

남은 G7 UI 게이트는 upload ID 수동 입력을 전용 관리자 asset picker로 교체하고 실제 G7 설치
브라우저에서 자산 선택·revision 적용·rollback을 확인하는 것입니다.

## 운영 설정 예시

```toml
[watermark]
enabled = true
asset_path = "/etc/g7mediabooster/watermark.png"
asset_sha256 = "64자리-소문자-sha256"
preset_revision = "site-v1"
position = "bottom_right"
margin_px = 24
max_width_percent = 20
opacity_percent = 80
```

직접 sandbox 검증은 `image-thumbnail` 또는 `video-thumbnail` 명령의 `--watermark` 계열
인자를 사용합니다. 이 인자는 worker가 검증·복사한 로컬 경로만 전달하는 내부 계약입니다.
