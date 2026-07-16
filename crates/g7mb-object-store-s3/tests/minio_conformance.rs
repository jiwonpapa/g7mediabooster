//! Live S3-compatible protocol conformance executed by the pinned MinIO harness.

use std::{collections::BTreeMap, env, path::Path, process::Command, time::Duration};

use aws_credential_types::Credentials;
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Region},
};
use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CompletedPart, CreateMultipartRequest,
    DownloadObjectRequest, ListObjectsRequest, ObjectStore as _, ObjectStoreError,
    PresignGetRequest, PresignPartRequest, PresignPutRequest, PutFileRequest,
};
use g7mb_config::StorageSettings;
use g7mb_domain::ObjectKey;
use g7mb_object_store_s3::{S3CompatibleStore, S3StorageAdmin};
use secrecy::{ExposeSecret as _, SecretString};

#[tokio::test]
#[ignore = "requires the repository MinIO container harness"]
async fn minio_single_multipart_abort_download_and_derivative_conformance()
-> Result<(), Box<dyn std::error::Error>> {
    let settings = settings_from_environment()?;
    let client = test_client(&settings).await;
    create_bucket(&client, &settings.raw_bucket).await?;
    create_bucket(&client, &settings.derivative_bucket).await?;
    let raw = S3CompatibleStore::for_raw_bucket(&settings).await?;
    let derivative = S3CompatibleStore::for_derivative_bucket(&settings).await?;
    let temp = tempfile::tempdir()?;

    let single_bytes = b"g7mb-presigned-single-put";
    let single_path = temp.path().join("single.bin");
    tokio::fs::write(&single_path, single_bytes).await?;
    let single_key = ObjectKey::new("raw/conformance/single/source")?;
    let signed = raw
        .presign_put(PresignPutRequest {
            key: single_key.clone(),
            content_length: u64::try_from(single_bytes.len())?,
            content_type: "application/octet-stream".to_owned(),
            expires_in: Duration::from_secs(60),
        })
        .await?;
    curl_put(
        signed.url.expose_secret(),
        &signed.required_headers,
        &single_path,
    )?;
    assert_eq!(
        raw.head(&single_key).await?.content_length,
        u64::try_from(single_bytes.len())?
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

    let multipart_key = ObjectKey::new("raw/conformance/multipart/source")?;
    let session = raw
        .create_multipart(CreateMultipartRequest {
            key: multipart_key.clone(),
            content_type: "application/octet-stream".to_owned(),
        })
        .await?;
    let first = vec![0x41_u8; 5 * 1024 * 1024];
    let second = vec![0x42_u8; 1024 * 1024];
    let mut completed = Vec::new();
    for (index, bytes) in [&first, &second].into_iter().enumerate() {
        let part_number = u16::try_from(index + 1)?;
        let path = temp.path().join(format!("part-{part_number}"));
        tokio::fs::write(&path, bytes).await?;
        let signed = raw
            .presign_part(PresignPartRequest {
                key: multipart_key.clone(),
                upload_id: session.upload_id.clone(),
                part_number,
                content_length: u64::try_from(bytes.len())?,
                expires_in: Duration::from_secs(60),
            })
            .await?;
        let etag = curl_put(signed.url.expose_secret(), &signed.required_headers, &path)?
            .ok_or_else(|| std::io::Error::other("multipart PUT returned no ETag"))?;
        completed.push(CompletedPart { part_number, etag });
    }
    raw.complete_multipart(CompleteMultipartRequest {
        key: multipart_key.clone(),
        upload_id: session.upload_id,
        parts: completed,
    })
    .await?;
    let total_length = u64::try_from(first.len() + second.len())?;
    assert_eq!(raw.head(&multipart_key).await?.content_length, total_length);
    let downloaded = temp.path().join("downloaded.bin");
    raw.download_to(DownloadObjectRequest {
        key: multipart_key.clone(),
        destination: downloaded.clone(),
        expected_length: total_length,
        max_length: total_length,
    })
    .await?;
    let downloaded_bytes = tokio::fs::read(downloaded).await?;
    assert_eq!(&downloaded_bytes[..first.len()], first.as_slice());
    assert_eq!(&downloaded_bytes[first.len()..], second.as_slice());

    let abort_key = ObjectKey::new("raw/conformance/aborted/source")?;
    let aborted = raw
        .create_multipart(CreateMultipartRequest {
            key: abort_key.clone(),
            content_type: "application/octet-stream".to_owned(),
        })
        .await?;
    let aborted_id = aborted.upload_id.expose_secret().to_owned();
    raw.abort_multipart(AbortMultipartRequest {
        key: abort_key.clone(),
        upload_id: aborted.upload_id,
    })
    .await?;
    let list_after_abort = client
        .list_parts()
        .bucket(&settings.raw_bucket)
        .key(abort_key.as_str())
        .upload_id(aborted_id)
        .send()
        .await;
    assert!(list_after_abort.is_err());

    let derivative_path = temp.path().join("thumbnail.jpg");
    tokio::fs::write(&derivative_path, b"\xff\xd8\xff\xe0conformance").await?;
    let derivative_key = ObjectKey::new("media/conformance/thumbnail.jpg")?;
    let stored = derivative
        .put_file(PutFileRequest {
            key: derivative_key.clone(),
            source: derivative_path,
            content_type: "image/jpeg".to_owned(),
        })
        .await?;
    assert_eq!(
        derivative.head(&derivative_key).await?.content_length,
        stored.content_length
    );
    let signed_get = derivative
        .presign_get(PresignGetRequest {
            key: derivative_key.clone(),
            expires_in: Duration::from_secs(60),
        })
        .await?;
    let delivered_path = temp.path().join("delivered-thumbnail.jpg");
    curl_get(signed_get.url.expose_secret(), &delivered_path)?;
    assert_eq!(
        tokio::fs::read(delivered_path).await?,
        b"\xff\xd8\xff\xe0conformance"
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
    assert!(matches!(
        derivative.head(&derivative_key).await,
        Err(ObjectStoreError::NotFound)
    ));
    // S3 DeleteObject is idempotent, which cleanup lease recovery depends on.
    derivative.delete(&derivative_key).await?;
    raw.delete(&multipart_key).await?;
    raw.delete(&single_key).await?;
    assert!(matches!(
        raw.head(&single_key).await,
        Err(ObjectStoreError::NotFound)
    ));
    assert!(matches!(
        raw.head(&multipart_key).await,
        Err(ObjectStoreError::NotFound)
    ));
    let missing_download = temp.path().join("missing.bin");
    assert!(matches!(
        raw.download_to(DownloadObjectRequest {
            key: single_key,
            destination: missing_download.clone(),
            expected_length: 1,
            max_length: 1,
        })
        .await,
        Err(ObjectStoreError::NotFound)
    ));
    assert!(!missing_download.exists());
    Ok(())
}

#[tokio::test]
#[ignore = "requires the repository MinIO container harness"]
async fn minio_admin_bucket_bootstrap_and_runtime_canary_are_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let mut settings = settings_from_environment()?;
    settings.raw_bucket.push_str("-admin");
    settings.derivative_bucket = settings.raw_bucket.clone();
    let admin = S3StorageAdmin::new(&settings);
    // MinIO's pinned S3 surface does not implement PutBucketCors. Provider CORS is
    // covered by rule unit tests and the credential-gated R2/AWS bootstrap.
    let origins = Vec::new();

    let first = admin.bootstrap(&settings, true, &origins).await?;
    assert_eq!(first.len(), 1);
    assert!(first[0].created);
    assert!(!first[0].cors_configured);
    let second = admin.bootstrap(&settings, true, &origins).await?;
    assert_eq!(second.len(), 1);
    assert!(!second[0].created);
    assert!(!second[0].cors_configured);

    let report = admin.canary(&settings).await?;
    assert_eq!(report.buckets_checked, 1);
    assert!(report.single_object);
    assert!(report.multipart);
    Ok(())
}

fn settings_from_environment() -> Result<StorageSettings, Box<dyn std::error::Error>> {
    let endpoint = env::var("G7MB_TEST_S3_ENDPOINT")?;
    let access_key = env::var("G7MB_TEST_S3_ACCESS_KEY")?;
    let secret_key = env::var("G7MB_TEST_S3_SECRET_KEY")?;
    let suffix = std::process::id();
    Ok(StorageSettings {
        endpoint_url: Some(endpoint),
        region: "us-east-1".to_owned(),
        raw_bucket: env::var("G7MB_TEST_S3_RAW_BUCKET")
            .unwrap_or_else(|_| format!("g7mb-raw-{suffix}")),
        derivative_bucket: env::var("G7MB_TEST_S3_DERIVATIVE_BUCKET")
            .unwrap_or_else(|_| format!("g7mb-media-{suffix}")),
        access_key_id: SecretString::from(access_key),
        access_key_id_file: None,
        secret_access_key: SecretString::from(secret_key),
        secret_access_key_file: None,
        force_path_style: true,
    })
}

async fn test_client(settings: &StorageSettings) -> Client {
    let credentials = Credentials::new(
        settings.access_key_id.expose_secret(),
        settings.secret_access_key.expose_secret(),
        None,
        None,
        "g7mb-minio-conformance",
    );
    let mut builder = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(settings.region.clone()))
        .credentials_provider(credentials)
        .force_path_style(true);
    if let Some(endpoint) = &settings.endpoint_url {
        builder = builder.endpoint_url(endpoint);
    }
    Client::from_conf(builder.build())
}

async fn create_bucket(client: &Client, bucket: &str) -> Result<(), Box<dyn std::error::Error>> {
    client.create_bucket().bucket(bucket).send().await?;
    Ok(())
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
