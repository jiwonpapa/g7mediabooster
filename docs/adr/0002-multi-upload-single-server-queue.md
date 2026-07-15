# ADR 0002: 멀티업로드와 서버 멀티노드 분리

- 상태: Accepted
- 기준일: 2026-07-15

## 배경

제품 요구의 `멀티`는 사용자가 이미지·영상을 여러 개 선택해 동시에 올리는
멀티업로드입니다. 이는 API나 worker 서버를 여러 대 운영하는 멀티노드와 다른 문제입니다.

멀티업로드에는 다음 두 병렬성이 있습니다.

1. 여러 파일을 동시에 전송하는 file concurrency
2. 큰 파일 하나를 여러 part로 전송하는 multipart concurrency

둘을 무제한으로 곱하면 브라우저 연결, object storage 요청, worker CPU와 메모리가 폭증합니다.

## 결정

- v1 기본 배포는 단일 Rust 서버와 로컬 SQLite WAL durable queue입니다.
- batch는 최대 100개를 받되 전체 연결과 파일별 part concurrency를 따로 제한합니다.
- 파일 바이트는 PHP나 Rust API를 통과하지 않고 S3/R2로 직접 전송합니다.
- 모든 가공은 SQLite 작업 queue로 넘기고 worker가 원자적으로 한 작업씩 선점합니다.
- worker가 죽으면 선점 만료 후 작업을 재시도하며 완료 동작은 멱등입니다.
- PostgreSQL, Redis, NATS 같은 공유 queue는 멀티노드가 실제 요구될 때만 별도 ADR로 도입합니다.

## 결과

- PHP `upload_max_filesize`와 request timeout 병목을 제거할 수 있습니다.
- 추가 DB daemon 없이 G5/G7 서버에 설치할 수 있습니다.
- 100개 업로드도 처리하지만 동시에 100개를 decode하지는 않습니다.
- 단일 서버 장애 중에는 처리가 멈추며 서버 수평 확장은 v1 보장 범위가 아닙니다.
- SQLite DB와 object storage 상태의 백업·복구 절차가 필요합니다.

## 초기 기본값

| 항목 | 값 |
|---|---:|
| batch 파일 수 | 100 |
| 브라우저 전체 연결 | 8 |
| 파일별 multipart part | 4 |
| multipart 기준 | 100 MiB 또는 영상 |
| part 크기 | 32 MiB |
| 일반 worker 동시성 | 2 |
| heavy worker 동시성 | 1 |

기본값은 benchmark와 운영 지표로 조정하지만 hard cap과 bounded 원칙은 변경하지 않습니다.
