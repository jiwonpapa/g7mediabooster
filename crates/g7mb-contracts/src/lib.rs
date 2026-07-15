//! Stable HTTP data transfer objects and OpenAPI schemas.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// Health endpoint response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct HealthResponse {
    /// `live` or `ready`.
    pub status: String,
    /// Running package version.
    pub version: String,
}

impl HealthResponse {
    /// Creates a health response without borrowing process state.
    #[must_use]
    pub fn new(status: &str, version: &str) -> Self {
        Self {
            status: status.to_owned(),
            version: version.to_owned(),
        }
    }
}

/// Stable error envelope returned to API consumers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Stable machine-readable error code.
    pub code: String,
    /// User-safe explanation without backend details.
    pub message: String,
    /// Correlation identifier for support and logs.
    pub request_id: String,
}

/// Runtime media capability response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct CapabilitiesResponse {
    /// Formats the native runtime can decode in this deployment.
    pub image_inputs: Vec<String>,
    /// Formats the native runtime can encode in this deployment.
    pub image_outputs: Vec<String>,
    /// Whether FFprobe and FFmpeg passed the embedded MP4/H.264 runtime fixture.
    pub mp4_thumbnail: bool,
    /// Whether the narrow MP4/H.264 OpenH264 fallback is compiled into the sandbox.
    pub mp4_h264_fallback: bool,
    /// Sanitized version lines from the native tools checked by the sandbox.
    pub native_versions: BTreeMap<String, String>,
}

impl CapabilitiesResponse {
    /// Returns true only when every v1-required native media path was verified.
    #[must_use]
    pub fn satisfies_v1(&self) -> bool {
        const REQUIRED_INPUTS: [&str; 6] = ["jpeg", "png", "gif", "webp", "avif", "heif"];
        const REQUIRED_OUTPUTS: [&str; 4] = ["jpeg", "webp", "avif", "png"];
        REQUIRED_INPUTS
            .iter()
            .all(|required| self.image_inputs.iter().any(|format| format == required))
            && REQUIRED_OUTPUTS
                .iter()
                .all(|required| self.image_outputs.iter().any(|format| format == required))
            && self.mp4_thumbnail
            && self.mp4_h264_fallback
            && ["vips", "ffmpeg", "ffprobe"].iter().all(|tool| {
                self.native_versions
                    .get(*tool)
                    .is_some_and(|value| !value.is_empty())
            })
    }
}

/// Allowlisted watermark anchor in a signed site policy snapshot.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SitePolicyWatermarkPosition {
    /// Center of the derivative.
    Center,
    /// Upper-left safe margin.
    TopLeft,
    /// Upper-right safe margin.
    TopRight,
    /// Lower-left safe margin.
    BottomLeft,
    /// Lower-right safe margin.
    BottomRight,
}

/// Admin-selected watermark settings referencing an already validated image upload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct SitePolicyWatermarkRequest {
    /// Tenant-owned Ready image upload selected as the source asset.
    pub asset_upload_id: Uuid,
    /// Allowlisted overlay anchor.
    pub position: SitePolicyWatermarkPosition,
    /// Safe edge margin.
    #[schema(maximum = 1024)]
    pub margin_px: u32,
    /// Maximum derivative width percentage.
    #[schema(minimum = 1, maximum = 50)]
    pub max_width_percent: u8,
    /// Alpha multiplier percentage.
    #[schema(minimum = 1, maximum = 100)]
    pub opacity_percent: u8,
}

/// HMAC-authenticated next site policy revision published by the PHP module.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct PublishSitePolicyRequest {
    /// Must equal the v1 policy schema.
    pub schema_version: u16,
    /// Strictly increasing tenant revision starting at one.
    #[schema(minimum = 1)]
    pub revision: u64,
    /// Unix issuance timestamp covered by the request signature.
    pub issued_at: i64,
    /// `null` explicitly disables watermarking.
    pub watermark: Option<SitePolicyWatermarkRequest>,
}

/// Pinned watermark facts returned from durable policy state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct SitePolicyWatermarkResponse {
    /// Original admin-selected upload.
    pub asset_upload_id: Uuid,
    /// Lowercase source digest independently captured by Rust.
    pub asset_sha256: String,
    /// Allowlisted overlay anchor.
    pub position: SitePolicyWatermarkPosition,
    /// Safe edge margin.
    pub margin_px: u32,
    /// Maximum derivative width percentage.
    pub max_width_percent: u8,
    /// Alpha multiplier percentage.
    pub opacity_percent: u8,
}

/// Active immutable site policy snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct SitePolicySnapshotResponse {
    /// Stable policy schema version.
    pub schema_version: u16,
    /// Active tenant revision.
    pub revision: u64,
    /// Original signed issuance timestamp.
    pub issued_at: i64,
    /// Rust-computed normalized settings hash.
    pub settings_sha256: String,
    /// `null` means watermarking is disabled.
    pub watermark: Option<SitePolicyWatermarkResponse>,
}

/// Client-declared media class used to reserve a safe byte budget.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UploadKind {
    /// Raster image input.
    Image,
    /// Video container input.
    Video,
}

/// One file requested inside a bounded multi-upload batch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct UploadFileIntentRequest {
    /// Caller-generated opaque identifier used only to match batch responses.
    pub client_ref: String,
    /// Client-declared media class. The worker verifies the actual bytes later.
    pub declared_kind: UploadKind,
    /// Exact byte length covered by the direct-upload reservation.
    #[schema(minimum = 1)]
    pub content_length: u64,
    /// Untrusted MIME hint retained for diagnostics and signing.
    pub content_type_hint: String,
}

/// Creates direct-upload reservations for multiple files at once.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct CreateUploadBatchRequest {
    /// One to one hundred file reservations.
    #[schema(min_items = 1, max_items = 100)]
    pub files: Vec<UploadFileIntentRequest>,
}

/// Direct-upload strategy selected by server policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UploadMethod {
    /// One presigned PUT request.
    SinglePut,
    /// Resumable S3-compatible multipart upload.
    Multipart,
}

/// One file reservation returned from a batch request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct UploadIntentResponse {
    /// Echo of the request identifier.
    pub client_ref: String,
    /// Server-generated media upload identifier.
    pub upload_id: Uuid,
    /// Selected direct-upload strategy.
    pub method: UploadMethod,
    /// Planned part size for multipart uploads.
    pub part_size_bytes: Option<u64>,
    /// Presigned URL for single PUT. Multipart uploads presign parts separately.
    pub upload_url: Option<String>,
    /// Headers that must be sent exactly as signed.
    pub required_headers: std::collections::BTreeMap<String, String>,
    /// Reservation and any returned signature expiration.
    pub expires_at: OffsetDateTime,
}

/// Result of creating one bounded multi-upload batch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct CreateUploadBatchResponse {
    /// Server-generated batch identifier.
    pub batch_id: Uuid,
    /// Response instructions in request order.
    pub uploads: Vec<UploadIntentResponse>,
}

/// Request for a short-lived URL for one multipart part.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct PresignUploadPartRequest {
    /// Exact part length; only the final part may be below 5 MiB.
    #[schema(minimum = 1)]
    pub content_length: u64,
}

/// Short-lived direct upload URL for one multipart part.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct PresignUploadPartResponse {
    /// One-based part number covered by the signature.
    pub part_number: u16,
    /// Sensitive presigned URL. API logs must redact it.
    pub upload_url: String,
    /// Signed request headers.
    pub required_headers: std::collections::BTreeMap<String, String>,
    /// Signature expiration.
    pub expires_at: OffsetDateTime,
}

/// One completed multipart part supplied to the finalization call.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct CompletedUploadPart {
    /// One-based part number.
    #[schema(minimum = 1, maximum = 10000)]
    pub part_number: u16,
    /// Opaque provider ETag returned by the part upload.
    pub etag: String,
}

/// Finalizes a previously created multipart upload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct CompleteMultipartUploadRequest {
    /// Ordered, unique completed parts.
    #[schema(min_items = 1, max_items = 10000)]
    pub parts: Vec<CompletedUploadPart>,
}

/// Public upload lifecycle value returned to the authenticated PHP module.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UploadStatusValue {
    /// Reservation exists; bytes are not confirmed.
    Created,
    /// Storage bytes were confirmed.
    Uploaded,
    /// Private source awaits or passed validation.
    Quarantined,
    /// Native derivative processing is active or retryable.
    Processing,
    /// Required derivatives are published.
    Ready,
    /// Source policy rejected the media permanently.
    Rejected,
    /// Processing exhausted its failure policy.
    Failed,
    /// Source record was tombstoned.
    Deleted,
}

/// One immutable generated derivative.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct UploadDerivativeResponse {
    /// Versioned transformation preset.
    pub preset_id: String,
    /// Stable output variant.
    pub variant: String,
    /// CDN-relative immutable media path.
    pub url_path: String,
    /// Trusted output content type.
    pub content_type: String,
    /// Exact encoded output length.
    pub byte_len: u64,
}

/// Short-lived private derivative delivery returned to the trusted PHP module.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct DerivativeDeliveryResponse {
    /// Upload owning the immutable derivative.
    pub upload_id: Uuid,
    /// Versioned transformation preset.
    pub preset_id: String,
    /// Stable `master` or `thumbnail` variant.
    pub variant: String,
    /// Sensitive provider GET URL. It must never be logged.
    pub delivery_url: String,
    /// Provider signature expiration.
    pub expires_at: OffsetDateTime,
    /// Trusted encoded content type.
    pub content_type: String,
    /// Exact encoded output length.
    pub byte_len: u64,
}

/// Tenant-scoped upload and derivative status.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct UploadStatusResponse {
    /// Upload identifier.
    pub upload_id: Uuid,
    /// Current lifecycle state.
    pub state: UploadStatusValue,
    /// Trusted detected source content type after validation.
    pub detected_content_type: Option<String>,
    /// Stable rejection or failure code without backend details.
    pub error_code: Option<String>,
    /// Whether irreversible object cleanup is durably queued.
    pub deletion_pending: bool,
    /// Immutable derivatives available to the application.
    pub derivatives: Vec<UploadDerivativeResponse>,
}
