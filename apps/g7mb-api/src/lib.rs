//! HTTP control-plane router and generated OpenAPI contract.

use std::{
    collections::BTreeMap, fmt, path::Path as FilePath, process::Stdio, sync::Arc, time::Duration,
};

use anyhow::{Context as _, bail};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, OriginalUri, Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse as _, Response},
    routing::{delete, get, post},
};
use g7mb_application::{
    CompletedPart, NonceStore, NonceStoreError, WatermarkPosition,
    delivery::{DerivativeDeliveryError, DerivativeDeliveryService},
    lifecycle::{DeletionRequestError, DeletionRequestOutcome, LifecycleService},
    policies::{
        PublishPolicyOutcome, PublishSitePolicy, PublishSitePolicyError, RequestedWatermarkPolicy,
        SitePolicyService, SitePolicySnapshot,
    },
    uploads::{
        CreateUploadBatch, CreateUploadBatchError, MultipartControlError, UploadFileIntent,
        UploadIntentService,
    },
};
use g7mb_auth::{SignedRequest, sha256_hex, verify};
use g7mb_contracts::{
    CapabilitiesResponse, CompleteMultipartUploadRequest, CreateUploadBatchRequest,
    CreateUploadBatchResponse, DerivativeDeliveryResponse, ErrorResponse, HealthResponse,
    PresignUploadPartRequest, PresignUploadPartResponse, PublishSitePolicyRequest,
    SitePolicySnapshotResponse, SitePolicyWatermarkPosition, SitePolicyWatermarkResponse,
    UploadDerivativeResponse, UploadIntentResponse, UploadKind, UploadMethod, UploadStatusResponse,
    UploadStatusValue,
};
use g7mb_domain::{MediaKind, UploadId, UploadTransfer};
use metrics_exporter_prometheus::PrometheusHandle;
use secrecy::{ExposeSecret as _, SecretString};
use time::OffsetDateTime;
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use utoipa::{OpenApi, openapi::OpenApi as OpenApiDocument};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cloneable state installed in the control-plane router.
#[derive(Clone)]
pub struct ApiState {
    ready: bool,
    metrics: Option<PrometheusHandle>,
    upload_service: Option<UploadIntentService>,
    nonce_store: Option<Arc<dyn NonceStore>>,
    auth: Option<ApiAuth>,
    policy_service: Option<SitePolicyService>,
    capabilities: Option<CapabilitiesResponse>,
    lifecycle_service: Option<LifecycleService>,
    delivery_service: Option<DerivativeDeliveryService>,
}

impl ApiState {
    /// Creates router state after dependency startup checks.
    #[must_use]
    pub const fn new(ready: bool, metrics: Option<PrometheusHandle>) -> Self {
        Self {
            ready,
            metrics,
            upload_service: None,
            nonce_store: None,
            auth: None,
            policy_service: None,
            capabilities: None,
            lifecycle_service: None,
            delivery_service: None,
        }
    }

    /// Adds authenticated upload control dependencies after startup validation.
    #[must_use]
    pub fn with_upload_control(
        mut self,
        upload_service: UploadIntentService,
        nonce_store: Arc<dyn NonceStore>,
        auth: ApiAuth,
    ) -> Self {
        self.upload_service = Some(upload_service);
        self.nonce_store = Some(nonce_store);
        self.auth = Some(auth);
        self
    }

    /// Adds durable versioned site policy control after startup validation.
    #[must_use]
    pub fn with_site_policy(mut self, policy_service: SitePolicyService) -> Self {
        self.policy_service = Some(policy_service);
        self
    }

    /// Adds the native capability snapshot verified by the credential-free sandbox.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: CapabilitiesResponse) -> Self {
        self.capabilities = Some(capabilities);
        self
    }

    /// Adds durable asynchronous object deletion after startup validation.
    #[must_use]
    pub fn with_lifecycle(mut self, lifecycle_service: LifecycleService) -> Self {
        self.lifecycle_service = Some(lifecycle_service);
        self
    }

    /// Adds private derivative delivery after startup validation.
    #[must_use]
    pub fn with_derivative_delivery(mut self, delivery_service: DerivativeDeliveryService) -> Self {
        self.delivery_service = Some(delivery_service);
        self
    }
}

impl fmt::Debug for ApiState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiState")
            .field("ready", &self.ready)
            .field("metrics", &self.metrics.is_some())
            .field("upload_service", &self.upload_service.is_some())
            .field("nonce_store", &self.nonce_store.is_some())
            .field("auth", &self.auth.is_some())
            .field("policy_service", &self.policy_service.is_some())
            .field("capabilities", &self.capabilities.is_some())
            .field("lifecycle_service", &self.lifecycle_service.is_some())
            .field("delivery_service", &self.delivery_service.is_some())
            .finish()
    }
}

/// One configured PHP control client for the initial single-site deployment.
#[derive(Clone, Debug)]
pub struct ApiAuth {
    key_id: String,
    tenant_id: String,
    secret: SecretString,
    allowed_skew_seconds: i64,
}

impl ApiAuth {
    /// Creates an already validated authentication configuration.
    #[must_use]
    pub fn new(
        key_id: String,
        tenant_id: String,
        secret: SecretString,
        allowed_skew_seconds: i64,
    ) -> Self {
        Self {
            key_id,
            tenant_id,
            secret,
            allowed_skew_seconds,
        }
    }
}

/// Builds the complete HTTP router with conservative API security headers.
pub fn router(state: ApiState, body_limit_bytes: usize) -> Router {
    let request_id = HeaderName::from_static("x-request-id");
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/metrics", get(metrics))
        .route("/v1/capabilities", get(capabilities))
        .route(
            "/v1/site-policy",
            get(get_site_policy).put(publish_site_policy),
        )
        .route("/v1/upload-batches", post(create_upload_batch))
        .route(
            "/v1/uploads/{upload_id}/parts/{part_number}/presign",
            post(presign_upload_part),
        )
        .route(
            "/v1/uploads/{upload_id}/multipart/complete",
            post(complete_multipart_upload),
        )
        .route(
            "/v1/uploads/{upload_id}/multipart",
            delete(abort_multipart_upload),
        )
        .route(
            "/v1/uploads/{upload_id}/complete",
            post(confirm_single_upload),
        )
        .route(
            "/v1/uploads/{upload_id}",
            get(get_upload_status).delete(request_upload_deletion),
        )
        .route(
            "/v1/uploads/{upload_id}/derivatives/{variant}/delivery",
            get(get_derivative_delivery),
        )
        .with_state(state)
        .layer(DefaultBodyLimit::max(body_limit_bytes))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
}

/// Returns the generated OpenAPI document.
#[must_use]
pub fn openapi() -> OpenApiDocument {
    ApiDoc::openapi()
}

/// Returns deterministic pretty JSON used by the contract drift harness.
pub fn openapi_json() -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&openapi())
}

#[utoipa::path(
    get,
    path = "/health/live",
    tag = "health",
    responses((status = 200, description = "Process is alive", body = HealthResponse))
)]
async fn liveness() -> Json<HealthResponse> {
    Json(HealthResponse::new("live", VERSION))
}

#[utoipa::path(
    get,
    path = "/health/ready",
    tag = "health",
    responses(
        (status = 200, description = "Required dependencies are ready", body = HealthResponse),
        (status = 503, description = "At least one dependency is not ready", body = HealthResponse)
    )
)]
async fn readiness(State(state): State<ApiState>) -> (StatusCode, Json<HealthResponse>) {
    if state.ready {
        (StatusCode::OK, Json(HealthResponse::new("ready", VERSION)))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse::new("not_ready", VERSION)),
        )
    }
}

#[utoipa::path(
    get,
    path = "/metrics",
    tag = "internal",
    responses((status = 200, description = "Prometheus text exposition"))
)]
async fn metrics(State(state): State<ApiState>) -> (StatusCode, String) {
    match state.metrics {
        Some(handle) => (StatusCode::OK, handle.render()),
        None => (StatusCode::SERVICE_UNAVAILABLE, String::new()),
    }
}

#[utoipa::path(
    get,
    path = "/v1/capabilities",
    tag = "media",
    responses(
        (status = 200, description = "Verified native capabilities", body = CapabilitiesResponse),
        (status = 400, description = "Malformed signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 503, description = "Native capability snapshot is unavailable", body = ErrorResponse)
    )
)]
async fn capabilities(
    State(state): State<ApiState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let result = async {
        authenticate_request(&state, &headers, &[], "GET", path_and_query(&uri)).await?;
        state.capabilities.clone().ok_or_else(ApiFailure::not_ready)
    }
    .await;
    match result {
        Ok(capabilities) => (StatusCode::OK, Json(capabilities)).into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

/// Runs the credential-free sandbox capability command with bounded output and timeout.
pub async fn probe_sandbox_capabilities(
    binary: &FilePath,
    timeout: Duration,
    max_output_bytes: usize,
) -> anyhow::Result<CapabilitiesResponse> {
    use tokio::io::AsyncReadExt as _;

    if binary.as_os_str().is_empty()
        || timeout.is_zero()
        || !(1024..=1_048_576).contains(&max_output_bytes)
    {
        bail!("sandbox capability probe limits are invalid");
    }
    let mut child = tokio::process::Command::new(binary)
        .arg("capabilities")
        .env_clear()
        .env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("failed to start sandbox capability probe")?;
    let mut stdout = child
        .stdout
        .take()
        .context("sandbox capability stdout was not piped")?;
    let mut bytes = Vec::with_capacity(max_output_bytes.min(64 * 1024));
    let probe = tokio::time::timeout(timeout, async {
        let mut bounded_stdout =
            (&mut stdout).take(u64::try_from(max_output_bytes)?.saturating_add(1));
        let read = bounded_stdout.read_to_end(&mut bytes);
        let (read_result, status_result) = tokio::join!(read, child.wait());
        read_result.context("failed to read sandbox capability output")?;
        status_result.context("failed to wait for sandbox capability probe")
    })
    .await;
    let status = match probe {
        Ok(result) => result?,
        Err(_) => {
            let _kill_result = child.kill().await;
            let _wait_result = child.wait().await;
            bail!("sandbox capability probe timed out");
        }
    };
    if !status.success() {
        bail!("sandbox capability probe exited unsuccessfully");
    }
    if bytes.len() > max_output_bytes {
        bail!("sandbox capability output exceeded its byte limit");
    }
    let capabilities: CapabilitiesResponse =
        serde_json::from_slice(&bytes).context("sandbox capability JSON is invalid")?;
    if !capabilities.satisfies_v1() {
        bail!("sandbox does not satisfy the required v1 media capabilities");
    }
    Ok(capabilities)
}

#[utoipa::path(
    put,
    path = "/v1/site-policy",
    tag = "policy",
    request_body = PublishSitePolicyRequest,
    responses(
        (status = 201, description = "Next immutable policy revision published", body = SitePolicySnapshotResponse),
        (status = 200, description = "Exact revision replay was idempotent", body = SitePolicySnapshotResponse),
        (status = 400, description = "Malformed JSON or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Selected watermark upload was not found", body = ErrorResponse),
        (status = 409, description = "Policy revision conflict or nonce replay", body = ErrorResponse),
        (status = 422, description = "Policy or watermark asset violates hard bounds", body = ErrorResponse),
        (status = 503, description = "Durable policy state unavailable", body = ErrorResponse)
    )
)]
async fn publish_site_policy(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let tenant_id =
            authenticate_request(&state, &headers, &body, "PUT", "/v1/site-policy").await?;
        let service = state
            .policy_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?;
        let request = serde_json::from_slice::<PublishSitePolicyRequest>(&body)
            .map_err(|_| ApiFailure::invalid_json())?;
        let issued_at = OffsetDateTime::from_unix_timestamp(request.issued_at)
            .map_err(|_| ApiFailure::invalid_json())?;
        let watermark = request
            .watermark
            .map(|watermark| {
                Ok(RequestedWatermarkPolicy {
                    asset_upload_id: watermark
                        .asset_upload_id
                        .to_string()
                        .parse::<UploadId>()
                        .map_err(|_| ApiFailure::invalid_json())?,
                    position: policy_position(watermark.position),
                    margin_px: watermark.margin_px,
                    max_width_percent: watermark.max_width_percent,
                    opacity_percent: watermark.opacity_percent,
                })
            })
            .transpose()?;
        service
            .publish(
                PublishSitePolicy {
                    tenant_id,
                    schema_version: request.schema_version,
                    revision: request.revision,
                    issued_at,
                    watermark,
                },
                OffsetDateTime::now_utc(),
            )
            .await
            .map_err(ApiFailure::from_site_policy)
    }
    .await;
    match result {
        Ok((snapshot, outcome)) => {
            let status = match outcome {
                PublishPolicyOutcome::Published => StatusCode::CREATED,
                PublishPolicyOutcome::Unchanged => StatusCode::OK,
            };
            (status, Json(site_policy_response(snapshot))).into_response()
        }
        Err(failure) => failure.into_response(&headers),
    }
}

#[utoipa::path(
    get,
    path = "/v1/site-policy",
    tag = "policy",
    responses(
        (status = 200, description = "Active immutable policy revision", body = SitePolicySnapshotResponse),
        (status = 400, description = "Malformed signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "No policy has been published", body = ErrorResponse),
        (status = 409, description = "Request nonce replay", body = ErrorResponse),
        (status = 503, description = "Durable policy state unavailable", body = ErrorResponse)
    )
)]
async fn get_site_policy(
    State(state): State<ApiState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let tenant_id =
            authenticate_request(&state, &headers, &[], "GET", path_and_query(&uri)).await?;
        let service = state
            .policy_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?;
        service
            .active(&tenant_id)
            .await
            .map_err(ApiFailure::from_site_policy)?
            .ok_or_else(ApiFailure::site_policy_not_found)
    }
    .await;
    match result {
        Ok(snapshot) => (StatusCode::OK, Json(site_policy_response(snapshot))).into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

const fn policy_position(position: SitePolicyWatermarkPosition) -> WatermarkPosition {
    match position {
        SitePolicyWatermarkPosition::Center => WatermarkPosition::Center,
        SitePolicyWatermarkPosition::TopLeft => WatermarkPosition::TopLeft,
        SitePolicyWatermarkPosition::TopRight => WatermarkPosition::TopRight,
        SitePolicyWatermarkPosition::BottomLeft => WatermarkPosition::BottomLeft,
        SitePolicyWatermarkPosition::BottomRight => WatermarkPosition::BottomRight,
    }
}

const fn response_policy_position(position: WatermarkPosition) -> SitePolicyWatermarkPosition {
    match position {
        WatermarkPosition::Center => SitePolicyWatermarkPosition::Center,
        WatermarkPosition::TopLeft => SitePolicyWatermarkPosition::TopLeft,
        WatermarkPosition::TopRight => SitePolicyWatermarkPosition::TopRight,
        WatermarkPosition::BottomLeft => SitePolicyWatermarkPosition::BottomLeft,
        WatermarkPosition::BottomRight => SitePolicyWatermarkPosition::BottomRight,
    }
}

fn site_policy_response(snapshot: SitePolicySnapshot) -> SitePolicySnapshotResponse {
    SitePolicySnapshotResponse {
        schema_version: snapshot.schema_version,
        revision: snapshot.revision,
        issued_at: snapshot.issued_at.unix_timestamp(),
        settings_sha256: snapshot.settings_sha256,
        watermark: snapshot
            .watermark
            .map(|watermark| SitePolicyWatermarkResponse {
                asset_upload_id: watermark.asset_upload_id.as_uuid(),
                asset_sha256: watermark.asset_sha256,
                position: response_policy_position(watermark.position),
                margin_px: watermark.margin_px,
                max_width_percent: watermark.max_width_percent,
                opacity_percent: watermark.opacity_percent,
            }),
    }
}

#[utoipa::path(
    post,
    path = "/v1/upload-batches",
    tag = "uploads",
    request_body = CreateUploadBatchRequest,
    responses(
        (status = 201, description = "Bounded direct-upload batch created", body = CreateUploadBatchResponse),
        (status = 400, description = "Malformed JSON or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 409, description = "Request nonce replay", body = ErrorResponse),
        (status = 429, description = "Active upload capacity exhausted", body = ErrorResponse),
        (status = 422, description = "Upload policy rejected the batch", body = ErrorResponse),
        (status = 503, description = "Storage or durable state unavailable", body = ErrorResponse)
    )
)]
async fn create_upload_batch(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match create_upload_batch_inner(&state, &headers, &body).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

async fn create_upload_batch_inner(
    state: &ApiState,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<CreateUploadBatchResponse, ApiFailure> {
    let tenant_id =
        authenticate_request(state, headers, body, "POST", "/v1/upload-batches").await?;
    let upload_service = state
        .upload_service
        .as_ref()
        .ok_or_else(ApiFailure::not_ready)?;

    let request = serde_json::from_slice::<CreateUploadBatchRequest>(body)
        .map_err(|_| ApiFailure::invalid_json())?;
    let files = request
        .files
        .into_iter()
        .map(|file| UploadFileIntent {
            client_ref: file.client_ref,
            declared_kind: match file.declared_kind {
                UploadKind::Image => MediaKind::Image,
                UploadKind::Video => MediaKind::Video,
            },
            content_length: file.content_length,
            content_type_hint: file.content_type_hint,
        })
        .collect();
    let created = upload_service
        .create_batch(CreateUploadBatch { tenant_id, files })
        .await
        .map_err(ApiFailure::from_batch)?;
    let uploads = created
        .uploads
        .into_iter()
        .map(|upload| {
            let (method, part_size_bytes) = match upload.transfer {
                UploadTransfer::SinglePut => (UploadMethod::SinglePut, None),
                UploadTransfer::Multipart { part_size_bytes } => {
                    (UploadMethod::Multipart, Some(part_size_bytes))
                }
            };
            let (upload_url, required_headers) = match upload.presigned_put {
                Some(presigned) => (
                    Some(presigned.url.expose_secret().to_owned()),
                    presigned.required_headers,
                ),
                None => (None, BTreeMap::new()),
            };
            UploadIntentResponse {
                client_ref: upload.client_ref,
                upload_id: upload.upload_id.as_uuid(),
                method,
                part_size_bytes,
                upload_url,
                required_headers,
                expires_at: upload.expires_at,
            }
        })
        .collect();
    Ok(CreateUploadBatchResponse {
        batch_id: created.batch_id.as_uuid(),
        uploads,
    })
}

#[utoipa::path(
    post,
    path = "/v1/uploads/{upload_id}/parts/{part_number}/presign",
    tag = "uploads",
    params(
        ("upload_id" = uuid::Uuid, Path, description = "Upload reservation identifier"),
        ("part_number" = u16, Path, minimum = 1, maximum = 10000)
    ),
    request_body = PresignUploadPartRequest,
    responses(
        (status = 200, description = "Exact multipart part URL created", body = PresignUploadPartResponse),
        (status = 400, description = "Malformed path, JSON, or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Upload not found", body = ErrorResponse),
        (status = 409, description = "Upload state conflict or nonce replay", body = ErrorResponse),
        (status = 422, description = "Part layout rejected", body = ErrorResponse),
        (status = 503, description = "Storage or durable state unavailable", body = ErrorResponse)
    )
)]
async fn presign_upload_part(
    State(state): State<ApiState>,
    Path((upload_id, part_number)): Path<(String, u16)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| ApiFailure::invalid_path())?;
        let tenant_id =
            authenticate_request(&state, &headers, &body, "POST", path_and_query(&uri)).await?;
        let request = serde_json::from_slice::<PresignUploadPartRequest>(&body)
            .map_err(|_| ApiFailure::invalid_json())?;
        let service = state
            .upload_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?;
        let presigned = service
            .presign_part(&tenant_id, upload_id, part_number, request.content_length)
            .await
            .map_err(ApiFailure::from_multipart)?;
        Ok::<_, ApiFailure>(PresignUploadPartResponse {
            part_number,
            upload_url: presigned.url.expose_secret().to_owned(),
            required_headers: presigned.required_headers,
            expires_at: presigned.expires_at,
        })
    }
    .await;
    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

#[utoipa::path(
    post,
    path = "/v1/uploads/{upload_id}/multipart/complete",
    tag = "uploads",
    params(("upload_id" = uuid::Uuid, Path, description = "Upload reservation identifier")),
    request_body = CompleteMultipartUploadRequest,
    responses(
        (status = 204, description = "Multipart object completed and byte length verified"),
        (status = 400, description = "Malformed path, JSON, or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Upload not found", body = ErrorResponse),
        (status = 409, description = "Upload state conflict or nonce replay", body = ErrorResponse),
        (status = 422, description = "Completion layout rejected", body = ErrorResponse),
        (status = 503, description = "Storage or durable state unavailable", body = ErrorResponse)
    )
)]
async fn complete_multipart_upload(
    State(state): State<ApiState>,
    Path(upload_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| ApiFailure::invalid_path())?;
        let tenant_id =
            authenticate_request(&state, &headers, &body, "POST", path_and_query(&uri)).await?;
        let request = serde_json::from_slice::<CompleteMultipartUploadRequest>(&body)
            .map_err(|_| ApiFailure::invalid_json())?;
        let parts = request
            .parts
            .into_iter()
            .map(|part| CompletedPart {
                part_number: part.part_number,
                etag: part.etag,
            })
            .collect();
        state
            .upload_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?
            .complete_multipart(&tenant_id, upload_id, parts)
            .await
            .map_err(ApiFailure::from_multipart)
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/uploads/{upload_id}/multipart",
    tag = "uploads",
    params(("upload_id" = uuid::Uuid, Path, description = "Upload reservation identifier")),
    responses(
        (status = 204, description = "Incomplete multipart upload aborted"),
        (status = 400, description = "Malformed path or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Upload not found", body = ErrorResponse),
        (status = 409, description = "Upload state conflict or nonce replay", body = ErrorResponse),
        (status = 503, description = "Storage or durable state unavailable", body = ErrorResponse)
    )
)]
async fn abort_multipart_upload(
    State(state): State<ApiState>,
    Path(upload_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| ApiFailure::invalid_path())?;
        let tenant_id =
            authenticate_request(&state, &headers, &body, "DELETE", path_and_query(&uri)).await?;
        state
            .upload_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?
            .abort_multipart(&tenant_id, upload_id)
            .await
            .map_err(ApiFailure::from_multipart)
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

#[utoipa::path(
    post,
    path = "/v1/uploads/{upload_id}/complete",
    tag = "uploads",
    params(("upload_id" = uuid::Uuid, Path, description = "Upload reservation identifier")),
    responses(
        (status = 204, description = "Single PUT object verified and validation queued"),
        (status = 400, description = "Malformed path or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Upload not found", body = ErrorResponse),
        (status = 409, description = "Upload state, stored size, or nonce conflict", body = ErrorResponse),
        (status = 503, description = "Storage or durable state unavailable", body = ErrorResponse)
    )
)]
async fn confirm_single_upload(
    State(state): State<ApiState>,
    Path(upload_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| ApiFailure::invalid_path())?;
        let tenant_id =
            authenticate_request(&state, &headers, &body, "POST", path_and_query(&uri)).await?;
        state
            .upload_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?
            .confirm_single_upload(&tenant_id, upload_id)
            .await
            .map_err(ApiFailure::from_multipart)
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

#[utoipa::path(
    get,
    path = "/v1/uploads/{upload_id}",
    tag = "uploads",
    params(("upload_id" = uuid::Uuid, Path, description = "Upload reservation identifier")),
    responses(
        (status = 200, description = "Tenant-scoped upload and derivative status", body = UploadStatusResponse),
        (status = 400, description = "Malformed path or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Upload not found", body = ErrorResponse),
        (status = 409, description = "Request nonce replay", body = ErrorResponse),
        (status = 503, description = "Durable state unavailable", body = ErrorResponse)
    )
)]
async fn get_upload_status(
    State(state): State<ApiState>,
    Path(upload_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| ApiFailure::invalid_path())?;
        let tenant_id =
            authenticate_request(&state, &headers, &body, "GET", path_and_query(&uri)).await?;
        let status = state
            .upload_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?
            .status(&tenant_id, upload_id)
            .await
            .map_err(ApiFailure::from_multipart)?;
        Ok::<_, ApiFailure>(UploadStatusResponse {
            upload_id: status.upload_id.as_uuid(),
            state: upload_status_value(status.state),
            detected_content_type: status.detected_content_type,
            error_code: status.error_code,
            deletion_pending: status.deletion_pending,
            derivatives: status
                .derivatives
                .into_iter()
                .map(|derivative| UploadDerivativeResponse {
                    preset_id: derivative.preset_id,
                    variant: derivative.variant,
                    url_path: format!("/{}", derivative.object_key.as_str()),
                    content_type: derivative.content_type,
                    byte_len: derivative.byte_len,
                })
                .collect(),
        })
    }
    .await;
    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

#[utoipa::path(
    get,
    path = "/v1/uploads/{upload_id}/derivatives/{variant}/delivery",
    tag = "uploads",
    params(
        ("upload_id" = uuid::Uuid, Path, description = "Upload reservation identifier"),
        ("variant" = String, Path, description = "Server-published master or thumbnail variant")
    ),
    responses(
        (status = 200, description = "Short-lived tenant-authorized private derivative URL", body = DerivativeDeliveryResponse),
        (status = 400, description = "Malformed path or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Upload or derivative not found", body = ErrorResponse),
        (status = 409, description = "Upload is not Ready or deletion is pending", body = ErrorResponse),
        (status = 503, description = "Storage or durable state unavailable", body = ErrorResponse)
    )
)]
async fn get_derivative_delivery(
    State(state): State<ApiState>,
    Path((upload_id, variant)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| ApiFailure::invalid_path())?;
        let tenant_id =
            authenticate_request(&state, &headers, &body, "GET", path_and_query(&uri)).await?;
        let delivery = state
            .delivery_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?
            .presign(&tenant_id, upload_id, &variant)
            .await
            .map_err(ApiFailure::from_delivery)?;
        Ok::<_, ApiFailure>(DerivativeDeliveryResponse {
            upload_id: upload_id.as_uuid(),
            preset_id: delivery.preset_id,
            variant: delivery.variant,
            delivery_url: delivery.url.expose_secret().to_owned(),
            expires_at: delivery.expires_at,
            content_type: delivery.content_type,
            byte_len: delivery.byte_len,
        })
    }
    .await;
    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/uploads/{upload_id}",
    tag = "uploads",
    params(("upload_id" = uuid::Uuid, Path, description = "Upload reservation identifier")),
    responses(
        (status = 202, description = "Durable object cleanup was accepted or already pending"),
        (status = 204, description = "Upload was already fully deleted"),
        (status = 400, description = "Malformed path or signing headers", body = ErrorResponse),
        (status = 401, description = "HMAC authentication failed", body = ErrorResponse),
        (status = 404, description = "Upload not found", body = ErrorResponse),
        (status = 409, description = "Upload is processing or pinned by a site policy", body = ErrorResponse),
        (status = 503, description = "Lifecycle state unavailable", body = ErrorResponse)
    )
)]
async fn request_upload_deletion(
    State(state): State<ApiState>,
    Path(upload_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| ApiFailure::invalid_path())?;
        let tenant_id =
            authenticate_request(&state, &headers, &body, "DELETE", path_and_query(&uri)).await?;
        let outcome = state
            .lifecycle_service
            .as_ref()
            .ok_or_else(ApiFailure::not_ready)?
            .request_deletion(&tenant_id, upload_id)
            .await
            .map_err(ApiFailure::from_deletion)?;
        if let Some(delivery) = &state.delivery_service {
            delivery.invalidate_upload(&tenant_id, upload_id).await;
        }
        Ok::<_, ApiFailure>(outcome)
    }
    .await;
    match result {
        Ok(DeletionRequestOutcome::Accepted | DeletionRequestOutcome::AlreadyPending) => {
            StatusCode::ACCEPTED.into_response()
        }
        Ok(DeletionRequestOutcome::AlreadyDeleted) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => failure.into_response(&headers),
    }
}

const fn upload_status_value(state: g7mb_domain::UploadState) -> UploadStatusValue {
    match state {
        g7mb_domain::UploadState::Created => UploadStatusValue::Created,
        g7mb_domain::UploadState::Uploaded => UploadStatusValue::Uploaded,
        g7mb_domain::UploadState::Quarantined => UploadStatusValue::Quarantined,
        g7mb_domain::UploadState::Processing => UploadStatusValue::Processing,
        g7mb_domain::UploadState::Ready => UploadStatusValue::Ready,
        g7mb_domain::UploadState::Rejected => UploadStatusValue::Rejected,
        g7mb_domain::UploadState::Failed => UploadStatusValue::Failed,
        g7mb_domain::UploadState::Deleted => UploadStatusValue::Deleted,
    }
}

async fn authenticate_request(
    state: &ApiState,
    headers: &HeaderMap,
    body: &[u8],
    method: &str,
    path_and_query: &str,
) -> Result<String, ApiFailure> {
    let auth = state.auth.as_ref().ok_or_else(ApiFailure::not_ready)?;
    let nonce_store = state
        .nonce_store
        .as_ref()
        .ok_or_else(ApiFailure::not_ready)?;
    let key_id = required_header(headers, "x-g7mb-key-id")?;
    if key_id != auth.key_id {
        return Err(ApiFailure::unauthorized());
    }
    let timestamp = required_header(headers, "x-g7mb-timestamp")?
        .parse::<i64>()
        .map_err(|_| ApiFailure::invalid_auth_field())?;
    let nonce = required_header(headers, "x-g7mb-nonce")?;
    let body_sha256 = required_header(headers, "x-g7mb-content-sha256")?;
    let signature = required_header(headers, "x-g7mb-signature")?;
    if body_sha256 != sha256_hex(body) {
        return Err(ApiFailure::unauthorized());
    }
    verify(
        &SignedRequest {
            key_id,
            timestamp,
            nonce,
            method,
            path_and_query,
            body_sha256,
            signature,
        },
        &auth.secret,
        OffsetDateTime::now_utc().unix_timestamp(),
        auth.allowed_skew_seconds,
    )
    .map_err(|_| ApiFailure::unauthorized())?;
    let now = OffsetDateTime::now_utc();
    nonce_store
        .consume(
            key_id,
            nonce,
            now,
            now + time::Duration::seconds(auth.allowed_skew_seconds),
        )
        .await
        .map_err(ApiFailure::from_nonce)?;
    Ok(auth.tenant_id.clone())
}

fn path_and_query(uri: &axum::http::Uri) -> &str {
    uri.path_and_query()
        .map_or_else(|| uri.path(), axum::http::uri::PathAndQuery::as_str)
}

struct ApiFailure {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ApiFailure {
    fn into_response(self, headers: &HeaderMap) -> Response {
        let request_id = headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        (
            self.status,
            Json(ErrorResponse {
                code: self.code.to_owned(),
                message: self.message.to_owned(),
                request_id,
            }),
        )
            .into_response()
    }

    const fn not_ready() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "UPLOAD_CONTROL_NOT_READY",
            message: "Upload control is not ready.",
        }
    }

    const fn invalid_auth_field() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_AUTH_FIELD",
            message: "A signing header is malformed.",
        }
    }

    const fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "AUTHENTICATION_FAILED",
            message: "Request authentication failed.",
        }
    }

    const fn invalid_json() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_JSON",
            message: "Request JSON is invalid.",
        }
    }

    const fn invalid_path() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_PATH",
            message: "A path parameter is malformed.",
        }
    }

    const fn site_policy_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "SITE_POLICY_NOT_FOUND",
            message: "No site policy has been published.",
        }
    }

    fn from_nonce(error: NonceStoreError) -> Self {
        match error {
            NonceStoreError::Replay => Self {
                status: StatusCode::CONFLICT,
                code: "NONCE_REPLAY",
                message: "Request nonce was already used.",
            },
            NonceStoreError::Backend(_) => Self::not_ready(),
        }
    }

    fn from_batch(error: CreateUploadBatchError) -> Self {
        match error {
            CreateUploadBatchError::InvalidTenant | CreateUploadBatchError::GeneratedObjectKey => {
                Self::not_ready()
            }
            CreateUploadBatchError::InvalidClientRef
            | CreateUploadBatchError::InvalidContentType
            | CreateUploadBatchError::Policy(_) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: "UPLOAD_POLICY_REJECTED",
                message: "Upload batch violates server policy.",
            },
            CreateUploadBatchError::Backpressure => Self {
                status: StatusCode::TOO_MANY_REQUESTS,
                code: "UPLOAD_CAPACITY_EXHAUSTED",
                message: "Active upload capacity is exhausted. Retry later.",
            },
            CreateUploadBatchError::ObjectStore(_)
            | CreateUploadBatchError::Repository(_)
            | CreateUploadBatchError::CleanupRequired => Self::not_ready(),
        }
    }

    fn from_site_policy(error: PublishSitePolicyError) -> Self {
        match error {
            PublishSitePolicyError::InvalidTenant | PublishSitePolicyError::Repository => {
                Self::not_ready()
            }
            PublishSitePolicyError::InvalidPolicy => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: "SITE_POLICY_REJECTED",
                message: "Site policy violates server hard limits.",
            },
            PublishSitePolicyError::AssetNotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "POLICY_ASSET_NOT_FOUND",
                message: "Selected watermark upload was not found.",
            },
            PublishSitePolicyError::AssetRejected => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: "POLICY_ASSET_REJECTED",
                message: "Selected watermark upload is not a validated supported image.",
            },
            PublishSitePolicyError::RevisionConflict => Self {
                status: StatusCode::CONFLICT,
                code: "SITE_POLICY_REVISION_CONFLICT",
                message: "Site policy revision conflicts with durable state.",
            },
        }
    }

    fn from_multipart(error: MultipartControlError) -> Self {
        match error {
            MultipartControlError::InvalidTenant => Self::not_ready(),
            MultipartControlError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "UPLOAD_NOT_FOUND",
                message: "Upload reservation was not found.",
            },
            MultipartControlError::InvalidState => Self {
                status: StatusCode::CONFLICT,
                code: "UPLOAD_STATE_CONFLICT",
                message: "Upload is not in the required state.",
            },
            MultipartControlError::InvalidPart | MultipartControlError::InvalidCompletion => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: "MULTIPART_LAYOUT_REJECTED",
                message: "Multipart data does not match the reserved layout.",
            },
            MultipartControlError::StoredSizeMismatch => Self {
                status: StatusCode::CONFLICT,
                code: "STORED_SIZE_MISMATCH",
                message: "Stored object size does not match the reservation.",
            },
            MultipartControlError::ObjectStore(_) | MultipartControlError::Repository(_) => {
                Self::not_ready()
            }
        }
    }

    fn from_delivery(error: DerivativeDeliveryError) -> Self {
        match error {
            DerivativeDeliveryError::InvalidTenant
            | DerivativeDeliveryError::InvalidPolicy
            | DerivativeDeliveryError::ObjectStore(_)
            | DerivativeDeliveryError::Repository(_) => Self::not_ready(),
            DerivativeDeliveryError::InvalidVariant => Self::invalid_path(),
            DerivativeDeliveryError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "DERIVATIVE_NOT_FOUND",
                message: "Requested derivative was not found.",
            },
            DerivativeDeliveryError::NotReady => Self {
                status: StatusCode::CONFLICT,
                code: "DERIVATIVE_NOT_READY",
                message: "Requested derivative is not deliverable.",
            },
        }
    }

    fn from_deletion(error: DeletionRequestError) -> Self {
        match error {
            DeletionRequestError::InvalidTenant | DeletionRequestError::Backend(_) => {
                Self::not_ready()
            }
            DeletionRequestError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "UPLOAD_NOT_FOUND",
                message: "Upload reservation was not found.",
            },
            DeletionRequestError::StateConflict => Self {
                status: StatusCode::CONFLICT,
                code: "UPLOAD_DELETE_STATE_CONFLICT",
                message: "Upload cannot be deleted while processing is active.",
            },
            DeletionRequestError::ReferencedByPolicy => Self {
                status: StatusCode::CONFLICT,
                code: "UPLOAD_REFERENCED_BY_POLICY",
                message: "Upload is pinned by a site policy revision.",
            },
        }
    }
}

fn required_header<'a>(headers: &'a HeaderMap, name: &'static str) -> Result<&'a str, ApiFailure> {
    headers
        .get(name)
        .ok_or_else(ApiFailure::invalid_auth_field)?
        .to_str()
        .map_err(|_| ApiFailure::invalid_auth_field())
}

#[derive(OpenApi)]
#[openapi(
    paths(
        liveness,
        readiness,
        metrics,
        capabilities,
        publish_site_policy,
        get_site_policy,
        create_upload_batch,
        presign_upload_part,
        complete_multipart_upload,
        abort_multipart_upload,
        confirm_single_upload,
        get_upload_status,
        get_derivative_delivery,
        request_upload_deletion
    ),
    components(schemas(
        HealthResponse,
        CapabilitiesResponse,
        PublishSitePolicyRequest,
        SitePolicySnapshotResponse,
        g7mb_contracts::SitePolicyWatermarkRequest,
        SitePolicyWatermarkResponse,
        SitePolicyWatermarkPosition,
        ErrorResponse,
        CreateUploadBatchRequest,
        CreateUploadBatchResponse,
        UploadIntentResponse,
        UploadKind,
        UploadMethod,
        PresignUploadPartRequest,
        PresignUploadPartResponse,
        CompleteMultipartUploadRequest,
        g7mb_contracts::CompletedUploadPart,
        UploadStatusResponse,
        UploadStatusValue,
        UploadDerivativeResponse,
        DerivativeDeliveryResponse
    )),
    tags(
        (name = "health", description = "Liveness and dependency readiness"),
        (name = "internal", description = "Private operational endpoints"),
        (name = "media", description = "Verified runtime media capabilities"),
        (name = "policy", description = "Authenticated immutable tenant policy snapshots"),
        (name = "uploads", description = "Authenticated bounded direct uploads")
    )
)]
struct ApiDoc;

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use g7mb_application::{
        AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest,
        DownloadObjectRequest, MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError,
        PresignGetRequest, PresignPartRequest, PresignPutRequest, PresignedDownload,
        PresignedUpload, PutFileRequest,
        delivery::{DerivativeDeliveryPolicy, DerivativeDeliveryService},
        lifecycle::{LifecyclePolicy, LifecycleService},
        policies::SitePolicyService,
        uploads::{UploadCapacityPolicy, UploadIntentService},
    };
    use g7mb_auth::{SignedRequest, sha256_hex, sign};
    use g7mb_contracts::{
        CapabilitiesResponse, CreateUploadBatchResponse, DerivativeDeliveryResponse,
        SitePolicySnapshotResponse, UploadMethod, UploadStatusResponse, UploadStatusValue,
    };
    use g7mb_domain::{ObjectKey, UploadBatchPolicy, UploadId};
    use g7mb_persistence_sqlite::SqliteStore;
    use http_body_util::BodyExt as _;
    use secrecy::SecretString;
    use time::OffsetDateTime;
    use tower::ServiceExt as _;

    use super::{ApiAuth, ApiState, router};

    #[derive(Default)]
    struct ApiFakeStore {
        head_length: AtomicU64,
    }

    #[async_trait]
    impl ObjectStore for ApiFakeStore {
        async fn presign_put(
            &self,
            request: PresignPutRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Ok(PresignedUpload {
                url: SecretString::from(format!("https://storage.invalid/{}", request.key)),
                required_headers: BTreeMap::new(),
                expires_at: OffsetDateTime::now_utc() + request.expires_in,
            })
        }

        async fn presign_get(
            &self,
            request: PresignGetRequest,
        ) -> Result<PresignedDownload, ObjectStoreError> {
            Ok(PresignedDownload {
                url: SecretString::from(format!(
                    "https://private-storage.invalid/{}?signed=redacted",
                    request.key
                )),
                expires_at: OffsetDateTime::now_utc() + request.expires_in,
            })
        }

        async fn create_multipart(
            &self,
            _request: CreateMultipartRequest,
        ) -> Result<MultipartSession, ObjectStoreError> {
            Ok(MultipartSession {
                upload_id: SecretString::from("provider-session".to_owned()),
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
            Ok(())
        }

        async fn abort_multipart(
            &self,
            _request: AbortMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
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
                "API fake does not download".to_owned(),
            ))
        }

        async fn put_file(
            &self,
            _request: PutFileRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "API fake does not upload files".to_owned(),
            ))
        }

        async fn delete(&self, _key: &ObjectKey) -> Result<(), ObjectStoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn liveness_has_security_headers() -> Result<(), Box<dyn std::error::Error>> {
        let response = router(ApiState::new(false, None), 1024)
            .oneshot(Request::get("/health/live").body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
            Some(&header::HeaderValue::from_static("nosniff"))
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        assert!(response.headers().contains_key("x-request-id"));
        Ok(())
    }

    #[tokio::test]
    async fn readiness_is_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
        let response = router(ApiState::new(false, None), 1024)
            .oneshot(Request::get("/health/ready").body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }

    #[tokio::test]
    async fn capabilities_require_hmac_and_return_the_verified_snapshot()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let capabilities = CapabilitiesResponse {
            image_inputs: vec![
                "avif".to_owned(),
                "gif".to_owned(),
                "heif".to_owned(),
                "jpeg".to_owned(),
                "png".to_owned(),
                "webp".to_owned(),
            ],
            image_outputs: vec![
                "avif".to_owned(),
                "jpeg".to_owned(),
                "png".to_owned(),
                "webp".to_owned(),
            ],
            mp4_thumbnail: true,
            mp4_h264_fallback: true,
            native_versions: BTreeMap::from([
                ("ffmpeg".to_owned(), "fixture".to_owned()),
                ("ffprobe".to_owned(), "fixture".to_owned()),
                ("vips".to_owned(), "fixture".to_owned()),
            ]),
        };
        let state = ApiState::new(true, None)
            .with_capabilities(capabilities.clone())
            .with_upload_control(
                UploadIntentService::new(
                    Arc::new(ApiFakeStore::default()),
                    database.clone(),
                    UploadBatchPolicy::default(),
                ),
                database,
                ApiAuth::new(
                    "g7-primary".to_owned(),
                    "site-a".to_owned(),
                    secret.clone(),
                    300,
                ),
            );
        let anonymous = router(state.clone(), 1024)
            .oneshot(Request::get("/v1/capabilities").body(Body::empty())?)
            .await?;
        assert_eq!(anonymous.status(), StatusCode::BAD_REQUEST);

        let signed = router(state, 1024)
            .oneshot(signed_request(
                "GET",
                "/v1/capabilities",
                Vec::new(),
                "0123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(signed.status(), StatusCode::OK);
        let body = signed.into_body().collect().await?.to_bytes();
        assert_eq!(
            serde_json::from_slice::<CapabilitiesResponse>(&body)?,
            capabilities
        );
        Ok(())
    }

    #[tokio::test]
    async fn signed_batch_creates_parallel_instructions_and_rejects_replay()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let storage = Arc::new(ApiFakeStore::default());
        storage
            .head_length
            .store(50 * 1024 * 1024, Ordering::Relaxed);
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let state = ApiState::new(true, None).with_upload_control(
            UploadIntentService::new(storage, database.clone(), UploadBatchPolicy::default()),
            database,
            ApiAuth::new(
                "g7-primary".to_owned(),
                "site-a".to_owned(),
                secret.clone(),
                300,
            ),
        );
        let body = serde_json::to_vec(&serde_json::json!({
            "files": [
                {
                    "client_ref": "image-one",
                    "declared_kind": "image",
                    "content_length": 1024,
                    "content_type_hint": "image/jpeg"
                },
                {
                    "client_ref": "video-one",
                    "declared_kind": "video",
                    "content_length": 52428800,
                    "content_type_hint": "video/mp4"
                }
            ]
        }))?;
        let nonce = "0123456789abcdef0123456789abcdef";
        let response = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "POST",
                "/v1/upload-batches",
                body.clone(),
                nonce,
                &secret,
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let response_body = response.into_body().collect().await?.to_bytes();
        let created = serde_json::from_slice::<CreateUploadBatchResponse>(&response_body)?;
        assert_eq!(created.uploads.len(), 2);
        assert_eq!(created.uploads[0].method, UploadMethod::SinglePut);
        assert_eq!(created.uploads[1].method, UploadMethod::Multipart);

        let replay = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "POST",
                "/v1/upload-batches",
                body,
                nonce,
                &secret,
            )?)
            .await?;
        assert_eq!(replay.status(), StatusCode::CONFLICT);

        let multipart_id = created.uploads[1].upload_id;
        let part_path = format!("/v1/uploads/{multipart_id}/parts/1/presign");
        let part_body = serde_json::to_vec(&serde_json::json!({
            "content_length": 32 * 1024 * 1024_u64
        }))?;
        let part_response = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "POST",
                &part_path,
                part_body,
                "1123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(part_response.status(), StatusCode::OK);

        let complete_path = format!("/v1/uploads/{multipart_id}/multipart/complete");
        let complete_body = serde_json::to_vec(&serde_json::json!({
            "parts": [
                {"part_number": 1, "etag": "etag-one"},
                {"part_number": 2, "etag": "etag-two"}
            ]
        }))?;
        let complete = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "POST",
                &complete_path,
                complete_body.clone(),
                "2123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(complete.status(), StatusCode::NO_CONTENT);
        let idempotent_complete = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "POST",
                &complete_path,
                complete_body,
                "3123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(idempotent_complete.status(), StatusCode::NO_CONTENT);

        let status_path = format!("/v1/uploads/{multipart_id}");
        let status_response = router(state, 1024 * 1024)
            .oneshot(signed_request(
                "GET",
                &status_path,
                Vec::new(),
                "4123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(status_response.status(), StatusCode::OK);
        let status_body = status_response.into_body().collect().await?.to_bytes();
        let status = serde_json::from_slice::<UploadStatusResponse>(&status_body)?;
        assert_eq!(status.state, UploadStatusValue::Quarantined);
        assert!(status.derivatives.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn signed_batch_returns_stable_backpressure_before_more_presigns()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let storage = Arc::new(ApiFakeStore::default());
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let service =
            UploadIntentService::new(storage, database.clone(), UploadBatchPolicy::default())
                .with_capacity_policy(UploadCapacityPolicy {
                    max_active_global: 1,
                    max_active_per_tenant: 1,
                });
        let state = ApiState::new(true, None).with_upload_control(
            service,
            database,
            ApiAuth::new(
                "g7-primary".to_owned(),
                "site-a".to_owned(),
                secret.clone(),
                300,
            ),
        );
        let body = serde_json::to_vec(&serde_json::json!({
            "files": [{
                "client_ref": "image-one",
                "declared_kind": "image",
                "content_length": 1024,
                "content_type_hint": "image/jpeg"
            }]
        }))?;
        let first = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "POST",
                "/v1/upload-batches",
                body.clone(),
                "6123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(first.status(), StatusCode::CREATED);

        let limited = router(state, 1024 * 1024)
            .oneshot(signed_request(
                "POST",
                "/v1/upload-batches",
                body,
                "7123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        let limited_body = limited.into_body().collect().await?.to_bytes();
        let error = serde_json::from_slice::<g7mb_contracts::ErrorResponse>(&limited_body)?;
        assert_eq!(error.code, "UPLOAD_CAPACITY_EXHAUSTED");
        Ok(())
    }

    #[tokio::test]
    async fn signed_ready_derivative_delivery_returns_only_a_short_lived_private_url()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let upload_id = UploadId::new();
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, detected_content_type, source_sha256,
                 created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'ready', 4096, 4096, 'image/jpeg',
                     'image/jpeg', ?, ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(format!("raw/site-a/{upload_id}/source"))
        .bind("a".repeat(64))
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(database.pool())
        .await?;
        for (variant, path, bytes) in [
            (
                "master",
                format!("media/site-a/{upload_id}/preset/master.jpg"),
                2048_i64,
            ),
            (
                "thumbnail",
                format!("media/site-a/{upload_id}/preset/thumbnail.jpg"),
                512_i64,
            ),
        ] {
            sqlx::query(
                "INSERT INTO derivatives
                    (upload_id, preset_id, variant, object_key, content_type, byte_len, sha256, created_at)
                 VALUES (?, 'board-v1', ?, ?, 'image/jpeg', ?, ?, ?)",
            )
            .bind(upload_id.to_string())
            .bind(variant)
            .bind(path)
            .bind(bytes)
            .bind(if variant == "master" {
                "b".repeat(64)
            } else {
                "c".repeat(64)
            })
            .bind(now.unix_timestamp())
            .execute(database.pool())
            .await?;
        }
        let storage = Arc::new(ApiFakeStore::default());
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let state = ApiState::new(true, None)
            .with_derivative_delivery(DerivativeDeliveryService::new(
                database.clone(),
                storage.clone(),
                DerivativeDeliveryPolicy::default(),
            )?)
            .with_upload_control(
                UploadIntentService::new(storage, database.clone(), UploadBatchPolicy::default()),
                database,
                ApiAuth::new(
                    "g7-primary".to_owned(),
                    "site-a".to_owned(),
                    secret.clone(),
                    300,
                ),
            );
        let path = format!("/v1/uploads/{upload_id}/derivatives/thumbnail/delivery");
        let response = router(state.clone(), 1024)
            .oneshot(signed_request(
                "GET",
                &path,
                Vec::new(),
                "d123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let body = response.into_body().collect().await?.to_bytes();
        let delivery = serde_json::from_slice::<DerivativeDeliveryResponse>(&body)?;
        assert_eq!(delivery.upload_id, upload_id.as_uuid());
        assert_eq!(delivery.variant, "thumbnail");
        assert_eq!(delivery.content_type, "image/jpeg");
        assert_eq!(delivery.byte_len, 512);
        assert!(
            delivery
                .delivery_url
                .starts_with("https://private-storage.invalid/media/")
        );
        assert!(delivery.expires_at > OffsetDateTime::now_utc());

        let invalid_path = format!("/v1/uploads/{upload_id}/derivatives/arbitrary/delivery");
        let invalid = router(state, 1024)
            .oneshot(signed_request(
                "GET",
                &invalid_path,
                Vec::new(),
                "e123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn signed_delete_is_idempotent_and_status_hides_pending_media()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let upload_id = UploadId::new();
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, detected_content_type, source_sha256,
                 created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'ready', 4096, 4096, 'image/jpeg',
                     'image/jpeg', ?, ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(format!("raw/site-a/{upload_id}/source"))
        .bind("a".repeat(64))
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(database.pool())
        .await?;
        for (variant, bytes, digest) in [
            ("master", 2048_i64, "b".repeat(64)),
            ("thumbnail", 512_i64, "c".repeat(64)),
        ] {
            sqlx::query(
                "INSERT INTO derivatives
                    (upload_id, preset_id, variant, object_key, content_type, byte_len, sha256, created_at)
                 VALUES (?, 'board-v1', ?, ?, 'image/jpeg', ?, ?, ?)",
            )
            .bind(upload_id.to_string())
            .bind(variant)
            .bind(format!("media/site-a/{upload_id}/{variant}.jpg"))
            .bind(bytes)
            .bind(digest)
            .bind(now.unix_timestamp())
            .execute(database.pool())
            .await?;
        }
        let storage = Arc::new(ApiFakeStore::default());
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let state = ApiState::new(true, None)
            .with_upload_control(
                UploadIntentService::new(
                    storage.clone(),
                    database.clone(),
                    UploadBatchPolicy::default(),
                ),
                database.clone(),
                ApiAuth::new(
                    "g7-primary".to_owned(),
                    "site-a".to_owned(),
                    secret.clone(),
                    300,
                ),
            )
            .with_lifecycle(LifecycleService::new(
                storage.clone(),
                storage.clone(),
                database.clone(),
                LifecyclePolicy::default(),
            )?)
            .with_derivative_delivery(DerivativeDeliveryService::new(
                database,
                storage,
                DerivativeDeliveryPolicy::default(),
            )?);
        let delivery_path = format!("/v1/uploads/{upload_id}/derivatives/thumbnail/delivery");
        let deliverable = router(state.clone(), 1024)
            .oneshot(signed_request(
                "GET",
                &delivery_path,
                Vec::new(),
                "7123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(deliverable.status(), StatusCode::OK);
        let path = format!("/v1/uploads/{upload_id}");
        let accepted = router(state.clone(), 1024)
            .oneshot(signed_request(
                "DELETE",
                &path,
                Vec::new(),
                "8123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
        let idempotent = router(state.clone(), 1024)
            .oneshot(signed_request(
                "DELETE",
                &path,
                Vec::new(),
                "9123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(idempotent.status(), StatusCode::ACCEPTED);
        let status = router(state.clone(), 1024)
            .oneshot(signed_request(
                "GET",
                &path,
                Vec::new(),
                "a123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(status.status(), StatusCode::OK);
        let body = status.into_body().collect().await?.to_bytes();
        let status = serde_json::from_slice::<UploadStatusResponse>(&body)?;
        assert!(status.deletion_pending);
        assert!(status.derivatives.is_empty());
        let revoked = router(state, 1024)
            .oneshot(signed_request(
                "GET",
                &delivery_path,
                Vec::new(),
                "b123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(revoked.status(), StatusCode::CONFLICT);
        Ok(())
    }

    #[tokio::test]
    async fn signed_site_policy_pins_a_ready_asset_and_enforces_revision_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 2).await?);
        let upload_id = UploadId::new();
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, detected_content_type, source_sha256,
                 created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'ready', 4096, 4096, 'image/png',
                     'image/png', ?, ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(format!("raw/site-a/{upload_id}/source"))
        .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(database.pool())
        .await?;
        let storage = Arc::new(ApiFakeStore::default());
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let state = ApiState::new(true, None)
            .with_upload_control(
                UploadIntentService::new(storage, database.clone(), UploadBatchPolicy::default()),
                database.clone(),
                ApiAuth::new(
                    "g7-primary".to_owned(),
                    "site-a".to_owned(),
                    secret.clone(),
                    300,
                ),
            )
            .with_site_policy(SitePolicyService::new(database, 300));
        let body = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "revision": 1,
            "issued_at": now.unix_timestamp(),
            "watermark": {
                "asset_upload_id": upload_id,
                "position": "bottom_right",
                "margin_px": 24,
                "max_width_percent": 20,
                "opacity_percent": 80
            }
        }))?;
        let published = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "PUT",
                "/v1/site-policy",
                body,
                "5123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(published.status(), StatusCode::CREATED);
        let published_body = published.into_body().collect().await?.to_bytes();
        let snapshot = serde_json::from_slice::<SitePolicySnapshotResponse>(&published_body)?;
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.settings_sha256.len(), 64);
        assert_eq!(
            snapshot
                .watermark
                .as_ref()
                .map(|watermark| watermark.asset_sha256.as_str()),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );

        let active = router(state.clone(), 1024 * 1024)
            .oneshot(signed_request(
                "GET",
                "/v1/site-policy",
                Vec::new(),
                "6123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(active.status(), StatusCode::OK);

        let skipped_body = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "revision": 3,
            "issued_at": OffsetDateTime::now_utc().unix_timestamp(),
            "watermark": null
        }))?;
        let skipped = router(state, 1024 * 1024)
            .oneshot(signed_request(
                "PUT",
                "/v1/site-policy",
                skipped_body,
                "7123456789abcdef0123456789abcdef",
                &secret,
            )?)
            .await?;
        assert_eq!(skipped.status(), StatusCode::CONFLICT);
        Ok(())
    }

    fn signed_request(
        method: &str,
        path: &str,
        body: Vec<u8>,
        nonce: &str,
        secret: &SecretString,
    ) -> Result<Request<Body>, Box<dyn std::error::Error>> {
        let timestamp = OffsetDateTime::now_utc().unix_timestamp();
        let body_hash = sha256_hex(&body);
        let signature = sign(
            &SignedRequest {
                key_id: "g7-primary",
                timestamp,
                nonce,
                method,
                path_and_query: path,
                body_sha256: &body_hash,
                signature: "",
            },
            secret,
        )?;
        Ok(Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-g7mb-key-id", "g7-primary")
            .header("x-g7mb-timestamp", timestamp.to_string())
            .header("x-g7mb-nonce", nonce)
            .header("x-g7mb-content-sha256", body_hash)
            .header("x-g7mb-signature", signature)
            .body(Body::from(body))?)
    }
}
