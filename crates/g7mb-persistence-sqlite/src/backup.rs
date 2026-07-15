//! Consistent SQLite snapshot creation and read-only restore verification.

use std::{path::Path, str::FromStr as _, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use thiserror::Error;

use crate::SqliteStore;

/// Latest embedded SQLx migration required by this binary.
pub const LATEST_SCHEMA_VERSION: i64 = 10;

/// Integrity and application invariants proven for one database snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseVerification {
    /// Highest successful embedded schema migration.
    pub schema_version: u64,
    /// Durable upload rows.
    pub uploads: u64,
    /// Durable derivative rows.
    pub derivatives: u64,
    /// Durable queue rows.
    pub jobs: u64,
    /// Active orphan suspicions, excluding deletion audit tombstones.
    pub orphan_suspects: u64,
    /// Retained source bytes reconciled against upload rows.
    pub reserved_source_bytes: u64,
}

/// Snapshot or restore verification failure without leaking database contents.
#[derive(Debug, Error)]
pub enum DatabaseBackupError {
    /// Database path is empty, relative, or not UTF-8 representable.
    #[error("database snapshot path is invalid")]
    InvalidPath,
    /// SQLite operation failed.
    #[error("database snapshot operation failed: {0}")]
    Sqlite(#[from] sqlx::Error),
    /// Quick-check, foreign keys, migrations, or quota counters disagree.
    #[error("database snapshot integrity check failed: {0}")]
    Integrity(&'static str),
}

impl SqliteStore {
    /// Creates a compact, transactionally consistent online snapshot with `VACUUM INTO`.
    pub async fn backup_to(
        &self,
        destination: &Path,
    ) -> Result<DatabaseVerification, DatabaseBackupError> {
        let destination = validated_absolute_path(destination)?;
        verify_pool(&self.pool).await?;
        sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(&self.pool)
            .await?;
        sqlx::query("VACUUM INTO ?")
            .bind(destination)
            .execute(&self.pool)
            .await?;
        verify_database_file(Path::new(destination)).await
    }

    /// Verifies the live database using the same checks applied to restore candidates.
    pub async fn verify_database(&self) -> Result<DatabaseVerification, DatabaseBackupError> {
        verify_pool(&self.pool).await
    }
}

/// Opens an existing snapshot read-only and proves SQLite plus application invariants.
pub async fn verify_database_file(
    path: &Path,
) -> Result<DatabaseVerification, DatabaseBackupError> {
    let path = validated_absolute_path(path)?;
    let options = SqliteConnectOptions::from_str("sqlite:")?
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?;
    let result = verify_pool(&pool).await;
    pool.close().await;
    result
}

async fn verify_pool(pool: &SqlitePool) -> Result<DatabaseVerification, DatabaseBackupError> {
    let quick_check = sqlx::query_scalar::<_, String>("PRAGMA quick_check")
        .fetch_all(pool)
        .await?;
    if quick_check.as_slice() != ["ok"] {
        return Err(DatabaseBackupError::Integrity("SQLite quick_check failed"));
    }
    if !sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await?
        .is_empty()
    {
        return Err(DatabaseBackupError::Integrity(
            "SQLite foreign_key_check failed",
        ));
    }
    let schema_version = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(pool)
    .await?
    .ok_or(DatabaseBackupError::Integrity(
        "schema migration history is empty",
    ))?;
    let failed_migrations =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE success <> 1")
            .fetch_one(pool)
            .await?;
    if schema_version != LATEST_SCHEMA_VERSION || failed_migrations != 0 {
        return Err(DatabaseBackupError::Integrity(
            "schema migration history does not match this binary",
        ));
    }

    let stored_reserved_bytes = sqlx::query_scalar::<_, i64>(
        "SELECT reserved_bytes FROM storage_usage_global WHERE singleton = 1",
    )
    .fetch_one(pool)
    .await?;
    let calculated_reserved_bytes = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(expected_size_bytes), 0)
         FROM uploads WHERE state <> 'deleted'",
    )
    .fetch_one(pool)
    .await?;
    if stored_reserved_bytes != calculated_reserved_bytes {
        return Err(DatabaseBackupError::Integrity(
            "global reserved byte counter disagrees with uploads",
        ));
    }
    let tenant_counter_mismatches = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM (
            SELECT tenant_id, reserved_bytes FROM tenant_storage_usage
            EXCEPT
            SELECT tenant_id, SUM(expected_size_bytes)
            FROM uploads WHERE state <> 'deleted' GROUP BY tenant_id
            UNION ALL
            SELECT tenant_id, SUM(expected_size_bytes)
            FROM uploads WHERE state <> 'deleted' GROUP BY tenant_id
            EXCEPT
            SELECT tenant_id, reserved_bytes FROM tenant_storage_usage
         )",
    )
    .fetch_one(pool)
    .await?;
    if tenant_counter_mismatches != 0 {
        return Err(DatabaseBackupError::Integrity(
            "tenant reserved byte counters disagree with uploads",
        ));
    }

    Ok(DatabaseVerification {
        schema_version: nonnegative(schema_version, "schema version is invalid")?,
        uploads: count(pool, "SELECT COUNT(*) FROM uploads").await?,
        derivatives: count(pool, "SELECT COUNT(*) FROM derivatives").await?,
        jobs: count(pool, "SELECT COUNT(*) FROM jobs").await?,
        orphan_suspects: count(
            pool,
            "SELECT COUNT(*) FROM orphan_objects WHERE state = 'suspected'",
        )
        .await?,
        reserved_source_bytes: nonnegative(
            stored_reserved_bytes,
            "reserved source byte count is invalid",
        )?,
    })
}

async fn count(pool: &SqlitePool, query: &'static str) -> Result<u64, DatabaseBackupError> {
    nonnegative(
        sqlx::query_scalar::<_, i64>(query).fetch_one(pool).await?,
        "database row count is invalid",
    )
}

fn nonnegative(value: i64, message: &'static str) -> Result<u64, DatabaseBackupError> {
    u64::try_from(value).map_err(|_| DatabaseBackupError::Integrity(message))
}

fn validated_absolute_path(path: &Path) -> Result<&str, DatabaseBackupError> {
    if !path.is_absolute() {
        return Err(DatabaseBackupError::InvalidPath);
    }
    path.to_str().ok_or(DatabaseBackupError::InvalidPath)
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::{LATEST_SCHEMA_VERSION, verify_database_file};
    use crate::SqliteStore;

    #[tokio::test]
    async fn online_snapshot_is_consistent_and_restore_candidate_is_read_only_verified()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("live.db");
        let source_url = format!("sqlite://{}", source.display());
        let store = SqliteStore::connect(&source_url, 2).await?;
        insert_upload(&store, "one", 1024).await?;

        let snapshot = directory.path().join("snapshot.db");
        let verification = store.backup_to(&snapshot).await?;
        assert_eq!(
            verification.schema_version,
            u64::try_from(LATEST_SCHEMA_VERSION)?
        );
        assert_eq!(verification.uploads, 1);
        assert_eq!(verification.reserved_source_bytes, 1024);

        insert_upload(&store, "two", 2048).await?;
        let snapshot_verification = verify_database_file(&snapshot).await?;
        assert_eq!(snapshot_verification.uploads, 1);
        assert_eq!(snapshot_verification.reserved_source_bytes, 1024);
        assert_eq!(store.verify_database().await?.uploads, 2);

        sqlx::query(
            "UPDATE tenant_storage_usage
             SET reserved_bytes = reserved_bytes + 1
             WHERE tenant_id = 'site-a'",
        )
        .execute(store.pool())
        .await?;
        assert!(store.verify_database().await.is_err());

        let corrupt = directory.path().join("corrupt.db");
        std::fs::write(&corrupt, b"not-a-sqlite-database")?;
        assert!(verify_database_file(&corrupt).await.is_err());
        Ok(())
    }

    async fn insert_upload(
        store: &SqliteStore,
        suffix: &str,
        expected_size: i64,
    ) -> Result<(), sqlx::Error> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 content_type_hint, transfer_kind, created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'created', ?, 'image/jpeg',
                     'single_put', ?, ?)",
        )
        .bind(format!("01900000-0000-7000-8000-0000000000{suffix}"))
        .bind(format!("raw/site-a/{suffix}/source"))
        .bind(expected_size)
        .bind(now)
        .bind(now)
        .execute(store.pool())
        .await?;
        Ok(())
    }
}
