//! Durable source-validation state used by bounded workers.

use async_trait::async_trait;
use g7mb_domain::{MediaKind, ObjectKey, UploadId, UploadState};
use thiserror::Error;
use time::OffsetDateTime;

/// Tenant-owned source facts loaded for a claimed processing job.
#[derive(Clone, Debug)]
pub struct ProcessingSource {
    /// Upload identifier.
    pub upload_id: UploadId,
    /// Owning tenant used for derivative key construction.
    pub tenant_id: String,
    /// Private raw object key.
    pub object_key: ObjectKey,
    /// Authenticated reservation media class.
    pub declared_kind: MediaKind,
    /// Exact immutable source length.
    pub expected_size_bytes: u64,
    /// Current lifecycle state.
    pub state: UploadState,
    /// Previously validated normalized content type, if any.
    pub detected_content_type: Option<String>,
    /// Previously computed lowercase source digest, if any.
    pub source_sha256: Option<String>,
}

/// Trusted immutable derivative facts published by a worker.
#[derive(Clone, Debug)]
pub struct PublishedDerivative {
    /// Server-generated immutable object key.
    pub object_key: ObjectKey,
    /// Versioned server-side preset identifier.
    pub preset_id: String,
    /// Stable derivative variant such as `thumbnail`.
    pub variant: String,
    /// Trusted encoded content type.
    pub content_type: String,
    /// Exact output byte length.
    pub byte_len: u64,
    /// Lowercase SHA-256 of the encoded derivative.
    pub sha256: String,
}

/// Durable source-state failure without exposing SQL details publicly.
#[derive(Debug, Error)]
#[error("processing source repository operation failed: {0}")]
pub struct ProcessingRepositoryError(pub String);

/// Atomic state transitions performed by source-validation workers.
#[async_trait]
pub trait ProcessingRepository: Send + Sync {
    /// Loads one globally unique upload referenced by a durable job.
    async fn find_processing_source(
        &self,
        upload_id: UploadId,
    ) -> Result<Option<ProcessingSource>, ProcessingRepositoryError>;

    /// Records trusted detection and atomically enters processing.
    async fn start_processing(
        &self,
        upload_id: UploadId,
        detected_content_type: &str,
        source_sha256: &str,
        now: OffsetDateTime,
    ) -> Result<(), ProcessingRepositoryError>;

    /// Records an uploaded immutable derivative and atomically marks the upload ready.
    async fn publish_derivative(
        &self,
        upload_id: UploadId,
        derivative: &PublishedDerivative,
        now: OffsetDateTime,
    ) -> Result<(), ProcessingRepositoryError>;

    /// Permanently rejects an uploaded source with a stable non-secret code.
    async fn mark_rejected(
        &self,
        upload_id: UploadId,
        error_code: &str,
        now: OffsetDateTime,
    ) -> Result<(), ProcessingRepositoryError>;
}
