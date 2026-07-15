//! Secure orchestration for bounded direct-upload batches.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use g7mb_domain::{
    MediaKind, ObjectKey, UploadBatchId, UploadBatchPolicy, UploadBatchPolicyError,
    UploadCandidate, UploadId, UploadState, UploadTransfer,
};
use secrecy::SecretString;
use thiserror::Error;
use time::OffsetDateTime;

use crate::{
    AbortMultipartRequest, CompleteMultipartRequest, CompletedPart, CreateMultipartRequest,
    ObjectMetadata, ObjectStore, ObjectStoreError, PresignPartRequest, PresignPutRequest,
    PresignedUpload,
};

const PRESIGN_TTL: Duration = Duration::from_secs(10 * 60);
const INITIAL_PROCESSING_PRESET: &str = "source-validation-v1";

/// One untrusted file reservation request after transport decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadFileIntent {
    /// Opaque request correlation value.
    pub client_ref: String,
    /// Client-declared media kind, verified after upload.
    pub declared_kind: MediaKind,
    /// Exact expected source length.
    pub content_length: u64,
    /// Client MIME hint, never trusted as detected type.
    pub content_type_hint: String,
}

/// Batch creation input after HMAC authentication resolved the tenant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateUploadBatch {
    /// Authenticated site or tenant identifier.
    pub tenant_id: String,
    /// One to one hundred file intents.
    pub files: Vec<UploadFileIntent>,
}

/// One durable upload reservation persisted before returning to the client.
#[derive(Clone, Debug)]
pub struct UploadReservation {
    /// Upload identifier.
    pub upload_id: UploadId,
    /// Server-owned raw key.
    pub object_key: ObjectKey,
    /// Untrusted declared media kind.
    pub declared_kind: MediaKind,
    /// Exact expected source length.
    pub expected_size_bytes: u64,
    /// Untrusted MIME hint.
    pub content_type_hint: String,
    /// Server-selected transfer strategy.
    pub transfer: UploadTransfer,
    /// Provider session identifier for multipart uploads only.
    pub multipart_upload_id: Option<SecretString>,
}

/// Atomic durable batch persisted by the repository port.
#[derive(Clone, Debug)]
pub struct UploadBatchReservation {
    /// Batch identifier.
    pub batch_id: UploadBatchId,
    /// Authenticated tenant identifier.
    pub tenant_id: String,
    /// Creation timestamp.
    pub created_at: OffsetDateTime,
    /// Uploads in request order.
    pub uploads: Vec<UploadReservation>,
}

/// Active reservation limits enforced before issuing direct-upload instructions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadCapacityPolicy {
    /// Maximum active reservations across the single-node service.
    pub max_active_global: usize,
    /// Maximum active reservations owned by one tenant.
    pub max_active_per_tenant: usize,
}

impl Default for UploadCapacityPolicy {
    fn default() -> Self {
        Self {
            max_active_global: 1_000,
            max_active_per_tenant: 200,
        }
    }
}

impl UploadCapacityPolicy {
    fn is_valid(self) -> bool {
        self.max_active_global > 0
            && self.max_active_per_tenant > 0
            && self.max_active_per_tenant <= self.max_active_global
    }
}

/// Repository failure without SQL or secret details in the public API.
#[derive(Debug, Error)]
pub enum UploadRepositoryError {
    /// An atomic reservation would exceed global or tenant capacity.
    #[error("active upload capacity is exhausted")]
    CapacityExhausted,
    /// Durable storage failed without exposing backend details publicly.
    #[error("upload repository operation failed: {0}")]
    Backend(String),
}

/// Durable upload reservation boundary.
#[async_trait]
pub trait UploadRepository: Send + Sync {
    /// Preflights capacity before provider sessions or signed URLs are created.
    async fn has_capacity(
        &self,
        tenant_id: &str,
        additional_uploads: usize,
        capacity: UploadCapacityPolicy,
    ) -> Result<bool, UploadRepositoryError>;

    /// Persists a complete batch atomically.
    async fn save_batch(
        &self,
        batch: &UploadBatchReservation,
        capacity: UploadCapacityPolicy,
    ) -> Result<(), UploadRepositoryError>;

    /// Loads one tenant-scoped reservation for multipart control operations.
    async fn find_upload(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<Option<StoredUploadReservation>, UploadRepositoryError>;

    /// Loads tenant-scoped lifecycle and immutable derivative status.
    async fn find_status(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<Option<UploadStatusSnapshot>, UploadRepositoryError>;

    /// Rechecks the mutable delivery guard without reloading immutable derivative rows.
    async fn is_delivery_allowed(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<Option<bool>, UploadRepositoryError> {
        Ok(self
            .find_status(tenant_id, upload_id)
            .await?
            .map(|status| status.state == UploadState::Ready && !status.deletion_pending))
    }

    /// Atomically confirms upload, enters private quarantine, and enqueues validation.
    async fn mark_quarantined_and_enqueue(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        actual_size_bytes: u64,
        preset_id: &str,
        now: OffsetDateTime,
    ) -> Result<(), UploadRepositoryError>;

    /// Tombstones an upload after remote multipart abort.
    async fn mark_deleted(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        now: OffsetDateTime,
    ) -> Result<(), UploadRepositoryError>;
}

/// Tenant-scoped reservation loaded from durable state.
#[derive(Clone, Debug)]
pub struct StoredUploadReservation {
    /// Upload identifier.
    pub upload_id: UploadId,
    /// Server-owned raw key.
    pub object_key: ObjectKey,
    /// Exact expected source length.
    pub expected_size_bytes: u64,
    /// Current lifecycle state.
    pub state: UploadState,
    /// Server-selected transfer strategy.
    pub transfer: UploadTransfer,
    /// Provider session for multipart uploads.
    pub multipart_upload_id: Option<SecretString>,
}

/// One immutable generated derivative returned to the control application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDerivative {
    /// Versioned transformation preset.
    pub preset_id: String,
    /// Stable derivative variant.
    pub variant: String,
    /// Server-generated storage key.
    pub object_key: ObjectKey,
    /// Trusted output content type.
    pub content_type: String,
    /// Exact output byte length.
    pub byte_len: u64,
}

/// Tenant-scoped lifecycle snapshot for G5/G7 attachment integration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadStatusSnapshot {
    /// Upload identifier.
    pub upload_id: UploadId,
    /// Current lifecycle state.
    pub state: UploadState,
    /// Trusted source content type after validation.
    pub detected_content_type: Option<String>,
    /// Stable rejection or failure code.
    pub error_code: Option<String>,
    /// Whether durable storage cleanup was requested but has not completed.
    pub deletion_pending: bool,
    /// Published immutable derivatives.
    pub derivatives: Vec<StoredDerivative>,
}

/// One client-facing result from batch creation.
#[derive(Clone, Debug)]
pub struct CreatedUploadIntent {
    /// Echoed request correlation value.
    pub client_ref: String,
    /// Server upload identifier.
    pub upload_id: UploadId,
    /// Server-selected transfer strategy.
    pub transfer: UploadTransfer,
    /// Present only for single PUT uploads.
    pub presigned_put: Option<PresignedUpload>,
    /// Reservation expiration.
    pub expires_at: OffsetDateTime,
}

/// Successful bounded batch result.
#[derive(Clone, Debug)]
pub struct CreatedUploadBatch {
    /// Server batch identifier.
    pub batch_id: UploadBatchId,
    /// Instructions in request order.
    pub uploads: Vec<CreatedUploadIntent>,
}

/// Batch orchestration failure mapped to stable API error codes by the HTTP layer.
#[derive(Debug, Error)]
pub enum CreateUploadBatchError {
    /// Authenticated tenant identifier is malformed.
    #[error("tenant identifier is invalid")]
    InvalidTenant,
    /// A client reference is malformed or duplicated.
    #[error("client_ref is invalid or duplicated")]
    InvalidClientRef,
    /// A MIME hint is malformed.
    #[error("content type hint is invalid")]
    InvalidContentType,
    /// Batch capacity policy rejected the reservation.
    #[error(transparent)]
    Policy(#[from] UploadBatchPolicyError),
    /// Global or tenant active reservation capacity is exhausted.
    #[error("upload service capacity is exhausted")]
    Backpressure,
    /// S3-compatible control operation failed.
    #[error(transparent)]
    ObjectStore(#[from] ObjectStoreError),
    /// Atomic durable persistence failed.
    #[error(transparent)]
    Repository(#[from] UploadRepositoryError),
    /// At least one remote multipart upload could not be aborted after failure.
    #[error("failed batch left a multipart session requiring lifecycle cleanup")]
    CleanupRequired,
    /// Generated object key failed the narrow invariant.
    #[error("generated object key failed validation")]
    GeneratedObjectKey,
}

/// Multipart part or lifecycle control failure.
#[derive(Debug, Error)]
pub enum MultipartControlError {
    /// Tenant identifier is malformed.
    #[error("tenant identifier is invalid")]
    InvalidTenant,
    /// No tenant-scoped upload exists.
    #[error("upload reservation was not found")]
    NotFound,
    /// Upload does not use multipart or is in the wrong state.
    #[error("upload is not an active multipart reservation")]
    InvalidState,
    /// Part number or exact byte length violates the server plan.
    #[error("multipart part violates the reserved layout")]
    InvalidPart,
    /// Completed part list is not complete, ordered, and unique.
    #[error("multipart completion list violates the reserved layout")]
    InvalidCompletion,
    /// Published object size differs from the signed reservation.
    #[error("completed object length differs from the reservation")]
    StoredSizeMismatch,
    /// S3-compatible operation failed.
    #[error(transparent)]
    ObjectStore(#[from] ObjectStoreError),
    /// Durable state operation failed.
    #[error(transparent)]
    Repository(#[from] UploadRepositoryError),
}

/// Bounded application service shared by HTTP and contract tests.
#[derive(Clone)]
pub struct UploadIntentService {
    object_store: Arc<dyn ObjectStore>,
    repository: Arc<dyn UploadRepository>,
    policy: UploadBatchPolicy,
    capacity: UploadCapacityPolicy,
}

impl UploadIntentService {
    /// Creates a service with explicit ports and resource policy.
    #[must_use]
    pub fn new(
        object_store: Arc<dyn ObjectStore>,
        repository: Arc<dyn UploadRepository>,
        policy: UploadBatchPolicy,
    ) -> Self {
        Self {
            object_store,
            repository,
            policy,
            capacity: UploadCapacityPolicy::default(),
        }
    }

    /// Applies explicit global and tenant reservation caps.
    #[must_use]
    pub fn with_capacity_policy(mut self, capacity: UploadCapacityPolicy) -> Self {
        self.capacity = capacity;
        self
    }

    /// Creates and atomically persists a batch of direct-upload instructions.
    pub async fn create_batch(
        &self,
        request: CreateUploadBatch,
    ) -> Result<CreatedUploadBatch, CreateUploadBatchError> {
        validate_tenant(&request.tenant_id)?;
        validate_file_metadata(&request.files)?;
        let candidates = request
            .files
            .iter()
            .map(|file| UploadCandidate {
                declared_kind: file.declared_kind,
                byte_len: file.content_length,
            })
            .collect::<Vec<_>>();
        let transfers = self.policy.plan(&candidates)?;
        if !self.capacity.is_valid() {
            return Err(CreateUploadBatchError::Backpressure);
        }
        if !self
            .repository
            .has_capacity(&request.tenant_id, request.files.len(), self.capacity)
            .await?
        {
            return Err(CreateUploadBatchError::Backpressure);
        }
        let batch_id = UploadBatchId::new();
        let created_at = OffsetDateTime::now_utc();
        let expires_at = created_at + PRESIGN_TTL;
        let mut reservations = Vec::with_capacity(request.files.len());
        let mut outputs = Vec::with_capacity(request.files.len());

        for (file, transfer) in request.files.into_iter().zip(transfers) {
            let upload_id = UploadId::new();
            let object_key =
                ObjectKey::new(format!("raw/{}/{upload_id}/source", request.tenant_id))
                    .map_err(|_| CreateUploadBatchError::GeneratedObjectKey)?;

            let (multipart_upload_id, presigned_put) = match transfer {
                UploadTransfer::SinglePut => {
                    let signed = match self
                        .object_store
                        .presign_put(PresignPutRequest {
                            key: object_key.clone(),
                            content_length: file.content_length,
                            content_type: file.content_type_hint.clone(),
                            expires_in: PRESIGN_TTL,
                        })
                        .await
                    {
                        Ok(signed) => signed,
                        Err(error) => {
                            return Err(self.cleanup_or_error(&reservations, error).await);
                        }
                    };
                    (None, Some(signed))
                }
                UploadTransfer::Multipart { .. } => {
                    let session = match self
                        .object_store
                        .create_multipart(CreateMultipartRequest {
                            key: object_key.clone(),
                            content_type: file.content_type_hint.clone(),
                        })
                        .await
                    {
                        Ok(session) => session,
                        Err(error) => {
                            return Err(self.cleanup_or_error(&reservations, error).await);
                        }
                    };
                    (Some(session.upload_id), None)
                }
            };

            reservations.push(UploadReservation {
                upload_id,
                object_key,
                declared_kind: file.declared_kind,
                expected_size_bytes: file.content_length,
                content_type_hint: file.content_type_hint,
                transfer,
                multipart_upload_id,
            });
            outputs.push(CreatedUploadIntent {
                client_ref: file.client_ref,
                upload_id,
                transfer,
                presigned_put,
                expires_at,
            });
        }

        let batch = UploadBatchReservation {
            batch_id,
            tenant_id: request.tenant_id,
            created_at,
            uploads: reservations,
        };
        if let Err(error) = self.repository.save_batch(&batch, self.capacity).await {
            if self.cleanup(&batch.uploads).await {
                return Err(match error {
                    UploadRepositoryError::CapacityExhausted => {
                        CreateUploadBatchError::Backpressure
                    }
                    UploadRepositoryError::Backend(_) => CreateUploadBatchError::Repository(error),
                });
            }
            return Err(CreateUploadBatchError::CleanupRequired);
        }

        Ok(CreatedUploadBatch {
            batch_id,
            uploads: outputs,
        })
    }

    /// Presigns exactly one server-planned multipart part.
    pub async fn presign_part(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        part_number: u16,
        content_length: u64,
    ) -> Result<PresignedUpload, MultipartControlError> {
        if !tenant_is_valid(tenant_id) {
            return Err(MultipartControlError::InvalidTenant);
        }
        let stored = self
            .repository
            .find_upload(tenant_id, upload_id)
            .await?
            .ok_or(MultipartControlError::NotFound)?;
        let (part_size_bytes, provider_upload_id) = multipart_layout(&stored)?;
        if stored.state != UploadState::Created {
            return Err(MultipartControlError::InvalidState);
        }
        if expected_part_length(stored.expected_size_bytes, part_size_bytes, part_number)
            != Some(content_length)
        {
            return Err(MultipartControlError::InvalidPart);
        }
        self.object_store
            .presign_part(PresignPartRequest {
                key: stored.object_key,
                upload_id: provider_upload_id,
                part_number,
                content_length,
                expires_in: PRESIGN_TTL,
            })
            .await
            .map_err(Into::into)
    }

    /// Publishes an exact, complete multipart layout and verifies stored bytes.
    pub async fn complete_multipart(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        parts: Vec<CompletedPart>,
    ) -> Result<(), MultipartControlError> {
        if !tenant_is_valid(tenant_id) {
            return Err(MultipartControlError::InvalidTenant);
        }
        let stored = self
            .repository
            .find_upload(tenant_id, upload_id)
            .await?
            .ok_or(MultipartControlError::NotFound)?;
        let (part_size_bytes, provider_upload_id) = multipart_layout(&stored)?;
        if upload_bytes_are_confirmed(stored.state) {
            return self.verify_stored_size(&stored).await;
        }
        if stored.state != UploadState::Created {
            return Err(MultipartControlError::InvalidState);
        }
        validate_completion(stored.expected_size_bytes, part_size_bytes, &parts)?;
        self.object_store
            .complete_multipart(CompleteMultipartRequest {
                key: stored.object_key.clone(),
                upload_id: provider_upload_id,
                parts,
            })
            .await?;
        self.verify_stored_size(&stored).await?;
        self.repository
            .mark_quarantined_and_enqueue(
                tenant_id,
                upload_id,
                stored.expected_size_bytes,
                INITIAL_PROCESSING_PRESET,
                OffsetDateTime::now_utc(),
            )
            .await?;
        Ok(())
    }

    /// Confirms a single PUT with trusted storage metadata and queues validation.
    pub async fn confirm_single_upload(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<(), MultipartControlError> {
        if !tenant_is_valid(tenant_id) {
            return Err(MultipartControlError::InvalidTenant);
        }
        let stored = self
            .repository
            .find_upload(tenant_id, upload_id)
            .await?
            .ok_or(MultipartControlError::NotFound)?;
        if stored.transfer != UploadTransfer::SinglePut {
            return Err(MultipartControlError::InvalidState);
        }
        if upload_bytes_are_confirmed(stored.state) {
            return self.verify_stored_size(&stored).await;
        }
        if stored.state != UploadState::Created {
            return Err(MultipartControlError::InvalidState);
        }
        self.verify_stored_size(&stored).await?;
        self.repository
            .mark_quarantined_and_enqueue(
                tenant_id,
                upload_id,
                stored.expected_size_bytes,
                INITIAL_PROCESSING_PRESET,
                OffsetDateTime::now_utc(),
            )
            .await?;
        Ok(())
    }

    /// Aborts an incomplete provider session and tombstones its reservation.
    pub async fn abort_multipart(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<(), MultipartControlError> {
        if !tenant_is_valid(tenant_id) {
            return Err(MultipartControlError::InvalidTenant);
        }
        let stored = self
            .repository
            .find_upload(tenant_id, upload_id)
            .await?
            .ok_or(MultipartControlError::NotFound)?;
        let (_, provider_upload_id) = multipart_layout(&stored)?;
        if stored.state == UploadState::Deleted {
            return Ok(());
        }
        if stored.state != UploadState::Created {
            return Err(MultipartControlError::InvalidState);
        }
        self.object_store
            .abort_multipart(AbortMultipartRequest {
                key: stored.object_key,
                upload_id: provider_upload_id,
            })
            .await?;
        self.repository
            .mark_deleted(tenant_id, upload_id, OffsetDateTime::now_utc())
            .await?;
        Ok(())
    }

    /// Returns one authenticated tenant-scoped upload status snapshot.
    pub async fn status(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<UploadStatusSnapshot, MultipartControlError> {
        if !tenant_is_valid(tenant_id) {
            return Err(MultipartControlError::InvalidTenant);
        }
        self.repository
            .find_status(tenant_id, upload_id)
            .await?
            .ok_or(MultipartControlError::NotFound)
    }

    async fn verify_stored_size(
        &self,
        stored: &StoredUploadReservation,
    ) -> Result<(), MultipartControlError> {
        let ObjectMetadata { content_length, .. } =
            self.object_store.head(&stored.object_key).await?;
        if content_length != stored.expected_size_bytes {
            return Err(MultipartControlError::StoredSizeMismatch);
        }
        Ok(())
    }

    async fn cleanup_or_error(
        &self,
        reservations: &[UploadReservation],
        original: ObjectStoreError,
    ) -> CreateUploadBatchError {
        if self.cleanup(reservations).await {
            CreateUploadBatchError::ObjectStore(original)
        } else {
            CreateUploadBatchError::CleanupRequired
        }
    }

    async fn cleanup(&self, reservations: &[UploadReservation]) -> bool {
        let mut all_aborted = true;
        for reservation in reservations {
            if let Some(upload_id) = &reservation.multipart_upload_id
                && self
                    .object_store
                    .abort_multipart(AbortMultipartRequest {
                        key: reservation.object_key.clone(),
                        upload_id: upload_id.clone(),
                    })
                    .await
                    .is_err()
            {
                all_aborted = false;
            }
        }
        all_aborted
    }
}

fn validate_tenant(tenant_id: &str) -> Result<(), CreateUploadBatchError> {
    if !tenant_is_valid(tenant_id) {
        return Err(CreateUploadBatchError::InvalidTenant);
    }
    Ok(())
}

fn tenant_is_valid(tenant_id: &str) -> bool {
    !tenant_id.is_empty()
        && tenant_id.len() <= 64
        && tenant_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

const fn upload_bytes_are_confirmed(state: UploadState) -> bool {
    matches!(
        state,
        UploadState::Uploaded
            | UploadState::Quarantined
            | UploadState::Processing
            | UploadState::Ready
    )
}

fn multipart_layout(
    stored: &StoredUploadReservation,
) -> Result<(u64, SecretString), MultipartControlError> {
    let UploadTransfer::Multipart { part_size_bytes } = stored.transfer else {
        return Err(MultipartControlError::InvalidState);
    };
    let upload_id = stored
        .multipart_upload_id
        .clone()
        .ok_or(MultipartControlError::InvalidState)?;
    Ok((part_size_bytes, upload_id))
}

fn expected_part_length(total_size: u64, part_size: u64, part_number: u16) -> Option<u64> {
    if total_size == 0 || part_size == 0 || part_number == 0 || part_number > 10_000 {
        return None;
    }
    let start = u64::from(part_number - 1).checked_mul(part_size)?;
    if start >= total_size {
        return None;
    }
    Some((total_size - start).min(part_size))
}

fn validate_completion(
    total_size: u64,
    part_size: u64,
    parts: &[CompletedPart],
) -> Result<(), MultipartControlError> {
    if total_size == 0 || part_size == 0 {
        return Err(MultipartControlError::InvalidCompletion);
    }
    let expected_count = total_size.div_ceil(part_size);
    if expected_count > 10_000
        || u64::try_from(parts.len()).ok() != Some(expected_count)
        || parts.iter().enumerate().any(|(index, part)| {
            let expected_number = u16::try_from(index + 1).ok();
            Some(part.part_number) != expected_number
                || part.etag.is_empty()
                || part.etag.len() > 1024
                || !part.etag.bytes().all(|byte| byte.is_ascii_graphic())
        })
    {
        return Err(MultipartControlError::InvalidCompletion);
    }
    Ok(())
}

fn validate_file_metadata(files: &[UploadFileIntent]) -> Result<(), CreateUploadBatchError> {
    let mut client_refs = std::collections::BTreeSet::new();
    for file in files {
        if file.client_ref.is_empty()
            || file.client_ref.len() > 128
            || !file
                .client_ref
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !client_refs.insert(file.client_ref.as_str())
        {
            return Err(CreateUploadBatchError::InvalidClientRef);
        }
        if file.content_type_hint.is_empty()
            || file.content_type_hint.len() > 255
            || !file
                .content_type_hint
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
        {
            return Err(CreateUploadBatchError::InvalidContentType);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use g7mb_domain::{
        MediaKind, ObjectKey, UploadBatchPolicy, UploadId, UploadState, UploadTransfer,
    };
    use secrecy::SecretString;
    use time::OffsetDateTime;

    use crate::{
        AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest,
        DownloadObjectRequest, MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError,
        PresignPartRequest, PresignPutRequest, PresignedUpload, PutFileRequest,
    };

    use super::{
        CreateUploadBatch, CreateUploadBatchError, INITIAL_PROCESSING_PRESET,
        MultipartControlError, StoredUploadReservation, UploadBatchReservation,
        UploadCapacityPolicy, UploadFileIntent, UploadIntentService, UploadRepository,
        UploadRepositoryError, UploadStatusSnapshot,
    };

    #[derive(Default)]
    struct FakeStore {
        puts: AtomicUsize,
        multiparts: AtomicUsize,
        aborts: AtomicUsize,
        completes: AtomicUsize,
        head_length: AtomicU64,
    }

    #[async_trait]
    impl ObjectStore for FakeStore {
        async fn presign_put(
            &self,
            request: PresignPutRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            self.puts.fetch_add(1, Ordering::Relaxed);
            Ok(PresignedUpload {
                url: SecretString::from(format!("https://storage.invalid/{}", request.key)),
                required_headers: BTreeMap::new(),
                expires_at: OffsetDateTime::now_utc() + request.expires_in,
            })
        }

        async fn create_multipart(
            &self,
            _request: CreateMultipartRequest,
        ) -> Result<MultipartSession, ObjectStoreError> {
            let number = self.multiparts.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(MultipartSession {
                upload_id: SecretString::from(format!("session-{number}")),
            })
        }

        async fn presign_part(
            &self,
            request: PresignPartRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Ok(PresignedUpload {
                url: SecretString::from("https://storage.invalid/part".to_owned()),
                required_headers: BTreeMap::new(),
                expires_at: OffsetDateTime::now_utc() + request.expires_in,
            })
        }

        async fn complete_multipart(
            &self,
            _request: CompleteMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            self.completes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn abort_multipart(
            &self,
            _request: AbortMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            self.aborts.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn head(&self, _key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError> {
            Ok(ObjectMetadata {
                content_length: self.head_length.load(Ordering::Relaxed),
                content_type: None,
                etag: None,
            })
        }

        async fn download_to(
            &self,
            _request: DownloadObjectRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "test fake does not download".to_owned(),
            ))
        }

        async fn put_file(
            &self,
            _request: PutFileRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "test fake does not upload files".to_owned(),
            ))
        }

        async fn delete(&self, _key: &ObjectKey) -> Result<(), ObjectStoreError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeRepository {
        saves: AtomicUsize,
        capacity_exhausted: AtomicBool,
        upload: Mutex<Option<StoredUploadReservation>>,
    }

    #[async_trait]
    impl UploadRepository for FakeRepository {
        async fn has_capacity(
            &self,
            _tenant_id: &str,
            _additional_uploads: usize,
            _capacity: UploadCapacityPolicy,
        ) -> Result<bool, UploadRepositoryError> {
            Ok(!self.capacity_exhausted.load(Ordering::Relaxed))
        }

        async fn save_batch(
            &self,
            batch: &UploadBatchReservation,
            _capacity: UploadCapacityPolicy,
        ) -> Result<(), UploadRepositoryError> {
            self.saves.fetch_add(1, Ordering::Relaxed);
            if let Some(upload) = batch.uploads.first() {
                let stored = StoredUploadReservation {
                    upload_id: upload.upload_id,
                    object_key: upload.object_key.clone(),
                    expected_size_bytes: upload.expected_size_bytes,
                    state: UploadState::Created,
                    transfer: upload.transfer,
                    multipart_upload_id: upload.multipart_upload_id.clone(),
                };
                *self.upload.lock().map_err(|_| {
                    UploadRepositoryError::Backend("test repository poisoned".to_owned())
                })? = Some(stored);
            }
            Ok(())
        }

        async fn find_upload(
            &self,
            tenant_id: &str,
            upload_id: UploadId,
        ) -> Result<Option<StoredUploadReservation>, UploadRepositoryError> {
            if tenant_id != "site-a" {
                return Ok(None);
            }
            Ok(self
                .upload
                .lock()
                .map_err(|_| UploadRepositoryError::Backend("test repository poisoned".to_owned()))?
                .as_ref()
                .filter(|upload| upload.upload_id == upload_id)
                .cloned())
        }

        async fn find_status(
            &self,
            tenant_id: &str,
            upload_id: UploadId,
        ) -> Result<Option<UploadStatusSnapshot>, UploadRepositoryError> {
            Ok(self
                .find_upload(tenant_id, upload_id)
                .await?
                .map(|upload| UploadStatusSnapshot {
                    upload_id: upload.upload_id,
                    state: upload.state,
                    detected_content_type: None,
                    error_code: None,
                    deletion_pending: false,
                    derivatives: Vec::new(),
                }))
        }

        async fn mark_quarantined_and_enqueue(
            &self,
            tenant_id: &str,
            upload_id: UploadId,
            actual_size_bytes: u64,
            preset_id: &str,
            _now: OffsetDateTime,
        ) -> Result<(), UploadRepositoryError> {
            let mut guard = self.upload.lock().map_err(|_| {
                UploadRepositoryError::Backend("test repository poisoned".to_owned())
            })?;
            let upload = guard
                .as_mut()
                .filter(|upload| tenant_id == "site-a" && upload.upload_id == upload_id)
                .ok_or_else(|| UploadRepositoryError::Backend("test upload missing".to_owned()))?;
            if actual_size_bytes != upload.expected_size_bytes {
                return Err(UploadRepositoryError::Backend(
                    "test size mismatch".to_owned(),
                ));
            }
            if preset_id != INITIAL_PROCESSING_PRESET {
                return Err(UploadRepositoryError::Backend(
                    "test preset mismatch".to_owned(),
                ));
            }
            upload.state = UploadState::Quarantined;
            Ok(())
        }

        async fn mark_deleted(
            &self,
            tenant_id: &str,
            upload_id: UploadId,
            _now: OffsetDateTime,
        ) -> Result<(), UploadRepositoryError> {
            let mut guard = self.upload.lock().map_err(|_| {
                UploadRepositoryError::Backend("test repository poisoned".to_owned())
            })?;
            let upload = guard
                .as_mut()
                .filter(|upload| tenant_id == "site-a" && upload.upload_id == upload_id)
                .ok_or_else(|| UploadRepositoryError::Backend("test upload missing".to_owned()))?;
            upload.state = UploadState::Deleted;
            Ok(())
        }
    }

    #[tokio::test]
    async fn creates_one_single_and_two_multipart_reservations()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = Arc::new(FakeStore::default());
        let repository = Arc::new(FakeRepository::default());
        let service = UploadIntentService::new(
            store.clone(),
            repository.clone(),
            UploadBatchPolicy::default(),
        );
        let result = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files: vec![
                    file("image-a", MediaKind::Image, 1024, "image/jpeg"),
                    file("video-a", MediaKind::Video, 50 * 1024 * 1024, "video/mp4"),
                    file("image-b", MediaKind::Image, 100 * 1024 * 1024, "image/avif"),
                ],
            })
            .await?;
        assert_eq!(result.uploads.len(), 3);
        assert_eq!(result.uploads[0].transfer, UploadTransfer::SinglePut);
        assert!(result.uploads[0].presigned_put.is_some());
        assert!(matches!(
            result.uploads[1].transfer,
            UploadTransfer::Multipart { .. }
        ));
        assert_eq!(store.puts.load(Ordering::Relaxed), 1);
        assert_eq!(store.multiparts.load(Ordering::Relaxed), 2);
        assert_eq!(repository.saves.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn capacity_backpressure_happens_before_provider_presign()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = Arc::new(FakeStore::default());
        let repository = Arc::new(FakeRepository::default());
        repository.capacity_exhausted.store(true, Ordering::Relaxed);
        let service = UploadIntentService::new(
            store.clone(),
            repository.clone(),
            UploadBatchPolicy::default(),
        );
        let error = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files: vec![file("image-a", MediaKind::Image, 1024, "image/jpeg")],
            })
            .await
            .err()
            .ok_or("capacity exhaustion was unexpectedly accepted")?;

        assert!(matches!(error, CreateUploadBatchError::Backpressure));
        assert_eq!(store.puts.load(Ordering::Relaxed), 0);
        assert_eq!(store.multiparts.load(Ordering::Relaxed), 0);
        assert_eq!(repository.saves.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn creates_one_atomic_reservation_for_one_hundred_upload_intents()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = Arc::new(FakeStore::default());
        let repository = Arc::new(FakeRepository::default());
        let service = UploadIntentService::new(
            store.clone(),
            repository.clone(),
            UploadBatchPolicy::default(),
        );
        let files = (0..100)
            .map(|index| {
                file(
                    &format!("image-{index}"),
                    MediaKind::Image,
                    1024,
                    "image/jpeg",
                )
            })
            .collect();

        let result = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files,
            })
            .await?;

        assert_eq!(result.uploads.len(), 100);
        assert!(
            result
                .uploads
                .iter()
                .all(|upload| upload.transfer == UploadTransfer::SinglePut)
        );
        assert_eq!(store.puts.load(Ordering::Relaxed), 100);
        assert_eq!(repository.saves.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_duplicate_client_refs_before_storage_calls() {
        let store = Arc::new(FakeStore::default());
        let repository = Arc::new(FakeRepository::default());
        let service =
            UploadIntentService::new(store.clone(), repository, UploadBatchPolicy::default());
        let result = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files: vec![
                    file("same", MediaKind::Image, 1, "image/jpeg"),
                    file("same", MediaKind::Image, 1, "image/png"),
                ],
            })
            .await;
        assert!(matches!(
            result,
            Err(CreateUploadBatchError::InvalidClientRef)
        ));
        assert_eq!(store.puts.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn multipart_requires_exact_parts_and_completes_idempotently()
    -> Result<(), Box<dyn std::error::Error>> {
        const TOTAL: u64 = 50 * 1024 * 1024;
        const PART: u64 = 32 * 1024 * 1024;
        let store = Arc::new(FakeStore::default());
        store.head_length.store(TOTAL, Ordering::Relaxed);
        let repository = Arc::new(FakeRepository::default());
        let service =
            UploadIntentService::new(store.clone(), repository, UploadBatchPolicy::default());
        let created = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files: vec![file("video-a", MediaKind::Video, TOTAL, "video/mp4")],
            })
            .await?;
        let upload_id = created.uploads[0].upload_id;

        service.presign_part("site-a", upload_id, 1, PART).await?;
        assert!(matches!(
            service.presign_part("site-a", upload_id, 2, PART).await,
            Err(MultipartControlError::InvalidPart)
        ));
        service
            .presign_part("site-a", upload_id, 2, TOTAL - PART)
            .await?;
        let parts = vec![
            crate::CompletedPart {
                part_number: 1,
                etag: "etag-one".to_owned(),
            },
            crate::CompletedPart {
                part_number: 2,
                etag: "etag-two".to_owned(),
            },
        ];
        service
            .complete_multipart("site-a", upload_id, parts.clone())
            .await?;
        service
            .complete_multipart("site-a", upload_id, parts)
            .await?;
        assert_eq!(store.completes.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn multipart_completion_rejects_missing_or_reordered_parts()
    -> Result<(), Box<dyn std::error::Error>> {
        const TOTAL: u64 = 50 * 1024 * 1024;
        let store = Arc::new(FakeStore::default());
        let repository = Arc::new(FakeRepository::default());
        let service =
            UploadIntentService::new(store.clone(), repository, UploadBatchPolicy::default());
        let created = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files: vec![file("video-a", MediaKind::Video, TOTAL, "video/mp4")],
            })
            .await?;
        let result = service
            .complete_multipart(
                "site-a",
                created.uploads[0].upload_id,
                vec![crate::CompletedPart {
                    part_number: 2,
                    etag: "etag-two".to_owned(),
                }],
            )
            .await;
        assert!(matches!(
            result,
            Err(MultipartControlError::InvalidCompletion)
        ));
        assert_eq!(store.completes.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn single_put_confirmation_verifies_size_and_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let store = Arc::new(FakeStore::default());
        store.head_length.store(1024, Ordering::Relaxed);
        let repository = Arc::new(FakeRepository::default());
        let service = UploadIntentService::new(store, repository, UploadBatchPolicy::default());
        let created = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files: vec![file("image-a", MediaKind::Image, 1024, "image/jpeg")],
            })
            .await?;
        let upload_id = created.uploads[0].upload_id;
        service.confirm_single_upload("site-a", upload_id).await?;
        service.confirm_single_upload("site-a", upload_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn multipart_abort_is_idempotent_and_disables_new_parts()
    -> Result<(), Box<dyn std::error::Error>> {
        const TOTAL: u64 = 50 * 1024 * 1024;
        let store = Arc::new(FakeStore::default());
        let repository = Arc::new(FakeRepository::default());
        let service =
            UploadIntentService::new(store.clone(), repository, UploadBatchPolicy::default());
        let created = service
            .create_batch(CreateUploadBatch {
                tenant_id: "site-a".to_owned(),
                files: vec![file("video-a", MediaKind::Video, TOTAL, "video/mp4")],
            })
            .await?;
        let upload_id = created.uploads[0].upload_id;
        service.abort_multipart("site-a", upload_id).await?;
        service.abort_multipart("site-a", upload_id).await?;
        assert_eq!(store.aborts.load(Ordering::Relaxed), 1);
        assert!(matches!(
            service
                .presign_part("site-a", upload_id, 1, 32 * 1024 * 1024)
                .await,
            Err(MultipartControlError::InvalidState)
        ));
        Ok(())
    }

    fn file(
        client_ref: &str,
        declared_kind: MediaKind,
        content_length: u64,
        content_type_hint: &str,
    ) -> UploadFileIntent {
        UploadFileIntent {
            client_ref: client_ref.to_owned(),
            declared_kind,
            content_length,
            content_type_hint: content_type_hint.to_owned(),
        }
    }
}
