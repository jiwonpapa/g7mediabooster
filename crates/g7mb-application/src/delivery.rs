//! Tenant-scoped private derivative delivery without proxying media bytes.

use std::{sync::Arc, time::Duration};

use g7mb_domain::{UploadId, UploadState};
use metrics::{counter, gauge};
use moka::future::Cache;
use secrecy::SecretString;
use thiserror::Error;
use time::OffsetDateTime;

use crate::{
    ObjectStore, ObjectStoreError, PresignGetRequest,
    uploads::{StoredDerivative, UploadRepository, UploadRepositoryError, UploadStatusSnapshot},
};

const MIN_DELIVERY_TTL: Duration = Duration::from_secs(30);
const MAX_DELIVERY_TTL: Duration = Duration::from_secs(15 * 60);
const MIN_MANIFEST_CACHE_TTL: Duration = Duration::from_secs(1);
const MAX_MANIFEST_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MIN_MANIFEST_CACHE_BYTES: u64 = 64 * 1024;
const MAX_MANIFEST_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// Bounded private-delivery and immutable-manifest cache policy.
#[derive(Clone, Copy, Debug)]
pub struct DerivativeDeliveryPolicy {
    /// Provider signature lifetime.
    pub signed_url_ttl: Duration,
    /// Maximum age of an immutable derivative manifest in memory.
    pub manifest_cache_ttl: Duration,
    /// Approximate total bytes admitted to the manifest cache.
    pub manifest_cache_max_bytes: u64,
}

impl Default for DerivativeDeliveryPolicy {
    fn default() -> Self {
        Self {
            signed_url_ttl: Duration::from_secs(5 * 60),
            manifest_cache_ttl: Duration::from_secs(60),
            manifest_cache_max_bytes: 4 * 1024 * 1024,
        }
    }
}

impl DerivativeDeliveryPolicy {
    fn is_valid(self) -> bool {
        (MIN_DELIVERY_TTL..=MAX_DELIVERY_TTL).contains(&self.signed_url_ttl)
            && (MIN_MANIFEST_CACHE_TTL..=MAX_MANIFEST_CACHE_TTL).contains(&self.manifest_cache_ttl)
            && self.manifest_cache_ttl <= self.signed_url_ttl
            && (MIN_MANIFEST_CACHE_BYTES..=MAX_MANIFEST_CACHE_BYTES)
                .contains(&self.manifest_cache_max_bytes)
    }
}

/// One authorized private derivative redirect target.
#[derive(Clone, Debug)]
pub struct DerivativeDelivery {
    /// Versioned server preset.
    pub preset_id: String,
    /// Stable `master` or `thumbnail` variant.
    pub variant: String,
    /// Trusted encoded content type.
    pub content_type: String,
    /// Exact encoded byte length.
    pub byte_len: u64,
    /// Sensitive provider GET URL.
    pub url: SecretString,
    /// Absolute provider signature expiration.
    pub expires_at: OffsetDateTime,
}

/// Private derivative delivery policy or dependency failure.
#[derive(Debug, Error)]
pub enum DerivativeDeliveryError {
    /// Configured signature lifetime is outside the safe delivery range.
    #[error("derivative delivery policy is invalid")]
    InvalidPolicy,
    /// Authenticated tenant does not satisfy the narrow tenant invariant.
    #[error("tenant identifier is invalid")]
    InvalidTenant,
    /// Variant is not one of the server-published delivery variants.
    #[error("derivative variant is invalid")]
    InvalidVariant,
    /// Upload or requested derivative is not owned by the tenant.
    #[error("derivative was not found")]
    NotFound,
    /// Media is not completely Ready or deletion already started.
    #[error("derivative is not deliverable")]
    NotReady,
    /// Durable state failed without exposing backend details.
    #[error(transparent)]
    Repository(#[from] UploadRepositoryError),
    /// Object-store signing failed without exposing the signed URL.
    #[error(transparent)]
    ObjectStore(#[from] ObjectStoreError),
}

/// Authorizes tenant ownership and signs the immutable derivative key only.
#[derive(Clone)]
pub struct DerivativeDeliveryService {
    repository: Arc<dyn UploadRepository>,
    derivative_store: Arc<dyn ObjectStore>,
    policy: DerivativeDeliveryPolicy,
    manifests: Cache<ManifestKey, Arc<DerivativeManifest>>,
}

impl DerivativeDeliveryService {
    /// Creates a service with bounded signatures, cache memory, and cache lifetime.
    pub fn new(
        repository: Arc<dyn UploadRepository>,
        derivative_store: Arc<dyn ObjectStore>,
        policy: DerivativeDeliveryPolicy,
    ) -> Result<Self, DerivativeDeliveryError> {
        if !policy.is_valid() {
            return Err(DerivativeDeliveryError::InvalidPolicy);
        }
        let manifests = Cache::builder()
            .max_capacity(policy.manifest_cache_max_bytes)
            .weigher(manifest_weight)
            .time_to_live(policy.manifest_cache_ttl)
            .build();
        Ok(Self {
            repository,
            derivative_store,
            policy,
            manifests,
        })
    }

    /// Invalidates immutable metadata after a durable deletion request.
    pub async fn invalidate_upload(&self, tenant_id: &str, upload_id: UploadId) {
        self.manifests
            .invalidate(&ManifestKey::new(tenant_id, upload_id))
            .await;
    }

    /// Returns a short-lived private URL only for a complete tenant-owned asset.
    pub async fn presign(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
        variant: &str,
    ) -> Result<DerivativeDelivery, DerivativeDeliveryError> {
        if !valid_tenant(tenant_id) {
            return Err(DerivativeDeliveryError::InvalidTenant);
        }
        if !matches!(variant, "master" | "thumbnail") {
            return Err(DerivativeDeliveryError::InvalidVariant);
        }
        let key = ManifestKey::new(tenant_id, upload_id);
        let manifest = if let Some(manifest) = self.manifests.get(&key).await {
            counter!("g7mb_delivery_manifest_cache_requests_total", "result" => "hit").increment(1);
            manifest
        } else {
            counter!("g7mb_delivery_manifest_cache_requests_total", "result" => "miss")
                .increment(1);
            let repository = self.repository.clone();
            let load_key = key.clone();
            self.manifests
                .try_get_with(key.clone(), async move {
                    load_manifest(repository, load_key).await
                })
                .await
                .map_err(map_manifest_load_error)?
        };
        let cached_bytes = u32::try_from(self.manifests.weighted_size()).unwrap_or(u32::MAX);
        gauge!("g7mb_delivery_manifest_cache_weight_bytes").set(cached_bytes);
        match self
            .repository
            .is_delivery_allowed(tenant_id, upload_id)
            .await?
        {
            Some(true) => {}
            Some(false) => {
                self.manifests.invalidate(&key).await;
                return Err(DerivativeDeliveryError::NotReady);
            }
            None => {
                self.manifests.invalidate(&key).await;
                return Err(DerivativeDeliveryError::NotFound);
            }
        }
        let derivative = manifest
            .derivative(variant)
            .ok_or(DerivativeDeliveryError::NotFound)?;
        let signed = self
            .derivative_store
            .presign_get(PresignGetRequest {
                key: derivative.object_key.clone(),
                expires_in: self.policy.signed_url_ttl,
            })
            .await?;
        Ok(DerivativeDelivery {
            preset_id: derivative.preset_id.clone(),
            variant: derivative.variant.clone(),
            content_type: derivative.content_type.clone(),
            byte_len: derivative.byte_len,
            url: signed.url,
            expires_at: signed.expires_at,
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ManifestKey {
    tenant_id: String,
    upload_id: UploadId,
}

impl ManifestKey {
    fn new(tenant_id: &str, upload_id: UploadId) -> Self {
        Self {
            tenant_id: tenant_id.to_owned(),
            upload_id,
        }
    }
}

#[derive(Debug)]
struct DerivativeManifest {
    derivatives: Vec<StoredDerivative>,
}

impl DerivativeManifest {
    fn from_status(status: UploadStatusSnapshot) -> Result<Self, ManifestLoadError> {
        if status.state != UploadState::Ready
            || status.deletion_pending
            || status.derivatives.len() != 2
        {
            return Err(ManifestLoadError::NotReady);
        }
        let master_count = status
            .derivatives
            .iter()
            .filter(|derivative| derivative.variant == "master")
            .count();
        let thumbnail_count = status
            .derivatives
            .iter()
            .filter(|derivative| derivative.variant == "thumbnail")
            .count();
        let preset = status
            .derivatives
            .first()
            .map(|item| item.preset_id.as_str());
        let valid = master_count == 1
            && thumbnail_count == 1
            && status.derivatives.iter().all(|derivative| {
                derivative.byte_len > 0
                    && !derivative.preset_id.is_empty()
                    && derivative.preset_id.len() <= 128
                    && derivative
                        .preset_id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    && preset == Some(derivative.preset_id.as_str())
                    && match derivative.variant.as_str() {
                        "master" => {
                            matches!(derivative.content_type.as_str(), "image/jpeg" | "video/mp4")
                        }
                        "thumbnail" => derivative.content_type == "image/jpeg",
                        _ => false,
                    }
            });
        if !valid {
            return Err(ManifestLoadError::NotReady);
        }
        Ok(Self {
            derivatives: status.derivatives,
        })
    }

    fn derivative(&self, variant: &str) -> Option<&StoredDerivative> {
        self.derivatives
            .iter()
            .find(|derivative| derivative.variant == variant)
    }
}

#[derive(Clone, Debug, Error)]
enum ManifestLoadError {
    #[error("derivative was not found")]
    NotFound,
    #[error("derivative is not deliverable")]
    NotReady,
    #[error("delivery manifest repository lookup failed")]
    Repository,
}

async fn load_manifest(
    repository: Arc<dyn UploadRepository>,
    key: ManifestKey,
) -> Result<Arc<DerivativeManifest>, ManifestLoadError> {
    let status = repository
        .find_status(&key.tenant_id, key.upload_id)
        .await
        .map_err(|_| ManifestLoadError::Repository)?
        .ok_or(ManifestLoadError::NotFound)?;
    if status.upload_id != key.upload_id {
        return Err(ManifestLoadError::NotReady);
    }
    DerivativeManifest::from_status(status).map(Arc::new)
}

fn map_manifest_load_error(error: Arc<ManifestLoadError>) -> DerivativeDeliveryError {
    match error.as_ref() {
        ManifestLoadError::NotFound => DerivativeDeliveryError::NotFound,
        ManifestLoadError::NotReady => DerivativeDeliveryError::NotReady,
        ManifestLoadError::Repository => DerivativeDeliveryError::Repository(
            UploadRepositoryError::Backend("delivery manifest lookup failed".to_owned()),
        ),
    }
}

fn manifest_weight(key: &ManifestKey, manifest: &Arc<DerivativeManifest>) -> u32 {
    let mut bytes = 128_usize.saturating_add(key.tenant_id.len());
    for derivative in &manifest.derivatives {
        bytes = bytes
            .saturating_add(derivative.preset_id.len())
            .saturating_add(derivative.variant.len())
            .saturating_add(derivative.object_key.as_str().len())
            .saturating_add(derivative.content_type.len())
            .saturating_add(std::mem::size_of::<StoredDerivative>());
    }
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

fn valid_tenant(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use g7mb_domain::{ObjectKey, UploadId, UploadState};
    use secrecy::SecretString;
    use time::OffsetDateTime;
    use tokio::{sync::Barrier, task::JoinSet};

    use super::{DerivativeDeliveryError, DerivativeDeliveryPolicy, DerivativeDeliveryService};
    use crate::{
        AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest,
        DownloadObjectRequest, MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError,
        PresignGetRequest, PresignPartRequest, PresignPutRequest, PresignedDownload,
        PresignedUpload, PutFileRequest,
        uploads::{
            StoredDerivative, StoredUploadReservation, UploadBatchReservation,
            UploadCapacityPolicy, UploadRepository, UploadRepositoryError, UploadStatusSnapshot,
        },
    };

    struct FakeRepository {
        find_status_calls: AtomicUsize,
        allowed: AtomicBool,
        delay: Duration,
        large_manifest: bool,
    }

    impl FakeRepository {
        fn new(delay: Duration, large_manifest: bool) -> Self {
            Self {
                find_status_calls: AtomicUsize::new(0),
                allowed: AtomicBool::new(true),
                delay,
                large_manifest,
            }
        }

        fn status(
            &self,
            upload_id: UploadId,
        ) -> Result<UploadStatusSnapshot, UploadRepositoryError> {
            let padding = if self.large_manifest {
                format!("{}/", "x".repeat(800))
            } else {
                String::new()
            };
            let derivatives = [
                ("master", "image/jpeg", 2048_u64),
                ("thumbnail", "image/jpeg", 512_u64),
            ]
            .into_iter()
            .map(|(variant, content_type, byte_len)| {
                Ok(StoredDerivative {
                    preset_id: "board-v1".to_owned(),
                    variant: variant.to_owned(),
                    object_key: ObjectKey::new(format!(
                        "media/site-a/{upload_id}/{padding}{variant}.jpg"
                    ))
                    .map_err(|_| {
                        UploadRepositoryError::Backend("test object key is invalid".to_owned())
                    })?,
                    content_type: content_type.to_owned(),
                    byte_len,
                })
            })
            .collect::<Result<Vec<_>, UploadRepositoryError>>()?;
            Ok(UploadStatusSnapshot {
                upload_id,
                state: UploadState::Ready,
                detected_content_type: Some("image/jpeg".to_owned()),
                error_code: None,
                deletion_pending: false,
                derivatives,
            })
        }
    }

    #[async_trait]
    impl UploadRepository for FakeRepository {
        async fn has_capacity(
            &self,
            _tenant_id: &str,
            _additional_uploads: usize,
            _additional_bytes: u64,
            _capacity: UploadCapacityPolicy,
        ) -> Result<bool, UploadRepositoryError> {
            Ok(true)
        }

        async fn save_batch(
            &self,
            _batch: &UploadBatchReservation,
            _capacity: UploadCapacityPolicy,
        ) -> Result<(), UploadRepositoryError> {
            Ok(())
        }

        async fn find_upload(
            &self,
            _tenant_id: &str,
            _upload_id: UploadId,
        ) -> Result<Option<StoredUploadReservation>, UploadRepositoryError> {
            Ok(None)
        }

        async fn find_status(
            &self,
            tenant_id: &str,
            upload_id: UploadId,
        ) -> Result<Option<UploadStatusSnapshot>, UploadRepositoryError> {
            self.find_status_calls.fetch_add(1, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            if tenant_id != "site-a" {
                return Ok(None);
            }
            self.status(upload_id).map(Some)
        }

        async fn is_delivery_allowed(
            &self,
            tenant_id: &str,
            _upload_id: UploadId,
        ) -> Result<Option<bool>, UploadRepositoryError> {
            Ok((tenant_id == "site-a").then(|| self.allowed.load(Ordering::SeqCst)))
        }

        async fn mark_quarantined_and_enqueue(
            &self,
            _tenant_id: &str,
            _upload_id: UploadId,
            _actual_size_bytes: u64,
            _preset_id: &str,
            _now: OffsetDateTime,
        ) -> Result<(), UploadRepositoryError> {
            Ok(())
        }

        async fn mark_deleted(
            &self,
            _tenant_id: &str,
            _upload_id: UploadId,
            _now: OffsetDateTime,
        ) -> Result<(), UploadRepositoryError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeStore {
        presign_calls: AtomicUsize,
    }

    impl FakeStore {
        fn unsupported() -> ObjectStoreError {
            ObjectStoreError::InvalidRequest("test operation is unavailable".to_owned())
        }
    }

    #[async_trait]
    impl ObjectStore for FakeStore {
        async fn presign_put(
            &self,
            _request: PresignPutRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn presign_get(
            &self,
            request: PresignGetRequest,
        ) -> Result<PresignedDownload, ObjectStoreError> {
            self.presign_calls.fetch_add(1, Ordering::SeqCst);
            let seconds =
                i64::try_from(request.expires_in.as_secs()).map_err(|_| Self::unsupported())?;
            Ok(PresignedDownload {
                url: SecretString::from(format!(
                    "https://private.invalid/{}",
                    request.key.as_str()
                )),
                expires_at: OffsetDateTime::now_utc() + time::Duration::seconds(seconds),
            })
        }

        async fn create_multipart(
            &self,
            _request: CreateMultipartRequest,
        ) -> Result<MultipartSession, ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn presign_part(
            &self,
            _request: PresignPartRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn complete_multipart(
            &self,
            _request: CompleteMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn abort_multipart(
            &self,
            _request: AbortMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn head(&self, _key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn download_to(
            &self,
            _request: DownloadObjectRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn put_file(
            &self,
            _request: PutFileRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(Self::unsupported())
        }

        async fn delete(&self, _key: &ObjectKey) -> Result<(), ObjectStoreError> {
            Err(Self::unsupported())
        }
    }

    fn service(
        repository: Arc<FakeRepository>,
        store: Arc<FakeStore>,
        policy: DerivativeDeliveryPolicy,
    ) -> Result<DerivativeDeliveryService, DerivativeDeliveryError> {
        DerivativeDeliveryService::new(repository, store, policy)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn coalesces_concurrent_manifest_misses_per_upload()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = Arc::new(FakeRepository::new(Duration::from_millis(30), false));
        let store = Arc::new(FakeStore::default());
        let service = service(
            repository.clone(),
            store.clone(),
            DerivativeDeliveryPolicy::default(),
        )?;
        let upload_id = UploadId::new();
        let barrier = Arc::new(Barrier::new(17));
        let mut tasks = JoinSet::new();
        for _ in 0..16 {
            let service = service.clone();
            let barrier = barrier.clone();
            tasks.spawn(async move {
                barrier.wait().await;
                service.presign("site-a", upload_id, "thumbnail").await
            });
        }
        barrier.wait().await;
        while let Some(result) = tasks.join_next().await {
            result??;
        }

        assert_eq!(repository.find_status_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.presign_calls.load(Ordering::SeqCst), 16);
        Ok(())
    }

    #[tokio::test]
    async fn expires_manifest_by_ttl_and_reloads_it() -> Result<(), Box<dyn std::error::Error>> {
        let repository = Arc::new(FakeRepository::new(Duration::ZERO, false));
        let store = Arc::new(FakeStore::default());
        let service = service(
            repository.clone(),
            store,
            DerivativeDeliveryPolicy {
                manifest_cache_ttl: Duration::from_secs(1),
                ..DerivativeDeliveryPolicy::default()
            },
        )?;
        let upload_id = UploadId::new();
        service.presign("site-a", upload_id, "thumbnail").await?;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        service.manifests.run_pending_tasks().await;
        service.presign("site-a", upload_id, "thumbnail").await?;

        assert_eq!(repository.find_status_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn cache_weight_never_exceeds_the_configured_byte_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = Arc::new(FakeRepository::new(Duration::ZERO, true));
        let store = Arc::new(FakeStore::default());
        let service = service(
            repository,
            store,
            DerivativeDeliveryPolicy {
                manifest_cache_max_bytes: 64 * 1024,
                ..DerivativeDeliveryPolicy::default()
            },
        )?;
        for _ in 0..64 {
            service
                .presign("site-a", UploadId::new(), "thumbnail")
                .await?;
        }
        service.manifests.run_pending_tasks().await;

        assert!(service.manifests.weighted_size() <= 64 * 1024);
        assert!(service.manifests.entry_count() < 64);
        Ok(())
    }

    #[tokio::test]
    async fn mutable_guard_revokes_and_invalidates_a_cached_manifest()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = Arc::new(FakeRepository::new(Duration::ZERO, false));
        let store = Arc::new(FakeStore::default());
        let service = service(
            repository.clone(),
            store,
            DerivativeDeliveryPolicy::default(),
        )?;
        let upload_id = UploadId::new();
        service.presign("site-a", upload_id, "thumbnail").await?;
        repository.allowed.store(false, Ordering::SeqCst);
        let error = match service.presign("site-a", upload_id, "thumbnail").await {
            Ok(_) => {
                return Err(std::io::Error::other("delivery was not revoked").into());
            }
            Err(error) => error,
        };
        assert!(matches!(error, DerivativeDeliveryError::NotReady));
        repository.allowed.store(true, Ordering::SeqCst);
        service.presign("site-a", upload_id, "thumbnail").await?;

        assert_eq!(repository.find_status_calls.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[test]
    fn rejects_unbounded_manifest_cache_policy() {
        let repository = Arc::new(FakeRepository::new(Duration::ZERO, false));
        let store = Arc::new(FakeStore::default());
        let result = service(
            repository,
            store,
            DerivativeDeliveryPolicy {
                manifest_cache_max_bytes: 1024,
                ..DerivativeDeliveryPolicy::default()
            },
        );
        assert!(matches!(
            result,
            Err(DerivativeDeliveryError::InvalidPolicy)
        ));
    }
}
