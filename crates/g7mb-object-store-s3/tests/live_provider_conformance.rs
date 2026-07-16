//! Credential-gated AWS S3, Lightsail, and Cloudflare R2 protocol conformance.

use std::{collections::BTreeMap, env, path::Path, process::Command, sync::Arc, time::Duration};

use g7mb_application::ObjectStoreError;
use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CompletedPart, CreateMultipartRequest,
    DownloadObjectRequest, ListObjectsRequest, ObjectStore as _, PresignGetRequest,
    PresignPartRequest, PresignPutRequest, PutFileRequest,
    lifecycle::{DeletionRequestOutcome, LifecyclePolicy, LifecycleService},
};
use g7mb_config::StorageSettings;
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
    curl_put(
        signed.url.expose_secret(),
        &signed.required_headers,
        &single_path,
    )?;
    assert_eq!(raw.head(&single_key).await?.content_length, single_length);
    let downloaded_path = temp.path().join("single-downloaded.bin");
    raw.download_to(DownloadObjectRequest {
        key: single_key.clone(),
        destination: downloaded_path.clone(),
        expected_length: single_length,
        max_length: single_length,
    })
    .await?;
    assert_eq!(
        tokio::fs::read(downloaded_path).await?,
        tokio::fs::read(&single_path).await?
    );
    let raw_inventory = raw
        .list_objects(ListObjectsRequest {
            prefix: "raw/".to_owned(),
            start_after: None,
            max_keys: 1000,
        })
        .await?;
    assert!(
        raw_inventory
            .objects
            .iter()
            .any(|object| object.key == single_key.as_str())
    );
    raw.delete(&single_key).await?;
    raw.delete(&single_key).await?;

    let abort_key = ObjectKey::new(format!("raw/conformance/{run_id}/aborted"))?;
    let abort_session = raw
        .create_multipart(CreateMultipartRequest {
            key: abort_key.clone(),
            content_type: "application/octet-stream".to_owned(),
        })
        .await?;
    let abort_request = AbortMultipartRequest {
        key: abort_key,
        upload_id: abort_session.upload_id,
    };
    raw.abort_multipart(abort_request.clone()).await?;
    raw.abort_multipart(abort_request).await?;

    let multipart_key = ObjectKey::new(format!("raw/conformance/{run_id}/multipart"))?;
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
    )
    .await?;
    assert_eq!(
        raw.head(&multipart_key).await?.content_length,
        requested_bytes
    );
    raw.delete(&multipart_key).await?;

    let derivative_path = temp.path().join("thumbnail.jpg");
    tokio::fs::write(&derivative_path, b"\xff\xd8\xff\xe0g7mb-live-thumbnail").await?;
    let derivative_key = ObjectKey::new(format!("media/conformance/{run_id}/thumbnail.jpg"))?;
    derivative
        .put_file(PutFileRequest {
            key: derivative_key.clone(),
            source: derivative_path.clone(),
            content_type: "image/jpeg".to_owned(),
        })
        .await?;
    assert_eq!(
        derivative.head(&derivative_key).await?.content_length,
        tokio::fs::metadata(&derivative_path).await?.len()
    );
    let signed_get = derivative
        .presign_get(PresignGetRequest {
            key: derivative_key.clone(),
            expires_in: Duration::from_secs(300),
        })
        .await?;
    let delivered_path = temp.path().join("delivered-thumbnail.jpg");
    curl_get(signed_get.url.expose_secret(), &delivered_path)?;
    assert_eq!(
        tokio::fs::read(delivered_path).await?,
        tokio::fs::read(&derivative_path).await?
    );
    let derivative_inventory = derivative
        .list_objects(ListObjectsRequest {
            prefix: "media/".to_owned(),
            start_after: None,
            max_keys: 1000,
        })
        .await?;
    assert!(
        derivative_inventory
            .objects
            .iter()
            .any(|object| object.key == derivative_key.as_str())
    );
    derivative.delete(&derivative_key).await?;

    assert_object_missing(&raw, &single_key).await?;
    assert_object_missing(&raw, &multipart_key).await?;
    assert_object_missing(&derivative, &derivative_key).await?;

    eprintln!(
        "live-provider-conformance PASS profile={} label={label} multipart_bytes={requested_bytes} large_5gib={} multipart_reconnect={} object_count=0",
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

    fn validate(self, settings: &StorageSettings) -> Result<(), Box<dyn std::error::Error>> {
        let aws_shape = settings.endpoint_url.is_none()
            && !settings.region.is_empty()
            && settings.region != "auto"
            && !settings.force_path_style;
        let valid = match self {
            Self::R2 => {
                settings
                    .endpoint_url
                    .as_deref()
                    .is_some_and(is_canonical_r2_endpoint)
                    && settings.region == "auto"
                    && !settings.force_path_style
            }
            Self::Lightsail => aws_shape && settings.raw_bucket == settings.derivative_bucket,
            Self::AwsS3 => aws_shape,
            Self::GenericS3 => settings
                .endpoint_url
                .as_deref()
                .is_some_and(is_https_endpoint),
        };
        if !valid {
            return Err(std::io::Error::other(
                "live provider settings do not match the declared profile",
            )
            .into());
        }
        Ok(())
    }
}

fn is_canonical_r2_endpoint(endpoint: &str) -> bool {
    let Some(account_id) = endpoint
        .strip_prefix("https://")
        .and_then(|value| value.strip_suffix(".r2.cloudflarestorage.com"))
    else {
        return false;
    };
    account_id.len() == 32 && account_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_https_endpoint(endpoint: &str) -> bool {
    endpoint.strip_prefix("https://").is_some_and(|authority| {
        !authority.is_empty() && !authority.bytes().any(|byte| byte.is_ascii_whitespace())
    })
}

#[test]
fn live_provider_label_is_log_safe() {
    assert!(valid_provider_label("r2-production_1"));
    assert!(!valid_provider_label("r2\nforged-log"));
    assert!(!valid_provider_label(""));
}

#[test]
fn live_provider_profile_is_bound_to_provider_specific_settings() {
    let mut settings = StorageSettings {
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
    assert!(LiveProviderProfile::AwsS3.validate(&settings).is_ok());
    assert!(LiveProviderProfile::Lightsail.validate(&settings).is_err());

    settings.derivative_bucket = settings.raw_bucket.clone();
    assert!(LiveProviderProfile::Lightsail.validate(&settings).is_ok());
    assert!(LiveProviderProfile::GenericS3.validate(&settings).is_err());
}

async fn upload_sparse_multipart(
    settings: &StorageSettings,
    key: ObjectKey,
    directory: &Path,
    total_bytes: u64,
    part_size: u64,
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
    if result.is_err()
        && let Ok(cleanup_store) = S3CompatibleStore::for_raw_bucket(settings).await
    {
        let _cleanup_result = cleanup_store
            .abort_multipart(AbortMultipartRequest { key, upload_id })
            .await;
    }
    result
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
    Ok(headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("etag")
            .then(|| value.trim().to_owned())
    }))
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
