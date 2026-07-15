# Provider orphan inventory

## 안전 계약

- `g7mb-worker inventory`는 기본적으로 **audit only**이며 객체를 삭제하지 않습니다.
- raw bucket은 정확히 `raw/`, derivative bucket은 정확히 `media/` prefix만 조회합니다.
- 기본 한 번 실행은 namespace별 1,000개 × 10 page로 제한하고 마지막 key cursor를 SQLite에
  저장해 다음 실행에서 이어갑니다.
- DB에 없는 객체는 즉시 삭제하지 않고 최초 관측 시각을 저장합니다. 기본 grace는 48시간입니다.
- `--prune`을 명시한 실행만 grace가 지난 후보를 처리하며, provider delete 직전에 DB 소유권을
  다시 확인합니다. 그 사이 upload/derivative row가 생겼으면 의심 기록만 제거합니다.
- 삭제 성공은 `orphan_objects.state=deleted` audit tombstone으로 남고, 실패는 안정된 error code와
  attempt 수만 저장합니다. credential, bucket 이름, provider URL은 저장하지 않습니다.
- `ObjectKey` 검증이나 prefix 검증을 통과하지 못한 provider key는 집계만 하고 자동 삭제하지
  않습니다.

## 실행

```bash
# 매일 실행해도 비파괴
g7mb-worker --config /etc/g7mediabooster/g7mb.toml inventory

# 최소 두 번의 audit, DB 백업·복원 검증, 의심 목록 검토 후 명시 실행
g7mb-worker --config /etc/g7mediabooster/g7mb.toml inventory --prune
```

기본 systemd timer는 audit 명령만 실행합니다. 자동 prune을 배포 기본값으로 활성화하지 않습니다.
운영자가 prune을 예약하려면 DB 복원 절차를 먼저 검증하고 별도 unit override에서 `--prune`을
명시해야 합니다.

## 운영 점검

```sql
SELECT namespace, state, COUNT(*) AS objects, SUM(content_length) AS bytes,
       MAX(delete_attempts) AS max_attempts
FROM orphan_objects
GROUP BY namespace, state;
```

inventory 실행 identity에는 bucket의 `ListBucket` 권한과 기존 prefix-scoped delete 권한이
필요합니다. 이 권한이 없어도 일반 upload/worker 경로는 동작하지만 inventory unit은 fail로
남아야 하며 성공으로 무시하면 안 됩니다.
