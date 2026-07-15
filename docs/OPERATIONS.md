# 운영 관측과 API 보호

## 공식 운영 계약

- API `/v1` 요청은 기본 sustained 50 req/s, burst 100 token bucket과 동시 처리 64개로 제한합니다.
- 한도 초과는 `429`와 `Retry-After: 1`을 반환하며 안정된 오류 코드는
  `API_RATE_LIMIT_EXCEEDED`, `API_CONCURRENCY_EXHAUSTED`입니다.
- health와 metrics는 이 limiter를 거치지 않아 과부하 중에도 운영 판단에 사용할 수 있습니다.
- API `/metrics`와 worker metrics listener는 public ingress에 노출하지 않습니다.
- worker listener는 loopback 주소만 허용하며 기본값은 `127.0.0.1:9091`입니다.

## 주요 Prometheus 메트릭

| 메트릭 | 의미 |
|---|---|
| `g7mb_api_admission_rejections_total{reason}` | rate 또는 concurrency 거부 |
| `g7mb_api_in_flight_requests` | 현재 처리 중인 `/v1` 요청 |
| `g7mb_api_request_duration_seconds{status_class}` | API 처리시간 |
| `g7mb_queue_jobs{state}` | queued, leased, dead-letter 작업 수 |
| `g7mb_queue_oldest_queued_age_seconds` | 가장 오래 기다린 작업의 나이 |
| `g7mb_worker_active_jobs` | worker가 현재 처리 중인 작업 수 |
| `g7mb_worker_job_age_at_claim_seconds` | queue-to-start 지연 |
| `g7mb_worker_job_duration_seconds{outcome}` | 작업 전체 처리시간 |
| `g7mb_worker_stage_duration_seconds{stage}` | download, inspect, transform, upload, commit 단계 시간 |
| `g7mb_worker_resource_wait_seconds{lane}` | heavy image 또는 video semaphore 대기 |
| `g7mb_worker_processing_failures_total{class,code}` | allowlist 오류 분류별 실패 |
| `g7mb_cleanup_pending_uploads` | 아직 tombstone되지 않은 삭제 대기 upload |
| `g7mb_upload_tombstones` | 보존 중인 upload 감사 tombstone |
| `g7mb_orphan_objects{state="suspected"}` | grace 기간 관찰 중인 provider orphan |
| `g7mb_orphan_delete_failures` | 최근 prune이 실패한 orphan 수 |
| `g7mb_reserved_source_bytes` | 현재 보존 source 예약량 |

upload ID, 사용자 ID, object key를 label로 사용하지 않습니다. queue·upload·orphan count는
SQLite trigger가 O(1) counter로 유지하고 oldest queued age만 `(state, created_at)` index에서
읽습니다. API scrape는 이를 한 read transaction으로 가져옵니다. snapshot을 읽지 못하면
`/metrics`가 `503`을 반환해 오래된 정상값으로 장애를 숨기지 않습니다. backup 검증은 counter와
원본 행 집계를 대조합니다.

## 초기 경보 기준

- `g7mb_queue_oldest_queued_age_seconds > 120`이 10분 지속
- `g7mb_queue_jobs{state="dead_letter"} > 0`
- `g7mb_orphan_delete_failures > 0` 또는 cleanup pending이 계속 증가
- `g7mb_worker_processing_failures_total{class="operational"}` 증가
- `g7mb_api_admission_rejections_total` 급증
- API 또는 worker scrape가 5분 이상 없음
- `g7mb_reserved_source_bytes`가 설정 quota의 80% 초과

절대값은 실제 서버의 p95와 처리량을 측정한 뒤 조정합니다. rate limit을 무제한으로 풀거나
worker 동시성만 올리는 방식으로 경보를 해소하지 않습니다.

## 연계 runbook

- 삭제·tombstone: [LIFECYCLE.md](LIFECYCLE.md)
- provider orphan: [ORPHAN_INVENTORY.md](ORPHAN_INVENTORY.md)
- SQLite 백업·복원: [BACKUP_RESTORE.md](BACKUP_RESTORE.md)
