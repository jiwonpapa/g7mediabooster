# 미디어 수명주기와 삭제

## 구현 범위

G7MediaBooster의 삭제는 요청 처리 중 S3/R2 객체를 지우지 않습니다. 인증된 tenant가
`DELETE /v1/uploads/{upload_id}`를 호출하면 SQLite에 삭제 요청을 기록하고 `202 Accepted`를
반환합니다. `g7mb-worker cleanup`이 제한된 batch를 lease한 뒤 다음 순서로 처리합니다.

1. `created` multipart이면 provider multipart session을 abort
2. derivative bucket의 파생 객체를 순서대로 삭제
3. raw/quarantine 객체 삭제
4. 모든 저장소 작업이 성공했을 때만 upload를 `deleted`로 tombstone

S3/R2의 abort `NoSuchUpload`와 `DeleteObject`는 재시도 가능한 성공으로 취급합니다. 객체 삭제
뒤 DB commit 전에 프로세스가 종료돼도 같은 작업을 안전하게 다시 실행할 수 있습니다.

## 자동 정리 정책

기본값은 Rust 운영 설정이 소유합니다.

| 설정 | 기본값 | 의미 |
|---|---:|---|
| `created_reservation_ttl_seconds` | 86,400 | 확인되지 않은 single/multipart 예약 보존 |
| `rejected_source_retention_seconds` | 604,800 | rejected/failed 비공개 원본 보존 |
| `cleanup_lease_seconds` | 300 | 한 cleanup owner의 독점 lease |
| `cleanup_retry_seconds` | 60 | 저장소 실패 후 재시도 지연 |
| `cleanup_batch_size` | 100 | 한 번의 실행이 선점하는 최대 개수 |
| `cleanup_max_attempts` | 10 | operator 확인 전 최대 시도 횟수 |
| `tombstone_retention_seconds` | 31,536,000 | upload·orphan 감사 tombstone 보존 기간 |
| `tombstone_purge_batch_size` | 100 | 한 실행에서 물리 삭제하는 최대 tombstone 수 |

timer는 기본 15분마다 oneshot cleanup을 실행합니다. attempt 상한에 도달한 행은 자동 선점하지
않고 `cleanup_error_code`와 함께 남겨 operator가 저장소/IAM 상태를 확인하게 합니다.
tombstone은 기본 365일 보존한 뒤 같은 cleanup transaction에서 bounded batch로 물리 삭제합니다.
설정 가능 범위는 30일~10년이며, retention 전 행과 최근 orphan 기록은 삭제하지 않습니다.

## 보안·경합 규칙

- HMAC의 tenant scope와 G7의 `upload_id + user_id + board_slug` 소유권을 모두 검사합니다.
- object key와 bucket은 요청에서 받지 않고 저장된 서버 생성 값만 사용합니다.
- `quarantined`와 `processing`은 worker 취소 계약이 없으므로 삭제 요청을 `409`로 거부합니다.
- 과거 또는 현재 site-policy의 watermark asset으로 참조된 upload는 삭제하지 않습니다.
- 삭제 대기 중에는 상태 API가 `deletion_pending=true`를 반환하고 파생 URL을 숨깁니다.
- 삭제 완료 뒤에는 retention 동안 tombstone을 유지해 같은 요청이 `204 No Content`로 멱등
  종료됩니다. retention이 지난 기록의 무기한 replay 보장은 제공하지 않습니다.

## 운영 명령

```bash
g7mb-worker --config /etc/g7mediabooster/g7mb.toml cleanup --worker-id manual-cleanup
systemctl enable --now g7mediabooster-cleanup.timer
systemctl list-timers g7mediabooster-cleanup.timer
journalctl -u g7mediabooster-cleanup.service
```

실 provider 승격 시 `cargo xtask live-storage-conformance`가 임시 SQLite에 Ready 사용자 삭제와
8일 지난 rejected 원본을 만들고 실제 lifecycle lease를 실행합니다. derivative→raw 삭제,
tombstone 2건과 각 key의 `HEAD NotFound`를 모두 확인하며 credential이 없을 때는 실행하지 않습니다.

현재 구현은 단일 노드 SQLite lease 기준입니다. PostgreSQL이나 멀티노드는 v1 범위가 아니며,
동시 수동 실행과 프로세스 강제 종료 복구는 같은 SQLite lease/CAS가 보호합니다.
