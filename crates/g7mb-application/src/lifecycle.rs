//! Bounded single-node cleanup orchestration for expired and deleted media.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use g7mb_domain::{ObjectKey, UploadId, UploadState, UploadTransfer};
use secrecy::SecretString;
use thiserror::Error;
use time::OffsetDateTime;

use crate::{AbortMultipartRequest, ObjectStore};

/// Why an upload became eligible for irreversible storage cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupReason {
    /// An authenticated application requested deletion.
    UserRequested,
    /// Direct-upload bytes were never confirmed before the reservation TTL.
    ExpiredReservation,
    /// A rejected or failed raw source exceeded its private retention period.
    RetentionExpired,
}

/// One cleanup item exclusively leased to a bounded maintenance run.
#[derive(Clone, Debug)]
pub struct CleanupCandidate {
    /// Authenticated tenant owning the upload.
    pub tenant_id: String,
    /// Stable upload identifier.
    pub upload_id: UploadId,
    /// Private raw/quarantine object key.
    pub raw_object_key: ObjectKey,
    /// Current lifecycle state before cleanup.
    pub state: UploadState,
    /// Direct-upload method originally reserved.
    pub transfer: UploadTransfer,
    /// Provider multipart session for an incomplete multipart reservation.
    pub multipart_upload_id: Option<SecretString>,
    /// Immutable derivative keys removed before the raw source.
    pub derivative_keys: Vec<ObjectKey>,
    /// Reason selected by the durable cleanup policy.
    pub reason: CleanupReason,
    /// Claim attempt number after the current lease was acquired.
    pub attempts: u32,
}

/// Result of an idempotent authenticated deletion request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletionRequestOutcome {
    /// A new durable deletion request was recorded.
    Accepted,
    /// The same upload already has a pending deletion request.
    AlreadyPending,
    /// Storage cleanup and tombstoning already completed.
    AlreadyDeleted,
}

/// Stable deletion-request failure mapped by the HTTP adapter.
#[derive(Debug, Error)]
pub enum DeletionRequestError {
    /// Tenant identifiers are restricted to the authenticated key namespace.
    #[error("tenant identifier is invalid")]
    InvalidTenant,
    /// No tenant-scoped upload exists.
    #[error("upload was not found")]
    NotFound,
    /// Active processing cannot be deleted without a worker cancellation protocol.
    #[error("upload cannot be deleted from its current state")]
    StateConflict,
    /// The upload is pinned by an immutable site-policy revision.
    #[error("upload is referenced by a site policy")]
    ReferencedByPolicy,
    /// Durable state failed without exposing backend details.
    #[error("deletion request repository failed: {0}")]
    Backend(String),
}

/// Durable cleanup queue failure without SQL details.
#[derive(Debug, Error)]
#[error("lifecycle repository failed: {0}")]
pub struct LifecycleRepositoryError(pub String);

/// Persistence boundary for cleanup request, lease, retry, and completion state.
#[async_trait]
pub trait LifecycleRepository: Send + Sync {
    /// Records an idempotent tenant-scoped deletion request.
    async fn request_deletion(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        now: OffsetDateTime,
    ) -> Result<DeletionRequestOutcome, DeletionRequestError>;

    /// Atomically leases a bounded batch of eligible cleanup work.
    #[allow(clippy::too_many_arguments)]
    async fn claim_cleanup_batch(
        &self,
        owner: &str,
        now: OffsetDateTime,
        created_before: OffsetDateTime,
        rejected_before: OffsetDateTime,
        lease_for: Duration,
        limit: usize,
        max_attempts: u32,
    ) -> Result<Vec<CleanupCandidate>, LifecycleRepositoryError>;

    /// Tombstones a candidate only when the caller still owns its cleanup lease.
    async fn complete_cleanup(
        &self,
        upload_id: UploadId,
        owner: &str,
        now: OffsetDateTime,
    ) -> Result<(), LifecycleRepositoryError>;

    /// Releases a failed lease with a bounded retry timestamp and stable error code.
    async fn fail_cleanup(
        &self,
        upload_id: UploadId,
        owner: &str,
        now: OffsetDateTime,
        retry_at: OffsetDateTime,
        error_code: &str,
    ) -> Result<(), LifecycleRepositoryError>;

    /// Purges a bounded batch of old upload and orphan audit tombstones.
    async fn purge_tombstones(
        &self,
        deleted_before: OffsetDateTime,
        limit: usize,
    ) -> Result<usize, LifecycleRepositoryError> {
        let _ = (deleted_before, limit);
        Ok(0)
    }
}

/// Hard cleanup limits owned by the Rust deployment rather than G5/G7 settings.
#[derive(Clone, Copy, Debug)]
pub struct LifecyclePolicy {
    /// Unconfirmed reservation lifetime before automatic cleanup.
    pub created_reservation_ttl: Duration,
    /// Private rejected/failed raw-source retention.
    pub rejected_source_retention: Duration,
    /// Exclusive database lease for one cleanup attempt.
    pub lease_for: Duration,
    /// Delay before a failed storage operation can be reclaimed.
    pub retry_delay: Duration,
    /// Maximum candidates claimed by one maintenance invocation.
    pub batch_size: usize,
    /// Maximum durable attempts before operator intervention is required.
    pub max_attempts: u32,
    /// Audit tombstone retention before bounded physical row removal.
    pub tombstone_retention: Duration,
    /// Maximum old tombstones physically removed per maintenance run.
    pub tombstone_purge_batch_size: usize,
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        Self {
            created_reservation_ttl: Duration::from_secs(24 * 60 * 60),
            rejected_source_retention: Duration::from_secs(7 * 24 * 60 * 60),
            lease_for: Duration::from_secs(5 * 60),
            retry_delay: Duration::from_secs(60),
            batch_size: 100,
            max_attempts: 10,
            tombstone_retention: Duration::from_secs(365 * 24 * 60 * 60),
            tombstone_purge_batch_size: 100,
        }
    }
}

impl LifecyclePolicy {
    fn is_valid(self) -> bool {
        !self.created_reservation_ttl.is_zero()
            && self.created_reservation_ttl <= Duration::from_secs(7 * 24 * 60 * 60)
            && !self.rejected_source_retention.is_zero()
            && self.rejected_source_retention <= Duration::from_secs(90 * 24 * 60 * 60)
            && !self.lease_for.is_zero()
            && self.lease_for <= Duration::from_secs(60 * 60)
            && !self.retry_delay.is_zero()
            && self.retry_delay <= Duration::from_secs(24 * 60 * 60)
            && (1..=100).contains(&self.batch_size)
            && (1..=100).contains(&self.max_attempts)
            && self.tombstone_retention >= Duration::from_secs(30 * 24 * 60 * 60)
            && self.tombstone_retention <= Duration::from_secs(10 * 365 * 24 * 60 * 60)
            && (1..=1000).contains(&self.tombstone_purge_batch_size)
    }
}

/// Aggregate result suitable for logs and Prometheus counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LifecycleRunSummary {
    /// Candidates leased during this invocation.
    pub claimed: usize,
    /// Candidates fully removed and tombstoned.
    pub completed: usize,
    /// Candidates released for retry or operator intervention.
    pub failed: usize,
    /// Failures that reached the configured attempt ceiling.
    pub dead_lettered: usize,
    /// Old audit tombstones physically purged after retention.
    pub tombstones_purged: usize,
}

/// Single-node lifecycle orchestrator using idempotent object operations and durable leases.
#[derive(Clone)]
pub struct LifecycleService {
    raw_store: Arc<dyn ObjectStore>,
    derivative_store: Arc<dyn ObjectStore>,
    repository: Arc<dyn LifecycleRepository>,
    policy: LifecyclePolicy,
}

impl std::fmt::Debug for LifecycleService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LifecycleService")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// Maintenance invocation failure. Per-object storage failures are durably retried instead.
#[derive(Debug, Error)]
pub enum LifecycleRunError {
    /// Cleanup owner identifiers must be safe bounded log and database values.
    #[error("cleanup owner identifier is invalid")]
    InvalidOwner,
    /// Policy duration cannot be represented by the wall-clock type.
    #[error("lifecycle duration is outside the supported range")]
    InvalidDuration,
    /// Durable cleanup state failed.
    #[error(transparent)]
    Repository(#[from] LifecycleRepositoryError),
}

impl LifecycleService {
    /// Creates the service only after all cleanup bounds are valid.
    pub fn new(
        raw_store: Arc<dyn ObjectStore>,
        derivative_store: Arc<dyn ObjectStore>,
        repository: Arc<dyn LifecycleRepository>,
        policy: LifecyclePolicy,
    ) -> Result<Self, LifecycleRunError> {
        if !policy.is_valid() {
            return Err(LifecycleRunError::InvalidDuration);
        }
        Ok(Self {
            raw_store,
            derivative_store,
            repository,
            policy,
        })
    }

    /// Records a deletion request after tenant and repository state checks.
    pub async fn request_deletion(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<DeletionRequestOutcome, DeletionRequestError> {
        if !valid_identifier(tenant_id, 64) {
            return Err(DeletionRequestError::InvalidTenant);
        }
        self.repository
            .request_deletion(tenant_id, upload_id, OffsetDateTime::now_utc())
            .await
    }

    /// Claims and processes one bounded cleanup batch.
    pub async fn run_once(&self, owner: &str) -> Result<LifecycleRunSummary, LifecycleRunError> {
        if !valid_identifier(owner, 128) {
            return Err(LifecycleRunError::InvalidOwner);
        }
        let now = OffsetDateTime::now_utc();
        let created_ttl = time::Duration::try_from(self.policy.created_reservation_ttl)
            .map_err(|_| LifecycleRunError::InvalidDuration)?;
        let rejected_retention = time::Duration::try_from(self.policy.rejected_source_retention)
            .map_err(|_| LifecycleRunError::InvalidDuration)?;
        let candidates = self
            .repository
            .claim_cleanup_batch(
                owner,
                now,
                now - created_ttl,
                now - rejected_retention,
                self.policy.lease_for,
                self.policy.batch_size,
                self.policy.max_attempts,
            )
            .await?;
        let mut summary = LifecycleRunSummary {
            claimed: candidates.len(),
            ..LifecycleRunSummary::default()
        };
        for candidate in candidates {
            match self.cleanup_storage(&candidate).await {
                Ok(()) => {
                    self.repository
                        .complete_cleanup(candidate.upload_id, owner, OffsetDateTime::now_utc())
                        .await?;
                    summary.completed += 1;
                }
                Err(error_code) => {
                    let failed_at = OffsetDateTime::now_utc();
                    let retry_delay = time::Duration::try_from(self.policy.retry_delay)
                        .map_err(|_| LifecycleRunError::InvalidDuration)?;
                    self.repository
                        .fail_cleanup(
                            candidate.upload_id,
                            owner,
                            failed_at,
                            failed_at + retry_delay,
                            error_code,
                        )
                        .await?;
                    summary.failed += 1;
                    if candidate.attempts >= self.policy.max_attempts {
                        summary.dead_lettered += 1;
                    }
                }
            }
        }
        let tombstone_retention = time::Duration::try_from(self.policy.tombstone_retention)
            .map_err(|_| LifecycleRunError::InvalidDuration)?;
        summary.tombstones_purged = self
            .repository
            .purge_tombstones(
                OffsetDateTime::now_utc() - tombstone_retention,
                self.policy.tombstone_purge_batch_size,
            )
            .await?;
        Ok(summary)
    }

    async fn cleanup_storage(&self, candidate: &CleanupCandidate) -> Result<(), &'static str> {
        if candidate.state == UploadState::Created
            && matches!(candidate.transfer, UploadTransfer::Multipart { .. })
        {
            let provider_upload_id = candidate
                .multipart_upload_id
                .clone()
                .ok_or("CLEANUP_STATE_INVALID")?;
            self.raw_store
                .abort_multipart(AbortMultipartRequest {
                    key: candidate.raw_object_key.clone(),
                    upload_id: provider_upload_id,
                })
                .await
                .map_err(|_| "MULTIPART_ABORT_FAILED")?;
        }
        for key in &candidate.derivative_keys {
            self.derivative_store
                .delete(key)
                .await
                .map_err(|_| "DERIVATIVE_DELETE_FAILED")?;
        }
        self.raw_store
            .delete(&candidate.raw_object_key)
            .await
            .map_err(|_| "RAW_DELETE_FAILED")
    }
}

fn valid_identifier(value: &str, max_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use g7mb_domain::{ObjectKey, UploadId, UploadState, UploadTransfer};
    use secrecy::SecretString;
    use time::OffsetDateTime;

    use super::{
        CleanupCandidate, CleanupReason, DeletionRequestError, DeletionRequestOutcome,
        LifecyclePolicy, LifecycleRepository, LifecycleRepositoryError, LifecycleService,
    };
    use crate::{
        AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest,
        DownloadObjectRequest, MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError,
        PresignPartRequest, PresignPutRequest, PresignedUpload, PutFileRequest,
    };

    #[derive(Default)]
    struct FakeStore {
        aborts: AtomicUsize,
        deletes: AtomicUsize,
        fail_delete: AtomicBool,
    }

    #[async_trait]
    impl ObjectStore for FakeStore {
        async fn presign_put(
            &self,
            _request: PresignPutRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }

        async fn create_multipart(
            &self,
            _request: CreateMultipartRequest,
        ) -> Result<MultipartSession, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }

        async fn presign_part(
            &self,
            _request: PresignPartRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }

        async fn complete_multipart(
            &self,
            _request: CompleteMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }

        async fn abort_multipart(
            &self,
            _request: AbortMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            self.aborts.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn head(&self, _key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }

        async fn download_to(
            &self,
            _request: DownloadObjectRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }

        async fn put_file(
            &self,
            _request: PutFileRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }

        async fn delete(&self, _key: &ObjectKey) -> Result<(), ObjectStoreError> {
            self.deletes.fetch_add(1, Ordering::Relaxed);
            if self.fail_delete.load(Ordering::Relaxed) {
                Err(ObjectStoreError::Backend("fixture failure".to_owned()))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct FakeRepository {
        candidates: Mutex<Vec<CleanupCandidate>>,
        completed: AtomicUsize,
        failed: AtomicUsize,
    }

    #[async_trait]
    impl LifecycleRepository for FakeRepository {
        async fn request_deletion(
            &self,
            _tenant_id: &str,
            _upload_id: UploadId,
            _now: OffsetDateTime,
        ) -> Result<DeletionRequestOutcome, DeletionRequestError> {
            Ok(DeletionRequestOutcome::Accepted)
        }

        async fn claim_cleanup_batch(
            &self,
            _owner: &str,
            _now: OffsetDateTime,
            _created_before: OffsetDateTime,
            _rejected_before: OffsetDateTime,
            _lease_for: std::time::Duration,
            _limit: usize,
            _max_attempts: u32,
        ) -> Result<Vec<CleanupCandidate>, LifecycleRepositoryError> {
            let mut candidates = self
                .candidates
                .lock()
                .map_err(|_| LifecycleRepositoryError("fixture lock poisoned".to_owned()))?;
            Ok(std::mem::take(&mut *candidates))
        }

        async fn complete_cleanup(
            &self,
            _upload_id: UploadId,
            _owner: &str,
            _now: OffsetDateTime,
        ) -> Result<(), LifecycleRepositoryError> {
            self.completed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn fail_cleanup(
            &self,
            _upload_id: UploadId,
            _owner: &str,
            _now: OffsetDateTime,
            _retry_at: OffsetDateTime,
            _error_code: &str,
        ) -> Result<(), LifecycleRepositoryError> {
            self.failed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cleanup_aborts_multipart_and_deletes_derivatives_before_tombstoning()
    -> Result<(), Box<dyn std::error::Error>> {
        let raw = Arc::new(FakeStore::default());
        let derivatives = Arc::new(FakeStore::default());
        let repository = Arc::new(FakeRepository::default());
        repository.candidates.lock().map_err(|_| "lock")?.extend([
            candidate(UploadState::Created, true, Vec::new(), 1)?,
            candidate(
                UploadState::Ready,
                false,
                vec![ObjectKey::new("media/site-a/asset/thumb.jpg")?],
                1,
            )?,
        ]);
        let service = LifecycleService::new(
            raw.clone(),
            derivatives.clone(),
            repository.clone(),
            LifecyclePolicy::default(),
        )?;

        let summary = service.run_once("cleanup-test").await?;

        assert_eq!(summary.claimed, 2);
        assert_eq!(summary.completed, 2);
        assert_eq!(raw.aborts.load(Ordering::Relaxed), 1);
        assert_eq!(raw.deletes.load(Ordering::Relaxed), 2);
        assert_eq!(derivatives.deletes.load(Ordering::Relaxed), 1);
        assert_eq!(repository.completed.load(Ordering::Relaxed), 2);
        Ok(())
    }

    #[tokio::test]
    async fn storage_failure_is_released_and_dead_lettered_at_the_attempt_ceiling()
    -> Result<(), Box<dyn std::error::Error>> {
        let raw = Arc::new(FakeStore::default());
        raw.fail_delete.store(true, Ordering::Relaxed);
        let repository = Arc::new(FakeRepository::default());
        repository
            .candidates
            .lock()
            .map_err(|_| "lock")?
            .push(candidate(UploadState::Rejected, false, Vec::new(), 1)?);
        let service = LifecycleService::new(
            raw,
            Arc::new(FakeStore::default()),
            repository.clone(),
            LifecyclePolicy {
                max_attempts: 1,
                ..LifecyclePolicy::default()
            },
        )?;

        let summary = service.run_once("cleanup-test").await?;

        assert_eq!(summary.failed, 1);
        assert_eq!(summary.dead_lettered, 1);
        assert_eq!(repository.failed.load(Ordering::Relaxed), 1);
        assert_eq!(repository.completed.load(Ordering::Relaxed), 0);
        Ok(())
    }

    fn candidate(
        state: UploadState,
        multipart: bool,
        derivative_keys: Vec<ObjectKey>,
        attempts: u32,
    ) -> Result<CleanupCandidate, Box<dyn std::error::Error>> {
        let upload_id = UploadId::new();
        Ok(CleanupCandidate {
            tenant_id: "site-a".to_owned(),
            upload_id,
            raw_object_key: ObjectKey::new(format!("raw/site-a/{upload_id}/source"))?,
            state,
            transfer: if multipart {
                UploadTransfer::Multipart {
                    part_size_bytes: 5 * 1024 * 1024,
                }
            } else {
                UploadTransfer::SinglePut
            },
            multipart_upload_id: multipart.then(|| SecretString::from("provider-id".to_owned())),
            derivative_keys,
            reason: if state == UploadState::Created {
                CleanupReason::ExpiredReservation
            } else {
                CleanupReason::UserRequested
            },
            attempts,
        })
    }
}
