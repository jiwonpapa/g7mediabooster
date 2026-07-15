# 요구사항 1~17 구현 계획

- 상태: Internal implementation complete; external validation pending
- 기준일: 2026-07-16
- 대상: G7MediaBooster v0.3 이후
- 원칙: 이 문서는 구현 순서와 인수 조건이며 기능 완료를 뜻하지 않습니다.

## 1. 최종 권고

1. **G7은 플러그인보다 전용 PHP 모듈이 맞습니다.** G7 7.0.4는 Laravel 12 기반이며
   모듈이 API, 서비스, 권한, DB, 관리자 메뉴와 레이아웃을 소유할 수 있습니다.
2. 파일 바이트는 PHP와 Rust API를 거치지 않고 **브라우저에서 S3/R2로 presigned
   multipart 직접 업로드**합니다. Rust API는 의도, 서명, 상태, 검증과 큐만 담당합니다.
3. 요구사항의 `멀티`는 **여러 파일의 동시 업로드**입니다. 기본 배포는 단일 서버와
   **SQLite WAL durable queue**로 확정합니다. 서버 여러 대를 쓰는 멀티노드는 향후 선택
   확장이며 PostgreSQL 전환도 그때 별도 ADR로 결정합니다.
4. 미디어는 raw/quarantine에 먼저 들어가며 magic byte만이 아니라 실제 decoder/ffprobe를
   통과해야 합니다. 파생물과 정규화된 master만 공개합니다.
5. 썸네일은 object storage/CDN에 불변 파일로 저장합니다. 메모리에는 작은 manifest만
   byte-bounded cache로 둡니다. 질문하신 오래된 항목 제거 방식은 **LRU**이며, 실제
   적용은 Moka의 frequency admission + LRU eviction, TTL, per-key singleflight입니다.
6. FFmpeg는 주 영상 엔진입니다. 독립 폴백은 MP4/H.264 한정 Rust 경로로 제공하고 모든
   codec을 순수 Rust로 대체할 수 있다고 주장하지 않습니다.

## 2. 목표 구조

```mermaid
flowchart LR
    G7["G7 PHP 모듈\n권한·관리자 설정"] -->|"HMAC control API"| API["Stateless Rust API"]
    Browser["다중 업로더"] -->|"Presigned multipart"| Raw["Private quarantine\nS3/R2"]
    API --> DB["SQLite WAL\nstate·nonce·job queue"]
    Worker["bounded worker"] -->|"원자적 작업 선점"]| DB
    Worker --> Raw
    Worker --> Sandbox["No-network sandbox\nlibvips·FFmpeg·Rust fallback"]
    Sandbox --> Public["Sanitized master·thumbnail\nS3/R2 + CDN"]
    Worker -->|"signed webhook"| G7
```

G7 모듈은 세션·게시판 권한·첨부 연결의 진실 원천이고, Rust는 object key·포맷 판정·
가공 상태·hard limit의 진실 원천입니다. Rust가 G7 DB를 직접 읽지 않습니다.

## 3. 요구사항별 계획

| 번호 | 구현 결정 | 단계 | 완료 증거 |
|---:|---|---|---|
| 1 | 이미지 batch intent, single/multipart 업로드, 취소·재개·상태 API | A | 브라우저 실제 업로드와 중단 재개 smoke |
| 2 | `ObjectStore` port로 AWS S3, R2, MinIO 호환성 분리 | A | 공급자별 create/part/complete/abort conformance |
| 3 | MP4/MOV 우선 영상 업로드, ffprobe 정책 검사 | B | 정상·손상·초과 duration fixture |
| 4 | JPEG/PNG/GIF/WebP/AVIF/HEIC/HEIF 필수, WebM·JPEG XL은 capability gate | B/P2 | 실제 decode/encode runtime capability smoke |
| 5 | quarantine, digest, magic+실제 decode, 폭탄 한도, sandbox, rate limit | B | 위장·polyglot·절단·픽셀/프레임 폭탄 거부 |
| 6 | `jiwonpapa-g7mediabooster` PHP **모듈**과 `sirsoft-board` 공개 확장점 | C | 코어 patch 없이 설치·활성화·게시물 연결 smoke |
| 7 | batch 최대 100개, 여러 파일과 multipart를 bounded 병렬 처리 | D | 100개에서 연결·메모리 상한과 유실 0 |
| 8 | 브라우저→object storage 직접 multipart로 PHP/Nginx body 한도 우회 | A | 5GiB fixture에서 API RSS가 파일 크기와 무관 |
| 9 | orientation 적용 후 EXIF/GPS/XMP/IPTC 제거, 안전한 ICC만 재작성 | B | metadata 재검사와 GPS/serial 부재 확인 |
| 10 | eager preset + allowlisted lazy preset, 불변 URL, CDN, byte-bounded cache | C | URL revision·TTL·eviction·stampede 테스트 |
| 11 | 일반 raster 200MP, AVIF/HEIF 64MP decoder cap, heavy 격리 lane | B/D | 25,000px JPEG와 64MP AVIF를 RSS 한도 내 처리 |
| 12 | FFmpeg 재시도 후 MP4/H.264 한정 Rust demux/decode 폴백 | B | FFmpeg 비활성 상태에서 H.264 frame 추출 |
| 13 | weighted semaphore, native thread 한도, cgroup CPU/RSS/process 제한 | B/D | CPUQuota 아래에서 API health 유지 |
| 14 | 변환은 항상 durable queue, 대량·heavy는 별도 lane과 backpressure | D | 100개 batch, crash·선점 만료 복구 |
| 15 | versioned watermark preset, 위치·비율·투명도 allowlist | C | watermark hash 기반 멱등 결과 |
| 16 | G7 관리자 설정→서명된 versioned policy snapshot→Rust 반영 | C | revision 적용·rollback·권한 테스트 |
| 17 | 수명주기, 삭제, quota, 관측, 장애복구 등 운영 필수 기능 | E | 운영 runbook과 장애 주입 smoke |

## 4. 단계별 실행 순서

현재 진행률:

- 단계 0 계약 변경: 완료
- 단계 A 직접·재개 가능 업로드: 내부·MinIO 게이트 완료, 실 provider 대기
- 단계 B 미디어 보안·가공: 완료
- 단계 C G7 모듈·썸네일·워터마크: 제어 업로더, 이미지·영상 poster 렌더링,
  서명 policy revision, form 자동 연결, Ready→native attachment bridge, 권한 viewer redirect와
  soft-delete 보존 대조, 관리자 asset picker와 실브라우저 권한 매트릭스 완료; upstream merge·
  실 provider 보존 삭제 대기
- 단계 D 멀티업로드 큐·자원 통제: lease/heartbeat, 100 JPEG RSS·crash 복구,
  25,000px heavy semaphore, tenant fair queue·active capacity·Linux cgroup 부하 통과
- 단계 E 운영 완성: lifecycle, quota, orphan inventory, backup·restore, API admission,
  queue·worker 관측, 장기 tombstone 보존 구현 완료

현재 코드 증거:

- Stage A: batch/single/multipart 계약, S3/R2 adapter, 정확한 크기 확인, 상태 API 구현
- Stage A conformance: pinned MinIO에서 실제 presigned PUT, 2-part complete, abort, HEAD,
  bounded download와 derivative PUT 통과
- Stage A 5GiB gate: API가 발급한 32MiB presigned part 160개로 정확히 5GiB를 MinIO에
  직접 전송하고 80-part 뒤 API 재기동·재개, complete 2회 멱등, 저장 길이·Quarantined
  진입을 검증. API RSS 33,840→34,304KiB(증가 464KiB)
- Stage B: bounded download, signature, libvips/FFprobe probe, EXIF 제거 기본 썸네일 구현
- Stage B publish: 이미지는 metadata-strip·sRGB·최대 8,192px JPEG master, 영상은 검증된
  원 container master, 공통 1,280px JPEG thumbnail/poster를 저장하고 전체 DB set이 원자적으로
  커밋된 뒤에만 Ready 처리
- Stage B native formats: 실제 AVIF encode/decode와 HEIC/HEIF signature·decoder probe·JPEG
  파생 runtime smoke 통과
- Stage B runtime capability: sandbox 내장 fixture로 필수 6개 image input, 4개 output과
  MP4/MOV H.264 poster를 검증하고 API startup fail-closed·HMAC capability snapshot 구현
- Stage B video: 실제 MP4/MOV H.264의 probe·원 container master·JPEG poster·private delivery 종단 통과
- Stage B video fallback: FFmpeg 실행 파일 부재를 주입한 MP4/H.264에서 Rust `mp4` demux,
  OpenH264 첫 frame decode, libvips JPEG 재가공·임시 파일 제거 통과. HEVC/AV1·MOV·WebM의 OpenH264 fallback 차단
- Stage C: G7 관리자 설정/HMAC client, upload ownership, browser direct multi-uploader,
  form state 자동 연결, 원자적 attachment materialization, private viewer redirect와
  lease 기반 soft-delete 보존 대조 구현
- Stage C G5: 5.6.24 core-free hook, HMAC control proxy, browser direct multi-uploader,
  MyISAM advisory-lock attachment 연결과 private delivery를 실제 MySQL·MinIO 브라우저 종단 검증
- Stage D: SQLite lease, heartbeat, retry/dead-letter, bounded worker pool, systemd quota 구현
- Stage D 계약 하네스: 브라우저 100개 batch를 제어 요청 1회·전체 연결 최대 8개로 제한하고,
  Rust 100개 예약을 저장 1회로 커밋하는 테스트 구현
- Stage D 실제 worker 하네스: 4000×3000 JPEG 100개, 동시성 4/native thread 1,
  Ready 100/100·master+thumbnail 200/200, 만료 lease 10/10 재선점, dead-letter 0,
  14.17 jobs/s, p95 274ms, process tree peak RSS 577,584 KiB 통과
- Stage D heavy-image gate: 16,384px/100MP 초과 class를 semaphore 기본 1로 제한하고,
  실제 25,000×4,000 JPEG를 native thread 1, 481ms, peak RSS 43,472 KiB로 처리
- Stage D AVIF memory gate: 실제 64MP AVIF를 peak RSS 1,221,776 KiB로 처리하고, 실측
  3,635,216 KiB였던 200MP AVIF는 header probe 뒤 full-frame decode 전에 거부
- Stage D fair/backpressure: tenant별 마지막 claim sequence로 단일 worker에서도 round-robin,
  활성 예약 global 1,000/tenant 200 hard cap, presign 전·원자적 저장 재검사와 429 응답 구현
- Stage D Linux cgroup: CPU 2 core·memory 2GiB·PID 64·network none 아래 API와 100개 worker
  부하 동시 실행, API health 267/267·작업 100/100·cgroup peak 1,066,602,496 bytes 통과
- Stage E lifecycle: HMAC 삭제 예약, G7 ownership proxy, 만료 multipart abort,
  rejected/failed 보존 만료, derivative/raw 삭제, SQLite lease·retry·attempt 상한과 15분
  systemd timer 구현
- Stage C 워터마크 코어: 이미지·영상 poster 파생물의 bounded 위치·비율·투명도 합성,
  등록 자산 16MiB 복사·SHA-256 pin, revision+digest 불변 key와 fail-closed worker 처리 구현
- Stage C 정책: G7 관리자 필드, 서명된 PUT/GET site policy, tenant Ready asset 검증,
  SQLite 단조 revision·settings hash, enqueue 시 revision 고정과 S3/R2 worker 재검증 구현
- Stage C 정책 종단: 실제 G7 PHP HMAC client로 revision 1을 게시해 worker 파생물의
  watermark digest preset과 출력 바이트 변경을 확인하고 revision 2 해제 후 원본 출력을 복원
- Stage C 관리자 UI: current-admin·Ready·최근 7일·PNG/WebP/JPEG·16MiB 경계 catalog와
  수동 UUID 없는 asset picker를 실제 G7 브라우저에서 선택·저장·재로드·rollback 검증
- Stage C 권한 브라우저: 작성자·다른 회원·비회원·관리자의 공개·비밀·블라인드·삭제글
  private 경로가 허용 시 upstream, 거부 시 upstream 전 403이 되는 매트릭스 검증
- Stage B sandbox egress: Linux seccomp-BPF로 socket 계열 syscall을 전 thread에서 차단하고
  FFmpeg/FFprobe child 상속 및 실제 Linux `EPERM` 테스트 구현
- 품질 게이트: 전체 CI와 API smoke, Rust line coverage 84.64%, G7 PHP/TS unit·build 통과

남은 핵심 게이트는 실제 R2/Lightsail credential별 conformance, G7 upstream merge와 실 provider
보존 삭제입니다. G7 MinIO 기반 실제 저장소 전송·create/update·private thumbnail 전달,
실브라우저 권한 매트릭스와 관리자 watermark asset picker는 통과했습니다. 외부 저장소 하네스와
2026-07-16 실행 인계서는 구현됐습니다.

### 단계 0 — 계약 변경

- `SPEC.md`를 1.1로 올리고 SQLite queue, batch/multipart, G7 module, high-res tier를 확정
- OpenAPI에 batch intent, part presign, complete, abort, resume, asset와
  policy snapshot 계약 추가
- 멀티업로드와 향후 멀티노드를 별개 개념으로 명시하고 v1 지원 경계를 ADR로 고정
- S3/R2의 checksum, ETag, CORS와 presigned 기능 차이를 provider contract에 기록

### 단계 A — 직접·재개 가능 업로드

- 작은 파일은 presigned PUT, 100MiB 이상 또는 영상은 multipart를 기본으로 선택
- part 기본 32MiB, 파일당 4개, 브라우저 전체 8개 연결에서 시작하고 운영 측정으로 조정
- upload ID와 part ETag/checksum을 브라우저 로컬 상태와 서버 session에 보관
- 실패 part만 재전송하고 완료 또는 abort를 반드시 호출
- batch intent는 최대 100개를 한 번에 만들되 tenant quota를 먼저 예약

직접 업로드는 PHP `upload_max_filesize`와 Nginx body buffering을 제거하지만 CORS, presign
만료, object storage 한도까지 없어지는 것은 아닙니다. object storage 없는 환경의 Rust/TUS
streaming gateway는 P2 선택 기능으로 둡니다.

### 단계 B — 미디어 보안·가공

- 확장자와 client MIME은 힌트로만 기록하고 bytes signature, parser, 실제 decode를 모두 확인
- raw key는 서버가 만들고 private quarantine에만 저장
- 이미지: dimension, 총 픽셀, frame 수, decoded byte 예산을 decode 전에 가능한 범위에서 검사
- 영상: ffprobe로 container, codec, duration, stream 수, resolution을 검사한 뒤 FFmpeg 실행
- sandbox는 credential 없이 실행하고 Linux seccomp로 socket 계열 syscall을 차단하며,
  read-only root, 임시 디스크 quota, timeout을 사용
- libvips `strip`, autorotate, sRGB 정규화로 sanitized master와 파생물을 생성
- raw 원본은 공개하지 않고 정책에 따라 성공 직후 또는 짧은 보존기간 뒤 삭제

고해상도는 한 변만 보지 않습니다. 예를 들어 25,000×4,000은 100MP지만
25,000×20,000은 500MP입니다. 후자는 hard cap을 넘으므로 거부하거나 별도 오프라인 tier가
필요합니다. libvips sequential access와 shrink-on-load를 사용해도 decoder가 요구하는 최소
메모리는 사라지지 않습니다.

#### 영상 썸네일 폴백

1. FFmpeg fast seek를 실행합니다.
2. 실패하면 정확 seek와 안전한 중앙 timestamp로 한 번만 재시도합니다.
3. FFmpeg 실행 불가 시 Rust MP4 demux + OpenH264 계열 decoder로 H.264 첫 유효 frame만
   추출하는 feature-gated 폴백을 실행합니다.
4. HEVC/AV1 등 미지원 codec은 가짜 frame을 만들지 않고 명확한 `UNSUPPORTED_CODEC`으로
   종료하거나 검증된 poster만 사용합니다.

3번은 Rust가 lifecycle과 container parsing을 소유하지만 OpenH264 native codec을 사용합니다.
순수 Rust 범용 영상 decoder로 표기하지 않습니다.

FFprobe 검사, FFmpeg fast seek와 실패 시 정확 seek·중앙 timestamp 1회 재시도를
구현했습니다. 재시도 두 번은 전체 timeout을 절반씩 나눠 사용하며 출력은 매 시도 후 다시
검증합니다. FFmpeg `spawn` 실패일 때만 MP4/H.264 allowlist가 OpenH264 폴백을 켭니다.
입력·sample·NAL·decode byte·dimension·pixel을 독립 제한하고 첫 유효 frame을 임시 PPM으로
쓴 뒤 기존 libvips metadata-strip/resize/JPEG 경로를 거칩니다. FFmpeg 부재 주입 smoke를
통과했습니다. sandbox egress는 Linux seccomp 필터와 실제 `EPERM` 테스트로 차단했습니다.

### 단계 C — G7 모듈·썸네일·워터마크

실제 G7 7.0.4 코드 기준으로 모듈 API는 `/api/modules/{module-id}` 아래 등록할 수 있고,
모듈은 관리자 메뉴·권한·layout·settings를 소유할 수 있습니다. 현재 `sirsoft-board` 업로드는
파일마다 PHP 요청을 받고 `file_get_contents()`로 전체 파일을 읽으므로 재사용하지 않습니다.

G7 모듈의 책임:

- 게시판 업로드 권한, 사용자 quota, 활성 게시판 확인
- batch uploader UI, 진행률, 취소·재개, 임시 asset 선택
- Rust control API HMAC 서명
- 완료 asset을 기존 `attachment_ids` 게시물 저장 흐름에 연결
- 관리자 설정과 Rust capability/health 표시
- signed webhook 검증과 멱등 attachment 연결

현재 모듈에는 관리자 설정, Laravel Crypt secret, Rust 호환 HMAC client, 게시판 권한,
`upload_id + user_id + board_slug` 소유권, direct single/multipart uploader, 전체 연결·파일·part
bounded 동시성, 진행률·취소·재시도·ETag/abort, Ready polling·native attachment 생성,
권한 기반 preview/download와 관리자 capability proxy가 구현됐습니다. PHP는 file body를
받지 않습니다. upstream patch가 실제 G7에 반영되기 전에는 runtime 계약 검사로 fail-closed 합니다.

준비한 G7 공개 확장 계약은 일곱 개입니다.

1. 사용자·관리자 첨부 UI와 submit을 겨냥하는 안정적인 layout extension ID
2. CDN/Rust URL을 반환하는 download/preview URL filter와 byte-free 권한 검사
3. `attachment_ids`의 owner·list·최대 수·전건 일치 연결
4. module-prefixed 레이아웃에도 다른 모듈 overlay를 적용하는 core 요청명 계약
5. 조건부 partial의 모든 동일 target에 overlay를 적용하는 core 병합 계약
6. `attachment_ids` bulk link 직후 게시글 첨부 수를 동기화하는 계약
7. 비밀·블라인드·삭제글 첨부 URL을 본문 권한과 동일하게 차단하는 계약

이는 현재 G7 기준 `sirsoft-board` 1.2.0과 patch 5개, 28항목 검증기로 고정했습니다. 설치·설정,
user/admin form과 MinIO 기반 실제 single/multipart 전송·create/update·private thumbnail 전달은
통과했습니다. 권한·삭제/복원·보존 lease G7 DB gate도 통과했습니다. upstream 미반영 배포와
실 provider 보존 삭제 종단은 공식 지원으로 게시하지 않습니다. 권한 차단 실브라우저 매트릭스는
`docs/evidence/G7_WATERMARK_PICKER_AUTH_MATRIX_20260716.md`에서 통과했습니다.

#### 썸네일 URL·캐시

```text
/media/{tenant}/{asset_id}/{source_rev}/{preset_rev}/{variant}.{ext}
```

- 게시판·목록·본문의 공통 preset은 업로드 직후 생성합니다.
- v1은 eager preset만 지원하고 임의 `width/quality` query와 요청 시 변환을 금지합니다.
- 생성 결과는 private object storage에 불변 key로 저장하고 G7 권한 확인 뒤 5분 signed GET으로
  전달합니다. 공개 게시판용 CDN profile은 별도 운영 선택 기능입니다.
- 수정은 overwrite하지 않고 `source_rev` 또는 `preset_rev`를 바꿉니다.
- Rust 메모리는 thumbnail bytes의 진실 원천이 아닙니다. manifest만 byte-weighted
  frequency admission/LRU eviction + TTL로 제한하고 같은 upload의 동시 조회는 singleflight로
  합칩니다. 기본 4MiB·60초이며 삭제 가능 여부는 SQLite에서 매 요청 다시 확인합니다.

워터마크는 관리자 등록 asset, 위치, 여백, 최대 비율, 투명도만 허용합니다. watermark
digest와 설정 revision을 결과 key에 포함해 변경 전후 결과가 섞이지 않게 합니다.

이미지 파생물의 libvips 합성, 위치·크기·투명도 hard bound, 작업별 자산 복사와 SHA-256
검증, revision+digest 불변 key, 영상 poster 2차 합성과 G7 서명 policy snapshot·worker
revision 고정까지 구현됐습니다. 전용 관리자 asset picker의 선택·저장·재로드·rollback
브라우저 smoke도 완료했습니다. 상세 계약은 `docs/WATERMARK.md`입니다.

### 단계 D — 멀티업로드 큐·자원 통제

- SQLite 트랜잭션 기반 작업 선점, heartbeat, bounded retry, dead-letter 구현
- outbox table로 상태 변경과 G7 webhook 전달을 분리
- API와 worker는 같은 서버에서 로컬 SQLite와 S3/R2를 사용
- tenant별 영속 claim sequence 공정 queue 적용
- 일반 slot 안에서 heavy image/video semaphore를 추가 적용
- 일반 image, heavy image/AVIF, video lane을 분리
- 초기 bulk 기준은 `10개 초과` 또는 `합계 256MiB 초과`; 실제 벤치 후 조정
- processing 자체는 크기와 관계없이 항상 queue이며, 기준은 우선순위와 UI 응답만 바꿈

CPU는 애플리케이션 제한과 OS 제한을 함께 사용합니다.

```text
active_sandbox_processes × native_threads <= allocated_cpu_cores
```

- libvips concurrency와 operation cache memory 명시
- FFmpeg `-threads`, timeout, process 수 명시
- 16,384px 초과 또는 100MP 초과 image와 모든 video full-pixel 구간은 일반 slot 안에서
  각각 별도 semaphore를 추가 획득하며 기본 동시성은 1
- systemd/cgroup `CPUQuota`, `MemoryMax`, `TasksMax`, 임시 디스크 quota 적용
- 서버 capacity를 넘으면 queue에서 새 작업을 선점하지 않고 API는 backpressure를 반환

API active reservation 기본값은 global 1,000개, tenant별 200개이며, retained source byte
quota 기본값은 global 1TiB, tenant별 100GiB입니다. batch 생성 전에 먼저 검사해 불필요한
provider session을 만들지 않고, 저장 시 `BEGIN IMMEDIATE` 안에서 다시 검사해 경합 우회도
막습니다. SQLite trigger가 O(1) 전역·tenant 사용량 counter를 원자 갱신하며 삭제 대기 중에는
용량을 반환하지 않습니다. 초과 응답은 `429 UPLOAD_CAPACITY_EXHAUSTED`입니다. 만료된 multipart와
created reservation의 자동 abort, rejected/failed 보존 정리와 사용자 삭제 예약은 Stage E
lifecycle worker로 구현했습니다.

### 단계 E — 운영 완성

- 완료: 미완료 multipart 자동 abort, 사용자 삭제 예약, rejected/failed 원본 보존 만료,
  derivative/raw 객체 정리, G7 soft-delete 보존 대조, tombstone, bounded lease/retry/attempt 상한
- 완료: 전역·tenant retained source byte quota, 원자적 경합 차단, tombstone 이후 용량 반환
- 완료: `raw/`·`media/` bounded provider inventory, durable cursor, 48시간 grace, 비파괴 audit,
  명시적 prune과 삭제 직전 ownership 재검사
- 완료: online SQLite snapshot, SHA-256 manifest, bounded retention, read-only 검증,
  writable rollback을 포함한 격리 restore rehearsal
- 완료: `/v1` token bucket·동시 처리 제한과 안정된 429, health/metrics 우회
- 완료: queue depth·oldest age·dead-letter·worker outcome과 download/inspect/transform/upload/
  commit 단계별 latency, resource wait, reject code의 low-cardinality Prometheus 관측
- 완료: upload·orphan tombstone 기본 365일 보존과 한 실행 최대 100개 bounded purge
- 선택 후속: tenant 내부 digest 중복 제거, CDN purge, ClamAV/moderation hook,
  영상 metadata 제거/remux. v1 공식 기능에는 포함하지 않음

## 5. G7 관리자 설정 소유권

| G7 관리자에서 설정 | Rust 운영자만 설정 |
|---|---|
| 활성 게시판, 사용자별/게시물별 파일 수 | S3/R2 access key, bucket IAM |
| 허용 포맷과 서버 hard cap 이하 크기 | SQLite 경로·백업, queue hard capacity |
| batch 병렬 수와 bulk 기준의 허용 범위 | sandbox CPU/RSS/timeout hard cap |
| thumbnail preset, crop, 품질, 워터마크 | codec build, network·filesystem sandbox |
| EXIF/raw 보존 정책의 허용 범위 | HMAC key 원본과 key rotation authority |
| MediaBooster endpoint, site ID, storage profile 이름 | secret manager와 배포 image digest |

G7 설정은 DB에 저장하되 Rust가 DB를 읽지 않습니다. 저장 시 PHP 모듈이 schema version,
revision, issued_at, settings hash를 가진 policy snapshot을 HMAC 서명해 Rust로 전달합니다.
G7 값은 Rust hard cap을 낮출 수만 있고 높일 수 없습니다. HMAC secret은 Laravel encrypted
sensitive 설정으로 보관하고 브라우저 schema에는 절대 노출하지 않습니다.

## 6. 필수 인수 테스트

- `.jpg` 이름의 PHP/실행파일, MIME 위조, 절단, polyglot, 손상·폭탄 파일 거부
- JPEG/PNG/WebP/GIF/AVIF/HEIC와 MP4/MOV 실제 fixture round-trip
- EXIF GPS, camera serial, XMP/IPTC 제거 후 orientation·색상 정상
- 25,000px panorama와 200MP/400MP tier 경계에서 peak RSS·timeout 검증
- S3와 R2 각각 multipart 중단·재개·abort·중복 complete 멱등성
- 100개 batch의 bounded 스케줄링 계약과 로컬 실제 JPEG worker/만료 lease 복구/RSS 게이트,
  로컬 5GiB multipart의 API 재기동·재개·중복 complete 멱등은 통과. 실 S3/R2에서 같은
  중단·재개와 worker 강제 종료·중복 게시 0은 별도 운영 게이트
- FFmpeg 제거 상태의 MP4/H.264 fallback은 통과. 미지원 codec은 worker allowlist 단위
  테스트로 폴백하지 않음을 확인했으며 손상 fixture 확대는 지속 수행
- cgroup CPU quota 중에도 API health 응답과 queue backpressure 유지
- 완료: thumbnail URL revision, 4MiB/60초 manifest cache, LRU/frequency eviction,
  singleflight, 삭제 guard·invalidation 검증. 공개 CDN profile은 선택 운영 게이트
- G7 관리자 설정 revision, 권한, 실제 게시물 attachment 연결 smoke
- watermark 위치·투명도·digest 기반 결과 멱등성

## 7. 검증 근거

G7 연동 판단은 로컬 Gnuboard7 7.0.4의 다음 실제 계약을 기준으로 했습니다.

- `composer.json`: PHP 8.2 / Laravel 12
- `docs/extension/module-basics.md`: 모듈 API·DB·서비스·권한
- `app/Providers/ModuleRouteServiceProvider.php`: 모듈 API prefix
- `modules/_bundled/sirsoft-board/src/Services/AttachmentService.php`: 현재 PHP 업로드 경로
- `modules/_bundled/sirsoft-board/src/Services/PostService.php`: `attachment_ids` 연결 경로
- `app/Services/ModuleSettingsService.php`: sensitive/프론트 비노출 설정

외부 프로토콜 기준:

- [AWS S3 multipart limits](https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html)
- [AWS S3 multipart process and checksum](https://docs.aws.amazon.com/AmazonS3/latest/userguide/mpuoverview.html)
- [Cloudflare R2 upload and multipart](https://developers.cloudflare.com/r2/objects/upload-objects/)
- [Cloudflare R2 S3 compatibility](https://developers.cloudflare.com/r2/api/s3/api/)
- [FFmpeg protocol allowlist](https://ffmpeg.org/ffmpeg-protocols.html)
- [Moka cache policies](https://docs.rs/moka/latest/moka/)
