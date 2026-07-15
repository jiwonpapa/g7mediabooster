# 멀티업로드와 작업 queue 설명

## 결론

- 멀티업로드는 여러 파일을 제한된 동시성으로 올리는 사용자 기능입니다.
- multipart는 큰 파일 하나를 여러 조각으로 나누는 전송 방식입니다.
- queue는 업로드 전송이 아니라 업로드 후 검사·썸네일·워터마크 작업을 순서대로 처리합니다.
- v1은 PostgreSQL이 아니라 단일 서버의 SQLite WAL queue를 사용합니다.
- 현재 default/all-features 활성 빌드 그래프에는 PostgreSQL runtime crate나 PostgreSQL
  queue 구현이 없습니다.

## lease queue란

`lease`는 worker가 작업을 영구 소유하는 것이 아니라 짧은 시간 동안 빌리는 방식입니다.

1. queue에 `QUEUED` 작업이 들어갑니다.
2. worker가 원자적으로 하나를 `LEASED`로 바꾸고 만료 시간을 기록합니다.
3. 성공하면 `COMPLETED`로 끝냅니다.
4. worker가 죽으면 lease 만료 뒤 다른 worker가 같은 작업을 다시 가져갑니다.
5. 재시도 한도를 넘으면 `DEAD_LETTER`로 격리합니다.

따라서 lease queue는 PostgreSQL 전용 개념이 아닙니다. 현재 구현은 SQLite
`BEGIN IMMEDIATE` claim transaction으로 한 서버 안의 중복 선점을 막습니다. tenant별 마지막
선점 순번도 영속화해 worker가 하나여도 backlog를 `A → B → A → B` 순으로 공정 선점합니다.
PostgreSQL은 여러 서버가
같은 queue를 공유해야 할 때 `FOR UPDATE SKIP LOCKED` 같은 기능으로 교체할 수 있는 미래
선택지일 뿐입니다.

`PostgreSQL lease queue`가 필요한 시점은 worker/API를 서로 다른 서버 여러 대에 배치하고,
그 노드들이 같은 작업 목록을 동시에 안전하게 선점해야 할 때입니다. 파일 100개를 한 번에
올리는 기능만으로는 필요하지 않습니다. v1에서 PostgreSQL을 넣으면 DB 데몬·연결 풀·백업·
장애조치 운영비만 늘어나므로 도입하지 않습니다.

향후 멀티노드 ADR을 여는 조건은 단일 노드의 CPU/RSS 조정으로 목표 처리량을 달성하지
못하거나, 무중단 worker 장애조치가 제품 요구가 되는 경우입니다. 그때 PostgreSQL의
`FOR UPDATE SKIP LOCKED` 같은 원자적 선점, fencing token, heartbeat, clock 기준,
dead-letter와 tenant 공정성을 별도 conformance test로 증명해야 합니다. 단순히 파일 선택
수가 100개라는 이유로 전환하지 않습니다.

## 100개 업로드 예시

- 브라우저는 파일 100개를 선택할 수 있습니다.
- 실제 네트워크 연결은 기본 8개까지만 동시에 사용합니다.
- 큰 파일 하나의 multipart도 기본 4개 part까지만 동시에 전송합니다.
- 업로드가 끝난 파일의 가공 작업은 queue에 쌓입니다.
- 일반 worker 2개와 heavy worker 1개가 서버 CPU·메모리 한도 안에서 처리합니다.
- 활성 예약은 기본 global 1,000개, tenant별 200개이며 초과 batch는 presign 전에
  `429 UPLOAD_CAPACITY_EXHAUSTED`를 받습니다.

즉, 100개를 받는 것과 100개를 동시에 디코딩하는 것은 전혀 다릅니다.
