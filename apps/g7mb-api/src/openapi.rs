//! Generated control-plane OpenAPI document.

use g7mb_contracts::{
    CapabilitiesResponse, CompleteMultipartUploadRequest, CreateUploadBatchRequest,
    CreateUploadBatchResponse, DerivativeDeliveryResponse, ErrorResponse, HealthResponse,
    PresignUploadPartRequest, PresignUploadPartResponse, PublishSitePolicyRequest,
    SitePolicySnapshotResponse, SitePolicyWatermarkPosition, SitePolicyWatermarkResponse,
    UploadDerivativeResponse, UploadIntentResponse, UploadKind, UploadMethod, UploadStatusResponse,
    UploadStatusValue,
};
use utoipa::{OpenApi, openapi::OpenApi as OpenApiDocument};

use crate::{
    __path_abort_multipart_upload, __path_capabilities, __path_complete_multipart_upload,
    __path_confirm_single_upload, __path_create_upload_batch, __path_get_derivative_delivery,
    __path_get_site_policy, __path_get_upload_status, __path_liveness, __path_metrics,
    __path_presign_upload_part, __path_publish_site_policy, __path_readiness,
    __path_request_upload_deletion,
};

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

pub(super) fn document() -> OpenApiDocument {
    ApiDoc::openapi()
}
