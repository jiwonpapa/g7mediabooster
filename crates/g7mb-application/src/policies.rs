//! Versioned tenant policy snapshots published by authenticated PHP modules.

use std::sync::Arc;

use async_trait::async_trait;
use g7mb_domain::{MediaKind, ObjectKey, UploadId, UploadState};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::OffsetDateTime;

use crate::WatermarkPosition;

/// Only this policy schema is accepted by the v1 control plane.
pub const SITE_POLICY_SCHEMA_VERSION: u16 = 1;
/// Watermark source bytes are deliberately much smaller than normal media uploads.
pub const MAX_WATERMARK_ASSET_BYTES: u64 = 16 * 1024 * 1024;

/// Admin-selected watermark settings before the source upload is trusted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestedWatermarkPolicy {
    /// Existing tenant-owned image upload selected by the administrator.
    pub asset_upload_id: UploadId,
    /// Allowlisted anchor.
    pub position: WatermarkPosition,
    /// Safe distance from selected edges.
    pub margin_px: u32,
    /// Maximum rendered width as a percentage of the derivative.
    pub max_width_percent: u8,
    /// Alpha multiplier percentage.
    pub opacity_percent: u8,
}

/// Signed control request converted into application values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishSitePolicy {
    /// Authenticated tenant identity; it never comes from request JSON.
    pub tenant_id: String,
    /// Wire schema version.
    pub schema_version: u16,
    /// Strictly increasing tenant revision.
    pub revision: u64,
    /// PHP-side issuance timestamp retained for audit.
    pub issued_at: OffsetDateTime,
    /// `None` explicitly disables watermarking.
    pub watermark: Option<RequestedWatermarkPolicy>,
}

/// Tenant-owned upload facts inspected before it may become policy input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyAssetCandidate {
    /// Selected upload identifier.
    pub upload_id: UploadId,
    /// Immutable private raw object key.
    pub object_key: ObjectKey,
    /// Authenticated declared media class.
    pub declared_kind: MediaKind,
    /// Current processing lifecycle.
    pub state: UploadState,
    /// Exact confirmed source byte length.
    pub byte_len: u64,
    /// Normalized decoder-confirmed content type.
    pub detected_content_type: Option<String>,
    /// Decoder-confirmed source SHA-256.
    pub source_sha256: Option<String>,
}

/// Immutable validated watermark settings stored inside one snapshot revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredWatermarkPolicy {
    /// Original tenant upload retained for admin display and audit.
    pub asset_upload_id: UploadId,
    /// Private object key captured when the snapshot was published.
    pub object_key: ObjectKey,
    /// Exact source length captured at publication.
    pub byte_len: u64,
    /// Lowercase SHA-256 pin captured at publication.
    pub asset_sha256: String,
    /// Allowlisted anchor.
    pub position: WatermarkPosition,
    /// Safe edge margin.
    pub margin_px: u32,
    /// Maximum derivative width percentage.
    pub max_width_percent: u8,
    /// Alpha multiplier percentage.
    pub opacity_percent: u8,
}

/// Durable tenant policy revision consumed by workers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SitePolicySnapshot {
    /// Owning tenant.
    pub tenant_id: String,
    /// Stable wire schema.
    pub schema_version: u16,
    /// Monotonic revision.
    pub revision: u64,
    /// PHP-side issuance time.
    pub issued_at: OffsetDateTime,
    /// Rust-computed hash over normalized settings and pinned asset facts.
    pub settings_sha256: String,
    /// `None` means watermarking is disabled at this revision.
    pub watermark: Option<StoredWatermarkPolicy>,
}

/// Result of a compare-and-swap policy publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishPolicyOutcome {
    /// A new revision became active.
    Published,
    /// The exact same revision and hash had already been committed.
    Unchanged,
}

/// Persistence errors retain conflict semantics without exposing SQL details.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum SitePolicyRepositoryError {
    /// Revision is not the next revision or reuses a revision with different settings.
    #[error("site policy revision conflicts with durable state")]
    RevisionConflict,
    /// Persistence is unavailable.
    #[error("site policy repository failed: {0}")]
    Backend(String),
}

/// Durable policy and asset lookup port.
#[async_trait]
pub trait SitePolicyRepository: Send + Sync {
    /// Finds an upload inside the authenticated tenant boundary.
    async fn find_policy_asset(
        &self,
        tenant_id: &str,
        upload_id: UploadId,
    ) -> Result<Option<PolicyAssetCandidate>, SitePolicyRepositoryError>;

    /// Atomically publishes exactly the next revision, with idempotent replay support.
    async fn publish_site_policy(
        &self,
        snapshot: &SitePolicySnapshot,
    ) -> Result<PublishPolicyOutcome, SitePolicyRepositoryError>;

    /// Returns the highest active revision for one tenant.
    async fn find_active_site_policy(
        &self,
        tenant_id: &str,
    ) -> Result<Option<SitePolicySnapshot>, SitePolicyRepositoryError>;

    /// Loads one exact immutable revision for deterministic worker retries.
    async fn find_site_policy_revision(
        &self,
        tenant_id: &str,
        revision: u64,
    ) -> Result<Option<SitePolicySnapshot>, SitePolicyRepositoryError>;
}

/// Validates and publishes authenticated policy snapshots below Rust hard caps.
#[derive(Clone)]
pub struct SitePolicyService {
    repository: Arc<dyn SitePolicyRepository>,
    max_issued_at_skew_seconds: u64,
}

impl std::fmt::Debug for SitePolicyService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SitePolicyService")
            .field(
                "max_issued_at_skew_seconds",
                &self.max_issued_at_skew_seconds,
            )
            .finish_non_exhaustive()
    }
}

impl SitePolicyService {
    /// Creates a service with an explicit issuance-clock tolerance.
    pub fn new(repository: Arc<dyn SitePolicyRepository>, max_issued_at_skew_seconds: u64) -> Self {
        Self {
            repository,
            max_issued_at_skew_seconds,
        }
    }

    /// Validates asset ownership and pins normalized settings into the next revision.
    pub async fn publish(
        &self,
        request: PublishSitePolicy,
        now: OffsetDateTime,
    ) -> Result<(SitePolicySnapshot, PublishPolicyOutcome), PublishSitePolicyError> {
        validate_snapshot_envelope(&request, now, self.max_issued_at_skew_seconds)?;
        let watermark = match request.watermark {
            Some(requested) => {
                validate_watermark_settings(&requested)?;
                let candidate = self
                    .repository
                    .find_policy_asset(&request.tenant_id, requested.asset_upload_id)
                    .await?
                    .ok_or(PublishSitePolicyError::AssetNotFound)?;
                Some(validate_asset(candidate, requested)?)
            }
            None => None,
        };
        let settings_sha256 = normalized_settings_sha256(watermark.as_ref());
        let snapshot = SitePolicySnapshot {
            tenant_id: request.tenant_id,
            schema_version: request.schema_version,
            revision: request.revision,
            issued_at: request.issued_at,
            settings_sha256,
            watermark,
        };
        let outcome = self.repository.publish_site_policy(&snapshot).await?;
        Ok((snapshot, outcome))
    }

    /// Returns the active snapshot for an authenticated tenant.
    pub async fn active(
        &self,
        tenant_id: &str,
    ) -> Result<Option<SitePolicySnapshot>, PublishSitePolicyError> {
        if !tenant_is_valid(tenant_id) {
            return Err(PublishSitePolicyError::InvalidTenant);
        }
        self.repository
            .find_active_site_policy(tenant_id)
            .await
            .map_err(Into::into)
    }
}

/// Stable publication failures mapped by the HTTP adapter.
#[derive(Debug, Error)]
pub enum PublishSitePolicyError {
    /// Authenticated tenant configuration is invalid.
    #[error("tenant identifier is invalid")]
    InvalidTenant,
    /// Schema, revision, timestamp, or watermark bounds are invalid.
    #[error("site policy envelope or settings are invalid")]
    InvalidPolicy,
    /// Selected upload is not owned by this tenant.
    #[error("watermark asset upload was not found")]
    AssetNotFound,
    /// Selected upload has not passed the strict image gate.
    #[error("watermark asset upload is not a validated supported image")]
    AssetRejected,
    /// Revision conflicts with durable state.
    #[error("site policy revision conflicts with durable state")]
    RevisionConflict,
    /// Durable state is unavailable.
    #[error("site policy persistence is unavailable")]
    Repository,
}

impl From<SitePolicyRepositoryError> for PublishSitePolicyError {
    fn from(error: SitePolicyRepositoryError) -> Self {
        match error {
            SitePolicyRepositoryError::RevisionConflict => Self::RevisionConflict,
            SitePolicyRepositoryError::Backend(_) => Self::Repository,
        }
    }
}

fn validate_snapshot_envelope(
    request: &PublishSitePolicy,
    now: OffsetDateTime,
    max_skew_seconds: u64,
) -> Result<(), PublishSitePolicyError> {
    let skew = now
        .unix_timestamp()
        .abs_diff(request.issued_at.unix_timestamp());
    if !tenant_is_valid(&request.tenant_id)
        || request.schema_version != SITE_POLICY_SCHEMA_VERSION
        || request.revision == 0
        || request.revision > i64::MAX as u64
        || skew > max_skew_seconds
    {
        return Err(if tenant_is_valid(&request.tenant_id) {
            PublishSitePolicyError::InvalidPolicy
        } else {
            PublishSitePolicyError::InvalidTenant
        });
    }
    Ok(())
}

fn validate_watermark_settings(
    watermark: &RequestedWatermarkPolicy,
) -> Result<(), PublishSitePolicyError> {
    if watermark.margin_px > 1024
        || !(1..=50).contains(&watermark.max_width_percent)
        || !(1..=100).contains(&watermark.opacity_percent)
    {
        return Err(PublishSitePolicyError::InvalidPolicy);
    }
    Ok(())
}

fn validate_asset(
    candidate: PolicyAssetCandidate,
    requested: RequestedWatermarkPolicy,
) -> Result<StoredWatermarkPolicy, PublishSitePolicyError> {
    let digest = candidate
        .source_sha256
        .filter(|value| is_lower_sha256(value))
        .ok_or(PublishSitePolicyError::AssetRejected)?;
    let content_type = candidate
        .detected_content_type
        .as_deref()
        .ok_or(PublishSitePolicyError::AssetRejected)?;
    if candidate.upload_id != requested.asset_upload_id
        || candidate.declared_kind != MediaKind::Image
        || candidate.state != UploadState::Ready
        || candidate.byte_len == 0
        || candidate.byte_len > MAX_WATERMARK_ASSET_BYTES
        || !matches!(content_type, "image/jpeg" | "image/png" | "image/webp")
    {
        return Err(PublishSitePolicyError::AssetRejected);
    }
    Ok(StoredWatermarkPolicy {
        asset_upload_id: candidate.upload_id,
        object_key: candidate.object_key,
        byte_len: candidate.byte_len,
        asset_sha256: digest,
        position: requested.position,
        margin_px: requested.margin_px,
        max_width_percent: requested.max_width_percent,
        opacity_percent: requested.opacity_percent,
    })
}

fn normalized_settings_sha256(watermark: Option<&StoredWatermarkPolicy>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"g7mb-site-policy-settings-v1\0");
    match watermark {
        None => hasher.update(b"watermark\0disabled\0"),
        Some(watermark) => {
            hasher.update(b"watermark\0enabled\0");
            hasher.update(watermark.asset_upload_id.as_uuid().as_bytes());
            hasher.update(b"\0");
            hasher.update(watermark.object_key.as_str().as_bytes());
            hasher.update(b"\0");
            hasher.update(watermark.byte_len.to_be_bytes());
            hasher.update(watermark.asset_sha256.as_bytes());
            hasher.update(watermark_position_name(watermark.position).as_bytes());
            hasher.update(watermark.margin_px.to_be_bytes());
            hasher.update([watermark.max_width_percent, watermark.opacity_percent]);
        }
    }
    hex::encode(hasher.finalize())
}

const fn watermark_position_name(position: WatermarkPosition) -> &'static str {
    match position {
        WatermarkPosition::Center => "center",
        WatermarkPosition::TopLeft => "top-left",
        WatermarkPosition::TopRight => "top-right",
        WatermarkPosition::BottomLeft => "bottom-left",
        WatermarkPosition::BottomRight => "bottom-right",
    }
}

fn tenant_is_valid(tenant_id: &str) -> bool {
    !tenant_id.is_empty()
        && tenant_id.len() <= 64
        && tenant_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use g7mb_domain::{MediaKind, ObjectKey, UploadId, UploadState};
    use time::OffsetDateTime;

    use super::{
        PolicyAssetCandidate, PublishPolicyOutcome, PublishSitePolicy, PublishSitePolicyError,
        RequestedWatermarkPolicy, SITE_POLICY_SCHEMA_VERSION, SitePolicyRepository,
        SitePolicyRepositoryError, SitePolicyService, SitePolicySnapshot,
    };
    use crate::WatermarkPosition;

    struct FakeRepository {
        candidate: Option<PolicyAssetCandidate>,
        snapshots: Mutex<Vec<SitePolicySnapshot>>,
    }

    impl FakeRepository {
        fn new(candidate: Option<PolicyAssetCandidate>) -> Self {
            Self {
                candidate,
                snapshots: Mutex::new(Vec::new()),
            }
        }

        fn snapshots(
            &self,
        ) -> Result<std::sync::MutexGuard<'_, Vec<SitePolicySnapshot>>, SitePolicyRepositoryError>
        {
            self.snapshots
                .lock()
                .map_err(|_| SitePolicyRepositoryError::Backend("test lock poisoned".to_owned()))
        }
    }

    #[async_trait]
    impl SitePolicyRepository for FakeRepository {
        async fn find_policy_asset(
            &self,
            tenant_id: &str,
            upload_id: UploadId,
        ) -> Result<Option<PolicyAssetCandidate>, SitePolicyRepositoryError> {
            Ok(self
                .candidate
                .clone()
                .filter(|candidate| tenant_id == "site-a" && candidate.upload_id == upload_id))
        }

        async fn publish_site_policy(
            &self,
            snapshot: &SitePolicySnapshot,
        ) -> Result<PublishPolicyOutcome, SitePolicyRepositoryError> {
            let mut snapshots = self.snapshots()?;
            if let Some(existing) = snapshots
                .iter()
                .find(|existing| existing.revision == snapshot.revision)
            {
                return if existing.settings_sha256 == snapshot.settings_sha256 {
                    Ok(PublishPolicyOutcome::Unchanged)
                } else {
                    Err(SitePolicyRepositoryError::RevisionConflict)
                };
            }
            let next = snapshots.last().map_or(1, |current| current.revision + 1);
            if snapshot.revision != next {
                return Err(SitePolicyRepositoryError::RevisionConflict);
            }
            snapshots.push(snapshot.clone());
            Ok(PublishPolicyOutcome::Published)
        }

        async fn find_active_site_policy(
            &self,
            tenant_id: &str,
        ) -> Result<Option<SitePolicySnapshot>, SitePolicyRepositoryError> {
            Ok(self
                .snapshots()?
                .iter()
                .rev()
                .find(|snapshot| snapshot.tenant_id == tenant_id)
                .cloned())
        }

        async fn find_site_policy_revision(
            &self,
            tenant_id: &str,
            revision: u64,
        ) -> Result<Option<SitePolicySnapshot>, SitePolicyRepositoryError> {
            Ok(self
                .snapshots()?
                .iter()
                .find(|snapshot| snapshot.tenant_id == tenant_id && snapshot.revision == revision)
                .cloned())
        }
    }

    #[tokio::test]
    async fn publishes_only_a_ready_tenant_image_and_pins_its_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        let now = OffsetDateTime::now_utc();
        let upload_id = UploadId::new();
        let repository = Arc::new(FakeRepository::new(Some(PolicyAssetCandidate {
            upload_id,
            object_key: ObjectKey::new(format!("raw/site-a/{upload_id}/source"))?,
            declared_kind: MediaKind::Image,
            state: UploadState::Ready,
            byte_len: 4096,
            detected_content_type: Some("image/png".to_owned()),
            source_sha256: Some("a".repeat(64)),
        })));
        let service = SitePolicyService::new(repository.clone(), 300);
        let (snapshot, outcome) = service
            .publish(
                PublishSitePolicy {
                    tenant_id: "site-a".to_owned(),
                    schema_version: SITE_POLICY_SCHEMA_VERSION,
                    revision: 1,
                    issued_at: now,
                    watermark: Some(RequestedWatermarkPolicy {
                        asset_upload_id: upload_id,
                        position: WatermarkPosition::BottomRight,
                        margin_px: 24,
                        max_width_percent: 20,
                        opacity_percent: 80,
                    }),
                },
                now,
            )
            .await?;

        assert_eq!(outcome, PublishPolicyOutcome::Published);
        assert_eq!(snapshot.settings_sha256.len(), 64);
        assert_eq!(
            snapshot
                .watermark
                .as_ref()
                .map(|watermark| watermark.asset_sha256.as_str()),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        let (_, replay) = service
            .publish(
                PublishSitePolicy {
                    tenant_id: "site-a".to_owned(),
                    schema_version: SITE_POLICY_SCHEMA_VERSION,
                    revision: 1,
                    issued_at: now,
                    watermark: Some(RequestedWatermarkPolicy {
                        asset_upload_id: upload_id,
                        position: WatermarkPosition::BottomRight,
                        margin_px: 24,
                        max_width_percent: 20,
                        opacity_percent: 80,
                    }),
                },
                now,
            )
            .await?;
        assert_eq!(replay, PublishPolicyOutcome::Unchanged);
        assert_eq!(repository.snapshots()?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_stale_or_not_ready_policy_inputs() -> Result<(), Box<dyn std::error::Error>> {
        let now = OffsetDateTime::now_utc();
        let upload_id = UploadId::new();
        let repository = Arc::new(FakeRepository::new(Some(PolicyAssetCandidate {
            upload_id,
            object_key: ObjectKey::new(format!("raw/site-a/{upload_id}/source"))?,
            declared_kind: MediaKind::Image,
            state: UploadState::Processing,
            byte_len: 4096,
            detected_content_type: Some("image/png".to_owned()),
            source_sha256: Some("b".repeat(64)),
        })));
        let service = SitePolicyService::new(repository, 300);
        let request = |issued_at| PublishSitePolicy {
            tenant_id: "site-a".to_owned(),
            schema_version: SITE_POLICY_SCHEMA_VERSION,
            revision: 1,
            issued_at,
            watermark: Some(RequestedWatermarkPolicy {
                asset_upload_id: upload_id,
                position: WatermarkPosition::Center,
                margin_px: 0,
                max_width_percent: 20,
                opacity_percent: 80,
            }),
        };
        assert!(matches!(
            service.publish(request(now), now).await,
            Err(PublishSitePolicyError::AssetRejected)
        ));
        assert!(matches!(
            service
                .publish(request(now - time::Duration::seconds(301)), now)
                .await,
            Err(PublishSitePolicyError::InvalidPolicy)
        ));
        Ok(())
    }
}
