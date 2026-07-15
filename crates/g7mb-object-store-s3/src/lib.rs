//! AWS S3, Lightsail, and Cloudflare R2 adapter using the official AWS SDK for Rust.

use std::collections::BTreeMap;

use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Region},
    presigning::PresigningConfig,
    primitives::ByteStream,
    types,
};
use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest, DownloadObjectRequest,
    MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError, PresignPartRequest,
    PresignPutRequest, PresignedUpload, PutFileRequest,
};
use g7mb_config::StorageSettings;
use g7mb_domain::ObjectKey;
use secrecy::{ExposeSecret, SecretString};
use time::OffsetDateTime;

/// S3-compatible object store bound to one bucket.
#[derive(Clone, Debug)]
pub struct S3CompatibleStore {
    client: Client,
    bucket: String,
}

impl S3CompatibleStore {
    /// Builds a raw-bucket adapter from common S3/R2 settings.
    pub async fn for_raw_bucket(settings: &StorageSettings) -> Result<Self, ObjectStoreError> {
        Self::new(settings, settings.raw_bucket.clone()).await
    }

    /// Builds a derivative-bucket adapter from common S3/R2 settings.
    pub async fn for_derivative_bucket(
        settings: &StorageSettings,
    ) -> Result<Self, ObjectStoreError> {
        Self::new(settings, settings.derivative_bucket.clone()).await
    }

    async fn new(settings: &StorageSettings, bucket: String) -> Result<Self, ObjectStoreError> {
        if bucket.is_empty() {
            return Err(ObjectStoreError::InvalidRequest(
                "bucket must not be empty".to_owned(),
            ));
        }

        let credentials = Credentials::new(
            settings.access_key_id.expose_secret(),
            settings.secret_access_key.expose_secret(),
            None,
            None,
            "g7mb-static-config",
        );
        let mut service_config = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(settings.region.clone()))
            .credentials_provider(credentials)
            .force_path_style(settings.force_path_style);
        if let Some(endpoint_url) = &settings.endpoint_url {
            service_config = service_config.endpoint_url(endpoint_url);
        }

        Ok(Self {
            client: Client::from_conf(service_config.build()),
            bucket,
        })
    }
}

#[async_trait]
impl ObjectStore for S3CompatibleStore {
    async fn presign_put(
        &self,
        request: PresignPutRequest,
    ) -> Result<PresignedUpload, ObjectStoreError> {
        let content_length = i64::try_from(request.content_length).map_err(|_| {
            ObjectStoreError::InvalidRequest("content length exceeds S3 range".to_owned())
        })?;
        let config = PresigningConfig::expires_in(request.expires_in).map_err(|error| {
            ObjectStoreError::InvalidRequest(format!("invalid presign expiry: {error}"))
        })?;
        let signed = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .content_length(content_length)
            .content_type(&request.content_type)
            .presigned(config)
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;

        let mut required_headers = BTreeMap::new();
        required_headers.insert("content-type".to_owned(), request.content_type);
        required_headers.insert("content-length".to_owned(), content_length.to_string());

        Ok(PresignedUpload {
            url: SecretString::from(signed.uri().to_string()),
            required_headers,
            expires_at: OffsetDateTime::now_utc() + request.expires_in,
        })
    }

    async fn create_multipart(
        &self,
        request: CreateMultipartRequest,
    ) -> Result<MultipartSession, ObjectStoreError> {
        let output = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .content_type(request.content_type)
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        let upload_id = output.upload_id().ok_or_else(|| {
            ObjectStoreError::Backend("storage returned no multipart upload id".to_owned())
        })?;
        Ok(MultipartSession {
            upload_id: SecretString::from(upload_id.to_owned()),
        })
    }

    async fn presign_part(
        &self,
        request: PresignPartRequest,
    ) -> Result<PresignedUpload, ObjectStoreError> {
        if request.part_number == 0 || request.part_number > 10_000 {
            return Err(ObjectStoreError::InvalidRequest(
                "multipart part number is outside 1..=10000".to_owned(),
            ));
        }
        if request.content_length == 0 {
            return Err(ObjectStoreError::InvalidRequest(
                "multipart part length must be positive".to_owned(),
            ));
        }
        let content_length = i64::try_from(request.content_length).map_err(|_| {
            ObjectStoreError::InvalidRequest("part length exceeds S3 range".to_owned())
        })?;
        let config = PresigningConfig::expires_in(request.expires_in).map_err(|error| {
            ObjectStoreError::InvalidRequest(format!("invalid presign expiry: {error}"))
        })?;
        let signed = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .upload_id(request.upload_id.expose_secret())
            .part_number(i32::from(request.part_number))
            .content_length(content_length)
            .presigned(config)
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        let mut required_headers = BTreeMap::new();
        required_headers.insert("content-length".to_owned(), content_length.to_string());
        Ok(PresignedUpload {
            url: SecretString::from(signed.uri().to_string()),
            required_headers,
            expires_at: OffsetDateTime::now_utc() + request.expires_in,
        })
    }

    async fn complete_multipart(
        &self,
        request: CompleteMultipartRequest,
    ) -> Result<(), ObjectStoreError> {
        validate_completed_parts(&request.parts)?;
        let parts = request
            .parts
            .into_iter()
            .map(|part| {
                types::CompletedPart::builder()
                    .part_number(i32::from(part.part_number))
                    .e_tag(part.etag)
                    .build()
            })
            .collect::<Vec<_>>();
        let multipart_upload = types::CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();
        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .upload_id(request.upload_id.expose_secret())
            .multipart_upload(multipart_upload)
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        Ok(())
    }

    async fn abort_multipart(
        &self,
        request: AbortMultipartRequest,
    ) -> Result<(), ObjectStoreError> {
        let result = self
            .client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .upload_id(request.upload_id.expose_secret())
            .send()
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|error| error.is_no_such_upload()) =>
            {
                // A previous attempt can abort successfully and lose its DB lease before commit.
                // Treating the provider's missing-session response as success keeps retries safe.
                Ok(())
            }
            Err(error) => Err(ObjectStoreError::Backend(error.to_string())),
        }
    }

    async fn head(&self, key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError> {
        let output = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        let signed_length = output.content_length().unwrap_or_default();
        let content_length = u64::try_from(signed_length).map_err(|_| {
            ObjectStoreError::Backend("storage returned a negative content length".to_owned())
        })?;
        Ok(ObjectMetadata {
            content_length,
            content_type: output.content_type().map(ToOwned::to_owned),
            etag: output.e_tag().map(ToOwned::to_owned),
        })
    }

    async fn download_to(
        &self,
        request: DownloadObjectRequest,
    ) -> Result<ObjectMetadata, ObjectStoreError> {
        if request.expected_length == 0 || request.expected_length > request.max_length {
            return Err(ObjectStoreError::InvalidRequest(
                "download length violates the worker byte cap".to_owned(),
            ));
        }
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        let reported_length = output
            .content_length()
            .and_then(|length| u64::try_from(length).ok());
        if reported_length != Some(request.expected_length) {
            return Err(ObjectStoreError::ContentLengthMismatch);
        }
        let metadata = ObjectMetadata {
            content_length: request.expected_length,
            content_type: output.content_type().map(ToOwned::to_owned),
            etag: output.e_tag().map(ToOwned::to_owned),
        };
        let result = stream_body_to_new_file(
            output.body.into_async_read(),
            &request.destination,
            request.expected_length,
            request.max_length,
        )
        .await;
        if result.is_err() {
            let _remove_result = tokio::fs::remove_file(&request.destination).await;
        }
        result.map(|()| metadata)
    }

    async fn put_file(&self, request: PutFileRequest) -> Result<ObjectMetadata, ObjectStoreError> {
        if request.content_type.is_empty() || request.content_type.len() > 255 {
            return Err(ObjectStoreError::InvalidRequest(
                "derivative content type is invalid".to_owned(),
            ));
        }
        let filesystem_metadata = tokio::fs::symlink_metadata(&request.source)
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        if !filesystem_metadata.file_type().is_file() || filesystem_metadata.len() == 0 {
            return Err(ObjectStoreError::InvalidRequest(
                "derivative source must be a non-empty regular file".to_owned(),
            ));
        }
        let content_length = i64::try_from(filesystem_metadata.len()).map_err(|_| {
            ObjectStoreError::InvalidRequest("derivative length exceeds S3 range".to_owned())
        })?;
        let body = ByteStream::from_path(&request.source)
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        let output = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .content_type(&request.content_type)
            .content_length(content_length)
            .body(body)
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        Ok(ObjectMetadata {
            content_length: filesystem_metadata.len(),
            content_type: Some(request.content_type),
            etag: output.e_tag().map(ToOwned::to_owned),
        })
    }

    async fn delete(&self, key: &ObjectKey) -> Result<(), ObjectStoreError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        Ok(())
    }
}

async fn stream_body_to_new_file(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    destination: &std::path::Path,
    expected_length: u64,
    max_length: u64,
) -> Result<(), ObjectStoreError> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .await
        .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).map_err(|_| {
                ObjectStoreError::Backend("download chunk length overflow".to_owned())
            })?)
            .ok_or_else(|| ObjectStoreError::Backend("download length overflow".to_owned()))?;
        if total > max_length || total > expected_length {
            return Err(ObjectStoreError::ContentLengthMismatch);
        }
        file.write_all(&buffer[..read])
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
    }
    if total != expected_length {
        return Err(ObjectStoreError::ContentLengthMismatch);
    }
    file.flush()
        .await
        .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
    Ok(())
}

fn validate_completed_parts(
    parts: &[g7mb_application::CompletedPart],
) -> Result<(), ObjectStoreError> {
    if parts.is_empty() || parts.len() > 10_000 {
        return Err(ObjectStoreError::InvalidRequest(
            "multipart completion has an invalid part count".to_owned(),
        ));
    }
    let mut previous = 0_u16;
    for part in parts {
        if part.part_number == 0
            || part.part_number <= previous
            || part.etag.is_empty()
            || part.etag.len() > 1024
        {
            return Err(ObjectStoreError::InvalidRequest(
                "multipart parts must be ordered, unique, and have an ETag".to_owned(),
            ));
        }
        previous = part.part_number;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use g7mb_application::{
        CompletedPart, ObjectStore as _, PresignPartRequest, PresignPutRequest,
    };
    use g7mb_config::StorageSettings;
    use g7mb_domain::ObjectKey;
    use secrecy::{ExposeSecret as _, SecretString};
    use tokio::io::AsyncWriteExt as _;

    use super::S3CompatibleStore;

    #[tokio::test]
    async fn presigns_cloudflare_r2_endpoint_without_network_io()
    -> Result<(), Box<dyn std::error::Error>> {
        let settings = StorageSettings {
            endpoint_url: Some("https://account-id.r2.cloudflarestorage.com".to_owned()),
            region: "auto".to_owned(),
            raw_bucket: "raw-private".to_owned(),
            derivative_bucket: "media".to_owned(),
            access_key_id: SecretString::from("test-access".to_owned()),
            secret_access_key: SecretString::from("test-secret".to_owned()),
            force_path_style: false,
        };
        let store = S3CompatibleStore::for_raw_bucket(&settings).await?;
        let signed = store
            .presign_put(PresignPutRequest {
                key: ObjectKey::new("raw/tenant/id/source")?,
                content_length: 1234,
                content_type: "image/jpeg".to_owned(),
                expires_in: Duration::from_secs(60),
            })
            .await?;
        let url = signed.url.expose_secret();
        assert!(url.starts_with("https://raw-private.account-id.r2.cloudflarestorage.com/"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(!url.contains("test-secret"));
        assert_eq!(
            signed.required_headers.get("content-length"),
            Some(&"1234".to_owned())
        );
        Ok(())
    }

    #[tokio::test]
    async fn presigns_bounded_multipart_part_without_network_io()
    -> Result<(), Box<dyn std::error::Error>> {
        let settings = StorageSettings {
            endpoint_url: Some("https://account-id.r2.cloudflarestorage.com".to_owned()),
            region: "auto".to_owned(),
            raw_bucket: "raw-private".to_owned(),
            derivative_bucket: "media".to_owned(),
            access_key_id: SecretString::from("test-access".to_owned()),
            secret_access_key: SecretString::from("test-secret".to_owned()),
            force_path_style: false,
        };
        let store = S3CompatibleStore::for_raw_bucket(&settings).await?;
        let signed = store
            .presign_part(PresignPartRequest {
                key: ObjectKey::new("raw/tenant/id/source")?,
                upload_id: SecretString::from("opaque-upload-id".to_owned()),
                part_number: 7,
                content_length: 32 * 1024 * 1024,
                expires_in: Duration::from_secs(60),
            })
            .await?;
        let url = signed.url.expose_secret();
        assert!(url.contains("partNumber=7"));
        assert!(url.contains("uploadId=opaque-upload-id"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(!url.contains("test-secret"));
        Ok(())
    }

    #[test]
    fn multipart_completion_rejects_duplicate_or_unordered_parts() {
        let result = super::validate_completed_parts(&[
            CompletedPart {
                part_number: 2,
                etag: "etag-two".to_owned(),
            },
            CompletedPart {
                part_number: 2,
                etag: "etag-duplicate".to_owned(),
            },
        ]);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn bounded_download_stream_requires_the_exact_reserved_length()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("source.bin");
        let (mut writer, reader) = tokio::io::duplex(64);
        let write = tokio::spawn(async move {
            writer.write_all(b"trusted-bytes").await?;
            writer.shutdown().await
        });
        super::stream_body_to_new_file(reader, &destination, 13, 13).await?;
        write.await??;
        assert_eq!(tokio::fs::read(&destination).await?, b"trusted-bytes");

        let too_long_destination = directory.path().join("too-long.bin");
        let (mut writer, reader) = tokio::io::duplex(64);
        let write = tokio::spawn(async move {
            writer.write_all(b"fourteen-bytes!").await?;
            writer.shutdown().await
        });
        let result = super::stream_body_to_new_file(reader, &too_long_destination, 13, 13).await;
        write.await??;
        assert!(matches!(
            result,
            Err(g7mb_application::ObjectStoreError::ContentLengthMismatch)
        ));
        Ok(())
    }
}
