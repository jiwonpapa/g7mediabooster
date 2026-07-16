//! Typed, redacted configuration loaded from TOML and environment variables.

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use config::{Config, ConfigError, Environment, File, FileFormat};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

/// Complete service configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct Settings {
    /// HTTP listener settings.
    pub server: ServerSettings,
    /// Authenticated G5/G7 control-client settings.
    pub auth: AuthSettings,
    /// Bounded multi-upload policy settings.
    pub upload: UploadSettings,
    /// S3-compatible object storage settings.
    pub storage: StorageSettings,
    /// Durable SQLite settings.
    pub database: DatabaseSettings,
    /// Native worker resource settings.
    pub worker: WorkerSettings,
    /// Durable raw-object cleanup and deletion settings.
    pub lifecycle: LifecycleSettings,
    /// Private derivative URL and in-memory manifest cache settings.
    pub delivery: DeliverySettings,
    /// Optional fail-closed watermark rendering policy.
    pub watermark: WatermarkSettings,
}

impl Settings {
    /// Loads defaults, an optional TOML file, then `G7MB__...` environment overrides.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut builder = Config::builder()
            .set_default("server.bind_addr", "127.0.0.1:8088")?
            .set_default("server.request_body_limit_bytes", 1_048_576_i64)?
            .set_default("server.rate_limit_requests_per_second", 50_i64)?
            .set_default("server.rate_limit_burst", 100_i64)?
            .set_default("server.max_in_flight_requests", 64_i64)?
            .set_default("auth.allowed_skew_seconds", 300_i64)?
            .set_default("upload.max_batch_files", 100_i64)?
            .set_default("upload.max_batch_bytes", 21_474_836_480_i64)?
            .set_default("upload.max_image_bytes", 134_217_728_i64)?
            .set_default("upload.max_video_bytes", 5_368_709_120_i64)?
            .set_default("upload.multipart_threshold_bytes", 104_857_600_i64)?
            .set_default("upload.multipart_part_size_bytes", 33_554_432_i64)?
            .set_default("upload.max_browser_connections", 8_i64)?
            .set_default("upload.max_parts_per_file", 4_i64)?
            .set_default("upload.max_active_uploads_global", 1_000_i64)?
            .set_default("upload.max_active_uploads_per_tenant", 200_i64)?
            .set_default("upload.max_reserved_bytes_global", 1_099_511_627_776_i64)?
            .set_default("upload.max_reserved_bytes_per_tenant", 107_374_182_400_i64)?
            .set_default("storage.region", "auto")?
            .set_default("storage.force_path_style", false)?
            .set_default("database.url", "sqlite://data/g7mb.db")?
            .set_default("database.max_connections", 4_i64)?
            .set_default(
                "database.backup_directory",
                "/var/lib/g7mediabooster/backups",
            )?
            .set_default("database.backup_retention_count", 14_i64)?
            .set_default("worker.max_concurrent_jobs", 2_i64)?
            .set_default("worker.max_concurrent_heavy_images", 1_i64)?
            .set_default("worker.max_concurrent_videos", 1_i64)?
            .set_default("worker.native_threads_per_job", 2_i64)?
            .set_default("worker.image_timeout_seconds", 30_i64)?
            .set_default("worker.video_timeout_seconds", 45_i64)?
            .set_default("worker.sandbox_binary", "g7mb-sandbox")?
            .set_default("worker.temp_directory", "/tmp/g7mb")?
            .set_default("worker.max_temp_disk_bytes", 12_884_901_888_i64)?
            .set_default("worker.lease_seconds", 120_i64)?
            .set_default("worker.heartbeat_seconds", 30_i64)?
            .set_default("worker.poll_interval_ms", 250_i64)?
            .set_default("worker.retry_delay_seconds", 10_i64)?
            .set_default("worker.max_attempts", 3_i64)?
            .set_default("worker.max_sandbox_output_bytes", 65_536_i64)?
            .set_default("worker.metrics_bind_addr", "127.0.0.1:9091")?
            .set_default("lifecycle.created_reservation_ttl_seconds", 86_400_i64)?
            .set_default("lifecycle.rejected_source_retention_seconds", 604_800_i64)?
            .set_default("lifecycle.cleanup_lease_seconds", 300_i64)?
            .set_default("lifecycle.cleanup_retry_seconds", 60_i64)?
            .set_default("lifecycle.cleanup_batch_size", 100_i64)?
            .set_default("lifecycle.cleanup_max_attempts", 10_i64)?
            .set_default("lifecycle.orphan_grace_period_seconds", 172_800_i64)?
            .set_default("lifecycle.inventory_page_size", 1_000_i64)?
            .set_default("lifecycle.inventory_max_pages_per_run", 10_i64)?
            .set_default("lifecycle.tombstone_retention_seconds", 31_536_000_i64)?
            .set_default("lifecycle.tombstone_purge_batch_size", 100_i64)?
            .set_default("delivery.signed_url_ttl_seconds", 300_i64)?
            .set_default("delivery.manifest_cache_ttl_seconds", 60_i64)?
            .set_default("delivery.manifest_cache_max_bytes", 4_194_304_i64)?;
        builder = builder
            .set_default("watermark.enabled", false)?
            .set_default("watermark.asset_path", "")?
            .set_default("watermark.asset_sha256", "")?
            .set_default("watermark.preset_revision", "v1")?
            .set_default("watermark.position", "bottom_right")?
            .set_default("watermark.margin_px", 24_i64)?
            .set_default("watermark.max_width_percent", 20_i64)?
            .set_default("watermark.opacity_percent", 80_i64)?;

        if let Some(path) = path {
            builder = builder.add_source(File::from(path).format(FileFormat::Toml).required(true));
        }

        let mut settings: Self = builder
            .add_source(
                Environment::with_prefix("G7MB")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?
            .try_deserialize()?;
        settings.resolve_secret_files()?;
        settings.validate()?;
        Ok(settings)
    }

    fn resolve_secret_files(&mut self) -> Result<(), ConfigError> {
        resolve_secret(
            &mut self.auth.hmac_secret,
            self.auth.hmac_secret_file.as_deref(),
            "auth.hmac_secret",
        )?;
        resolve_secret(
            &mut self.storage.access_key_id,
            self.storage.access_key_id_file.as_deref(),
            "storage.access_key_id",
        )?;
        resolve_secret(
            &mut self.storage.secret_access_key,
            self.storage.secret_access_key_file.as_deref(),
            "storage.secret_access_key",
        )?;
        Ok(())
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.server.request_body_limit_bytes == 0
            || self.server.request_body_limit_bytes > 16 * 1024 * 1024
            || !(1..=10_000).contains(&self.server.rate_limit_requests_per_second)
            || self.server.rate_limit_burst < self.server.rate_limit_requests_per_second
            || self.server.rate_limit_burst > 100_000
            || !(1..=1024).contains(&self.server.max_in_flight_requests)
        {
            return Err(ConfigError::Message(
                "server settings violate body, rate, burst, or in-flight limits".to_owned(),
            ));
        }
        if self.auth.key_id.is_empty()
            || self.auth.key_id.len() > 128
            || self.auth.tenant_id.is_empty()
            || self.auth.tenant_id.len() > 64
            || !(32..=256).contains(&self.auth.hmac_secret.expose_secret().len())
            || self.auth.allowed_skew_seconds > 900
        {
            return Err(ConfigError::Message(
                "auth settings violate identifier, secret, or clock-skew limits".to_owned(),
            ));
        }
        if self.storage.raw_bucket.is_empty()
            || self.storage.raw_bucket.len() > 255
            || self.storage.derivative_bucket.is_empty()
            || self.storage.derivative_bucket.len() > 255
            || self.storage.region.is_empty()
            || self.storage.region.len() > 128
            || self.storage.access_key_id.expose_secret().is_empty()
            || self.storage.access_key_id.expose_secret().len() > 256
            || self.storage.secret_access_key.expose_secret().is_empty()
            || self.storage.secret_access_key.expose_secret().len() > 1024
        {
            return Err(ConfigError::Message(
                "storage settings violate bucket, region, or credential limits".to_owned(),
            ));
        }
        self.storage.validate_provider_contract()?;
        if self.upload.max_batch_files == 0
            || self.upload.max_batch_files > 100
            || self.upload.max_batch_bytes == 0
            || self.upload.max_image_bytes == 0
            || self.upload.max_video_bytes == 0
            || self.upload.multipart_threshold_bytes == 0
            || self.upload.multipart_part_size_bytes < 5 * 1024 * 1024
            || self.upload.max_browser_connections == 0
            || self.upload.max_parts_per_file == 0
            || self.upload.max_active_uploads_global < self.upload.max_batch_files
            || self.upload.max_active_uploads_per_tenant < self.upload.max_batch_files
            || self.upload.max_active_uploads_per_tenant > self.upload.max_active_uploads_global
            || self.upload.max_reserved_bytes_global < self.upload.max_batch_bytes
            || self.upload.max_reserved_bytes_per_tenant < self.upload.max_batch_bytes
            || self.upload.max_reserved_bytes_per_tenant > self.upload.max_reserved_bytes_global
            || self.upload.max_reserved_bytes_global > i64::MAX as u64
        {
            return Err(ConfigError::Message(
                "upload settings violate bounded multi-upload limits".to_owned(),
            ));
        }
        let minimum_temp_disk_bytes = self
            .upload
            .max_video_bytes
            .max(self.upload.max_image_bytes)
            .saturating_add(self.upload.max_image_bytes.saturating_mul(2))
            .saturating_add(16 * 1024 * 1024);
        if self.worker.max_concurrent_jobs == 0
            || self.worker.max_concurrent_jobs > 32
            || self.worker.max_concurrent_heavy_images == 0
            || self.worker.max_concurrent_heavy_images > self.worker.max_concurrent_jobs
            || self.worker.max_concurrent_videos == 0
            || self.worker.max_concurrent_videos > self.worker.max_concurrent_jobs
            || self.worker.native_threads_per_job == 0
            || self.worker.native_threads_per_job > 16
            || self.worker.image_timeout_seconds == 0
            || self.worker.video_timeout_seconds == 0
            || self.worker.heartbeat_seconds == 0
            || self.worker.lease_seconds <= self.worker.heartbeat_seconds.saturating_mul(2)
            || self.worker.poll_interval_ms == 0
            || self.worker.retry_delay_seconds == 0
            || self.worker.max_attempts == 0
            || !(1024..=1_048_576).contains(&self.worker.max_sandbox_output_bytes)
            || !self.worker.metrics_bind_addr.ip().is_loopback()
            || self.worker.sandbox_binary.as_os_str().is_empty()
            || self.worker.temp_directory.as_os_str().is_empty()
            || self.worker.max_temp_disk_bytes < minimum_temp_disk_bytes
            || self.worker.max_temp_disk_bytes > 1024 * 1024 * 1024 * 1024
        {
            return Err(ConfigError::Message(
                "worker settings violate concurrency, lease, timeout, or output limits".to_owned(),
            ));
        }
        if self.database.url.is_empty()
            || !(1..=16).contains(&self.database.max_connections)
            || !self.database.backup_directory.is_absolute()
            || !(2..=365).contains(&self.database.backup_retention_count)
        {
            return Err(ConfigError::Message(
                "database settings violate URL, pool, backup path, or retention limits".to_owned(),
            ));
        }
        if self.watermark.enabled
            && (self.worker.video_timeout_seconds < 4
                || self.watermark.asset_path.as_os_str().is_empty()
                || self.watermark.asset_sha256.len() != 64
                || !self
                    .watermark
                    .asset_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || !(1..=32).contains(&self.watermark.preset_revision.len())
                || !self
                    .watermark
                    .preset_revision
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                || self.watermark.margin_px > 1024
                || !(1..=50).contains(&self.watermark.max_width_percent)
                || !(1..=100).contains(&self.watermark.opacity_percent))
        {
            return Err(ConfigError::Message(
                "watermark settings violate asset pin, revision, size, or opacity limits"
                    .to_owned(),
            ));
        }
        if self.lifecycle.created_reservation_ttl_seconds == 0
            || self.lifecycle.created_reservation_ttl_seconds > 7 * 24 * 60 * 60
            || self.lifecycle.rejected_source_retention_seconds == 0
            || self.lifecycle.rejected_source_retention_seconds > 90 * 24 * 60 * 60
            || self.lifecycle.cleanup_lease_seconds == 0
            || self.lifecycle.cleanup_lease_seconds > 60 * 60
            || self.lifecycle.cleanup_retry_seconds == 0
            || self.lifecycle.cleanup_retry_seconds > 24 * 60 * 60
            || !(1..=100).contains(&self.lifecycle.cleanup_batch_size)
            || !(1..=100).contains(&self.lifecycle.cleanup_max_attempts)
            || !(60 * 60..=30 * 24 * 60 * 60).contains(&self.lifecycle.orphan_grace_period_seconds)
            || !(1..=1000).contains(&self.lifecycle.inventory_page_size)
            || !(1..=100).contains(&self.lifecycle.inventory_max_pages_per_run)
            || !(30 * 24 * 60 * 60..=10 * 365 * 24 * 60 * 60)
                .contains(&self.lifecycle.tombstone_retention_seconds)
            || !(1..=1000).contains(&self.lifecycle.tombstone_purge_batch_size)
        {
            return Err(ConfigError::Message(
                "lifecycle settings violate retention, lease, retry, batch, or attempt limits"
                    .to_owned(),
            ));
        }
        if !(30..=15 * 60).contains(&self.delivery.signed_url_ttl_seconds)
            || !(1..=5 * 60).contains(&self.delivery.manifest_cache_ttl_seconds)
            || self.delivery.manifest_cache_ttl_seconds > self.delivery.signed_url_ttl_seconds
            || !(64 * 1024..=64 * 1024 * 1024).contains(&self.delivery.manifest_cache_max_bytes)
        {
            return Err(ConfigError::Message(
                "delivery settings violate signature, manifest TTL, or cache byte limits"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

fn empty_secret() -> SecretString {
    SecretString::from(String::new())
}

fn resolve_secret(
    inline: &mut SecretString,
    path: Option<&Path>,
    setting_name: &str,
) -> Result<(), ConfigError> {
    let inline_present = !inline.expose_secret().is_empty();
    let Some(path) = path else {
        return Ok(());
    };
    if inline_present {
        return Err(ConfigError::Message(format!(
            "{setting_name} and {setting_name}_file are mutually exclusive"
        )));
    }
    if !path.is_absolute() {
        return Err(ConfigError::Message(format!(
            "{setting_name}_file must be an absolute path"
        )));
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ConfigError::Message(format!("failed to inspect {setting_name}_file: {error}"))
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ConfigError::Message(format!(
            "{setting_name}_file must be a regular non-symlink file"
        )));
    }
    if metadata.len() == 0 || metadata.len() > 4096 {
        return Err(ConfigError::Message(format!(
            "{setting_name}_file violates the 1..=4096 byte limit"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        if metadata.permissions().mode() & 0o037 != 0 {
            return Err(ConfigError::Message(format!(
                "{setting_name}_file must not be writable by group or accessible by others"
            )));
        }
    }
    let bytes = fs::read(path).map_err(|error| {
        ConfigError::Message(format!("failed to read {setting_name}_file: {error}"))
    })?;
    let value = String::from_utf8(bytes).map_err(|_| {
        ConfigError::Message(format!("{setting_name}_file must contain UTF-8 text"))
    })?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty()
        || value.len() > 4096
        || value.contains(['\r', '\n', '\0'])
        || value.trim() != value
    {
        return Err(ConfigError::Message(format!(
            "{setting_name}_file contains an invalid secret value"
        )));
    }
    *inline = SecretString::from(value.to_owned());
    Ok(())
}

/// HTTP server configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct ServerSettings {
    /// Loopback or private listener address.
    pub bind_addr: SocketAddr,
    /// Maximum JSON request body.
    pub request_body_limit_bytes: usize,
    /// Sustained `/v1` control requests admitted per second.
    pub rate_limit_requests_per_second: u32,
    /// Short `/v1` token-bucket burst capacity.
    pub rate_limit_burst: u32,
    /// Maximum `/v1` handlers simultaneously in flight.
    pub max_in_flight_requests: usize,
}

/// One authenticated PHP control client for the initial single-site deployment.
#[derive(Clone, Debug, Deserialize)]
pub struct AuthSettings {
    /// Public key identifier included in signed requests.
    pub key_id: String,
    /// Tenant/site identifier used in object keys and state isolation.
    pub tenant_id: String,
    /// HMAC-SHA256 secret, redacted in debug output.
    #[serde(default = "empty_secret")]
    pub hmac_secret: SecretString,
    /// Absolute root-owned file containing the HMAC secret.
    #[serde(default)]
    pub hmac_secret_file: Option<PathBuf>,
    /// Maximum absolute request clock skew.
    pub allowed_skew_seconds: u64,
}

/// Bounded direct multi-upload configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct UploadSettings {
    /// Maximum files accepted by one batch call.
    pub max_batch_files: usize,
    /// Maximum aggregate bytes reserved by one batch.
    pub max_batch_bytes: u64,
    /// Maximum one encoded image.
    pub max_image_bytes: u64,
    /// Maximum one encoded video.
    pub max_video_bytes: u64,
    /// Single files at or above this value use multipart.
    pub multipart_threshold_bytes: u64,
    /// Planned multipart part size.
    pub multipart_part_size_bytes: u64,
    /// Client guidance for total concurrent upload requests.
    pub max_browser_connections: usize,
    /// Client guidance for concurrent parts of one file.
    pub max_parts_per_file: usize,
    /// Maximum active reservations across the single-node service.
    pub max_active_uploads_global: usize,
    /// Maximum active reservations for one authenticated tenant.
    pub max_active_uploads_per_tenant: usize,
    /// Maximum retained source bytes across this service.
    pub max_reserved_bytes_global: u64,
    /// Maximum retained source bytes owned by one authenticated tenant.
    pub max_reserved_bytes_per_tenant: u64,
}

/// S3-compatible storage configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct StorageSettings {
    /// Explicit provider contract used to reject endpoint/region drift at startup.
    pub provider: StorageProvider,
    /// Optional custom endpoint, required for R2.
    pub endpoint_url: Option<String>,
    /// AWS region or `auto` for R2.
    pub region: String,
    /// Private raw/quarantine bucket.
    pub raw_bucket: String,
    /// Derivative output bucket.
    pub derivative_bucket: String,
    /// Static access key identifier, redacted in debug output.
    #[serde(default = "empty_secret")]
    pub access_key_id: SecretString,
    /// Absolute root-owned file containing the access key identifier.
    #[serde(default)]
    pub access_key_id_file: Option<PathBuf>,
    /// Static secret key, redacted in debug output.
    #[serde(default = "empty_secret")]
    pub secret_access_key: SecretString,
    /// Absolute root-owned file containing the secret access key.
    #[serde(default)]
    pub secret_access_key_file: Option<PathBuf>,
    /// Enables path-style requests for compatible local services.
    pub force_path_style: bool,
}

impl StorageSettings {
    /// Rejects provider, endpoint, region, and bucket combinations that could be mislabelled.
    pub fn validate_provider_contract(&self) -> Result<(), ConfigError> {
        let endpoint = self
            .endpoint_url
            .as_deref()
            .map(parse_storage_endpoint)
            .transpose()?;
        let concrete_aws_region = self.region != "auto" && valid_region(&self.region);
        let valid = match self.provider {
            StorageProvider::R2 => {
                endpoint.as_ref().is_some_and(is_canonical_r2_endpoint)
                    && self.region == "auto"
                    && !self.force_path_style
            }
            StorageProvider::AwsS3 => {
                endpoint.is_none() && concrete_aws_region && !self.force_path_style
            }
            StorageProvider::Lightsail => {
                endpoint.is_none()
                    && concrete_aws_region
                    && !self.force_path_style
                    && self.raw_bucket == self.derivative_bucket
            }
            StorageProvider::Generic => endpoint.is_some(),
        };
        if !valid {
            return Err(ConfigError::Message(
                "storage settings do not match the declared provider contract".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Runtime storage profile persisted by `g7mbctl` and enforced before network access.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum StorageProvider {
    /// Cloudflare R2 account S3 endpoint with signing region `auto`.
    R2,
    /// AWS S3 regional endpoint selected by the official SDK.
    AwsS3,
    /// Amazon Lightsail bucket-scoped credentials on the AWS S3 endpoint.
    Lightsail,
    /// Explicit S3-compatible endpoint, including loopback-only HTTP for local MinIO.
    Generic,
}

fn parse_storage_endpoint(value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value)
        .map_err(|_| ConfigError::Message("storage endpoint URL is invalid".to_owned()))?;
    let host = url
        .host_str()
        .ok_or_else(|| ConfigError::Message("storage endpoint URL has no host".to_owned()))?;
    let loopback = host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
    {
        return Err(ConfigError::Message(
            "storage endpoint must be path-free HTTPS or loopback HTTP".to_owned(),
        ));
    }
    Ok(url)
}

fn is_canonical_r2_endpoint(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let Some(account_id) = host.strip_suffix(".r2.cloudflarestorage.com") else {
        return false;
    };
    url.scheme() == "https"
        && url.port().is_none()
        && account_id.len() == 32
        && account_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_region(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// SQLite database settings.
#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseSettings {
    /// SQLite connection URL.
    pub url: String,
    /// Bounded pool size.
    pub max_connections: u32,
    /// Absolute directory for create-new online snapshots and manifests.
    pub backup_directory: std::path::PathBuf,
    /// Number of verified local snapshot pairs retained after a successful backup.
    pub backup_retention_count: usize,
}

/// Native worker resource settings.
#[derive(Clone, Debug, Deserialize)]
pub struct WorkerSettings {
    /// Maximum leased jobs per worker process.
    pub max_concurrent_jobs: usize,
    /// Heavy-image transformations allowed inside the general worker slots.
    pub max_concurrent_heavy_images: usize,
    /// Video transformations allowed inside the general worker slots.
    pub max_concurrent_videos: usize,
    /// libvips/FFmpeg thread budget per job.
    pub native_threads_per_job: usize,
    /// Image child timeout.
    pub image_timeout_seconds: u64,
    /// Video child timeout.
    pub video_timeout_seconds: u64,
    /// Credential-free sandbox executable path.
    pub sandbox_binary: std::path::PathBuf,
    /// Parent directory for per-job private temporary directories.
    pub temp_directory: std::path::PathBuf,
    /// Total in-process temporary-disk reservation shared by active jobs.
    pub max_temp_disk_bytes: u64,
    /// Initial and renewed job lease duration.
    pub lease_seconds: u64,
    /// Lease renewal interval while a job is active.
    pub heartbeat_seconds: u64,
    /// Empty-queue polling delay.
    pub poll_interval_ms: u64,
    /// Initial bounded retry delay.
    pub retry_delay_seconds: u64,
    /// Attempts before a transient job enters dead-letter.
    pub max_attempts: u32,
    /// Maximum JSON bytes accepted from the sandbox process.
    pub max_sandbox_output_bytes: usize,
    /// Loopback-only Prometheus listener for the long-running worker process.
    pub metrics_bind_addr: SocketAddr,
}

/// Single-node durable storage cleanup limits.
#[derive(Clone, Debug, Deserialize)]
pub struct LifecycleSettings {
    /// Unconfirmed upload reservation TTL.
    pub created_reservation_ttl_seconds: u64,
    /// Private rejected/failed source retention.
    pub rejected_source_retention_seconds: u64,
    /// Exclusive SQLite cleanup lease duration.
    pub cleanup_lease_seconds: u64,
    /// Delay before reclaiming a failed cleanup attempt.
    pub cleanup_retry_seconds: u64,
    /// Maximum object groups processed per invocation.
    pub cleanup_batch_size: usize,
    /// Attempt ceiling before manual intervention.
    pub cleanup_max_attempts: u32,
    /// Minimum repeated-observation age before explicit prune may delete an orphan.
    pub orphan_grace_period_seconds: u64,
    /// Provider keys requested per inventory page.
    pub inventory_page_size: u16,
    /// Pages scanned per namespace and inventory invocation.
    pub inventory_max_pages_per_run: usize,
    /// Audit tombstone retention before physical row removal.
    pub tombstone_retention_seconds: u64,
    /// Maximum old tombstones purged per cleanup invocation.
    pub tombstone_purge_batch_size: usize,
}

/// Bounded private derivative delivery settings.
#[derive(Clone, Debug, Deserialize)]
pub struct DeliverySettings {
    /// Short-lived provider GET signature lifetime.
    pub signed_url_ttl_seconds: u64,
    /// Maximum immutable manifest age in process memory.
    pub manifest_cache_ttl_seconds: u64,
    /// Approximate total manifest cache weight in bytes.
    pub manifest_cache_max_bytes: u64,
}

/// Versioned watermark settings loaded only by the Rust worker.
#[derive(Clone, Debug, Deserialize)]
pub struct WatermarkSettings {
    /// Enables watermarking for image derivatives and video poster derivatives.
    pub enabled: bool,
    /// Registered local image path; copied into each private job directory.
    pub asset_path: std::path::PathBuf,
    /// Lowercase SHA-256 pin checked before every render.
    pub asset_sha256: String,
    /// Safe revision included in the immutable preset ID.
    pub preset_revision: String,
    /// Allowlisted anchor.
    pub position: WatermarkPositionSetting,
    /// Edge margin.
    pub margin_px: u32,
    /// Maximum width percentage.
    pub max_width_percent: u8,
    /// Alpha multiplier percentage.
    pub opacity_percent: u8,
}

/// Config-file representation of the five supported watermark anchors.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkPositionSetting {
    /// Centered.
    Center,
    /// Upper-left.
    TopLeft,
    /// Upper-right.
    TopRight,
    /// Lower-left.
    BottomLeft,
    /// Lower-right.
    BottomRight,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use secrecy::{ExposeSecret, SecretString};
    use tempfile::{NamedTempFile, tempdir};

    use super::{Settings, StorageProvider, StorageSettings};

    #[test]
    fn storage_provider_contract_is_fail_closed() {
        let mut storage = StorageSettings {
            provider: StorageProvider::R2,
            endpoint_url: Some(
                "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com".to_owned(),
            ),
            region: "auto".to_owned(),
            raw_bucket: "private-raw".to_owned(),
            derivative_bucket: "private-media".to_owned(),
            access_key_id: SecretString::from("test-access"),
            access_key_id_file: None,
            secret_access_key: SecretString::from("test-secret"),
            secret_access_key_file: None,
            force_path_style: false,
        };
        assert!(storage.validate_provider_contract().is_ok());

        storage.region = "us-east-1".to_owned();
        assert!(storage.validate_provider_contract().is_err());
        storage.provider = StorageProvider::Generic;
        assert!(storage.validate_provider_contract().is_ok());
        storage.endpoint_url = Some("http://storage.example.com".to_owned());
        assert!(storage.validate_provider_contract().is_err());

        storage.provider = StorageProvider::Lightsail;
        storage.endpoint_url = None;
        storage.region = "ap-northeast-2".to_owned();
        assert!(storage.validate_provider_contract().is_err());
        storage.derivative_bucket = storage.raw_bucket.clone();
        assert!(storage.validate_provider_contract().is_ok());
    }

    #[test]
    fn storage_provider_is_required_in_toml() -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"
"#,
        )?;
        let error = Settings::load(Some(file.path()))
            .err()
            .ok_or_else(|| std::io::Error::other("missing storage provider was accepted"))?;
        assert!(error.to_string().contains("provider"));
        Ok(())
    }

    #[test]
    fn loads_toml_and_redacts_secrets() -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "r2"
endpoint_url = "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"
"#,
        )?;
        let settings = Settings::load(Some(file.path()))?;
        assert_eq!(
            settings.storage.access_key_id.expose_secret(),
            "test-access"
        );
        let debug = format!("{settings:?}");
        assert!(!debug.contains("test-secret"));
        Ok(())
    }

    #[test]
    fn loads_root_owned_secret_files_and_rejects_inline_conflicts()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let access = directory.path().join("access");
        let secret = directory.path().join("secret");
        let hmac = directory.path().join("hmac");
        fs::write(&access, "test-access\n")?;
        fs::write(&secret, "test-secret\n")?;
        fs::write(&hmac, "0123456789abcdef0123456789abcdef\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            for path in [&access, &secret, &hmac] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            }
        }
        let config = directory.path().join("g7mb.toml");
        fs::write(
            &config,
            format!(
                r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id_file = "{}"
secret_access_key_file = "{}"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret_file = "{}"
"#,
                access.display(),
                secret.display(),
                hmac.display()
            ),
        )?;
        let settings = Settings::load(Some(&config))?;
        assert_eq!(
            settings.storage.access_key_id.expose_secret(),
            "test-access"
        );
        assert_eq!(
            settings.auth.hmac_secret.expose_secret(),
            "0123456789abcdef0123456789abcdef"
        );

        let conflicted = fs::read_to_string(&config)?.replace(
            "access_key_id_file =",
            "access_key_id = \"inline\"\naccess_key_id_file =",
        );
        fs::write(&config, conflicted)?;
        let error = Settings::load(Some(&config))
            .err()
            .ok_or_else(|| std::io::Error::other("inline/file conflict was accepted"))?;
        assert!(error.to_string().contains("mutually exclusive"));
        Ok(())
    }

    #[test]
    fn enabled_watermark_requires_an_exact_lowercase_sha256_pin()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[watermark]
enabled = true
asset_path = "/etc/g7mediabooster/watermark.png"
asset_sha256 = "NOT-A-SHA256"
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(
                    std::io::Error::other("watermark pin was unexpectedly accepted").into(),
                );
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("watermark settings"));
        Ok(())
    }

    #[test]
    fn reserved_storage_quota_must_cover_one_batch_and_fit_sqlite()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[upload]
max_reserved_bytes_global = 21474836480
max_reserved_bytes_per_tenant = 1024
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(std::io::Error::other("invalid quota was accepted").into());
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("upload settings"));
        Ok(())
    }

    #[test]
    fn database_backup_path_and_retention_are_operator_bounded()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[database]
backup_directory = "relative/backups"
backup_retention_count = 1
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(std::io::Error::other("unsafe backup policy was accepted").into());
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("database settings"));
        Ok(())
    }

    #[test]
    fn api_rate_burst_and_concurrency_cannot_be_unbounded() -> Result<(), Box<dyn std::error::Error>>
    {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[server]
rate_limit_requests_per_second = 100
rate_limit_burst = 10
max_in_flight_requests = 0
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(
                    std::io::Error::other("unsafe API admission policy was accepted").into(),
                );
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("server settings"));
        Ok(())
    }

    #[test]
    fn resource_lane_concurrency_cannot_exceed_general_worker_slots()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[worker]
max_concurrent_jobs = 2
max_concurrent_heavy_images = 3
max_concurrent_videos = 1
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(std::io::Error::other(
                    "oversubscribed heavy-image lane was unexpectedly accepted",
                )
                .into());
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("worker settings"));
        Ok(())
    }

    #[test]
    fn temporary_disk_cap_must_fit_one_worst_case_job() -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[upload]
max_image_bytes = 1073741824
max_video_bytes = 536870912

[worker]
max_temp_disk_bytes = 2684354560
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(std::io::Error::other(
                    "temporary disk cap smaller than one image job was accepted",
                )
                .into());
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("worker settings"));
        Ok(())
    }

    #[test]
    fn lifecycle_cleanup_bounds_cannot_be_disabled_or_oversubscribed()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[lifecycle]
cleanup_batch_size = 101
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(std::io::Error::other(
                    "unbounded cleanup batch was unexpectedly accepted",
                )
                .into());
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("lifecycle settings"));
        Ok(())
    }

    #[test]
    fn orphan_inventory_requires_a_grace_period_and_bounded_pages()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[lifecycle]
orphan_grace_period_seconds = 0
inventory_page_size = 1001
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(std::io::Error::other("unsafe inventory policy was accepted").into());
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("lifecycle settings"));
        Ok(())
    }

    #[test]
    fn derivative_manifest_cache_requires_bounded_bytes_and_ttl()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
provider = "generic"
endpoint_url = "http://127.0.0.1:9000"
raw_bucket = "raw"
derivative_bucket = "media"
access_key_id = "test-access"
secret_access_key = "test-secret"

[auth]
key_id = "g7-primary"
tenant_id = "site-a"
hmac_secret = "0123456789abcdef0123456789abcdef"

[delivery]
manifest_cache_ttl_seconds = 600
manifest_cache_max_bytes = 1024
"#,
        )?;
        let error = match Settings::load(Some(file.path())) {
            Ok(_) => {
                return Err(std::io::Error::other(
                    "unbounded derivative manifest cache was unexpectedly accepted",
                )
                .into());
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("delivery settings"));
        Ok(())
    }
}
