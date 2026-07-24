//! Public listener isolation and signed immutable-thumbnail regression.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use g7mb_api::{PublicDeliveryPolicy, PublicDeliveryState, public_delivery_router};
use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest, DownloadObjectRequest,
    MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError, PresignGetRequest,
    PresignPartRequest, PresignPutRequest, PresignedDownload, PresignedUpload, PutFileRequest,
    delivery::{DerivativeDeliveryPolicy, DerivativeDeliveryService},
};
use g7mb_auth::{SignedMediaUrl, sign_media_url};
use g7mb_domain::{ObjectKey, UploadId};
use g7mb_persistence_sqlite::SqliteStore;
use secrecy::SecretString;
use time::OffsetDateTime;
use tower::ServiceExt as _;

struct DeliveryStore;

impl DeliveryStore {
    fn unsupported() -> ObjectStoreError {
        ObjectStoreError::InvalidRequest("test operation is unavailable".to_owned())
    }
}

#[async_trait]
impl ObjectStore for DeliveryStore {
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

    async fn put_file(&self, _request: PutFileRequest) -> Result<ObjectMetadata, ObjectStoreError> {
        Err(Self::unsupported())
    }

    async fn delete(&self, _key: &ObjectKey) -> Result<(), ObjectStoreError> {
        Err(Self::unsupported())
    }
}

async fn ready_thumbnail(
    database: &SqliteStore,
    upload_id: UploadId,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
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
    .bind(now)
    .bind(now)
    .execute(database.pool())
    .await?;
    for (variant, byte_len, digest) in [
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
        .bind(format!("media/site-a/{upload_id}/board-v1/{variant}.jpg"))
        .bind(byte_len)
        .bind(digest)
        .bind(now)
        .execute(database.pool())
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn public_router_exposes_only_exact_signed_thumbnail_redirects()
-> Result<(), Box<dyn std::error::Error>> {
    let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
    let upload_id = UploadId::new();
    ready_thumbnail(database.as_ref(), upload_id).await?;
    let delivery = DerivativeDeliveryService::new(
        database,
        Arc::new(DeliveryStore),
        DerivativeDeliveryPolicy {
            redirect_allowed_authorities: vec!["private-storage.invalid".to_owned()],
            ..DerivativeDeliveryPolicy::default()
        },
    )?;
    let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
    let state = PublicDeliveryState::new(
        "site-a".to_owned(),
        delivery,
        secret.clone(),
        PublicDeliveryPolicy {
            token_max_ttl: Duration::from_secs(300),
            requests_per_second: 100,
            burst: 200,
            max_in_flight: 16,
        },
    )?;
    let app = public_delivery_router(state);
    let path = format!("/media/v1/site-a/{upload_id}/board-v1/thumbnail.jpg");
    let expires_at = OffsetDateTime::now_utc().unix_timestamp() + 120;
    let signature = sign_media_url(
        &SignedMediaUrl {
            path: &path,
            expires_at,
            signature: "",
        },
        &secret,
    )?;
    let signed_uri = format!("{path}?expires={expires_at}&signature={signature}");
    let response = app
        .clone()
        .oneshot(Request::get(&signed_uri).body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("https://private-storage.invalid/"))
    );
    let head = app
        .clone()
        .oneshot(Request::head(&signed_uri).body(Body::empty())?)
        .await?;
    assert_eq!(head.status(), StatusCode::NOT_FOUND);

    for uri in [
        "/v1/capabilities".to_owned(),
        "/metrics".to_owned(),
        format!("{signed_uri}&width=99999"),
        format!(
            "/media/v1/site-a/{upload_id}/other/thumbnail.jpg?expires={expires_at}&signature={signature}"
        ),
    ] {
        let rejected = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty())?)
            .await?;
        assert_eq!(rejected.status(), StatusCode::NOT_FOUND);
    }
    Ok(())
}
