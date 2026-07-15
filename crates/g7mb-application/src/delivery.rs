//! Tenant-scoped private derivative delivery without proxying media bytes.

use std::{sync::Arc, time::Duration};

use g7mb_domain::{UploadId, UploadState};
use secrecy::SecretString;
use thiserror::Error;
use time::OffsetDateTime;

use crate::{
    ObjectStore, ObjectStoreError, PresignGetRequest,
    uploads::{UploadRepository, UploadRepositoryError},
};

const MIN_DELIVERY_TTL: Duration = Duration::from_secs(30);
const MAX_DELIVERY_TTL: Duration = Duration::from_secs(15 * 60);

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
    expires_in: Duration,
}

impl DerivativeDeliveryService {
    /// Creates a service with a bounded signature lifetime.
    pub fn new(
        repository: Arc<dyn UploadRepository>,
        derivative_store: Arc<dyn ObjectStore>,
        expires_in: Duration,
    ) -> Result<Self, DerivativeDeliveryError> {
        if expires_in < MIN_DELIVERY_TTL || expires_in > MAX_DELIVERY_TTL {
            return Err(DerivativeDeliveryError::InvalidPolicy);
        }
        Ok(Self {
            repository,
            derivative_store,
            expires_in,
        })
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
        let status = self
            .repository
            .find_status(tenant_id, upload_id)
            .await?
            .ok_or(DerivativeDeliveryError::NotFound)?;
        if status.state != UploadState::Ready || status.deletion_pending {
            return Err(DerivativeDeliveryError::NotReady);
        }
        let mut matching = status
            .derivatives
            .into_iter()
            .filter(|derivative| derivative.variant == variant);
        let derivative = matching.next().ok_or(DerivativeDeliveryError::NotFound)?;
        if matching.next().is_some() {
            return Err(DerivativeDeliveryError::NotReady);
        }
        let signed = self
            .derivative_store
            .presign_get(PresignGetRequest {
                key: derivative.object_key,
                expires_in: self.expires_in,
            })
            .await?;
        Ok(DerivativeDelivery {
            preset_id: derivative.preset_id,
            variant: derivative.variant,
            content_type: derivative.content_type,
            byte_len: derivative.byte_len,
            url: signed.url,
            expires_at: signed.expires_at,
        })
    }
}

fn valid_tenant(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
