//! AWS S3, Lightsail, and Cloudflare R2 adapter using the official AWS SDK for Rust.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Region},
    error::ProvideErrorMetadata as _,
    presigning::PresigningConfig,
    primitives::ByteStream,
    types,
};
use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest, DownloadObjectRequest,
    ListObjectsRequest, ListedObject, ListedObjectsPage, MultipartSession, ObjectMetadata,
    ObjectStore, ObjectStoreError, PresignGetRequest, PresignPartRequest, PresignPutRequest,
    PresignedDownload, PresignedUpload, PutFileRequest,
};
use g7mb_config::StorageSettings;
use g7mb_domain::{ObjectKey, UploadId};
use secrecy::{ExposeSecret, SecretString};
use time::OffsetDateTime;

/// S3-compatible object store bound to one bucket.
#[derive(Clone, Debug)]
pub struct S3CompatibleStore {
    client: Client,
    bucket: String,
}

/// Result of an idempotent bucket and CORS bootstrap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BucketBootstrapReport {
    /// Bucket name checked or created.
    pub bucket: String,
    /// Whether this invocation created the bucket.
    pub created: bool,
    /// Whether the managed browser CORS rule was installed or replaced.
    pub cors_configured: bool,
}

/// Result of a destructive-only-to-canary-keys live runtime check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageCanaryReport {
    /// Unique buckets checked.
    pub buckets_checked: usize,
    /// Whether single-object PUT/HEAD/GET/LIST/DELETE passed.
    pub single_object: bool,
    /// Whether multipart create/upload/complete and abort passed.
    pub multipart: bool,
}

/// S3-compatible control-plane helper used only by the installation CLI.
#[derive(Clone, Debug)]
pub struct S3StorageAdmin {
    client: Client,
    region: String,
    custom_endpoint: bool,
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

        Ok(Self {
            client: build_client(settings),
            bucket,
        })
    }
}

impl S3StorageAdmin {
    /// Builds an administrator client from the same runtime credentials and endpoint.
    pub fn new(settings: &StorageSettings) -> Self {
        Self {
            client: build_client(settings),
            region: settings.region.clone(),
            custom_endpoint: settings.endpoint_url.is_some(),
        }
    }

    /// Checks or creates each configured bucket and merges one managed browser CORS rule.
    pub async fn bootstrap(
        &self,
        settings: &StorageSettings,
        create_missing: bool,
        cors_origins: &[String],
    ) -> Result<Vec<BucketBootstrapReport>, ObjectStoreError> {
        if cors_origins.len() > 32 {
            return Err(ObjectStoreError::InvalidRequest(
                "CORS origin count exceeds 32".to_owned(),
            ));
        }
        let buckets = unique_buckets(settings)?;
        let mut reports = Vec::with_capacity(buckets.len());
        for bucket in buckets {
            let created = self.ensure_bucket(&bucket, create_missing).await?;
            let cors_configured = if cors_origins.is_empty() {
                false
            } else {
                self.merge_cors(&bucket, cors_origins).await?;
                true
            };
            reports.push(BucketBootstrapReport {
                bucket,
                created,
                cors_configured,
            });
        }
        Ok(reports)
    }

    /// Executes bounded live runtime operations and removes every created canary object.
    pub async fn canary(
        &self,
        settings: &StorageSettings,
    ) -> Result<StorageCanaryReport, ObjectStoreError> {
        let buckets = unique_buckets(settings)?;
        for (index, bucket) in buckets.iter().enumerate() {
            let prefix = if index == 0 { "raw/" } else { "media/" };
            self.canary_bucket(bucket, prefix).await?;
        }
        Ok(StorageCanaryReport {
            buckets_checked: buckets.len(),
            single_object: true,
            multipart: true,
        })
    }

    async fn ensure_bucket(
        &self,
        bucket: &str,
        create_missing: bool,
    ) -> Result<bool, ObjectStoreError> {
        match self.client.head_bucket().bucket(bucket).send().await {
            Ok(_) => return Ok(false),
            Err(error) if !create_missing => {
                return Err(ObjectStoreError::Backend(format!(
                    "bucket {bucket} is unavailable: {error}"
                )));
            }
            Err(_) => {}
        }

        let mut request = self.client.create_bucket().bucket(bucket);
        if !self.custom_endpoint && self.region != "us-east-1" {
            let configuration = types::CreateBucketConfiguration::builder()
                .location_constraint(types::BucketLocationConstraint::from(self.region.as_str()))
                .build();
            request = request.create_bucket_configuration(configuration);
        }
        request.send().await.map_err(|error| {
            ObjectStoreError::Backend(format!("failed to create bucket {bucket}: {error}"))
        })?;
        self.client
            .head_bucket()
            .bucket(bucket)
            .send()
            .await
            .map_err(|error| {
                ObjectStoreError::Backend(format!(
                    "created bucket {bucket} did not pass HEAD: {error}"
                ))
            })?;
        Ok(true)
    }

    async fn merge_cors(&self, bucket: &str, origins: &[String]) -> Result<(), ObjectStoreError> {
        const RULE_ID: &str = "g7mediabooster-browser-v1";
        let mut rules = match self.client.get_bucket_cors().bucket(bucket).send().await {
            Ok(output) => output.cors_rules.unwrap_or_default(),
            Err(error)
                if matches!(
                    error.as_service_error().and_then(|value| value.code()),
                    Some("NoSuchCORSConfiguration" | "NoSuchCORS")
                ) =>
            {
                Vec::new()
            }
            Err(error) => {
                return Err(ObjectStoreError::Backend(format!(
                    "failed to read CORS for bucket {bucket}: {error}"
                )));
            }
        };
        rules = merge_managed_cors_rules(rules, origins).map_err(|error| match error {
            ObjectStoreError::InvalidRequest(message) => ObjectStoreError::InvalidRequest(format!(
                "bucket {bucket} CORS cannot be updated: {message}"
            )),
            other => other,
        })?;
        let configuration = types::CorsConfiguration::builder()
            .set_cors_rules(Some(rules))
            .build()
            .map_err(|error| ObjectStoreError::InvalidRequest(error.to_string()))?;
        self.client
            .put_bucket_cors()
            .bucket(bucket)
            .cors_configuration(configuration)
            .send()
            .await
            .map_err(|error| {
                ObjectStoreError::Backend(format!(
                    "failed to configure CORS for bucket {bucket}: {error}"
                ))
            })?;
        let verified = self
            .client
            .get_bucket_cors()
            .bucket(bucket)
            .send()
            .await
            .map_err(|error| {
                ObjectStoreError::Backend(format!(
                    "failed to verify CORS for bucket {bucket}: {error}"
                ))
            })?;
        let managed = verified
            .cors_rules()
            .iter()
            .find(|rule| rule.id() == Some(RULE_ID))
            .ok_or_else(|| {
                ObjectStoreError::Backend(format!(
                    "bucket {bucket} did not retain the managed CORS rule"
                ))
            })?;
        if managed.allowed_origins() != origins
            || !["GET", "PUT", "HEAD"].iter().all(|method| {
                managed
                    .allowed_methods()
                    .iter()
                    .any(|value| value == method)
            })
            || !managed
                .expose_headers()
                .iter()
                .any(|header| header.eq_ignore_ascii_case("etag"))
        {
            return Err(ObjectStoreError::Backend(format!(
                "bucket {bucket} retained an incomplete managed CORS rule"
            )));
        }
        Ok(())
    }

    async fn canary_bucket(&self, bucket: &str, prefix: &str) -> Result<(), ObjectStoreError> {
        let run_id = UploadId::new();
        let root = format!("{prefix}g7mb-canary/{run_id}");
        let single_key = format!("{root}/single");
        let multipart_key = format!("{root}/multipart");
        let abort_key = format!("{root}/abort");
        let single_body = b"g7mediabooster-storage-canary-v1".to_vec();

        self.client
            .put_object()
            .bucket(bucket)
            .key(&single_key)
            .content_type("application/octet-stream")
            .body(ByteStream::from(single_body.clone()))
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(format!("canary PUT failed: {error}")))?;
        let result = async {
            let head = self
                .client
                .head_object()
                .bucket(bucket)
                .key(&single_key)
                .send()
                .await
                .map_err(|error| {
                    ObjectStoreError::Backend(format!("canary HEAD failed: {error}"))
                })?;
            if head.content_length() != Some(i64::try_from(single_body.len()).unwrap_or(-1)) {
                return Err(ObjectStoreError::ContentLengthMismatch);
            }
            let body = self
                .client
                .get_object()
                .bucket(bucket)
                .key(&single_key)
                .send()
                .await
                .map_err(|error| ObjectStoreError::Backend(format!("canary GET failed: {error}")))?
                .body
                .collect()
                .await
                .map_err(|error| {
                    ObjectStoreError::Backend(format!("canary GET body failed: {error}"))
                })?
                .into_bytes();
            if body.as_ref() != single_body.as_slice() {
                return Err(ObjectStoreError::ContentLengthMismatch);
            }
            let listed = self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(&root)
                .max_keys(10)
                .send()
                .await
                .map_err(|error| {
                    ObjectStoreError::Backend(format!("canary LIST failed: {error}"))
                })?;
            if !listed
                .contents()
                .iter()
                .any(|object| object.key() == Some(single_key.as_str()))
            {
                return Err(ObjectStoreError::Backend(
                    "canary LIST did not return the uploaded object".to_owned(),
                ));
            }
            self.complete_canary_multipart(bucket, &multipart_key)
                .await?;
            self.abort_canary_multipart(bucket, &abort_key).await?;
            Ok(())
        }
        .await;

        let cleanup_keys = [&single_key, &multipart_key, &abort_key];
        let mut cleanup_error = None;
        for key in cleanup_keys {
            if let Err(error) = self
                .client
                .delete_object()
                .bucket(bucket)
                .key(key)
                .send()
                .await
            {
                cleanup_error = Some(ObjectStoreError::Backend(format!(
                    "canary cleanup DELETE failed: {error}"
                )));
            }
        }
        result?;
        if let Some(error) = cleanup_error {
            return Err(error);
        }
        Ok(())
    }

    async fn complete_canary_multipart(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<(), ObjectStoreError> {
        let created = self
            .client
            .create_multipart_upload()
            .bucket(bucket)
            .key(key)
            .content_type("application/octet-stream")
            .send()
            .await
            .map_err(|error| {
                ObjectStoreError::Backend(format!("canary multipart create failed: {error}"))
            })?;
        let upload_id = created
            .upload_id()
            .ok_or_else(|| {
                ObjectStoreError::Backend("canary multipart returned no upload id".to_owned())
            })?
            .to_owned();
        let result = async {
            let bodies = [vec![0x47_u8; 5 * 1024 * 1024], b"g7mb-final-part".to_vec()];
            let expected_length = bodies.iter().try_fold(0_i64, |total, body| {
                i64::try_from(body.len())
                    .ok()
                    .and_then(|length| total.checked_add(length))
                    .ok_or_else(|| {
                        ObjectStoreError::Backend(
                            "canary multipart content length overflow".to_owned(),
                        )
                    })
            })?;
            let mut parts = Vec::with_capacity(bodies.len());
            for (index, body) in bodies.into_iter().enumerate() {
                let part_number = i32::try_from(index + 1).map_err(|_| {
                    ObjectStoreError::Backend("canary multipart part overflow".to_owned())
                })?;
                let uploaded = self
                    .client
                    .upload_part()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .part_number(part_number)
                    .body(ByteStream::from(body))
                    .send()
                    .await
                    .map_err(|error| {
                        ObjectStoreError::Backend(format!(
                            "canary multipart part {part_number} failed: {error}"
                        ))
                    })?;
                let etag = uploaded.e_tag().ok_or_else(|| {
                    ObjectStoreError::Backend(format!(
                        "canary multipart part {part_number} returned no ETag"
                    ))
                })?;
                parts.push(
                    types::CompletedPart::builder()
                        .part_number(part_number)
                        .e_tag(etag)
                        .build(),
                );
            }
            let upload = types::CompletedMultipartUpload::builder()
                .set_parts(Some(parts))
                .build();
            self.client
                .complete_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .multipart_upload(upload)
                .send()
                .await
                .map_err(|error| {
                    ObjectStoreError::Backend(format!("canary multipart complete failed: {error}"))
                })?;
            let head = self
                .client
                .head_object()
                .bucket(bucket)
                .key(key)
                .send()
                .await
                .map_err(|error| {
                    ObjectStoreError::Backend(format!("canary multipart HEAD failed: {error}"))
                })?;
            if head.content_length() != Some(expected_length) {
                return Err(ObjectStoreError::ContentLengthMismatch);
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            let _abort_result = self
                .client
                .abort_multipart_upload()
                .bucket(bucket)
                .key(key)
                .upload_id(&upload_id)
                .send()
                .await;
        }
        result
    }

    async fn abort_canary_multipart(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<(), ObjectStoreError> {
        let created = self
            .client
            .create_multipart_upload()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(|error| {
                ObjectStoreError::Backend(format!("canary multipart abort create failed: {error}"))
            })?;
        let upload_id = created.upload_id().ok_or_else(|| {
            ObjectStoreError::Backend("canary abort returned no upload id".to_owned())
        })?;
        self.client
            .abort_multipart_upload()
            .bucket(bucket)
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(|error| {
                ObjectStoreError::Backend(format!("canary multipart abort failed: {error}"))
            })?;
        Ok(())
    }
}

fn build_client(settings: &StorageSettings) -> Client {
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
    Client::from_conf(service_config.build())
}

fn unique_buckets(settings: &StorageSettings) -> Result<Vec<String>, ObjectStoreError> {
    let buckets = BTreeSet::from([
        settings.raw_bucket.clone(),
        settings.derivative_bucket.clone(),
    ]);
    if buckets.iter().any(String::is_empty) {
        return Err(ObjectStoreError::InvalidRequest(
            "bucket must not be empty".to_owned(),
        ));
    }
    Ok(buckets.into_iter().collect())
}

fn managed_cors_rule(origins: &[String]) -> Result<types::CorsRule, ObjectStoreError> {
    types::CorsRule::builder()
        .id("g7mediabooster-browser-v1")
        .set_allowed_headers(Some(vec!["content-type".to_owned(), "x-amz-*".to_owned()]))
        .set_allowed_methods(Some(vec![
            "GET".to_owned(),
            "PUT".to_owned(),
            "HEAD".to_owned(),
        ]))
        .set_allowed_origins(Some(origins.to_vec()))
        .set_expose_headers(Some(vec!["ETag".to_owned()]))
        .max_age_seconds(3600)
        .build()
        .map_err(|error| ObjectStoreError::InvalidRequest(error.to_string()))
}

fn merge_managed_cors_rules(
    mut rules: Vec<types::CorsRule>,
    origins: &[String],
) -> Result<Vec<types::CorsRule>, ObjectStoreError> {
    rules.retain(|rule| rule.id() != Some("g7mediabooster-browser-v1"));
    if rules.len() >= 100 {
        return Err(ObjectStoreError::InvalidRequest(
            "provider already has the maximum 100 CORS rules".to_owned(),
        ));
    }
    rules.push(managed_cors_rule(origins)?);
    Ok(rules)
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

    async fn presign_get(
        &self,
        request: PresignGetRequest,
    ) -> Result<PresignedDownload, ObjectStoreError> {
        let config = PresigningConfig::expires_in(request.expires_in).map_err(|error| {
            ObjectStoreError::InvalidRequest(format!("invalid presign expiry: {error}"))
        })?;
        let signed = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(request.key.as_str())
            .presigned(config)
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        Ok(PresignedDownload {
            url: SecretString::from(signed.uri().to_string()),
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

    async fn list_objects(
        &self,
        request: ListObjectsRequest,
    ) -> Result<ListedObjectsPage, ObjectStoreError> {
        if !matches!(request.prefix.as_str(), "raw/" | "media/")
            || request.max_keys == 0
            || request.max_keys > 1000
            || request.start_after.as_ref().is_some_and(|cursor| {
                cursor.len() > 1024 || !cursor.starts_with(request.prefix.as_str())
            })
        {
            return Err(ObjectStoreError::InvalidRequest(
                "inventory request violates prefix, cursor, or page bounds".to_owned(),
            ));
        }
        let output = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&request.prefix)
            .set_start_after(request.start_after)
            .max_keys(i32::from(request.max_keys))
            .send()
            .await
            .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
        let mut objects = Vec::with_capacity(output.contents().len());
        for object in output.contents() {
            let key = object.key().ok_or_else(|| {
                ObjectStoreError::Backend("storage inventory returned no object key".to_owned())
            })?;
            let content_length =
                u64::try_from(object.size().unwrap_or_default()).map_err(|_| {
                    ObjectStoreError::Backend(
                        "storage inventory returned a negative object length".to_owned(),
                    )
                })?;
            objects.push(ListedObject {
                key: key.to_owned(),
                content_length,
            });
        }
        let truncated = output.is_truncated().unwrap_or(false);
        let next_start_after = if truncated {
            Some(
                objects
                    .last()
                    .ok_or_else(|| {
                        ObjectStoreError::Backend(
                            "storage returned an empty truncated inventory page".to_owned(),
                        )
                    })?
                    .key
                    .clone(),
            )
        } else {
            None
        };
        Ok(ListedObjectsPage {
            objects,
            next_start_after,
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

    use aws_sdk_s3::types;
    use g7mb_application::{
        CompletedPart, ListObjectsRequest, ObjectStore as _, ObjectStoreError, PresignGetRequest,
        PresignPartRequest, PresignPutRequest,
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
            access_key_id_file: None,
            secret_access_key: SecretString::from("test-secret".to_owned()),
            secret_access_key_file: None,
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
            access_key_id_file: None,
            secret_access_key: SecretString::from("test-secret".to_owned()),
            secret_access_key_file: None,
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

    #[tokio::test]
    async fn presigns_private_derivative_get_without_credentials_in_the_url()
    -> Result<(), Box<dyn std::error::Error>> {
        let settings = StorageSettings {
            endpoint_url: Some("https://account-id.r2.cloudflarestorage.com".to_owned()),
            region: "auto".to_owned(),
            raw_bucket: "raw-private".to_owned(),
            derivative_bucket: "media-private".to_owned(),
            access_key_id: SecretString::from("test-access".to_owned()),
            access_key_id_file: None,
            secret_access_key: SecretString::from("test-secret".to_owned()),
            secret_access_key_file: None,
            force_path_style: false,
        };
        let store = S3CompatibleStore::for_derivative_bucket(&settings).await?;
        let signed = store
            .presign_get(PresignGetRequest {
                key: ObjectKey::new("media/site/upload/digest/preset/thumbnail.jpg")?,
                expires_in: Duration::from_secs(300),
            })
            .await?;
        let url = signed.url.expose_secret();
        assert!(url.starts_with("https://media-private.account-id.r2.cloudflarestorage.com/"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(!url.contains("test-secret"));
        assert!(signed.expires_at > time::OffsetDateTime::now_utc());
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

    #[test]
    fn managed_browser_cors_rule_is_bounded_and_exposes_multipart_etag() {
        let origins = vec!["https://example.com".to_owned()];
        let rule = super::managed_cors_rule(&origins);
        assert!(rule.is_ok());
        let Some(rule) = rule.ok() else {
            return;
        };
        assert_eq!(rule.id(), Some("g7mediabooster-browser-v1"));
        assert_eq!(rule.allowed_origins(), origins);
        assert_eq!(rule.allowed_methods(), ["GET", "PUT", "HEAD"]);
        assert_eq!(rule.allowed_headers(), ["content-type", "x-amz-*"]);
        assert_eq!(rule.expose_headers(), ["ETag"]);
        assert_eq!(rule.max_age_seconds(), Some(3600));
    }

    #[test]
    fn managed_cors_merge_preserves_unrelated_rules_and_replaces_its_own_rule() {
        let existing = types::CorsRule::builder()
            .id("operator-rule")
            .allowed_methods("GET")
            .allowed_origins("https://static.example.com")
            .build();
        assert!(existing.is_ok());
        let Some(existing) = existing.ok() else {
            return;
        };
        let origins = vec!["https://example.com".to_owned()];
        let first = super::merge_managed_cors_rules(vec![existing.clone()], &origins);
        assert!(first.is_ok());
        let Some(first) = first.ok() else {
            return;
        };
        assert_eq!(first.len(), 2);
        let second = super::merge_managed_cors_rules(first, &origins);
        assert!(second.is_ok());
        let Some(second) = second.ok() else {
            return;
        };
        assert_eq!(second.len(), 2);
        assert!(second.iter().any(|rule| rule.id() == Some("operator-rule")));
        assert_eq!(
            second
                .iter()
                .filter(|rule| rule.id() == Some("g7mediabooster-browser-v1"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn inventory_rejects_arbitrary_prefixes_before_network_io()
    -> Result<(), Box<dyn std::error::Error>> {
        let settings = StorageSettings {
            endpoint_url: Some("https://account-id.r2.cloudflarestorage.com".to_owned()),
            region: "auto".to_owned(),
            raw_bucket: "raw-private".to_owned(),
            derivative_bucket: "media".to_owned(),
            access_key_id: SecretString::from("test-access".to_owned()),
            access_key_id_file: None,
            secret_access_key: SecretString::from("test-secret".to_owned()),
            secret_access_key_file: None,
            force_path_style: false,
        };
        let store = S3CompatibleStore::for_raw_bucket(&settings).await?;
        let error = store
            .list_objects(ListObjectsRequest {
                prefix: "foreign/".to_owned(),
                start_after: None,
                max_keys: 1001,
            })
            .await
            .err()
            .ok_or("unsafe inventory request was accepted")?;
        assert!(matches!(error, ObjectStoreError::InvalidRequest(_)));
        Ok(())
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
