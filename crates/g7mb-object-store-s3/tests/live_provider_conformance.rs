//! Credential-gated AWS S3, Lightsail, and Cloudflare R2 protocol conformance.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::Path,
    process::Command,
    sync::Arc,
    time::Duration,
};

use g7mb_application::ObjectStoreError;
use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CompletedPart, CreateMultipartRequest,
    DownloadObjectRequest, ListObjectsRequest, ObjectStore as _, PresignGetRequest,
    PresignPartRequest, PresignPutRequest, PutFileRequest,
    lifecycle::{DeletionRequestOutcome, LifecyclePolicy, LifecycleService},
};
use g7mb_config::{StorageProvider, StorageSettings};
use g7mb_domain::{ObjectKey, UploadId};
use g7mb_object_store_s3::S3CompatibleStore;
use g7mb_persistence_sqlite::SqliteStore;
use secrecy::{ExposeSecret as _, SecretString};
use time::OffsetDateTime;

const FIVE_GIB: u64 = 5 * 1024 * 1024 * 1024;

#[tokio::test]
#[ignore = "requires explicit S3-compatible provider credentials and an existing bucket"]
async fn live_provider_single_multipart_and_delete_conformance()
-> Result<(), Box<dyn std::error::Error>> {
    let (profile, settings) = settings_from_environment()?;
    let label = provider_label_from_environment()?;
    let browser_origin = browser_origin_from_environment()?;
    let requested_bytes = env::var("G7MB_LIVE_S3_LARGE_BYTES")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(6 * 1024 * 1024);
    if !(5 * 1024 * 1024..=FIVE_GIB).contains(&requested_bytes) {
        return Err(std::io::Error::other(
            "G7MB_LIVE_S3_LARGE_BYTES must be between 5 MiB and 5 GiB",
        )
        .into());
    }
    let raw = S3CompatibleStore::for_raw_bucket(&settings).await?;
    let derivative = S3CompatibleStore::for_derivative_bucket(&settings).await?;
    let run_id = UploadId::new();
    let temp = tempfile::tempdir()?;
    let single_key = ObjectKey::new(format!("raw/conformance/{run_id}/single"))?;
    let abort_key = ObjectKey::new(format!("raw/conformance/{run_id}/aborted"))?;
    let multipart_key = ObjectKey::new(format!("raw/conformance/{run_id}/multipart"))?;
    let derivative_key = ObjectKey::new(format!("media/conformance/{run_id}/thumbnail.jpg"))?;
    let result: Result<bool, Box<dyn std::error::Error>> = async {
        let single_path = temp.path().join("single.bin");
        tokio::fs::write(&single_path, b"g7mb-live-provider-single-put").await?;
        let single_length = tokio::fs::metadata(&single_path).await?.len();
        let signed = raw
            .presign_put(PresignPutRequest {
                key: single_key.clone(),
                content_length: single_length,
                content_type: "application/octet-stream".to_owned(),
                expires_in: Duration::from_secs(300),
            })
            .await?;
        assert_browser_put_preflight(
            signed.url.expose_secret(),
            &signed.required_headers,
            &browser_origin,
        )?;
        curl_put(
            signed.url.expose_secret(),
            &signed.required_headers,
            &single_path,
            &browser_origin,
        )?;
        if raw.head(&single_key).await?.content_length != single_length {
            return Err(std::io::Error::other("single PUT length mismatch").into());
        }
        let downloaded_path = temp.path().join("single-downloaded.bin");
        raw.download_to(DownloadObjectRequest {
            key: single_key.clone(),
            destination: downloaded_path.clone(),
            expected_length: single_length,
            max_length: single_length,
        })
        .await?;
        if tokio::fs::read(downloaded_path).await? != tokio::fs::read(&single_path).await? {
            return Err(std::io::Error::other("single GET bytes mismatch").into());
        }
        let raw_inventory = raw
            .list_objects(ListObjectsRequest {
                prefix: "raw/".to_owned(),
                start_after: None,
                max_keys: 1000,
            })
            .await?;
        if !raw_inventory
            .objects
            .iter()
            .any(|object| object.key == single_key.as_str())
        {
            return Err(std::io::Error::other("single PUT was absent from inventory").into());
        }
        raw.delete(&single_key).await?;
        raw.delete(&single_key).await?;

        let abort_session = raw
            .create_multipart(CreateMultipartRequest {
                key: abort_key.clone(),
                content_type: "application/octet-stream".to_owned(),
            })
            .await?;
        let abort_request = AbortMultipartRequest {
            key: abort_key.clone(),
            upload_id: abort_session.upload_id,
        };
        abort_multipart_idempotently(&raw, abort_request).await?;

        let part_size = if requested_bytes >= 100 * 1024 * 1024 {
            32 * 1024 * 1024
        } else {
            5 * 1024 * 1024
        };
        let multipart_reconnected = upload_sparse_multipart(
            &settings,
            multipart_key.clone(),
            temp.path(),
            requested_bytes,
            part_size,
            &browser_origin,
        )
        .await?;
        if raw.head(&multipart_key).await?.content_length != requested_bytes {
            return Err(std::io::Error::other("multipart length mismatch").into());
        }
        raw.delete(&multipart_key).await?;

        let derivative_path = temp.path().join("thumbnail.jpg");
        tokio::fs::write(&derivative_path, b"\xff\xd8\xff\xe0g7mb-live-thumbnail").await?;
        derivative
            .put_file(PutFileRequest {
                key: derivative_key.clone(),
                source: derivative_path.clone(),
                content_type: "image/jpeg".to_owned(),
            })
            .await?;
        if derivative.head(&derivative_key).await?.content_length
            != tokio::fs::metadata(&derivative_path).await?.len()
        {
            return Err(std::io::Error::other("derivative PUT length mismatch").into());
        }
        let signed_get = derivative
            .presign_get(PresignGetRequest {
                key: derivative_key.clone(),
                expires_in: Duration::from_secs(300),
            })
            .await?;
        let delivered_path = temp.path().join("delivered-thumbnail.jpg");
        curl_get(signed_get.url.expose_secret(), &delivered_path)?;
        if tokio::fs::read(delivered_path).await? != tokio::fs::read(&derivative_path).await? {
            return Err(std::io::Error::other("derivative GET bytes mismatch").into());
        }
        let derivative_inventory = derivative
            .list_objects(ListObjectsRequest {
                prefix: "media/".to_owned(),
                start_after: None,
                max_keys: 1000,
            })
            .await?;
        if !derivative_inventory
            .objects
            .iter()
            .any(|object| object.key == derivative_key.as_str())
        {
            return Err(std::io::Error::other("derivative was absent from inventory").into());
        }
        derivative.delete(&derivative_key).await?;
        Ok(multipart_reconnected)
    }
    .await;

    let cleanup = cleanup_protocol_objects(
        &raw,
        &derivative,
        [&single_key, &abort_key, &multipart_key],
        &derivative_key,
    )
    .await;
    let multipart_reconnected = match (result, cleanup) {
        (Ok(value), Ok(())) => value,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(std::io::Error::other(format!(
                "provider conformance failed: {error}; cleanup also failed: {cleanup_error}"
            ))
            .into());
        }
    };
    assert_object_missing(&raw, &single_key).await?;
    assert_object_missing(&raw, &abort_key).await?;
    assert_object_missing(&raw, &multipart_key).await?;
    assert_object_missing(&derivative, &derivative_key).await?;

    eprintln!(
        "live-provider-conformance PASS profile={} label={label} multipart_bytes={requested_bytes} large_5gib={} multipart_reconnect={} browser_cors=1 object_count=0",
        profile.as_str(),
        requested_bytes == FIVE_GIB,
        multipart_reconnected
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit S3-compatible provider credentials and an existing bucket"]
async fn live_provider_lifecycle_retention_and_delete_conformance()
-> Result<(), Box<dyn std::error::Error>> {
    let (profile, settings) = settings_from_environment()?;
    let label = provider_label_from_environment()?;
    let raw = Arc::new(S3CompatibleStore::for_raw_bucket(&settings).await?);
    let derivative = Arc::new(S3CompatibleStore::for_derivative_bucket(&settings).await?);
    let ready_upload_id = UploadId::new();
    let rejected_upload_id = UploadId::new();
    let ready_raw_key = ObjectKey::new(format!("raw/live-retention/{ready_upload_id}/source"))?;
    let rejected_raw_key =
        ObjectKey::new(format!("raw/live-retention/{rejected_upload_id}/source"))?;
    let derivative_key = ObjectKey::new(format!(
        "media/live-retention/{ready_upload_id}/board-v1/thumbnail.jpg"
    ))?;
    let temp = tempfile::tempdir()?;
    let source_path = temp.path().join("source.bin");
    let derivative_path = temp.path().join("thumbnail.jpg");
    tokio::fs::write(&source_path, b"g7mb-live-retention-source").await?;
    tokio::fs::write(
        &derivative_path,
        b"\xff\xd8\xff\xe0g7mb-live-retention-thumbnail",
    )
    .await?;

    let result: Result<(), Box<dyn std::error::Error>> = async {
        let source_length = tokio::fs::metadata(&source_path).await?.len();
        let derivative_length = tokio::fs::metadata(&derivative_path).await?.len();
        raw.put_file(PutFileRequest {
            key: ready_raw_key.clone(),
            source: source_path.clone(),
            content_type: "application/octet-stream".to_owned(),
        })
        .await?;
        raw.put_file(PutFileRequest {
            key: rejected_raw_key.clone(),
            source: source_path.clone(),
            content_type: "application/octet-stream".to_owned(),
        })
        .await?;
        derivative
            .put_file(PutFileRequest {
                key: derivative_key.clone(),
                source: derivative_path.clone(),
                content_type: "image/jpeg".to_owned(),
            })
            .await?;

        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let now = OffsetDateTime::now_utc();
        insert_lifecycle_upload(
            &database,
            ready_upload_id,
            &ready_raw_key,
            "ready",
            source_length,
            now,
        )
        .await?;
        insert_lifecycle_upload(
            &database,
            rejected_upload_id,
            &rejected_raw_key,
            "rejected",
            source_length,
            now - time::Duration::days(8),
        )
        .await?;
        sqlx::query(
            "INSERT INTO derivatives
                (upload_id, preset_id, variant, object_key, content_type, byte_len, sha256, created_at)
             VALUES (?, 'board-v1', 'thumbnail', ?, 'image/jpeg', ?, ?, ?)",
        )
        .bind(ready_upload_id.to_string())
        .bind(derivative_key.as_str())
        .bind(i64::try_from(derivative_length)?)
        .bind("a".repeat(64))
        .bind(now.unix_timestamp())
        .execute(database.pool())
        .await?;

        let lifecycle = LifecycleService::new(
            raw.clone(),
            derivative.clone(),
            database.clone(),
            LifecyclePolicy::default(),
        )?;
        assert_eq!(
            lifecycle
                .request_deletion("live-retention", ready_upload_id)
                .await?,
            DeletionRequestOutcome::Accepted
        );
        let summary = lifecycle.run_once("live-provider-cleanup").await?;
        assert_eq!(summary.claimed, 2);
        assert_eq!(summary.completed, 2);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.dead_lettered, 0);
        let deleted = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM uploads WHERE id IN (?, ?) AND state = 'deleted'",
        )
        .bind(ready_upload_id.to_string())
        .bind(rejected_upload_id.to_string())
        .fetch_one(database.pool())
        .await?;
        assert_eq!(deleted, 2);
        assert_object_missing(&raw, &ready_raw_key).await?;
        assert_object_missing(&raw, &rejected_raw_key).await?;
        assert_object_missing(&derivative, &derivative_key).await?;
        Ok(())
    }
    .await;

    let cleanup_ready = raw.delete(&ready_raw_key).await;
    let cleanup_rejected = raw.delete(&rejected_raw_key).await;
    let cleanup_derivative = derivative.delete(&derivative_key).await;
    result?;
    cleanup_ready?;
    cleanup_rejected?;
    cleanup_derivative?;
    eprintln!(
        "live-provider-lifecycle PASS profile={} label={label} user_delete=1 retention_expired=1 tombstones=2 object_count=0",
        profile.as_str()
    );
    Ok(())
}

async fn insert_lifecycle_upload(
    database: &SqliteStore,
    upload_id: UploadId,
    object_key: &ObjectKey,
    state: &str,
    source_length: u64,
    timestamp: OffsetDateTime,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        "INSERT INTO uploads
            (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
             content_type_hint, transfer_kind, created_at, updated_at)
         VALUES (?, 'live-retention', ?, 'image', ?, ?, 'image/jpeg',
                 'single_put', ?, ?)",
    )
    .bind(upload_id.to_string())
    .bind(object_key.as_str())
    .bind(state)
    .bind(i64::try_from(source_length)?)
    .bind(timestamp.unix_timestamp())
    .bind(timestamp.unix_timestamp())
    .execute(database.pool())
    .await?;
    Ok(())
}

async fn assert_object_missing(
    store: &S3CompatibleStore,
    key: &ObjectKey,
) -> Result<(), Box<dyn std::error::Error>> {
    match store.head(key).await {
        Err(ObjectStoreError::NotFound) => Ok(()),
        Ok(_) => Err(std::io::Error::other("provider object remained after cleanup").into()),
        Err(error) => Err(error.into()),
    }
}

async fn cleanup_protocol_objects(
    raw: &S3CompatibleStore,
    derivative: &S3CompatibleStore,
    raw_keys: [&ObjectKey; 3],
    derivative_key: &ObjectKey,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut failures = Vec::new();
    for key in raw_keys {
        if let Err(error) = raw.delete(key).await {
            failures.push(format!("raw cleanup failed: {error}"));
        }
    }
    if let Err(error) = derivative.delete(derivative_key).await {
        failures.push(format!("derivative cleanup failed: {error}"));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(std::io::Error::other(failures.join("; ")).into())
    }
}

fn provider_label_from_environment() -> Result<String, Box<dyn std::error::Error>> {
    let label = env::var("G7MB_LIVE_S3_LABEL").unwrap_or_else(|_| "external".to_owned());
    if !valid_provider_label(&label) {
        return Err(std::io::Error::other("live provider label is invalid").into());
    }
    Ok(label)
}

fn valid_provider_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn browser_origin_from_environment() -> Result<String, Box<dyn std::error::Error>> {
    parse_browser_origin(&env::var("G7MB_LIVE_S3_ORIGIN")?)
}

fn parse_browser_origin(value: &str) -> Result<String, Box<dyn std::error::Error>> {
    let parsed = url::Url::parse(value)?;
    let normalized = parsed.origin().ascii_serialization();
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || normalized != value
    {
        return Err(std::io::Error::other(
            "G7MB_LIVE_S3_ORIGIN must be an exact HTTPS origin without path, query, or credentials",
        )
        .into());
    }
    Ok(normalized)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveProviderProfile {
    R2,
    Lightsail,
    AwsS3,
    GenericS3,
}

impl LiveProviderProfile {
    fn from_environment() -> Result<Self, Box<dyn std::error::Error>> {
        match env::var("G7MB_LIVE_S3_PROFILE")?.as_str() {
            "r2" => Ok(Self::R2),
            "lightsail" => Ok(Self::Lightsail),
            "aws-s3" => Ok(Self::AwsS3),
            "generic" => Ok(Self::GenericS3),
            _ => Err(std::io::Error::other(
                "live provider profile must be r2, lightsail, aws-s3, or generic",
            )
            .into()),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::R2 => "r2",
            Self::Lightsail => "lightsail",
            Self::AwsS3 => "aws-s3",
            Self::GenericS3 => "generic",
        }
    }

    const fn storage_provider(self) -> StorageProvider {
        match self {
            Self::R2 => StorageProvider::R2,
            Self::Lightsail => StorageProvider::Lightsail,
            Self::AwsS3 => StorageProvider::AwsS3,
            Self::GenericS3 => StorageProvider::Generic,
        }
    }

    fn validate(self, settings: &StorageSettings) -> Result<(), Box<dyn std::error::Error>> {
        if settings.provider != self.storage_provider()
            || settings.validate_provider_contract().is_err()
        {
            return Err(std::io::Error::other(
                "live provider settings do not match the declared profile",
            )
            .into());
        }
        Ok(())
    }
}

#[test]
fn live_provider_label_is_log_safe() {
    assert!(valid_provider_label("r2-production_1"));
    assert!(!valid_provider_label("r2\nforged-log"));
    assert!(!valid_provider_label(""));
}

#[test]
fn live_browser_origin_is_exact_and_https_only() {
    assert_eq!(
        parse_browser_origin("https://g7.example.com:8443")
            .ok()
            .as_deref(),
        Some("https://g7.example.com:8443")
    );
    assert!(parse_browser_origin("http://g7.example.com").is_err());
    assert!(parse_browser_origin("https://g7.example.com/path").is_err());
    assert!(parse_browser_origin("https://user@g7.example.com").is_err());
}

#[test]
fn live_browser_cors_header_contract_is_exact() {
    let headers = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Access-Control-Allow-Origin: https://g7.example.com\r\n",
        "Access-Control-Allow-Methods: GET, PUT, HEAD\r\n",
        "Access-Control-Allow-Headers: content-type, x-amz-meta-test\r\n",
        "Access-Control-Expose-Headers: ETag\r\n\r\n",
    );
    assert!(
        require_exact_header(
            headers,
            "access-control-allow-origin",
            "https://g7.example.com"
        )
        .is_ok()
    );
    assert!(require_header_token(headers, "access-control-allow-methods", "put").is_ok());
    assert!(require_header_token(headers, "access-control-expose-headers", "etag").is_ok());
    assert!(
        require_exact_header(
            headers,
            "access-control-allow-origin",
            "https://other.example.com"
        )
        .is_err()
    );
}

#[test]
fn live_provider_profile_is_bound_to_provider_specific_settings() {
    let mut settings = StorageSettings {
        provider: StorageProvider::R2,
        endpoint_url: Some(
            "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com".to_owned(),
        ),
        region: "auto".to_owned(),
        raw_bucket: "private-raw".to_owned(),
        derivative_bucket: "private-media".to_owned(),
        access_key_id: SecretString::from("redacted-access"),
        access_key_id_file: None,
        secret_access_key: SecretString::from("redacted-secret"),
        secret_access_key_file: None,
        force_path_style: false,
    };
    assert!(LiveProviderProfile::R2.validate(&settings).is_ok());
    assert!(LiveProviderProfile::AwsS3.validate(&settings).is_err());

    settings.endpoint_url = None;
    settings.region = "ap-northeast-2".to_owned();
    settings.provider = StorageProvider::AwsS3;
    assert!(LiveProviderProfile::AwsS3.validate(&settings).is_ok());
    assert!(LiveProviderProfile::Lightsail.validate(&settings).is_err());

    settings.derivative_bucket = settings.raw_bucket.clone();
    settings.provider = StorageProvider::Lightsail;
    assert!(LiveProviderProfile::Lightsail.validate(&settings).is_ok());
    settings.provider = StorageProvider::Generic;
    assert!(LiveProviderProfile::GenericS3.validate(&settings).is_err());
}

async fn upload_sparse_multipart(
    settings: &StorageSettings,
    key: ObjectKey,
    directory: &Path,
    total_bytes: u64,
    part_size: u64,
    browser_origin: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let part_count = total_bytes.div_ceil(part_size);
    if part_count == 0 || part_count > 10_000 {
        return Err(std::io::Error::other("multipart part count is invalid").into());
    }
    let mut store = S3CompatibleStore::for_raw_bucket(settings).await?;
    let session = store
        .create_multipart(CreateMultipartRequest {
            key: key.clone(),
            content_type: "application/octet-stream".to_owned(),
        })
        .await?;
    let upload_id = session.upload_id.clone();
    let result: Result<bool, Box<dyn std::error::Error>> = async {
        let part_path = directory.join("sparse-part.bin");
        let mut completed = Vec::with_capacity(usize::try_from(part_count)?);
        let reconnect_at = if part_count == 1 {
            0
        } else {
            (part_count / 2).max(1)
        };
        let mut reconnected = false;
        for index in 0..part_count {
            if index == reconnect_at {
                store = S3CompatibleStore::for_raw_bucket(settings).await?;
                reconnected = true;
            }
            let part_number = u16::try_from(index + 1)?;
            let offset = index * part_size;
            let content_length = (total_bytes - offset).min(part_size);
            let file = tokio::fs::File::create(&part_path).await?;
            file.set_len(content_length).await?;
            drop(file);
            let signed = store
                .presign_part(PresignPartRequest {
                    key: key.clone(),
                    upload_id: session.upload_id.clone(),
                    part_number,
                    content_length,
                    expires_in: Duration::from_secs(900),
                })
                .await?;
            let etag = curl_put(
                signed.url.expose_secret(),
                &signed.required_headers,
                &part_path,
                browser_origin,
            )?
            .ok_or_else(|| std::io::Error::other("multipart PUT returned no ETag"))?;
            completed.push(CompletedPart { part_number, etag });
        }
        store
            .complete_multipart(CompleteMultipartRequest {
                key: key.clone(),
                upload_id: session.upload_id,
                parts: completed,
            })
            .await?;
        Ok(reconnected)
    }
    .await;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let cleanup_store =
                S3CompatibleStore::for_raw_bucket(settings)
                    .await
                    .map_err(|cleanup_error| {
                        std::io::Error::other(format!(
                            "multipart failed: {error}; cleanup client failed: {cleanup_error}"
                        ))
                    })?;
            if let Err(cleanup_error) = abort_multipart_idempotently(
                &cleanup_store,
                AbortMultipartRequest { key, upload_id },
            )
            .await
            {
                return Err(std::io::Error::other(format!(
                    "multipart failed: {error}; abort cleanup failed: {cleanup_error}"
                ))
                .into());
            }
            Err(error)
        }
    }
}

async fn abort_multipart_idempotently(
    store: &S3CompatibleStore,
    request: AbortMultipartRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let first = store.abort_multipart(request.clone()).await;
    let second = store.abort_multipart(request).await;
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error.into()),
        (Err(first_error), Err(second_error)) => Err(std::io::Error::other(format!(
            "multipart abort failed twice: {first_error}; {second_error}"
        ))
        .into()),
    }
}

fn settings_from_environment()
-> Result<(LiveProviderProfile, StorageSettings), Box<dyn std::error::Error>> {
    let profile = LiveProviderProfile::from_environment()?;
    let force_path_style = match env::var("G7MB_LIVE_S3_FORCE_PATH_STYLE")
        .unwrap_or_else(|_| "false".to_owned())
        .as_str()
    {
        "true" => true,
        "false" => false,
        _ => {
            return Err(std::io::Error::other(
                "G7MB_LIVE_S3_FORCE_PATH_STYLE must be true or false",
            )
            .into());
        }
    };
    let settings = StorageSettings {
        provider: profile.storage_provider(),
        endpoint_url: env::var("G7MB_LIVE_S3_ENDPOINT")
            .ok()
            .filter(|value| !value.is_empty()),
        region: env::var("G7MB_LIVE_S3_REGION")?,
        raw_bucket: env::var("G7MB_LIVE_S3_RAW_BUCKET")?,
        derivative_bucket: env::var("G7MB_LIVE_S3_DERIVATIVE_BUCKET")?,
        access_key_id: SecretString::from(env::var("G7MB_LIVE_S3_ACCESS_KEY")?),
        access_key_id_file: None,
        secret_access_key: SecretString::from(env::var("G7MB_LIVE_S3_SECRET_KEY")?),
        secret_access_key_file: None,
        force_path_style,
    };
    profile.validate(&settings)?;
    Ok((profile, settings))
}

fn curl_put(
    signed_url: &str,
    required_headers: &BTreeMap<String, String>,
    body: &Path,
    browser_origin: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let mut command = Command::new("curl");
    command.args([
        "--fail-with-body",
        "--silent",
        "--show-error",
        "--request",
        "PUT",
        "--dump-header",
        "-",
        "--output",
        "/dev/null",
    ]);
    command
        .arg("--header")
        .arg(format!("Origin: {browser_origin}"));
    for (name, value) in required_headers {
        command.arg("--header").arg(format!("{name}: {value}"));
    }
    command
        .arg("--data-binary")
        .arg(format!("@{}", body.display()))
        .arg(signed_url);
    let output = command.output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("presigned object-store PUT failed").into());
    }
    let headers = String::from_utf8(output.stdout)?;
    require_exact_header(&headers, "access-control-allow-origin", browser_origin)?;
    require_header_token(&headers, "access-control-expose-headers", "etag")?;
    Ok(headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("etag")
            .then(|| value.trim().to_owned())
    }))
}

fn assert_browser_put_preflight(
    signed_url: &str,
    required_headers: &BTreeMap<String, String>,
    browser_origin: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let requested_headers = required_headers
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .filter(|name| !matches!(name.as_str(), "content-length" | "host" | "origin"))
        .collect::<BTreeSet<_>>();
    let mut command = Command::new("curl");
    command.args([
        "--fail-with-body",
        "--silent",
        "--show-error",
        "--request",
        "OPTIONS",
        "--dump-header",
        "-",
        "--output",
        "/dev/null",
        "--header",
        &format!("Origin: {browser_origin}"),
        "--header",
        "Access-Control-Request-Method: PUT",
    ]);
    if !requested_headers.is_empty() {
        command.arg("--header").arg(format!(
            "Access-Control-Request-Headers: {}",
            requested_headers
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    let output = command.arg(signed_url).output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("object-store browser CORS preflight failed").into());
    }
    let headers = String::from_utf8(output.stdout)?;
    require_exact_header(&headers, "access-control-allow-origin", browser_origin)?;
    require_header_token(&headers, "access-control-allow-methods", "put")?;
    for name in requested_headers {
        let wildcard = header_tokens(&headers, "access-control-allow-headers")
            .iter()
            .any(|value| value == "*");
        if !wildcard {
            require_header_token(&headers, "access-control-allow-headers", &name)?;
        }
    }
    Ok(())
}

fn require_exact_header(
    headers: &str,
    name: &str,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let matched = headers.lines().any(|line| {
        line.split_once(':').is_some_and(|(candidate, value)| {
            candidate.eq_ignore_ascii_case(name) && value.trim() == expected
        })
    });
    if !matched {
        return Err(std::io::Error::other(format!(
            "object-store CORS response omitted exact {name}"
        ))
        .into());
    }
    Ok(())
}

fn require_header_token(
    headers: &str,
    name: &str,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !header_tokens(headers, name)
        .iter()
        .any(|value| value.eq_ignore_ascii_case(expected))
    {
        return Err(std::io::Error::other(format!(
            "object-store CORS response omitted {expected} from {name}"
        ))
        .into());
    }
    Ok(())
}

fn header_tokens(headers: &str, name: &str) -> Vec<String> {
    headers
        .lines()
        .filter_map(|line| {
            let (candidate, value) = line.split_once(':')?;
            candidate.eq_ignore_ascii_case(name).then_some(value)
        })
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn curl_get(signed_url: &str, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("curl")
        .args(["--fail-with-body", "--silent", "--show-error", "--output"])
        .arg(output)
        .arg(signed_url)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("presigned object-store GET failed").into());
    }
    Ok(())
}
