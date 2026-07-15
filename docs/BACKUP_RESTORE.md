# SQLite backup and restore

## 공식 백업 계약

- 실행 중인 SQLite WAL DB는 `VACUUM INTO`로 일관된 새 snapshot을 만듭니다.
- snapshot 생성 전 live DB, 생성 후 backup DB에 각각 `quick_check`, `foreign_key_check`,
  SQLx migration version, 전역·tenant byte counter 일치 검사를 수행합니다.
- 결과는 create-new `0600` 파일과 SHA-256 manifest 쌍으로 게시합니다. 부분 실패 파일은 정리하며
  기존 snapshot을 덮어쓰지 않습니다.
- 기본 로컬 보존 수는 14개이고 설정 범위는 2~365개입니다. 새 snapshot 검증이 성공한 뒤에만
  오래된 G7MediaBooster 명명 파일을 회전합니다.
- 로컬 backup은 장애 복구 편의를 위한 1차 사본입니다. 별도 host/object storage에 암호화해
  복제하고 그 보존·접근 로그를 따로 운영해야 합니다.

```bash
# 설정의 database.backup_directory에 새 snapshot + manifest 생성
g7mb-worker --config /etc/g7mediabooster/g7mb.toml database backup

# manifest의 sha256을 별도 신뢰 경로에서 읽어 검증
g7mb-worker --config /etc/g7mediabooster/g7mb.toml database verify \
  --input /var/lib/g7mediabooster/backups/g7mb-TIMESTAMP.db \
  --expected-sha256 LOWERCASE_SHA256

# 임시 격리 DB로 복사해 writable transaction rollback과 모든 invariant 재검증
g7mb-worker --config /etc/g7mediabooster/g7mb.toml database restore-rehearsal \
  --input /var/lib/g7mediabooster/backups/g7mb-TIMESTAMP.db \
  --expected-sha256 LOWERCASE_SHA256
```

저장소 자체 하네스는 `cargo xtask database-recovery`입니다.

## 실제 복구 순서

1. 선택한 snapshot의 `database verify`와 `restore-rehearsal`을 모두 PASS시킵니다.
2. API, worker, cleanup/inventory/backup timer와 oneshot을 모두 중지합니다.
3. 현재 `g7mb.db`, `g7mb.db-wal`, `g7mb.db-shm`을 삭제하지 말고 시각이 붙은 장애 사본으로
   같은 filesystem에 이동합니다.
4. 검증한 snapshot을 `g7mediabooster:g7mediabooster`, `0600`인 restore candidate로 복사한 뒤
   DB 경로로 원자 rename합니다. snapshot 원본은 그대로 보존합니다.
5. API만 시작해 readiness와 schema version을 확인하고, worker를 시작한 뒤 queue/dead-letter,
   upload/derivative 수, reserved byte 수를 manifest와 대조합니다.
6. 비파괴 provider inventory audit을 실행해 DB와 object storage의 시간 차이를 확인합니다.
7. 관측 기간이 끝날 때까지 장애 사본과 선택한 snapshot을 삭제하지 않습니다.

서비스가 동작 중인 DB 파일을 `cp`로 바로 복사하거나, live DB 위에 snapshot을 덮어쓰는 절차는
지원하지 않습니다. production restore는 반드시 위 offline cutover 절차로만 수행합니다.
