//! SQLite WAL persistence and migration boundary for single-node deployments.

use std::{collections::BTreeSet, str::FromStr, time::Duration};

use async_trait::async_trait;
use g7mb_application::{
    JobFailureDisposition, JobQueue, JobQueueError, LeasedProcessingJob, NonceStore,
    NonceStoreError, ProcessingJob, WatermarkPosition,
    lifecycle::{
        CleanupCandidate, CleanupReason, DeletionRequestError, DeletionRequestOutcome,
        LifecycleRepository, LifecycleRepositoryError,
    },
    policies::{
        PolicyAssetCandidate, PublishPolicyOutcome, SitePolicyRepository,
        SitePolicyRepositoryError, SitePolicySnapshot, StoredWatermarkPolicy,
    },
    processing::{
        ProcessingRepository, ProcessingRepositoryError, ProcessingSource, PublishedDerivative,
    },
    uploads::{
        StoredDerivative, StoredUploadReservation, UploadBatchReservation, UploadCapacityPolicy,
        UploadRepository, UploadRepositoryError, UploadStatusSnapshot,
    },
};
use g7mb_domain::{MediaKind, ObjectKey, UploadId, UploadState, UploadTransfer};
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::{
    Row as _, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

/// Migrated SQLite connection pool.
#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Connects with WAL, foreign keys, bounded concurrency, and applies migrations.
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, StoreError> {
        if max_connections == 0 {
            return Err(StoreError::InvalidConfiguration(
                "max_connections must be at least one".to_owned(),
            ));
        }
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// Exposes the pool to focused persistence adapters, not to HTTP handlers.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[async_trait]
impl SitePolicyRepository for SqliteStore {
    async fn find_policy_asset(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<Option<PolicyAssetCandidate>, SitePolicyRepositoryError> {
        let row = sqlx::query(
            "SELECT id, object_key, declared_kind, state, actual_size_bytes,
                    detected_content_type, source_sha256
             FROM uploads WHERE tenant_id = ? AND id = ?",
        )
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(policy_backend)?;
        row.map(policy_asset_from_row).transpose()
    }

    async fn publish_site_policy(
        &self,
        snapshot: &SitePolicySnapshot,
    ) -> Result<PublishPolicyOutcome, SitePolicyRepositoryError> {
        let revision = i64::try_from(snapshot.revision)
            .map_err(|_| SitePolicyRepositoryError::RevisionConflict)?;
        let schema_version = i64::from(snapshot.schema_version);
        let created_at = OffsetDateTime::now_utc().unix_timestamp();
        let (
            watermark_enabled,
            watermark_upload_id,
            watermark_object_key,
            watermark_byte_len,
            watermark_sha256,
            watermark_position,
            watermark_margin_px,
            watermark_max_width_percent,
            watermark_opacity_percent,
        ) = match snapshot.watermark.as_ref() {
            Some(watermark) => (
                1_i64,
                Some(watermark.asset_upload_id.to_string()),
                Some(watermark.object_key.as_str()),
                Some(i64::try_from(watermark.byte_len).map_err(|_| {
                    SitePolicyRepositoryError::Backend(
                        "watermark length exceeds SQLite range".to_owned(),
                    )
                })?),
                Some(watermark.asset_sha256.as_str()),
                Some(watermark_position_storage(watermark.position)),
                Some(i64::from(watermark.margin_px)),
                Some(i64::from(watermark.max_width_percent)),
                Some(i64::from(watermark.opacity_percent)),
            ),
            None => (0, None, None, None, None, None, None, None, None),
        };
        let result = sqlx::query(
            "INSERT INTO site_policy_snapshots
                (tenant_id, revision, schema_version, issued_at, settings_sha256,
                 watermark_enabled, watermark_upload_id, watermark_object_key,
                 watermark_byte_len, watermark_sha256, watermark_position,
                 watermark_margin_px, watermark_max_width_percent,
                 watermark_opacity_percent, created_at)
             SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
             WHERE ? = COALESCE(
                 (SELECT MAX(revision) + 1 FROM site_policy_snapshots WHERE tenant_id = ?),
                 1
             )
             ON CONFLICT(tenant_id, revision) DO NOTHING",
        )
        .bind(&snapshot.tenant_id)
        .bind(revision)
        .bind(schema_version)
        .bind(snapshot.issued_at.unix_timestamp())
        .bind(&snapshot.settings_sha256)
        .bind(watermark_enabled)
        .bind(watermark_upload_id)
        .bind(watermark_object_key)
        .bind(watermark_byte_len)
        .bind(watermark_sha256)
        .bind(watermark_position)
        .bind(watermark_margin_px)
        .bind(watermark_max_width_percent)
        .bind(watermark_opacity_percent)
        .bind(created_at)
        .bind(revision)
        .bind(&snapshot.tenant_id)
        .execute(&self.pool)
        .await
        .map_err(policy_backend)?;
        if result.rows_affected() == 1 {
            return Ok(PublishPolicyOutcome::Published);
        }
        let existing = sqlx::query_scalar::<_, String>(
            "SELECT settings_sha256 FROM site_policy_snapshots
             WHERE tenant_id = ? AND revision = ?",
        )
        .bind(&snapshot.tenant_id)
        .bind(revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(policy_backend)?;
        if existing.as_deref() == Some(snapshot.settings_sha256.as_str()) {
            Ok(PublishPolicyOutcome::Unchanged)
        } else {
            Err(SitePolicyRepositoryError::RevisionConflict)
        }
    }

    async fn find_active_site_policy(
        &self,
        tenant_id: &str,
    ) -> Result<Option<SitePolicySnapshot>, SitePolicyRepositoryError> {
        let row = sqlx::query(
            "SELECT tenant_id, revision, schema_version, issued_at, settings_sha256,
                    watermark_enabled, watermark_upload_id, watermark_object_key,
                    watermark_byte_len, watermark_sha256, watermark_position,
                    watermark_margin_px, watermark_max_width_percent,
                    watermark_opacity_percent
             FROM site_policy_snapshots
             WHERE tenant_id = ? ORDER BY revision DESC LIMIT 1",
        )
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(policy_backend)?;
        row.map(site_policy_from_row).transpose()
    }

    async fn find_site_policy_revision(
        &self,
        tenant_id: &str,
        revision: u64,
    ) -> Result<Option<SitePolicySnapshot>, SitePolicyRepositoryError> {
        let revision =
            i64::try_from(revision).map_err(|_| SitePolicyRepositoryError::RevisionConflict)?;
        let row = sqlx::query(
            "SELECT tenant_id, revision, schema_version, issued_at, settings_sha256,
                    watermark_enabled, watermark_upload_id, watermark_object_key,
                    watermark_byte_len, watermark_sha256, watermark_position,
                    watermark_margin_px, watermark_max_width_percent,
                    watermark_opacity_percent
             FROM site_policy_snapshots WHERE tenant_id = ? AND revision = ?",
        )
        .bind(tenant_id)
        .bind(revision)
        .fetch_optional(&self.pool)
        .await
        .map_err(policy_backend)?;
        row.map(site_policy_from_row).transpose()
    }
}

#[async_trait]
impl JobQueue for SqliteStore {
    async fn enqueue(&self, job: ProcessingJob) -> Result<(), JobQueueError> {
        if job.preset_id.is_empty() || job.preset_id.len() > 128 {
            return Err(JobQueueError("preset_id is invalid".to_owned()));
        }
        let now = OffsetDateTime::now_utc().unix_timestamp();
        sqlx::query(
            "INSERT OR IGNORE INTO jobs
                (id, upload_id, preset_id, site_policy_revision, state, attempts,
                 available_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, 'queued', 0, ?, ?, ?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(job.upload_id.to_string())
        .bind(job.preset_id)
        .bind(
            job.site_policy_revision
                .map(i64::try_from)
                .transpose()
                .map_err(|_| {
                    JobQueueError("site policy revision exceeds SQLite range".to_owned())
                })?,
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(queue_backend)?;
        Ok(())
    }

    async fn claim_next(
        &self,
        worker_id: &str,
        now: OffsetDateTime,
        lease_for: Duration,
    ) -> Result<Option<LeasedProcessingJob>, JobQueueError> {
        validate_worker_id(worker_id)?;
        let lease_seconds = i64::try_from(lease_for.as_secs())
            .map_err(|_| JobQueueError("lease duration exceeds SQLite range".to_owned()))?;
        if lease_seconds == 0 {
            return Err(JobQueueError("lease duration must be positive".to_owned()));
        }
        let now_timestamp = now.unix_timestamp();
        let lease_until_timestamp = now_timestamp
            .checked_add(lease_seconds)
            .ok_or_else(|| JobQueueError("lease expiration overflow".to_owned()))?;

        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(queue_backend)?;
        let candidate = sqlx::query(
            "SELECT jobs.id, uploads.tenant_id
             FROM jobs
             JOIN uploads ON uploads.id = jobs.upload_id
             LEFT JOIN tenant_queue_state
                ON tenant_queue_state.tenant_id = uploads.tenant_id
             WHERE (jobs.state = 'queued' AND jobs.available_at <= ?)
                OR (jobs.state = 'leased' AND jobs.lease_until IS NOT NULL
                    AND jobs.lease_until <= ?)
             ORDER BY COALESCE(tenant_queue_state.last_claim_sequence, 0) ASC,
                      jobs.available_at ASC, jobs.created_at ASC, jobs.id ASC
             LIMIT 1",
        )
        .bind(now_timestamp)
        .bind(now_timestamp)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(queue_backend)?;
        let Some(candidate) = candidate else {
            transaction.commit().await.map_err(queue_backend)?;
            return Ok(None);
        };
        let job_id = candidate
            .try_get::<String, _>("id")
            .map_err(queue_backend)?;
        let tenant_id = candidate
            .try_get::<String, _>("tenant_id")
            .map_err(queue_backend)?;
        let row = sqlx::query(
            "UPDATE jobs
             SET state = 'leased', attempts = attempts + 1, lease_owner = ?,
                 lease_until = ?, updated_at = ?
             WHERE id = ?
             RETURNING id, upload_id, preset_id, site_policy_revision, attempts, lease_until",
        )
        .bind(worker_id)
        .bind(lease_until_timestamp)
        .bind(now_timestamp)
        .bind(job_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(queue_backend)?;
        let claim_sequence = sqlx::query_scalar::<_, i64>(
            "UPDATE queue_sequence
             SET next_claim_sequence = next_claim_sequence + 1
             WHERE singleton = 1
             RETURNING next_claim_sequence - 1",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(queue_backend)?;
        sqlx::query(
            "INSERT INTO tenant_queue_state (tenant_id, last_claim_sequence)
             VALUES (?, ?)
             ON CONFLICT(tenant_id) DO UPDATE SET
                last_claim_sequence = excluded.last_claim_sequence",
        )
        .bind(tenant_id)
        .bind(claim_sequence)
        .execute(&mut *transaction)
        .await
        .map_err(queue_backend)?;
        transaction.commit().await.map_err(queue_backend)?;

        let upload_id = row
            .try_get::<String, _>("upload_id")
            .map_err(queue_backend)?
            .parse::<UploadId>()
            .map_err(|_| JobQueueError("stored upload_id is invalid".to_owned()))?;
        let attempts = row.try_get::<i64, _>("attempts").map_err(queue_backend)?;
        let attempts = u32::try_from(attempts)
            .map_err(|_| JobQueueError("stored attempt count is invalid".to_owned()))?;
        let lease_until = OffsetDateTime::from_unix_timestamp(
            row.try_get::<i64, _>("lease_until")
                .map_err(queue_backend)?,
        )
        .map_err(|_| JobQueueError("stored lease expiration is invalid".to_owned()))?;
        Ok(Some(LeasedProcessingJob {
            job_id: row.try_get("id").map_err(queue_backend)?,
            job: ProcessingJob {
                upload_id,
                preset_id: row.try_get("preset_id").map_err(queue_backend)?,
                site_policy_revision: row
                    .try_get::<Option<i64>, _>("site_policy_revision")
                    .map_err(queue_backend)?
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| {
                        JobQueueError("stored site policy revision is invalid".to_owned())
                    })?,
            },
            attempts,
            lease_until,
        }))
    }

    async fn renew(
        &self,
        job_id: &str,
        worker_id: &str,
        now: OffsetDateTime,
        lease_for: Duration,
    ) -> Result<OffsetDateTime, JobQueueError> {
        validate_worker_id(worker_id)?;
        let lease_seconds = i64::try_from(lease_for.as_secs())
            .map_err(|_| JobQueueError("lease duration exceeds SQLite range".to_owned()))?;
        if lease_seconds == 0 {
            return Err(JobQueueError("lease duration must be positive".to_owned()));
        }
        let lease_until = now
            .unix_timestamp()
            .checked_add(lease_seconds)
            .ok_or_else(|| JobQueueError("lease expiration overflow".to_owned()))?;
        let row = sqlx::query(
            "UPDATE jobs
             SET lease_until = ?, updated_at = ?
             WHERE id = ? AND state = 'leased' AND lease_owner = ? AND lease_until > ?
             RETURNING lease_until",
        )
        .bind(lease_until)
        .bind(now.unix_timestamp())
        .bind(job_id)
        .bind(worker_id)
        .bind(now.unix_timestamp())
        .fetch_optional(&self.pool)
        .await
        .map_err(queue_backend)?
        .ok_or_else(|| JobQueueError("job lease is not renewable by this worker".to_owned()))?;
        OffsetDateTime::from_unix_timestamp(
            row.try_get::<i64, _>("lease_until")
                .map_err(queue_backend)?,
        )
        .map_err(|_| JobQueueError("renewed lease expiration is invalid".to_owned()))
    }

    async fn complete(
        &self,
        job_id: &str,
        worker_id: &str,
        now: OffsetDateTime,
    ) -> Result<(), JobQueueError> {
        validate_worker_id(worker_id)?;
        let result = sqlx::query(
            "UPDATE jobs
             SET state = 'completed', lease_owner = NULL, lease_until = NULL, updated_at = ?
             WHERE id = ? AND state = 'leased' AND lease_owner = ?",
        )
        .bind(now.unix_timestamp())
        .bind(job_id)
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(queue_backend)?;
        if result.rows_affected() != 1 {
            return Err(JobQueueError(
                "job is not leased by the requesting worker".to_owned(),
            ));
        }
        Ok(())
    }

    async fn fail(
        &self,
        job_id: &str,
        worker_id: &str,
        now: OffsetDateTime,
        retry_at: OffsetDateTime,
        max_attempts: u32,
        error_code: &str,
    ) -> Result<JobFailureDisposition, JobQueueError> {
        validate_worker_id(worker_id)?;
        if max_attempts == 0 || error_code.is_empty() || error_code.len() > 64 {
            return Err(JobQueueError("failure policy is invalid".to_owned()));
        }
        let row = sqlx::query(
            "UPDATE jobs
             SET state = CASE WHEN attempts >= ? THEN 'dead_letter' ELSE 'queued' END,
                 available_at = ?, lease_owner = NULL, lease_until = NULL,
                 last_error_code = ?, updated_at = ?
             WHERE id = ? AND state = 'leased' AND lease_owner = ?
             RETURNING state",
        )
        .bind(i64::from(max_attempts))
        .bind(retry_at.unix_timestamp())
        .bind(error_code)
        .bind(now.unix_timestamp())
        .bind(job_id)
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(queue_backend)?
        .ok_or_else(|| JobQueueError("job is not leased by the requesting worker".to_owned()))?;
        match row
            .try_get::<String, _>("state")
            .map_err(queue_backend)?
            .as_str()
        {
            "queued" => Ok(JobFailureDisposition::RetryScheduled),
            "dead_letter" => Ok(JobFailureDisposition::DeadLetter),
            _ => Err(JobQueueError("stored job state is invalid".to_owned())),
        }
    }
}

#[async_trait]
impl UploadRepository for SqliteStore {
    async fn has_capacity(
        &self,
        tenant_id: &str,
        additional_uploads: usize,
        capacity: UploadCapacityPolicy,
    ) -> Result<bool, UploadRepositoryError> {
        if tenant_id.is_empty() || additional_uploads == 0 {
            return Err(UploadRepositoryError::Backend(
                "capacity preflight input is invalid".to_owned(),
            ));
        }
        let global = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads
             WHERE state IN ('created', 'uploaded', 'quarantined', 'processing')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(upload_backend)?;
        let tenant = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads
             WHERE tenant_id = ?
               AND state IN ('created', 'uploaded', 'quarantined', 'processing')",
        )
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(upload_backend)?;
        capacity_allows(global, tenant, additional_uploads, capacity)
    }

    async fn save_batch(
        &self,
        batch: &UploadBatchReservation,
        capacity: UploadCapacityPolicy,
    ) -> Result<(), UploadRepositoryError> {
        if batch.uploads.is_empty() || batch.uploads.len() > 100 {
            return Err(UploadRepositoryError::Backend(
                "batch file count is outside 1..=100".to_owned(),
            ));
        }
        let total_size = batch.uploads.iter().try_fold(0_u64, |total, upload| {
            total.checked_add(upload.expected_size_bytes)
        });
        let total_size = total_size.ok_or_else(|| {
            UploadRepositoryError::Backend("batch byte total exceeds integer range".to_owned())
        })?;
        let file_count = i64::try_from(batch.uploads.len()).map_err(|_| {
            UploadRepositoryError::Backend("batch file count exceeds SQLite range".to_owned())
        })?;
        let total_size = i64::try_from(total_size).map_err(|_| {
            UploadRepositoryError::Backend("batch byte total exceeds SQLite range".to_owned())
        })?;
        let timestamp = batch.created_at.unix_timestamp();
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(upload_backend)?;
        let global = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads
             WHERE state IN ('created', 'uploaded', 'quarantined', 'processing')",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(upload_backend)?;
        let tenant = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads
             WHERE tenant_id = ?
               AND state IN ('created', 'uploaded', 'quarantined', 'processing')",
        )
        .bind(&batch.tenant_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(upload_backend)?;
        if !capacity_allows(global, tenant, batch.uploads.len(), capacity)? {
            return Err(UploadRepositoryError::CapacityExhausted);
        }

        sqlx::query(
            "INSERT INTO upload_batches (id, tenant_id, state, file_count, expected_size_bytes, created_at, updated_at) VALUES (?, ?, 'created', ?, ?, ?, ?)",
        )
        .bind(batch.batch_id.to_string())
        .bind(&batch.tenant_id)
        .bind(file_count)
        .bind(total_size)
        .bind(timestamp)
        .bind(timestamp)
        .execute(&mut *transaction)
        .await
        .map_err(upload_backend)?;

        for upload in &batch.uploads {
            let expected_size = i64::try_from(upload.expected_size_bytes).map_err(|_| {
                UploadRepositoryError::Backend("upload byte length exceeds SQLite range".to_owned())
            })?;
            let declared_kind = match upload.declared_kind {
                MediaKind::Image => "image",
                MediaKind::Video => "video",
            };
            let (transfer_kind, multipart_part_size, multipart_upload_id) = match &upload.transfer {
                UploadTransfer::SinglePut => ("single_put", None, None),
                UploadTransfer::Multipart { part_size_bytes } => (
                    "multipart",
                    Some(i64::try_from(*part_size_bytes).map_err(|_| {
                        UploadRepositoryError::Backend(
                            "multipart part size exceeds SQLite range".to_owned(),
                        )
                    })?),
                    upload
                        .multipart_upload_id
                        .as_ref()
                        .map(|value| value.expose_secret()),
                ),
            };
            if transfer_kind == "multipart" && multipart_upload_id.is_none() {
                return Err(UploadRepositoryError::Backend(
                    "multipart reservation has no provider session".to_owned(),
                ));
            }
            sqlx::query(
                "INSERT INTO uploads (id, tenant_id, object_key, declared_kind, state, expected_size_bytes, content_type_hint, batch_id, transfer_kind, multipart_part_size_bytes, multipart_upload_id, created_at, updated_at) VALUES (?, ?, ?, ?, 'created', ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(upload.upload_id.to_string())
            .bind(&batch.tenant_id)
            .bind(upload.object_key.as_str())
            .bind(declared_kind)
            .bind(expected_size)
            .bind(&upload.content_type_hint)
            .bind(batch.batch_id.to_string())
            .bind(transfer_kind)
            .bind(multipart_part_size)
            .bind(multipart_upload_id)
            .bind(timestamp)
            .bind(timestamp)
            .execute(&mut *transaction)
            .await
            .map_err(upload_backend)?;
        }

        transaction.commit().await.map_err(upload_backend)
    }

    async fn find_upload(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<Option<StoredUploadReservation>, UploadRepositoryError> {
        let row = sqlx::query(
            "SELECT id, object_key, expected_size_bytes, state, transfer_kind,
                    multipart_part_size_bytes, multipart_upload_id
             FROM uploads
             WHERE tenant_id = ? AND id = ?",
        )
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(upload_backend)?;

        row.map(stored_upload_from_row).transpose()
    }

    async fn find_status(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<Option<UploadStatusSnapshot>, UploadRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(upload_backend)?;
        let upload = sqlx::query(
            "SELECT state, detected_content_type, error_code, delete_requested_at
             FROM uploads WHERE tenant_id = ? AND id = ?",
        )
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(upload_backend)?;
        let Some(upload) = upload else {
            return Ok(None);
        };
        let state = parse_upload_state(
            &upload
                .try_get::<String, _>("state")
                .map_err(upload_backend)?,
        )?;
        let deletion_pending = upload
            .try_get::<Option<i64>, _>("delete_requested_at")
            .map_err(upload_backend)?
            .is_some()
            && state != UploadState::Deleted;
        let rows = sqlx::query(
            "SELECT preset_id, variant, object_key, content_type, byte_len
             FROM derivatives WHERE upload_id = ? ORDER BY preset_id, variant",
        )
        .bind(upload_id.to_string())
        .fetch_all(&mut *transaction)
        .await
        .map_err(upload_backend)?;
        let mut derivatives = rows
            .into_iter()
            .map(|row| {
                let byte_len = row.try_get::<i64, _>("byte_len").map_err(upload_backend)?;
                Ok(StoredDerivative {
                    preset_id: row.try_get("preset_id").map_err(upload_backend)?,
                    variant: row.try_get("variant").map_err(upload_backend)?,
                    object_key: ObjectKey::new(
                        row.try_get::<String, _>("object_key")
                            .map_err(upload_backend)?,
                    )
                    .map_err(|_| {
                        UploadRepositoryError::Backend(
                            "stored derivative key is invalid".to_owned(),
                        )
                    })?,
                    content_type: row.try_get("content_type").map_err(upload_backend)?,
                    byte_len: u64::try_from(byte_len).map_err(|_| {
                        UploadRepositoryError::Backend(
                            "stored derivative length is invalid".to_owned(),
                        )
                    })?,
                })
            })
            .collect::<Result<Vec<_>, UploadRepositoryError>>()?;
        if deletion_pending || state == UploadState::Deleted {
            derivatives.clear();
        }
        transaction.commit().await.map_err(upload_backend)?;
        Ok(Some(UploadStatusSnapshot {
            upload_id,
            state,
            detected_content_type: upload
                .try_get("detected_content_type")
                .map_err(upload_backend)?,
            error_code: upload.try_get("error_code").map_err(upload_backend)?,
            deletion_pending,
            derivatives,
        }))
    }

    async fn mark_quarantined_and_enqueue(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        actual_size_bytes: u64,
        preset_id: &str,
        now: OffsetDateTime,
    ) -> Result<(), UploadRepositoryError> {
        if preset_id.is_empty() || preset_id.len() > 128 {
            return Err(UploadRepositoryError::Backend(
                "initial processing preset is invalid".to_owned(),
            ));
        }
        let actual_size_bytes = i64::try_from(actual_size_bytes).map_err(|_| {
            UploadRepositoryError::Backend("actual upload size exceeds SQLite range".to_owned())
        })?;
        let mut transaction = self.pool.begin().await.map_err(upload_backend)?;
        let result = sqlx::query(
            "UPDATE uploads
             SET state = 'uploaded', actual_size_bytes = ?, updated_at = ?
             WHERE tenant_id = ? AND id = ? AND state = 'created'",
        )
        .bind(actual_size_bytes)
        .bind(now.unix_timestamp())
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(upload_backend)?;
        if result.rows_affected() != 1 {
            return Err(UploadRepositoryError::Backend(
                "upload is not in the created state".to_owned(),
            ));
        }
        let quarantined = sqlx::query(
            "UPDATE uploads
             SET state = 'quarantined', updated_at = ?
             WHERE tenant_id = ? AND id = ? AND state = 'uploaded'",
        )
        .bind(now.unix_timestamp())
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(upload_backend)?;
        if quarantined.rows_affected() != 1 {
            return Err(UploadRepositoryError::Backend(
                "upload could not enter quarantine".to_owned(),
            ));
        }
        sqlx::query(
            "INSERT INTO jobs
                (id, upload_id, preset_id, site_policy_revision, state, attempts,
                 available_at, created_at, updated_at)
             VALUES (?, ?, ?,
                     (SELECT MAX(revision) FROM site_policy_snapshots WHERE tenant_id = ?),
                     'queued', 0, ?, ?, ?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(upload_id.to_string())
        .bind(preset_id)
        .bind(tenant_id)
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(&mut *transaction)
        .await
        .map_err(upload_backend)?;
        transaction.commit().await.map_err(upload_backend)
    }

    async fn mark_deleted(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        now: OffsetDateTime,
    ) -> Result<(), UploadRepositoryError> {
        let result = sqlx::query(
            "UPDATE uploads
             SET state = 'deleted', updated_at = ?
             WHERE tenant_id = ? AND id = ? AND state = 'created'",
        )
        .bind(now.unix_timestamp())
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(upload_backend)?;
        if result.rows_affected() == 1 {
            return Ok(());
        }
        let state = sqlx::query_scalar::<_, String>(
            "SELECT state FROM uploads WHERE tenant_id = ? AND id = ?",
        )
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(upload_backend)?;
        if state.as_deref() == Some("deleted") {
            Ok(())
        } else {
            Err(UploadRepositoryError::Backend(
                "upload is not abortable from its current state".to_owned(),
            ))
        }
    }
}

#[async_trait]
impl LifecycleRepository for SqliteStore {
    async fn request_deletion(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        now: OffsetDateTime,
    ) -> Result<DeletionRequestOutcome, DeletionRequestError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(deletion_backend)?;
        let stored = sqlx::query(
            "SELECT state, delete_requested_at
             FROM uploads WHERE tenant_id = ? AND id = ?",
        )
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(deletion_backend)?
        .ok_or(DeletionRequestError::NotFound)?;
        let state = stored
            .try_get::<String, _>("state")
            .map_err(deletion_backend)?;
        if state == "deleted" {
            return Ok(DeletionRequestOutcome::AlreadyDeleted);
        }
        if stored
            .try_get::<Option<i64>, _>("delete_requested_at")
            .map_err(deletion_backend)?
            .is_some()
        {
            return Ok(DeletionRequestOutcome::AlreadyPending);
        }
        if !matches!(state.as_str(), "created" | "ready" | "rejected" | "failed") {
            return Err(DeletionRequestError::StateConflict);
        }
        let policy_references = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM site_policy_snapshots WHERE watermark_upload_id = ?",
        )
        .bind(upload_id.to_string())
        .fetch_one(&mut *transaction)
        .await
        .map_err(deletion_backend)?;
        if policy_references != 0 {
            return Err(DeletionRequestError::ReferencedByPolicy);
        }
        let updated = sqlx::query(
            "UPDATE uploads
             SET delete_requested_at = ?, cleanup_retry_at = ?, cleanup_error_code = NULL,
                 updated_at = ?
             WHERE tenant_id = ? AND id = ? AND delete_requested_at IS NULL
               AND state IN ('created', 'ready', 'rejected', 'failed')",
        )
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .bind(tenant_id)
        .bind(upload_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(deletion_backend)?;
        if updated.rows_affected() != 1 {
            return Err(DeletionRequestError::StateConflict);
        }
        transaction.commit().await.map_err(deletion_backend)?;
        Ok(DeletionRequestOutcome::Accepted)
    }

    async fn claim_cleanup_batch(
        &self,
        owner: &str,
        now: OffsetDateTime,
        created_before: OffsetDateTime,
        rejected_before: OffsetDateTime,
        lease_for: Duration,
        limit: usize,
        max_attempts: u32,
    ) -> Result<Vec<CleanupCandidate>, LifecycleRepositoryError> {
        if owner.is_empty()
            || owner.len() > 128
            || !owner
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || lease_for.is_zero()
            || lease_for > Duration::from_secs(60 * 60)
            || !(1..=100).contains(&limit)
            || !(1..=100).contains(&max_attempts)
        {
            return Err(LifecycleRepositoryError(
                "cleanup claim policy is invalid".to_owned(),
            ));
        }
        let lease_seconds = i64::try_from(lease_for.as_secs())
            .map_err(|_| LifecycleRepositoryError("cleanup lease is invalid".to_owned()))?;
        let lease_until = now
            .unix_timestamp()
            .checked_add(lease_seconds)
            .ok_or_else(|| LifecycleRepositoryError("cleanup lease overflow".to_owned()))?;
        let limit = i64::try_from(limit)
            .map_err(|_| LifecycleRepositoryError("cleanup limit is invalid".to_owned()))?;
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(lifecycle_backend)?;
        let rows = sqlx::query(
            "SELECT id, tenant_id, object_key, expected_size_bytes, state, transfer_kind,
                    multipart_part_size_bytes, multipart_upload_id, delete_requested_at,
                    cleanup_attempts
             FROM uploads
             WHERE state != 'deleted'
               AND cleanup_attempts < ?
               AND (cleanup_retry_at IS NULL OR cleanup_retry_at <= ?)
               AND (cleanup_lease_until IS NULL OR cleanup_lease_until <= ?)
               AND (
                    delete_requested_at IS NOT NULL
                    OR (state = 'created' AND created_at <= ?)
                    OR (state IN ('rejected', 'failed') AND updated_at <= ?)
               )
             ORDER BY CASE WHEN delete_requested_at IS NOT NULL THEN 0 ELSE 1 END,
                      COALESCE(delete_requested_at, updated_at), id
             LIMIT ?",
        )
        .bind(i64::from(max_attempts))
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .bind(created_before.unix_timestamp())
        .bind(rejected_before.unix_timestamp())
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await
        .map_err(lifecycle_backend)?;
        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows {
            let tenant_id = row
                .try_get::<String, _>("tenant_id")
                .map_err(lifecycle_backend)?;
            let delete_requested = row
                .try_get::<Option<i64>, _>("delete_requested_at")
                .map_err(lifecycle_backend)?
                .is_some();
            let previous_attempts = row
                .try_get::<i64, _>("cleanup_attempts")
                .map_err(lifecycle_backend)?;
            let attempts = previous_attempts
                .checked_add(1)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    LifecycleRepositoryError("cleanup attempts are invalid".to_owned())
                })?;
            let stored = stored_upload_from_row(row).map_err(|error| {
                LifecycleRepositoryError(format!("cleanup upload row is invalid: {error}"))
            })?;
            let reason = if delete_requested {
                CleanupReason::UserRequested
            } else if stored.state == UploadState::Created {
                CleanupReason::ExpiredReservation
            } else {
                CleanupReason::RetentionExpired
            };
            let leased = sqlx::query(
                "UPDATE uploads
                 SET cleanup_lease_owner = ?, cleanup_lease_until = ?,
                     cleanup_attempts = cleanup_attempts + 1, cleanup_error_code = NULL
                 WHERE id = ?",
            )
            .bind(owner)
            .bind(lease_until)
            .bind(stored.upload_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(lifecycle_backend)?;
            if leased.rows_affected() != 1 {
                return Err(LifecycleRepositoryError(
                    "cleanup candidate could not be leased".to_owned(),
                ));
            }
            let derivative_rows = sqlx::query(
                "SELECT object_key FROM derivatives WHERE upload_id = ? ORDER BY object_key",
            )
            .bind(stored.upload_id.to_string())
            .fetch_all(&mut *transaction)
            .await
            .map_err(lifecycle_backend)?;
            let derivative_keys = derivative_rows
                .into_iter()
                .map(|derivative| {
                    ObjectKey::new(
                        derivative
                            .try_get::<String, _>("object_key")
                            .map_err(lifecycle_backend)?,
                    )
                    .map_err(|_| {
                        LifecycleRepositoryError("cleanup derivative key is invalid".to_owned())
                    })
                })
                .collect::<Result<Vec<_>, LifecycleRepositoryError>>()?;
            candidates.push(CleanupCandidate {
                tenant_id,
                upload_id: stored.upload_id,
                raw_object_key: stored.object_key,
                state: stored.state,
                transfer: stored.transfer,
                multipart_upload_id: stored.multipart_upload_id,
                derivative_keys,
                reason,
                attempts,
            });
        }
        transaction.commit().await.map_err(lifecycle_backend)?;
        Ok(candidates)
    }

    async fn complete_cleanup(
        &self,
        upload_id: UploadId,
        owner: &str,
        now: OffsetDateTime,
    ) -> Result<(), LifecycleRepositoryError> {
        let updated = sqlx::query(
            "UPDATE uploads
             SET state = 'deleted', deleted_at = ?, updated_at = ?,
                 cleanup_lease_owner = NULL, cleanup_lease_until = NULL,
                 cleanup_retry_at = NULL, cleanup_error_code = NULL
             WHERE id = ? AND state != 'deleted' AND cleanup_lease_owner = ?",
        )
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .bind(upload_id.to_string())
        .bind(owner)
        .execute(&self.pool)
        .await
        .map_err(lifecycle_backend)?;
        if updated.rows_affected() != 1 {
            return Err(LifecycleRepositoryError(
                "cleanup lease is not owned by this worker".to_owned(),
            ));
        }
        Ok(())
    }

    async fn fail_cleanup(
        &self,
        upload_id: UploadId,
        owner: &str,
        now: OffsetDateTime,
        retry_at: OffsetDateTime,
        error_code: &str,
    ) -> Result<(), LifecycleRepositoryError> {
        if retry_at <= now
            || error_code.is_empty()
            || error_code.len() > 64
            || !error_code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(LifecycleRepositoryError(
                "cleanup failure policy is invalid".to_owned(),
            ));
        }
        let updated = sqlx::query(
            "UPDATE uploads
             SET cleanup_lease_owner = NULL, cleanup_lease_until = NULL,
                 cleanup_retry_at = ?, cleanup_error_code = ?, updated_at = ?
             WHERE id = ? AND state != 'deleted' AND cleanup_lease_owner = ?",
        )
        .bind(retry_at.unix_timestamp())
        .bind(error_code)
        .bind(now.unix_timestamp())
        .bind(upload_id.to_string())
        .bind(owner)
        .execute(&self.pool)
        .await
        .map_err(lifecycle_backend)?;
        if updated.rows_affected() != 1 {
            return Err(LifecycleRepositoryError(
                "failed cleanup lease is not owned by this worker".to_owned(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ProcessingRepository for SqliteStore {
    async fn find_processing_source(
        &self,
        upload_id: UploadId,
    ) -> Result<Option<ProcessingSource>, ProcessingRepositoryError> {
        let row = sqlx::query(
            "SELECT id, tenant_id, object_key, declared_kind, expected_size_bytes, state,
                    detected_content_type, source_sha256
             FROM uploads WHERE id = ?",
        )
        .bind(upload_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(processing_backend)?;
        row.map(processing_source_from_row).transpose()
    }

    async fn start_processing(
        &self,
        upload_id: UploadId,
        detected_content_type: &str,
        source_sha256: &str,
        now: OffsetDateTime,
    ) -> Result<(), ProcessingRepositoryError> {
        if detected_content_type.is_empty()
            || detected_content_type.len() > 255
            || !detected_content_type
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
            || source_sha256.len() != 64
            || !source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ProcessingRepositoryError(
                "detected source facts are invalid".to_owned(),
            ));
        }
        let result = sqlx::query(
            "UPDATE uploads
             SET state = 'processing', detected_content_type = ?, source_sha256 = ?,
                 error_code = NULL, updated_at = ?
             WHERE id = ? AND state = 'quarantined'",
        )
        .bind(detected_content_type)
        .bind(source_sha256)
        .bind(now.unix_timestamp())
        .bind(upload_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(processing_backend)?;
        if result.rows_affected() == 1 {
            return Ok(());
        }
        let stored = sqlx::query(
            "SELECT state, detected_content_type, source_sha256 FROM uploads WHERE id = ?",
        )
        .bind(upload_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(processing_backend)?;
        if stored.is_some_and(|row| {
            row.try_get::<String, _>("state").ok().as_deref() == Some("processing")
                && row
                    .try_get::<String, _>("detected_content_type")
                    .ok()
                    .as_deref()
                    == Some(detected_content_type)
                && row.try_get::<String, _>("source_sha256").ok().as_deref() == Some(source_sha256)
        }) {
            Ok(())
        } else {
            Err(ProcessingRepositoryError(
                "source is not startable from its current state".to_owned(),
            ))
        }
    }

    async fn publish_derivatives(
        &self,
        upload_id: UploadId,
        derivatives: &[PublishedDerivative],
        now: OffsetDateTime,
    ) -> Result<(), ProcessingRepositoryError> {
        if derivatives.is_empty() || derivatives.len() > 16 {
            return Err(ProcessingRepositoryError(
                "published derivative set size is invalid".to_owned(),
            ));
        }
        let mut identities = BTreeSet::new();
        for derivative in derivatives {
            validate_derivative(derivative)?;
            if !identities.insert((&derivative.preset_id, &derivative.variant)) {
                return Err(ProcessingRepositoryError(
                    "published derivative set contains a duplicate identity".to_owned(),
                ));
            }
        }
        // Reserve SQLite's single writer before reading the current state. A deferred
        // transaction can otherwise fail with SQLITE_BUSY_SNAPSHOT when concurrent
        // workers try to upgrade their read snapshots after another worker commits.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(processing_backend)?;
        let state = sqlx::query_scalar::<_, String>("SELECT state FROM uploads WHERE id = ?")
            .bind(upload_id.to_string())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(processing_backend)?
            .ok_or_else(|| ProcessingRepositoryError("upload was not found".to_owned()))?;
        if state != "processing" && state != "ready" {
            return Err(ProcessingRepositoryError(
                "source is not publishing from its current state".to_owned(),
            ));
        }
        for derivative in derivatives {
            let byte_len = i64::try_from(derivative.byte_len).map_err(|_| {
                ProcessingRepositoryError("derivative byte length exceeds SQLite range".to_owned())
            })?;
            let derivative_result = sqlx::query(
                "INSERT INTO derivatives
                    (upload_id, preset_id, variant, object_key, content_type, byte_len, sha256, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(upload_id, preset_id, variant) DO UPDATE SET
                    object_key = excluded.object_key,
                    content_type = excluded.content_type,
                    byte_len = excluded.byte_len,
                    sha256 = excluded.sha256
                 WHERE derivatives.object_key = excluded.object_key
                   AND derivatives.content_type = excluded.content_type
                   AND derivatives.byte_len = excluded.byte_len
                   AND derivatives.sha256 = excluded.sha256",
            )
            .bind(upload_id.to_string())
            .bind(&derivative.preset_id)
            .bind(&derivative.variant)
            .bind(derivative.object_key.as_str())
            .bind(&derivative.content_type)
            .bind(byte_len)
            .bind(&derivative.sha256)
            .bind(now.unix_timestamp())
            .execute(&mut *transaction)
            .await
            .map_err(processing_backend)?;
            if derivative_result.rows_affected() != 1 {
                return Err(ProcessingRepositoryError(
                    "published derivative conflicts with immutable state".to_owned(),
                ));
            }
        }
        if state == "processing" {
            let updated = sqlx::query(
                "UPDATE uploads SET state = 'ready', updated_at = ?
                 WHERE id = ? AND state = 'processing'",
            )
            .bind(now.unix_timestamp())
            .bind(upload_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(processing_backend)?;
            if updated.rows_affected() != 1 {
                return Err(ProcessingRepositoryError(
                    "source could not enter ready state".to_owned(),
                ));
            }
        }
        transaction.commit().await.map_err(processing_backend)
    }

    async fn mark_rejected(
        &self,
        upload_id: UploadId,
        error_code: &str,
        now: OffsetDateTime,
    ) -> Result<(), ProcessingRepositoryError> {
        if error_code.is_empty()
            || error_code.len() > 64
            || !error_code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ProcessingRepositoryError(
                "source rejection code is invalid".to_owned(),
            ));
        }
        let result = sqlx::query(
            "UPDATE uploads
             SET state = 'rejected', error_code = ?, updated_at = ?
             WHERE id = ? AND state IN ('quarantined', 'processing')",
        )
        .bind(error_code)
        .bind(now.unix_timestamp())
        .bind(upload_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(processing_backend)?;
        if result.rows_affected() == 1 {
            return Ok(());
        }
        let stored = sqlx::query("SELECT state, error_code FROM uploads WHERE id = ?")
            .bind(upload_id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(processing_backend)?;
        if stored.is_some_and(|row| {
            row.try_get::<String, _>("state").ok().as_deref() == Some("rejected")
                && row.try_get::<String, _>("error_code").ok().as_deref() == Some(error_code)
        }) {
            Ok(())
        } else {
            Err(ProcessingRepositoryError(
                "source is not rejectable from its current state".to_owned(),
            ))
        }
    }
}

#[async_trait]
impl NonceStore for SqliteStore {
    async fn consume(
        &self,
        key_id: &str,
        nonce: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<(), NonceStoreError> {
        if key_id.is_empty()
            || key_id.len() > 128
            || nonce.len() < 16
            || nonce.len() > 128
            || expires_at <= now
        {
            return Err(NonceStoreError::Backend(
                "nonce reservation input is invalid".to_owned(),
            ));
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| NonceStoreError::Backend(error.to_string()))?;
        sqlx::query("DELETE FROM request_nonces WHERE expires_at <= ?")
            .bind(now.unix_timestamp())
            .execute(&mut *transaction)
            .await
            .map_err(|error| NonceStoreError::Backend(error.to_string()))?;
        let result = sqlx::query(
            "INSERT OR IGNORE INTO request_nonces (key_id, nonce, expires_at) VALUES (?, ?, ?)",
        )
        .bind(key_id)
        .bind(nonce)
        .bind(expires_at.unix_timestamp())
        .execute(&mut *transaction)
        .await
        .map_err(|error| NonceStoreError::Backend(error.to_string()))?;
        if result.rows_affected() != 1 {
            return Err(NonceStoreError::Replay);
        }
        transaction
            .commit()
            .await
            .map_err(|error| NonceStoreError::Backend(error.to_string()))
    }
}

fn validate_worker_id(worker_id: &str) -> Result<(), JobQueueError> {
    if worker_id.is_empty()
        || worker_id.len() > 128
        || !worker_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(JobQueueError("worker_id is invalid".to_owned()));
    }
    Ok(())
}

fn queue_backend(error: impl std::fmt::Display) -> JobQueueError {
    JobQueueError(format!("SQLite backend: {error}"))
}

fn capacity_allows(
    global: i64,
    tenant: i64,
    additional_uploads: usize,
    capacity: UploadCapacityPolicy,
) -> Result<bool, UploadRepositoryError> {
    let global = usize::try_from(global).map_err(|_| {
        UploadRepositoryError::Backend("global active upload count is invalid".to_owned())
    })?;
    let tenant = usize::try_from(tenant).map_err(|_| {
        UploadRepositoryError::Backend("tenant active upload count is invalid".to_owned())
    })?;
    if additional_uploads == 0
        || capacity.max_active_global == 0
        || capacity.max_active_per_tenant == 0
        || capacity.max_active_per_tenant > capacity.max_active_global
    {
        return Err(UploadRepositoryError::Backend(
            "upload capacity policy is invalid".to_owned(),
        ));
    }
    Ok(global
        .checked_add(additional_uploads)
        .is_some_and(|next| next <= capacity.max_active_global)
        && tenant
            .checked_add(additional_uploads)
            .is_some_and(|next| next <= capacity.max_active_per_tenant))
}

fn upload_backend(error: impl std::fmt::Display) -> UploadRepositoryError {
    UploadRepositoryError::Backend(format!("SQLite backend: {error}"))
}

fn deletion_backend(error: impl std::fmt::Display) -> DeletionRequestError {
    DeletionRequestError::Backend(format!("SQLite backend: {error}"))
}

fn lifecycle_backend(error: impl std::fmt::Display) -> LifecycleRepositoryError {
    LifecycleRepositoryError(format!("SQLite backend: {error}"))
}

fn policy_backend(error: impl std::fmt::Display) -> SitePolicyRepositoryError {
    SitePolicyRepositoryError::Backend(format!("SQLite backend: {error}"))
}

fn policy_asset_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<PolicyAssetCandidate, SitePolicyRepositoryError> {
    let upload_id = row
        .try_get::<String, _>("id")
        .map_err(policy_backend)?
        .parse::<UploadId>()
        .map_err(|_| policy_backend("stored policy asset upload_id is invalid"))?;
    let object_key = ObjectKey::new(
        row.try_get::<String, _>("object_key")
            .map_err(policy_backend)?,
    )
    .map_err(|_| policy_backend("stored policy asset object key is invalid"))?;
    let declared_kind = match row
        .try_get::<String, _>("declared_kind")
        .map_err(policy_backend)?
        .as_str()
    {
        "image" => MediaKind::Image,
        "video" => MediaKind::Video,
        _ => return Err(policy_backend("stored policy asset media kind is invalid")),
    };
    let state = parse_upload_state(&row.try_get::<String, _>("state").map_err(policy_backend)?)
        .map_err(|error| policy_backend(error.to_string()))?;
    let byte_len = row
        .try_get::<Option<i64>, _>("actual_size_bytes")
        .map_err(policy_backend)?
        .ok_or_else(|| policy_backend("stored policy asset has no confirmed length"))?;
    Ok(PolicyAssetCandidate {
        upload_id,
        object_key,
        declared_kind,
        state,
        byte_len: u64::try_from(byte_len)
            .map_err(|_| policy_backend("stored policy asset length is invalid"))?,
        detected_content_type: row
            .try_get("detected_content_type")
            .map_err(policy_backend)?,
        source_sha256: row.try_get("source_sha256").map_err(policy_backend)?,
    })
}

fn site_policy_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<SitePolicySnapshot, SitePolicyRepositoryError> {
    let revision = row.try_get::<i64, _>("revision").map_err(policy_backend)?;
    let schema_version = row
        .try_get::<i64, _>("schema_version")
        .map_err(policy_backend)?;
    let issued_at = OffsetDateTime::from_unix_timestamp(
        row.try_get::<i64, _>("issued_at").map_err(policy_backend)?,
    )
    .map_err(policy_backend)?;
    let watermark_enabled = row
        .try_get::<i64, _>("watermark_enabled")
        .map_err(policy_backend)?;
    let watermark = match watermark_enabled {
        0 => None,
        1 => {
            let upload_id = required_policy_column::<String>(&row, "watermark_upload_id")?
                .parse::<UploadId>()
                .map_err(|_| policy_backend("stored watermark upload_id is invalid"))?;
            let object_key = ObjectKey::new(required_policy_column::<String>(
                &row,
                "watermark_object_key",
            )?)
            .map_err(|_| policy_backend("stored watermark object key is invalid"))?;
            let byte_len = required_policy_column::<i64>(&row, "watermark_byte_len")?;
            let margin_px = required_policy_column::<i64>(&row, "watermark_margin_px")?;
            let max_width_percent =
                required_policy_column::<i64>(&row, "watermark_max_width_percent")?;
            let opacity_percent = required_policy_column::<i64>(&row, "watermark_opacity_percent")?;
            Some(StoredWatermarkPolicy {
                asset_upload_id: upload_id,
                object_key,
                byte_len: u64::try_from(byte_len)
                    .map_err(|_| policy_backend("stored watermark length is invalid"))?,
                asset_sha256: required_policy_column(&row, "watermark_sha256")?,
                position: parse_watermark_position(&required_policy_column::<String>(
                    &row,
                    "watermark_position",
                )?)?,
                margin_px: u32::try_from(margin_px)
                    .map_err(|_| policy_backend("stored watermark margin is invalid"))?,
                max_width_percent: u8::try_from(max_width_percent)
                    .map_err(|_| policy_backend("stored watermark width is invalid"))?,
                opacity_percent: u8::try_from(opacity_percent)
                    .map_err(|_| policy_backend("stored watermark opacity is invalid"))?,
            })
        }
        _ => return Err(policy_backend("stored watermark enabled flag is invalid")),
    };
    Ok(SitePolicySnapshot {
        tenant_id: row.try_get("tenant_id").map_err(policy_backend)?,
        schema_version: u16::try_from(schema_version)
            .map_err(|_| policy_backend("stored policy schema version is invalid"))?,
        revision: u64::try_from(revision)
            .map_err(|_| policy_backend("stored policy revision is invalid"))?,
        issued_at,
        settings_sha256: row.try_get("settings_sha256").map_err(policy_backend)?,
        watermark,
    })
}

fn required_policy_column<T>(
    row: &sqlx::sqlite::SqliteRow,
    name: &'static str,
) -> Result<T, SitePolicyRepositoryError>
where
    T: for<'row> sqlx::Decode<'row, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    row.try_get::<Option<T>, _>(name)
        .map_err(policy_backend)?
        .ok_or_else(|| policy_backend(format!("stored policy column {name} is missing")))
}

const fn watermark_position_storage(position: WatermarkPosition) -> &'static str {
    match position {
        WatermarkPosition::Center => "center",
        WatermarkPosition::TopLeft => "top_left",
        WatermarkPosition::TopRight => "top_right",
        WatermarkPosition::BottomLeft => "bottom_left",
        WatermarkPosition::BottomRight => "bottom_right",
    }
}

fn parse_watermark_position(value: &str) -> Result<WatermarkPosition, SitePolicyRepositoryError> {
    match value {
        "center" => Ok(WatermarkPosition::Center),
        "top_left" => Ok(WatermarkPosition::TopLeft),
        "top_right" => Ok(WatermarkPosition::TopRight),
        "bottom_left" => Ok(WatermarkPosition::BottomLeft),
        "bottom_right" => Ok(WatermarkPosition::BottomRight),
        _ => Err(policy_backend("stored watermark position is invalid")),
    }
}

fn processing_backend(error: impl std::fmt::Display) -> ProcessingRepositoryError {
    ProcessingRepositoryError(format!("SQLite backend: {error}"))
}

fn validate_derivative(derivative: &PublishedDerivative) -> Result<(), ProcessingRepositoryError> {
    let narrow_token = |value: &str, max_len: usize| {
        !value.is_empty()
            && value.len() <= max_len
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    };
    if !narrow_token(&derivative.preset_id, 128)
        || !narrow_token(&derivative.variant, 64)
        || derivative.content_type.is_empty()
        || derivative.content_type.len() > 255
        || !derivative
            .content_type
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
        || derivative.byte_len == 0
        || derivative.sha256.len() != 64
        || !derivative
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ProcessingRepositoryError(
            "published derivative facts are invalid".to_owned(),
        ));
    }
    Ok(())
}

fn processing_source_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<ProcessingSource, ProcessingRepositoryError> {
    let upload_id = row
        .try_get::<String, _>("id")
        .map_err(processing_backend)?
        .parse::<UploadId>()
        .map_err(|_| ProcessingRepositoryError("stored upload_id is invalid".to_owned()))?;
    let declared_kind = match row
        .try_get::<String, _>("declared_kind")
        .map_err(processing_backend)?
        .as_str()
    {
        "image" => MediaKind::Image,
        "video" => MediaKind::Video,
        _ => {
            return Err(ProcessingRepositoryError(
                "stored declared media kind is invalid".to_owned(),
            ));
        }
    };
    let expected_size = row
        .try_get::<i64, _>("expected_size_bytes")
        .map_err(processing_backend)?;
    let state = parse_upload_state(
        &row.try_get::<String, _>("state")
            .map_err(processing_backend)?,
    )
    .map_err(|error| ProcessingRepositoryError(error.to_string()))?;
    Ok(ProcessingSource {
        upload_id,
        tenant_id: row.try_get("tenant_id").map_err(processing_backend)?,
        object_key: ObjectKey::new(
            row.try_get::<String, _>("object_key")
                .map_err(processing_backend)?,
        )
        .map_err(|_| ProcessingRepositoryError("stored object key is invalid".to_owned()))?,
        declared_kind,
        expected_size_bytes: u64::try_from(expected_size)
            .map_err(|_| ProcessingRepositoryError("stored source size is invalid".to_owned()))?,
        state,
        detected_content_type: row
            .try_get::<Option<String>, _>("detected_content_type")
            .map_err(processing_backend)?,
        source_sha256: row
            .try_get::<Option<String>, _>("source_sha256")
            .map_err(processing_backend)?,
    })
}

fn stored_upload_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<StoredUploadReservation, UploadRepositoryError> {
    let upload_id = row
        .try_get::<String, _>("id")
        .map_err(upload_backend)?
        .parse::<UploadId>()
        .map_err(|_| UploadRepositoryError::Backend("stored upload_id is invalid".to_owned()))?;
    let object_key = ObjectKey::new(
        row.try_get::<String, _>("object_key")
            .map_err(upload_backend)?,
    )
    .map_err(|_| UploadRepositoryError::Backend("stored object key is invalid".to_owned()))?;
    let expected_size_bytes = row
        .try_get::<i64, _>("expected_size_bytes")
        .map_err(upload_backend)?;
    let expected_size_bytes = u64::try_from(expected_size_bytes)
        .map_err(|_| UploadRepositoryError::Backend("stored upload size is invalid".to_owned()))?;
    let state = parse_upload_state(&row.try_get::<String, _>("state").map_err(upload_backend)?)?;
    let transfer = match row
        .try_get::<String, _>("transfer_kind")
        .map_err(upload_backend)?
        .as_str()
    {
        "single_put" => UploadTransfer::SinglePut,
        "multipart" => {
            let part_size = row
                .try_get::<Option<i64>, _>("multipart_part_size_bytes")
                .map_err(upload_backend)?
                .ok_or_else(|| {
                    UploadRepositoryError::Backend("multipart upload has no part size".to_owned())
                })?;
            UploadTransfer::Multipart {
                part_size_bytes: u64::try_from(part_size).map_err(|_| {
                    UploadRepositoryError::Backend(
                        "stored multipart part size is invalid".to_owned(),
                    )
                })?,
            }
        }
        _ => {
            return Err(UploadRepositoryError::Backend(
                "stored transfer kind is invalid".to_owned(),
            ));
        }
    };
    let multipart_upload_id = row
        .try_get::<Option<String>, _>("multipart_upload_id")
        .map_err(upload_backend)?
        .map(SecretString::from);
    if matches!(transfer, UploadTransfer::Multipart { .. }) && multipart_upload_id.is_none() {
        return Err(UploadRepositoryError::Backend(
            "multipart upload has no provider session".to_owned(),
        ));
    }
    Ok(StoredUploadReservation {
        upload_id,
        object_key,
        expected_size_bytes,
        state,
        transfer,
        multipart_upload_id,
    })
}

fn parse_upload_state(value: &str) -> Result<UploadState, UploadRepositoryError> {
    match value {
        "created" => Ok(UploadState::Created),
        "uploaded" => Ok(UploadState::Uploaded),
        "quarantined" => Ok(UploadState::Quarantined),
        "processing" => Ok(UploadState::Processing),
        "ready" => Ok(UploadState::Ready),
        "rejected" => Ok(UploadState::Rejected),
        "failed" => Ok(UploadState::Failed),
        "deleted" => Ok(UploadState::Deleted),
        _ => Err(UploadRepositoryError::Backend(
            "stored upload state is invalid".to_owned(),
        )),
    }
}

/// SQLite setup or migration failure.
#[derive(Debug, Error)]
pub enum StoreError {
    /// A bounded resource setting is invalid.
    #[error("invalid SQLite configuration: {0}")]
    InvalidConfiguration(String),
    /// Connection or query failed.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    /// Embedded migration failed.
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use g7mb_application::{
        JobFailureDisposition, JobQueue as _, ProcessingJob, WatermarkPosition,
        lifecycle::{
            CleanupReason, DeletionRequestError, DeletionRequestOutcome, LifecycleRepository as _,
        },
        policies::{
            PublishPolicyOutcome, SitePolicyRepository as _, SitePolicyRepositoryError,
            SitePolicySnapshot, StoredWatermarkPolicy,
        },
        processing::{ProcessingRepository as _, PublishedDerivative},
        uploads::{
            UploadBatchReservation, UploadCapacityPolicy, UploadRepository as _,
            UploadRepositoryError, UploadReservation,
        },
    };
    use g7mb_domain::{MediaKind, ObjectKey, UploadBatchId, UploadId, UploadState, UploadTransfer};
    use secrecy::SecretString;
    use sqlx::Row as _;
    use time::OffsetDateTime;

    use super::SqliteStore;

    #[tokio::test]
    async fn migrations_create_queue_and_nonce_tables() -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let rows = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('upload_batches', 'upload_parts', 'uploads', 'jobs', 'request_nonces', 'queue_sequence', 'tenant_queue_state') ORDER BY name",
        )
        .fetch_all(store.pool())
        .await?;
        let names = rows
            .iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            names,
            vec![
                "jobs",
                "queue_sequence",
                "request_nonces",
                "tenant_queue_state",
                "upload_batches",
                "upload_parts",
                "uploads"
            ]
        );

        sqlx::query("INSERT INTO request_nonces (key_id, nonce, expires_at) VALUES (?, ?, ?)")
            .bind("g5-primary")
            .bind("0123456789abcdef")
            .bind(1_800_000_000_i64)
            .execute(store.pool())
            .await?;
        let replay =
            sqlx::query("INSERT INTO request_nonces (key_id, nonce, expires_at) VALUES (?, ?, ?)")
                .bind("g5-primary")
                .bind("0123456789abcdef")
                .bind(1_800_000_000_i64)
                .execute(store.pool())
                .await;
        assert!(replay.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn sequential_claims_round_robin_across_tenant_backlogs()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000)?;
        let mut owners = Vec::new();
        for tenant_id in ["tenant-a", "tenant-a", "tenant-b", "tenant-b"] {
            let upload_id = UploadId::new();
            sqlx::query(
                "INSERT INTO uploads
                    (id, tenant_id, object_key, state, expected_size_bytes,
                     content_type_hint, created_at, updated_at)
                 VALUES (?, ?, ?, 'quarantined', 1024, 'image/jpeg', ?, ?)",
            )
            .bind(upload_id.to_string())
            .bind(tenant_id)
            .bind(format!("raw/{tenant_id}/{upload_id}/source"))
            .bind(now.unix_timestamp())
            .bind(now.unix_timestamp())
            .execute(store.pool())
            .await?;
            store
                .enqueue(ProcessingJob {
                    upload_id,
                    preset_id: "board-v1".to_owned(),
                    site_policy_revision: None,
                })
                .await?;
            owners.push((upload_id, tenant_id));
        }

        let mut claimed_tenants = Vec::new();
        for index in 0..4 {
            let worker_id = format!("worker-{index}");
            let leased = store
                .claim_next(&worker_id, now, Duration::from_secs(30))
                .await?
                .ok_or("fair queue did not return an eligible job")?;
            let tenant_id = owners
                .iter()
                .find_map(|(upload_id, tenant_id)| {
                    (*upload_id == leased.job.upload_id).then_some(*tenant_id)
                })
                .ok_or("claimed upload owner was not found")?;
            claimed_tenants.push(tenant_id);
            store.complete(&leased.job_id, &worker_id, now).await?;
        }

        assert_eq!(
            claimed_tenants,
            vec!["tenant-a", "tenant-b", "tenant-a", "tenant-b"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn expired_job_claim_is_recovered_without_two_active_owners()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let upload_id = UploadId::new();
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000)?;
        sqlx::query(
            "INSERT INTO uploads (id, tenant_id, object_key, state, expected_size_bytes, content_type_hint, created_at, updated_at) VALUES (?, ?, ?, 'quarantined', ?, ?, ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind("tenant-a")
        .bind(format!("raw/tenant-a/{upload_id}/source"))
        .bind(1024_i64)
        .bind("image/jpeg")
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(store.pool())
        .await?;

        store
            .enqueue(ProcessingJob {
                upload_id,
                preset_id: "board-v1".to_owned(),
                site_policy_revision: None,
            })
            .await?;
        let first = store
            .claim_next("worker-a", now, Duration::from_secs(30))
            .await?
            .ok_or("first worker did not receive queued job")?;
        assert_eq!(first.attempts, 1);
        assert!(
            store
                .claim_next("worker-b", now, Duration::from_secs(30))
                .await?
                .is_none()
        );
        let renewed_until = store
            .renew(
                &first.job_id,
                "worker-a",
                now + time::Duration::seconds(20),
                Duration::from_secs(30),
            )
            .await?;
        assert_eq!(renewed_until, now + time::Duration::seconds(50));
        assert!(
            store
                .claim_next(
                    "worker-b",
                    now + time::Duration::seconds(31),
                    Duration::from_secs(30),
                )
                .await?
                .is_none()
        );

        let recovered = store
            .claim_next(
                "worker-b",
                now + time::Duration::seconds(51),
                Duration::from_secs(30),
            )
            .await?
            .ok_or("expired job was not recovered")?;
        assert_eq!(recovered.job_id, first.job_id);
        assert_eq!(recovered.attempts, 2);
        assert!(
            store
                .complete(&first.job_id, "worker-a", now)
                .await
                .is_err()
        );
        let disposition = store
            .fail(
                &recovered.job_id,
                "worker-b",
                now + time::Duration::seconds(51),
                now + time::Duration::minutes(1),
                2,
                "DECODE_FAILED",
            )
            .await?;
        assert_eq!(disposition, JobFailureDisposition::DeadLetter);
        Ok(())
    }

    #[tokio::test]
    async fn saves_single_and_multipart_reservations_atomically()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let batch_id = UploadBatchId::new();
        let single_id = UploadId::new();
        let multipart_id = UploadId::new();
        let batch = UploadBatchReservation {
            batch_id,
            tenant_id: "tenant-a".to_owned(),
            created_at: OffsetDateTime::from_unix_timestamp(1_800_000_000)?,
            uploads: vec![
                UploadReservation {
                    upload_id: single_id,
                    object_key: ObjectKey::new(format!("raw/tenant-a/{single_id}/source"))?,
                    declared_kind: MediaKind::Image,
                    expected_size_bytes: 1024,
                    content_type_hint: "image/jpeg".to_owned(),
                    transfer: UploadTransfer::SinglePut,
                    multipart_upload_id: None,
                },
                UploadReservation {
                    upload_id: multipart_id,
                    object_key: ObjectKey::new(format!("raw/tenant-a/{multipart_id}/source"))?,
                    declared_kind: MediaKind::Video,
                    expected_size_bytes: 64 * 1024 * 1024,
                    content_type_hint: "video/mp4".to_owned(),
                    transfer: UploadTransfer::Multipart {
                        part_size_bytes: 32 * 1024 * 1024,
                    },
                    multipart_upload_id: Some(SecretString::from("provider-session".to_owned())),
                },
            ],
        };
        store
            .save_batch(&batch, UploadCapacityPolicy::default())
            .await?;

        let batch_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM upload_batches WHERE id = ? AND file_count = 2",
        )
        .bind(batch_id.to_string())
        .fetch_one(store.pool())
        .await?;
        let multipart_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads WHERE batch_id = ? AND transfer_kind = 'multipart' AND multipart_upload_id = 'provider-session'",
        )
        .bind(batch_id.to_string())
        .fetch_one(store.pool())
        .await?;
        assert_eq!(batch_count, 1);
        assert_eq!(multipart_count, 1);
        let stored = store
            .find_upload("tenant-a", multipart_id)
            .await?
            .ok_or("multipart upload was not found")?;
        assert_eq!(stored.state, UploadState::Created);
        assert_eq!(stored.expected_size_bytes, 64 * 1024 * 1024);
        assert_eq!(
            stored.transfer,
            UploadTransfer::Multipart {
                part_size_bytes: 32 * 1024 * 1024
            }
        );
        assert!(stored.multipart_upload_id.is_some());
        store
            .mark_quarantined_and_enqueue(
                "tenant-a",
                multipart_id,
                64 * 1024 * 1024,
                "source-validation-v1",
                OffsetDateTime::from_unix_timestamp(1_800_000_001)?,
            )
            .await?;
        let uploaded = store
            .find_upload("tenant-a", multipart_id)
            .await?
            .ok_or("uploaded reservation disappeared")?;
        assert_eq!(uploaded.state, UploadState::Quarantined);
        let queued = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM jobs WHERE upload_id = ? AND preset_id = 'source-validation-v1' AND state = 'queued'",
        )
        .bind(multipart_id.to_string())
        .fetch_one(store.pool())
        .await?;
        assert_eq!(queued, 1);
        let processing_source = store
            .find_processing_source(multipart_id)
            .await?
            .ok_or("processing source was not found")?;
        assert_eq!(processing_source.tenant_id, "tenant-a");
        assert_eq!(processing_source.declared_kind, MediaKind::Video);
        store
            .start_processing(
                multipart_id,
                "video/mp4",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                OffsetDateTime::from_unix_timestamp(1_800_000_002)?,
            )
            .await?;
        let validation_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads WHERE id = ? AND state = 'processing' AND detected_content_type = 'video/mp4' AND source_sha256 = ?",
        )
        .bind(multipart_id.to_string())
        .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .fetch_one(store.pool())
        .await?;
        assert_eq!(validation_count, 1);
        let thumbnail_key = ObjectKey::new(format!(
            "media/tenant-a/{multipart_id}/source-v1/board-default-v1/thumbnail.jpg"
        ))?;
        let thumbnail = PublishedDerivative {
            object_key: thumbnail_key,
            preset_id: "board-default-v1".to_owned(),
            variant: "thumbnail".to_owned(),
            content_type: "image/jpeg".to_owned(),
            byte_len: 2048,
            sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
        };
        let master = PublishedDerivative {
            object_key: ObjectKey::new(format!(
                "media/tenant-a/{multipart_id}/source-v1/board-default-v1/master.mp4"
            ))?,
            preset_id: "board-default-v1".to_owned(),
            variant: "master".to_owned(),
            content_type: "video/mp4".to_owned(),
            byte_len: 64 * 1024 * 1024,
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        };
        let derivatives = [master, thumbnail];
        sqlx::query(
            "INSERT INTO derivatives
                (upload_id, preset_id, variant, object_key, content_type, byte_len, sha256, created_at)
             VALUES (?, 'board-default-v1', 'thumbnail', ?, 'image/jpeg', 1, ?, ?)",
        )
        .bind(multipart_id.to_string())
        .bind(format!(
            "media/tenant-a/{multipart_id}/source-v1/board-default-v1/conflict.jpg"
        ))
        .bind("c".repeat(64))
        .bind(1_800_000_002_i64)
        .execute(store.pool())
        .await?;
        assert!(
            store
                .publish_derivatives(
                    multipart_id,
                    &derivatives,
                    OffsetDateTime::from_unix_timestamp(1_800_000_003)?,
                )
                .await
                .is_err()
        );
        let rollback_state =
            sqlx::query_scalar::<_, String>("SELECT state FROM uploads WHERE id = ?")
                .bind(multipart_id.to_string())
                .fetch_one(store.pool())
                .await?;
        let rolled_back_master = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM derivatives WHERE upload_id = ? AND variant = 'master'",
        )
        .bind(multipart_id.to_string())
        .fetch_one(store.pool())
        .await?;
        assert_eq!(rollback_state, "processing");
        assert_eq!(rolled_back_master, 0);
        sqlx::query("DELETE FROM derivatives WHERE upload_id = ?")
            .bind(multipart_id.to_string())
            .execute(store.pool())
            .await?;
        store
            .publish_derivatives(
                multipart_id,
                &derivatives,
                OffsetDateTime::from_unix_timestamp(1_800_000_003)?,
            )
            .await?;
        store
            .publish_derivatives(
                multipart_id,
                &derivatives,
                OffsetDateTime::from_unix_timestamp(1_800_000_004)?,
            )
            .await?;
        let ready_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads u JOIN derivatives d ON d.upload_id = u.id WHERE u.id = ? AND u.state = 'ready'",
        )
        .bind(multipart_id.to_string())
        .fetch_one(store.pool())
        .await?;
        assert_eq!(ready_count, 2);
        let status = store
            .find_status("tenant-a", multipart_id)
            .await?
            .ok_or("ready status was not found")?;
        assert_eq!(status.state, UploadState::Ready);
        assert_eq!(status.derivatives.len(), 2);
        assert_eq!(status.derivatives[0].variant, "master");
        assert_eq!(status.derivatives[1].variant, "thumbnail");

        store
            .mark_deleted(
                "tenant-a",
                single_id,
                OffsetDateTime::from_unix_timestamp(1_800_000_002)?,
            )
            .await?;
        store
            .mark_deleted(
                "tenant-a",
                single_id,
                OffsetDateTime::from_unix_timestamp(1_800_000_003)?,
            )
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn active_reservation_capacity_is_atomic_global_and_tenant_scoped()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let capacity = UploadCapacityPolicy {
            max_active_global: 2,
            max_active_per_tenant: 1,
        };
        let tenant_a = single_image_batch("tenant-a", 1_800_000_000)?;
        store.save_batch(&tenant_a, capacity).await?;
        assert!(!store.has_capacity("tenant-a", 1, capacity).await?);
        assert!(store.has_capacity("tenant-b", 1, capacity).await?);

        let tenant_a_overflow = single_image_batch("tenant-a", 1_800_000_001)?;
        assert!(matches!(
            store.save_batch(&tenant_a_overflow, capacity).await,
            Err(UploadRepositoryError::CapacityExhausted)
        ));
        let tenant_b = single_image_batch("tenant-b", 1_800_000_001)?;
        store.save_batch(&tenant_b, capacity).await?;
        assert!(!store.has_capacity("tenant-c", 1, capacity).await?);

        sqlx::query("UPDATE uploads SET state = 'ready' WHERE tenant_id = 'tenant-a'")
            .execute(store.pool())
            .await?;
        assert!(store.has_capacity("tenant-c", 1, capacity).await?);
        Ok(())
    }

    #[tokio::test]
    async fn site_policy_revisions_are_monotonic_idempotent_and_asset_pinned()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 2).await?;
        let upload_id = UploadId::new();
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000)?;
        let object_key = ObjectKey::new(format!("raw/site-a/{upload_id}/source"))?;
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, detected_content_type, source_sha256,
                 created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'ready', 4096, 4096, 'image/png',
                     'image/png', ?, ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(object_key.as_str())
        .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(store.pool())
        .await?;

        let candidate = store
            .find_policy_asset("site-a", upload_id)
            .await?
            .ok_or("policy asset was not found")?;
        assert_eq!(candidate.state, UploadState::Ready);
        assert_eq!(candidate.byte_len, 4096);
        assert!(
            store
                .find_policy_asset("other-site", upload_id)
                .await?
                .is_none()
        );

        let first = SitePolicySnapshot {
            tenant_id: "site-a".to_owned(),
            schema_version: 1,
            revision: 1,
            issued_at: now,
            settings_sha256: "b".repeat(64),
            watermark: Some(StoredWatermarkPolicy {
                asset_upload_id: upload_id,
                object_key,
                byte_len: 4096,
                asset_sha256: "a".repeat(64),
                position: WatermarkPosition::BottomRight,
                margin_px: 24,
                max_width_percent: 20,
                opacity_percent: 80,
            }),
        };
        assert_eq!(
            store.publish_site_policy(&first).await?,
            PublishPolicyOutcome::Published
        );
        assert_eq!(
            store.publish_site_policy(&first).await?,
            PublishPolicyOutcome::Unchanged
        );
        let skipped = SitePolicySnapshot {
            revision: 3,
            settings_sha256: "c".repeat(64),
            ..first.clone()
        };
        assert!(matches!(
            store.publish_site_policy(&skipped).await,
            Err(SitePolicyRepositoryError::RevisionConflict)
        ));
        let conflicting = SitePolicySnapshot {
            settings_sha256: "d".repeat(64),
            ..first.clone()
        };
        assert!(matches!(
            store.publish_site_policy(&conflicting).await,
            Err(SitePolicyRepositoryError::RevisionConflict)
        ));
        let second = SitePolicySnapshot {
            revision: 2,
            issued_at: now + time::Duration::seconds(1),
            settings_sha256: "e".repeat(64),
            watermark: None,
            ..first
        };
        assert_eq!(
            store.publish_site_policy(&second).await?,
            PublishPolicyOutcome::Published
        );
        assert_eq!(
            store
                .find_active_site_policy("site-a")
                .await?
                .ok_or("active policy missing")?
                .revision,
            2
        );
        assert!(
            store
                .find_site_policy_revision("site-a", 1)
                .await?
                .and_then(|snapshot| snapshot.watermark)
                .is_some()
        );
        let source_upload_id = UploadId::new();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 content_type_hint, created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'created', 1024, 'image/jpeg', ?, ?)",
        )
        .bind(source_upload_id.to_string())
        .bind(format!("raw/site-a/{source_upload_id}/source"))
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(store.pool())
        .await?;
        store
            .mark_quarantined_and_enqueue(
                "site-a",
                source_upload_id,
                1024,
                "source-validation-v1",
                now,
            )
            .await?;
        let pinned_revision = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT site_policy_revision FROM jobs WHERE upload_id = ?",
        )
        .bind(source_upload_id.to_string())
        .fetch_one(store.pool())
        .await?;
        assert_eq!(pinned_revision, Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn deletion_request_is_tenant_scoped_idempotent_leased_and_tombstoned()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000)?;
        let upload_id = insert_lifecycle_upload(&store, "tenant-a", "ready", now, false).await?;
        let derivative_key = format!("media/tenant-a/{upload_id}/thumbnail.jpg");
        sqlx::query(
            "INSERT INTO derivatives
                (upload_id, preset_id, variant, object_key, content_type, byte_len, sha256, created_at)
             VALUES (?, 'board-v1', 'thumbnail', ?, 'image/jpeg', 512, ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(&derivative_key)
        .bind("a".repeat(64))
        .bind(now.unix_timestamp())
        .execute(store.pool())
        .await?;

        assert!(matches!(
            store.request_deletion("other-tenant", upload_id, now).await,
            Err(DeletionRequestError::NotFound)
        ));
        assert_eq!(
            store.request_deletion("tenant-a", upload_id, now).await?,
            DeletionRequestOutcome::Accepted
        );
        assert_eq!(
            store.request_deletion("tenant-a", upload_id, now).await?,
            DeletionRequestOutcome::AlreadyPending
        );
        let pending = store
            .find_status("tenant-a", upload_id)
            .await?
            .ok_or("pending upload disappeared")?;
        assert!(pending.deletion_pending);
        assert!(pending.derivatives.is_empty());

        let leased = store
            .claim_cleanup_batch(
                "cleanup-a",
                now,
                now - time::Duration::days(1),
                now - time::Duration::days(7),
                Duration::from_secs(300),
                1,
                10,
            )
            .await?;
        assert_eq!(leased.len(), 1);
        assert_eq!(leased[0].reason, CleanupReason::UserRequested);
        assert_eq!(leased[0].derivative_keys[0].as_str(), derivative_key);
        assert!(
            store
                .claim_cleanup_batch(
                    "cleanup-b",
                    now,
                    now - time::Duration::days(1),
                    now - time::Duration::days(7),
                    Duration::from_secs(300),
                    1,
                    10,
                )
                .await?
                .is_empty()
        );
        assert!(
            store
                .complete_cleanup(upload_id, "cleanup-b", now)
                .await
                .is_err()
        );
        store.complete_cleanup(upload_id, "cleanup-a", now).await?;
        assert_eq!(
            store.request_deletion("tenant-a", upload_id, now).await?,
            DeletionRequestOutcome::AlreadyDeleted
        );
        let deleted = store
            .find_status("tenant-a", upload_id)
            .await?
            .ok_or("deleted tombstone disappeared")?;
        assert_eq!(deleted.state, UploadState::Deleted);
        assert!(!deleted.deletion_pending);
        assert!(deleted.derivatives.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn cleanup_retry_is_delayed_and_stops_at_attempt_ceiling()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000)?;
        let created_at = now - time::Duration::days(2);
        let upload_id =
            insert_lifecycle_upload(&store, "tenant-a", "created", created_at, true).await?;

        let first = store
            .claim_cleanup_batch(
                "cleanup-a",
                now,
                now - time::Duration::days(1),
                now - time::Duration::days(7),
                Duration::from_secs(300),
                100,
                2,
            )
            .await?;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].reason, CleanupReason::ExpiredReservation);
        assert_eq!(first[0].attempts, 1);
        assert!(matches!(
            first[0].transfer,
            UploadTransfer::Multipart { .. }
        ));
        assert!(first[0].multipart_upload_id.is_some());
        let retry_at = now + time::Duration::minutes(1);
        store
            .fail_cleanup(upload_id, "cleanup-a", now, retry_at, "RAW_DELETE_FAILED")
            .await?;
        assert!(
            store
                .claim_cleanup_batch(
                    "cleanup-b",
                    now + time::Duration::seconds(59),
                    now - time::Duration::days(1),
                    now - time::Duration::days(7),
                    Duration::from_secs(300),
                    100,
                    2,
                )
                .await?
                .is_empty()
        );
        let second = store
            .claim_cleanup_batch(
                "cleanup-b",
                retry_at,
                now - time::Duration::days(1),
                now - time::Duration::days(7),
                Duration::from_secs(300),
                100,
                2,
            )
            .await?;
        assert_eq!(second[0].attempts, 2);
        store
            .fail_cleanup(
                upload_id,
                "cleanup-b",
                retry_at,
                retry_at + time::Duration::minutes(1),
                "RAW_DELETE_FAILED",
            )
            .await?;
        assert!(
            store
                .claim_cleanup_batch(
                    "cleanup-c",
                    retry_at + time::Duration::minutes(2),
                    now - time::Duration::days(1),
                    now - time::Duration::days(7),
                    Duration::from_secs(300),
                    100,
                    2,
                )
                .await?
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_and_policy_referenced_uploads_cannot_be_deleted()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = SqliteStore::connect("sqlite::memory:", 1).await?;
        let now = OffsetDateTime::from_unix_timestamp(1_800_000_000)?;
        let active = insert_lifecycle_upload(&store, "tenant-a", "processing", now, false).await?;
        assert!(matches!(
            store.request_deletion("tenant-a", active, now).await,
            Err(DeletionRequestError::StateConflict)
        ));

        let pinned = insert_lifecycle_upload(&store, "tenant-a", "ready", now, false).await?;
        sqlx::query(
            "INSERT INTO site_policy_snapshots
                (tenant_id, revision, schema_version, issued_at, settings_sha256,
                 watermark_enabled, watermark_upload_id, watermark_object_key,
                 watermark_byte_len, watermark_sha256, watermark_position,
                 watermark_margin_px, watermark_max_width_percent,
                 watermark_opacity_percent, created_at)
             VALUES ('tenant-a', 1, 1, ?, ?, 1, ?, ?, 1024, ?,
                     'bottom_right', 24, 20, 80, ?)",
        )
        .bind(now.unix_timestamp())
        .bind("b".repeat(64))
        .bind(pinned.to_string())
        .bind(format!("raw/tenant-a/{pinned}/source"))
        .bind("a".repeat(64))
        .bind(now.unix_timestamp())
        .execute(store.pool())
        .await?;
        assert!(matches!(
            store.request_deletion("tenant-a", pinned, now).await,
            Err(DeletionRequestError::ReferencedByPolicy)
        ));
        Ok(())
    }

    async fn insert_lifecycle_upload(
        store: &SqliteStore,
        tenant_id: &str,
        state: &str,
        now: OffsetDateTime,
        multipart: bool,
    ) -> Result<UploadId, Box<dyn std::error::Error>> {
        let upload_id = UploadId::new();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 content_type_hint, transfer_kind, multipart_part_size_bytes,
                 multipart_upload_id, created_at, updated_at)
             VALUES (?, ?, ?, 'image', ?, 1024, 'image/jpeg', ?, ?, ?, ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(tenant_id)
        .bind(format!("raw/{tenant_id}/{upload_id}/source"))
        .bind(state)
        .bind(if multipart { "multipart" } else { "single_put" })
        .bind(multipart.then_some(5_i64 * 1024 * 1024))
        .bind(multipart.then_some("provider-session"))
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(store.pool())
        .await?;
        Ok(upload_id)
    }

    fn single_image_batch(
        tenant_id: &str,
        timestamp: i64,
    ) -> Result<UploadBatchReservation, Box<dyn std::error::Error>> {
        let batch_id = UploadBatchId::new();
        let upload_id = UploadId::new();
        Ok(UploadBatchReservation {
            batch_id,
            tenant_id: tenant_id.to_owned(),
            created_at: OffsetDateTime::from_unix_timestamp(timestamp)?,
            uploads: vec![UploadReservation {
                upload_id,
                object_key: ObjectKey::new(format!("raw/{tenant_id}/{upload_id}/source"))?,
                declared_kind: MediaKind::Image,
                expected_size_bytes: 1024,
                content_type_hint: "image/jpeg".to_owned(),
                transfer: UploadTransfer::SinglePut,
                multipart_upload_id: None,
            }],
        })
    }
}
