//! Credential-gated AWS S3, Lightsail, and Cloudflare R2 protocol conformance.

use std::{collections::BTreeMap, env, path::Path, process::Command, time::Duration};

use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CompletedPart, CreateMultipartRequest,
    DownloadObjectRequest, ListObjectsRequest, ObjectStore as _, PresignGetRequest,
    PresignPartRequest, PresignPutRequest, PutFileRequest,
};
use g7mb_config::StorageSettings;
use g7mb_domain::{ObjectKey, UploadId};
use g7mb_object_store_s3::S3CompatibleStore;
use secrecy::{ExposeSecret as _, SecretString};

const FIVE_GIB: u64 = 5 * 1024 * 1024 * 1024;

#[tokio::test]
#[ignore = "requires explicit S3-compatible provider credentials and an existing bucket"]
async fn live_provider_single_multipart_and_delete_conformance()
-> Result<(), Box<dyn std::error::Error>> {
    let settings = settings_from_environment()?;
    let label = env::var("G7MB_LIVE_S3_LABEL").unwrap_or_else(|_| "external".to_owned());
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
    upload_sparse_multipart(
        &raw,
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

    eprintln!(
        "live-provider-conformance PASS label={label} multipart_bytes={requested_bytes} large_5gib={}",
        requested_bytes == FIVE_GIB
    );
    Ok(())
}

async fn upload_sparse_multipart(
    store: &S3CompatibleStore,
    key: ObjectKey,
    directory: &Path,
    total_bytes: u64,
    part_size: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let session = store
        .create_multipart(CreateMultipartRequest {
            key: key.clone(),
            content_type: "application/octet-stream".to_owned(),
        })
        .await?;
    let part_count = total_bytes.div_ceil(part_size);
    if part_count == 0 || part_count > 10_000 {
        return Err(std::io::Error::other("multipart part count is invalid").into());
    }
    let part_path = directory.join("sparse-part.bin");
    let mut completed = Vec::with_capacity(usize::try_from(part_count)?);
    for index in 0..part_count {
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
            key,
            upload_id: session.upload_id,
            parts: completed,
        })
        .await?;
    Ok(())
}

fn settings_from_environment() -> Result<StorageSettings, Box<dyn std::error::Error>> {
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
    Ok(StorageSettings {
        endpoint_url: env::var("G7MB_LIVE_S3_ENDPOINT")
            .ok()
            .filter(|value| !value.is_empty()),
        region: env::var("G7MB_LIVE_S3_REGION")?,
        raw_bucket: env::var("G7MB_LIVE_S3_RAW_BUCKET")?,
        derivative_bucket: env::var("G7MB_LIVE_S3_DERIVATIVE_BUCKET")?,
        access_key_id: SecretString::from(env::var("G7MB_LIVE_S3_ACCESS_KEY")?),
        secret_access_key: SecretString::from(env::var("G7MB_LIVE_S3_SECRET_KEY")?),
        force_path_style,
    })
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
