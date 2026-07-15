//! Typed, redacted configuration loaded from TOML and environment variables.

use std::{net::SocketAddr, path::Path};

use config::{Config, ConfigError, Environment, File, FileFormat};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

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
            .set_default("worker.lease_seconds", 120_i64)?
            .set_default("worker.heartbeat_seconds", 30_i64)?
            .set_default("worker.poll_interval_ms", 250_i64)?
            .set_default("worker.retry_delay_seconds", 10_i64)?
            .set_default("worker.max_attempts", 3_i64)?
            .set_default("worker.max_sandbox_output_bytes", 65_536_i64)?
            .set_default("lifecycle.created_reservation_ttl_seconds", 86_400_i64)?
            .set_default("lifecycle.rejected_source_retention_seconds", 604_800_i64)?
            .set_default("lifecycle.cleanup_lease_seconds", 300_i64)?
            .set_default("lifecycle.cleanup_retry_seconds", 60_i64)?
            .set_default("lifecycle.cleanup_batch_size", 100_i64)?
            .set_default("lifecycle.cleanup_max_attempts", 10_i64)?
            .set_default("lifecycle.orphan_grace_period_seconds", 172_800_i64)?
            .set_default("lifecycle.inventory_page_size", 1_000_i64)?
            .set_default("lifecycle.inventory_max_pages_per_run", 10_i64)?
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

        let settings: Self = builder
            .add_source(
                Environment::with_prefix("G7MB")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?
            .try_deserialize()?;
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&self) -> Result<(), ConfigError> {
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
            || self.worker.sandbox_binary.as_os_str().is_empty()
            || self.worker.temp_directory.as_os_str().is_empty()
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

/// HTTP server configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct ServerSettings {
    /// Loopback or private listener address.
    pub bind_addr: SocketAddr,
    /// Maximum JSON request body.
    pub request_body_limit_bytes: usize,
}

/// One authenticated PHP control client for the initial single-site deployment.
#[derive(Clone, Debug, Deserialize)]
pub struct AuthSettings {
    /// Public key identifier included in signed requests.
    pub key_id: String,
    /// Tenant/site identifier used in object keys and state isolation.
    pub tenant_id: String,
    /// HMAC-SHA256 secret, redacted in debug output.
    pub hmac_secret: SecretString,
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
    /// Optional custom endpoint, required for R2.
    pub endpoint_url: Option<String>,
    /// AWS region or `auto` for R2.
    pub region: String,
    /// Private raw/quarantine bucket.
    pub raw_bucket: String,
    /// Derivative output bucket.
    pub derivative_bucket: String,
    /// Static access key identifier, redacted in debug output.
    pub access_key_id: SecretString,
    /// Static secret key, redacted in debug output.
    pub secret_access_key: SecretString,
    /// Enables path-style requests for compatible local services.
    pub force_path_style: bool,
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

    use secrecy::ExposeSecret;
    use tempfile::NamedTempFile;

    use super::Settings;

    #[test]
    fn loads_toml_and_redacts_secrets() -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
endpoint_url = "https://example.r2.cloudflarestorage.com"
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
    fn enabled_watermark_requires_an_exact_lowercase_sha256_pin()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
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
    fn resource_lane_concurrency_cannot_exceed_general_worker_slots()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
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
    fn lifecycle_cleanup_bounds_cannot_be_disabled_or_oversubscribed()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = NamedTempFile::new()?;
        fs::write(
            file.path(),
            r#"
[storage]
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
