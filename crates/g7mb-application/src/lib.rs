//! Application ports shared by API and infrastructure adapters.

pub mod delivery;
mod delivery_security;
pub mod inventory;
pub mod lifecycle;
pub mod operations;
pub mod policies;
pub mod processing;
pub mod uploads;

use std::{collections::BTreeMap, path::Path, time::Duration};

use async_trait::async_trait;
use g7mb_domain::{ObjectKey, UploadId};
use secrecy::SecretString;
use thiserror::Error;
use time::OffsetDateTime;

/// Request for a tightly scoped direct upload URL.
#[derive(Clone, Debug)]
pub struct PresignPutRequest {
    /// Server-generated destination key.
    pub key: ObjectKey,
    /// Exact encoded body length.
    pub content_length: u64,
    /// Client-declared MIME hint that will be verified later.
    pub content_type: String,
    /// Short URL validity period.
    pub expires_in: Duration,
}

/// A sensitive presigned URL and the exact headers the uploader must send.
#[derive(Clone, Debug)]
pub struct PresignedUpload {
    /// Redacted-on-debug signed URL.
    pub url: SecretString,
    /// Headers covered by the signature.
    pub required_headers: BTreeMap<String, String>,
    /// Absolute expiration time.
    pub expires_at: OffsetDateTime,
}

/// Request for one short-lived private derivative URL.
#[derive(Clone, Debug)]
pub struct PresignGetRequest {
    /// Immutable server-generated derivative key.
    pub key: ObjectKey,
    /// Short URL validity period.
    pub expires_in: Duration,
}

/// Sensitive private object URL returned only after application authorization.
#[derive(Clone, Debug)]
pub struct PresignedDownload {
    /// Redacted-on-debug signed URL.
    pub url: SecretString,
    /// Absolute expiration time.
    pub expires_at: OffsetDateTime,
}

/// Request to create one resumable S3-compatible upload.
#[derive(Clone, Debug)]
pub struct CreateMultipartRequest {
    /// Server-generated destination key.
    pub key: ObjectKey,
    /// Client-declared MIME hint that will be verified later.
    pub content_type: String,
}

/// Opaque provider multipart session kept server-side.
#[derive(Clone, Debug)]
pub struct MultipartSession {
    /// Redacted provider upload identifier.
    pub upload_id: SecretString,
}

/// Request for one short-lived multipart part URL.
#[derive(Clone, Debug)]
pub struct PresignPartRequest {
    /// Server-generated destination key.
    pub key: ObjectKey,
    /// Provider session identifier.
    pub upload_id: SecretString,
    /// One-based part number.
    pub part_number: u16,
    /// Exact byte length for this part.
    pub content_length: u64,
    /// Short URL validity period.
    pub expires_in: Duration,
}

/// Provider ETag for one successfully uploaded part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedPart {
    /// One-based part number.
    pub part_number: u16,
    /// Opaque provider ETag.
    pub etag: String,
}

/// Request to publish all completed parts as one object.
#[derive(Clone, Debug)]
pub struct CompleteMultipartRequest {
    /// Server-generated destination key.
    pub key: ObjectKey,
    /// Provider session identifier.
    pub upload_id: SecretString,
    /// Strictly increasing completed parts.
    pub parts: Vec<CompletedPart>,
}

/// Request to discard an incomplete multipart upload.
#[derive(Clone, Debug)]
pub struct AbortMultipartRequest {
    /// Server-generated destination key.
    pub key: ObjectKey,
    /// Provider session identifier.
    pub upload_id: SecretString,
}

/// Trusted object facts returned by storage after upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectMetadata {
    /// Stored byte length.
    pub content_length: u64,
    /// Storage-reported content type, if any.
    pub content_type: Option<String>,
    /// Opaque storage ETag. It is not treated as a SHA-256 digest.
    pub etag: Option<String>,
}

/// Bounded request to stream one private object into a new local file.
#[derive(Clone, Debug)]
pub struct DownloadObjectRequest {
    /// Server-generated source key.
    pub key: ObjectKey,
    /// Worker-owned destination that must not already exist.
    pub destination: std::path::PathBuf,
    /// Exact reservation length expected from storage.
    pub expected_length: u64,
    /// Absolute streaming cap independent of provider metadata.
    pub max_length: u64,
}

/// Request to stream one worker-produced derivative to object storage.
#[derive(Clone, Debug)]
pub struct PutFileRequest {
    /// Immutable server-generated derivative key.
    pub key: ObjectKey,
    /// Worker-owned regular local file.
    pub source: std::path::PathBuf,
    /// Trusted server-selected content type.
    pub content_type: String,
}

/// Bounded provider inventory page request under one server-owned prefix.
#[derive(Clone, Debug)]
pub struct ListObjectsRequest {
    /// Exact allowlisted prefix (`raw/` or `media/`).
    pub prefix: String,
    /// Last key from the previous page, used as a durable lexicographic cursor.
    pub start_after: Option<String>,
    /// Provider page bound in `1..=1000`.
    pub max_keys: u16,
}

/// One object returned by a bounded provider inventory page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListedObject {
    /// Provider key; application code validates it before any destructive action.
    pub key: String,
    /// Provider-reported object length.
    pub content_length: u64,
}

/// One provider inventory page with a resumable key cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListedObjectsPage {
    /// Objects in provider lexicographic order.
    pub objects: Vec<ListedObject>,
    /// Last returned key when another page exists; absent at end of prefix.
    pub next_start_after: Option<String>,
}

/// Errors exposed by the object storage port.
#[derive(Debug, Error)]
pub enum ObjectStoreError {
    /// Request values cannot be represented by the backend.
    #[error("invalid object storage request: {0}")]
    InvalidRequest(String),
    /// Storage returned no matching object.
    #[error("object was not found")]
    NotFound,
    /// Provider bytes disagree with the immutable reservation.
    #[error("object length differs from the reservation")]
    ContentLengthMismatch,
    /// Backend operation failed without exposing credentials or signed URLs.
    #[error("object storage backend failed: {0}")]
    Backend(String),
}

/// S3-compatible storage operations needed by the control plane.
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Produces a short-lived direct upload request.
    async fn presign_put(
        &self,
        request: PresignPutRequest,
    ) -> Result<PresignedUpload, ObjectStoreError>;

    /// Produces a short-lived private GET URL for one server-owned derivative.
    async fn presign_get(
        &self,
        _request: PresignGetRequest,
    ) -> Result<PresignedDownload, ObjectStoreError> {
        Err(ObjectStoreError::InvalidRequest(
            "private derivative delivery is unavailable".to_owned(),
        ))
    }

    /// Starts one resumable multipart upload.
    async fn create_multipart(
        &self,
        request: CreateMultipartRequest,
    ) -> Result<MultipartSession, ObjectStoreError>;

    /// Produces a short-lived upload URL for one part.
    async fn presign_part(
        &self,
        request: PresignPartRequest,
    ) -> Result<PresignedUpload, ObjectStoreError>;

    /// Atomically publishes a complete ordered part list.
    async fn complete_multipart(
        &self,
        request: CompleteMultipartRequest,
    ) -> Result<(), ObjectStoreError>;

    /// Aborts an incomplete multipart upload idempotently at the application layer.
    async fn abort_multipart(&self, request: AbortMultipartRequest)
    -> Result<(), ObjectStoreError>;

    /// Reads trusted metadata after direct upload.
    async fn head(&self, key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError>;

    /// Streams a private object to a new file with a hard byte cap.
    async fn download_to(
        &self,
        request: DownloadObjectRequest,
    ) -> Result<ObjectMetadata, ObjectStoreError>;

    /// Streams a trusted local derivative to an immutable object key.
    async fn put_file(&self, request: PutFileRequest) -> Result<ObjectMetadata, ObjectStoreError>;

    /// Lists one bounded page under an allowlisted server-owned prefix.
    async fn list_objects(
        &self,
        _request: ListObjectsRequest,
    ) -> Result<ListedObjectsPage, ObjectStoreError> {
        Err(ObjectStoreError::InvalidRequest(
            "provider inventory is unavailable".to_owned(),
        ))
    }

    /// Deletes one server-owned object key. Implementations must be idempotent when absent.
    async fn delete(&self, key: &ObjectKey) -> Result<(), ObjectStoreError>;
}

/// Durable media work item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessingJob {
    /// Upload owning this job.
    pub upload_id: UploadId,
    /// Server-controlled transformation preset.
    pub preset_id: String,
    /// Site policy revision pinned when uploaded bytes enter quarantine.
    pub site_policy_revision: Option<u64>,
}

/// A durable job currently owned by one worker for a bounded interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeasedProcessingJob {
    /// Queue-generated stable job identifier.
    pub job_id: String,
    /// Processing payload.
    pub job: ProcessingJob,
    /// Number of times this job has been claimed.
    pub attempts: u32,
    /// Worker lease expiration.
    pub lease_until: OffsetDateTime,
    /// Original durable enqueue time used for queue-age observability.
    pub enqueued_at: OffsetDateTime,
}

/// Result of recording one failed processing attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobFailureDisposition {
    /// The job will become eligible at its new `available_at` time.
    RetryScheduled,
    /// The configured attempt cap was reached.
    DeadLetter,
}

/// Durable queue operation failure.
#[derive(Debug, Error)]
#[error("job queue operation failed: {0}")]
pub struct JobQueueError(pub String);

/// Durable replay-protection failure.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum NonceStoreError {
    /// The same application key already consumed this nonce.
    #[error("request nonce was already consumed")]
    Replay,
    /// Persistence failed without exposing backend details publicly.
    #[error("nonce store backend failed: {0}")]
    Backend(String),
}

/// Atomic replay protection for authenticated control requests.
#[async_trait]
pub trait NonceStore: Send + Sync {
    /// Consumes a nonce exactly once until its expiration time.
    async fn consume(
        &self,
        key_id: &str,
        nonce: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<(), NonceStoreError>;
}

/// Durable, at-least-once processing queue.
#[async_trait]
pub trait JobQueue: Send + Sync {
    /// Enqueues a job idempotently.
    async fn enqueue(&self, job: ProcessingJob) -> Result<(), JobQueueError>;

    /// Atomically claims the next eligible or expired job.
    async fn claim_next(
        &self,
        worker_id: &str,
        now: OffsetDateTime,
        lease_for: Duration,
    ) -> Result<Option<LeasedProcessingJob>, JobQueueError>;

    /// Extends a live lease only while it is owned by the same worker.
    async fn renew(
        &self,
        job_id: &str,
        worker_id: &str,
        now: OffsetDateTime,
        lease_for: Duration,
    ) -> Result<OffsetDateTime, JobQueueError>;

    /// Completes a job only while it is owned by the same worker.
    async fn complete(
        &self,
        job_id: &str,
        worker_id: &str,
        now: OffsetDateTime,
    ) -> Result<(), JobQueueError>;

    /// Schedules a bounded retry or moves the job to dead-letter state.
    #[allow(clippy::too_many_arguments)]
    async fn fail(
        &self,
        job_id: &str,
        worker_id: &str,
        now: OffsetDateTime,
        retry_at: OffsetDateTime,
        max_attempts: u32,
        error_code: &str,
    ) -> Result<JobFailureDisposition, JobQueueError>;
}

/// Supported derivative output type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageOutputFormat {
    /// JPEG output.
    Jpeg,
    /// WebP output.
    Webp,
    /// AVIF output.
    Avif,
    /// PNG output.
    Png,
}

/// Server-controlled watermark anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatermarkPosition {
    /// Keep equal distance from every edge.
    Center,
    /// Anchor to the upper-left safe margin.
    TopLeft,
    /// Anchor to the upper-right safe margin.
    TopRight,
    /// Anchor to the lower-left safe margin.
    BottomLeft,
    /// Anchor to the lower-right safe margin.
    BottomRight,
}

/// Bounded watermark input selected by a versioned server-side preset.
#[derive(Clone, Debug)]
pub struct WatermarkSpec<'a> {
    /// Trusted local watermark asset path supplied by the worker.
    pub input: &'a Path,
    /// Allowlisted anchor point.
    pub position: WatermarkPosition,
    /// Safe distance from the selected edges.
    pub margin_px: u32,
    /// Maximum watermark width as 1..=50 percent of the derivative width.
    pub max_width_percent: u8,
    /// Final alpha multiplier as 1..=100 percent.
    pub opacity_percent: u8,
}

/// A bounded image thumbnail request passed to the native sandbox.
#[derive(Clone, Debug)]
pub struct ImageThumbnailRequest<'a> {
    /// Trusted local source path.
    pub input: &'a Path,
    /// Trusted local output path.
    pub output: &'a Path,
    /// Maximum width and height.
    pub max_edge: u32,
    /// Server-controlled output type.
    pub format: ImageOutputFormat,
    /// Optional server-selected watermark; arbitrary user paths and options are forbidden.
    pub watermark: Option<WatermarkSpec<'a>>,
}
