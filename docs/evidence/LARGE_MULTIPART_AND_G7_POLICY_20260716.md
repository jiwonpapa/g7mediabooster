# 5GiB 직접 multipart·G7 정책 종단 증거

- 실행일: 2026-07-16
- 범위: pinned MinIO, 실제 Rust API/worker/sandbox, G7 PHP HMAC client
- 외부 계정: 사용하지 않음

## 정확한 5GiB 직접 multipart

실행 명령:

```bash
cargo xtask large-multipart-smoke
```

결과:

```text
large-multipart-smoke PASS bytes=5368709120 parts=160 api_rss_start_kib=33840 api_rss_peak_kib=34304 api_rss_delta_kib=464 direct_body=1 api_restarts=1 duplicate_complete=1 quarantined=1
```

Rust API가 batch intent와 160개 part 서명, complete만 처리했습니다. 각 32MiB 본문은
presigned URL로 MinIO에 직접 전송했습니다. 80번째 part 뒤 API를 종료·재기동한 다음 같은
SQLite/provider multipart session으로 나머지를 이어 전송했고, 동일 complete를 두 번 호출해
멱등성을 확인했습니다. HEAD 길이 확인 뒤 upload는 `Quarantined`로 진입했습니다. API RSS
증가 상한은 32MiB이며 실측 증가는 464KiB였습니다.

## G7 정책→worker 워터마크→rollback

실행 명령:

```bash
cargo xtask g7-policy-smoke
```

결과:

```text
g7-policy-smoke PASS php_hmac=1 applied_revision=1 rollback_revision=2 worker_pinned=1
full-stack-smoke PASS single_put=1 multipart_parts=2 ready=3 derivatives=6 mov_h264=1 exif_removed=1
```

실제 G7 PHP client가 HMAC으로 revision 1 policy를 게시했고, Ready PNG 자산의 SHA-256을
고정한 worker preset과 워터마크 적용 출력 바이트를 확인했습니다. revision 2에서 워터마크를
해제한 뒤 같은 입력의 master가 기존 무워터마크 출력과 일치하는 것도 확인했습니다.

## 공식 범위 경계

이 증거는 5GiB direct multipart 구조와 G7 정책 종단의 로컬 구현 증거입니다. R2 또는
Lightsail profile, 외부 네트워크 처리량·중단 재개, 실 provider 보존 삭제를 대신하지 않습니다.
