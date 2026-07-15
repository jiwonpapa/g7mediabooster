//! G7MediaBooster control-plane process entrypoint.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context as _;
use clap::Parser;
use g7mb_api::{ApiAuth, ApiRateLimitPolicy, ApiState, probe_sandbox_capabilities, router};
use g7mb_application::{
    delivery::{DerivativeDeliveryPolicy, DerivativeDeliveryService},
    lifecycle::{LifecyclePolicy, LifecycleService},
    policies::SitePolicyService,
    uploads::{UploadCapacityPolicy, UploadIntentService},
};
use g7mb_config::Settings;
use g7mb_domain::UploadBatchPolicy;
use g7mb_object_store_s3::S3CompatibleStore;
use g7mb_persistence_sqlite::SqliteStore;

#[derive(Debug, Parser)]
#[command(version, about = "G7MediaBooster HTTP control plane")]
struct Cli {
    /// TOML configuration file.
    #[arg(long, env = "G7MB_CONFIG")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    g7mb_telemetry::init_tracing()?;
    let cli = Cli::parse();
    let settings = Settings::load(Some(&cli.config)).context("failed to load configuration")?;
    let capabilities = probe_sandbox_capabilities(
        &settings.worker.sandbox_binary,
        Duration::from_secs(
            settings
                .worker
                .image_timeout_seconds
                .max(settings.worker.video_timeout_seconds),
        ),
        settings.worker.max_sandbox_output_bytes,
    )
    .await
    .context("required sandbox media capabilities are unavailable")?;
    let database = Arc::new(
        SqliteStore::connect(&settings.database.url, settings.database.max_connections)
            .await
            .context("failed to initialize SQLite")?,
    );
    let raw_store = Arc::new(
        S3CompatibleStore::for_raw_bucket(&settings.storage)
            .await
            .context("failed to initialize object storage")?,
    );
    let derivative_store = Arc::new(
        S3CompatibleStore::for_derivative_bucket(&settings.storage)
            .await
            .context("failed to initialize derivative object storage")?,
    );
    let upload_service = UploadIntentService::new(
        raw_store.clone(),
        database.clone(),
        UploadBatchPolicy {
            max_files: settings.upload.max_batch_files,
            max_batch_bytes: settings.upload.max_batch_bytes,
            max_image_bytes: settings.upload.max_image_bytes,
            max_video_bytes: settings.upload.max_video_bytes,
            multipart_threshold_bytes: settings.upload.multipart_threshold_bytes,
            multipart_part_size_bytes: settings.upload.multipart_part_size_bytes,
        },
    )
    .with_capacity_policy(UploadCapacityPolicy {
        max_active_global: settings.upload.max_active_uploads_global,
        max_active_per_tenant: settings.upload.max_active_uploads_per_tenant,
        max_reserved_bytes_global: settings.upload.max_reserved_bytes_global,
        max_reserved_bytes_per_tenant: settings.upload.max_reserved_bytes_per_tenant,
    });
    let lifecycle_service = LifecycleService::new(
        raw_store,
        derivative_store.clone(),
        database.clone(),
        LifecyclePolicy {
            created_reservation_ttl: Duration::from_secs(
                settings.lifecycle.created_reservation_ttl_seconds,
            ),
            rejected_source_retention: Duration::from_secs(
                settings.lifecycle.rejected_source_retention_seconds,
            ),
            lease_for: Duration::from_secs(settings.lifecycle.cleanup_lease_seconds),
            retry_delay: Duration::from_secs(settings.lifecycle.cleanup_retry_seconds),
            batch_size: settings.lifecycle.cleanup_batch_size,
            max_attempts: settings.lifecycle.cleanup_max_attempts,
            tombstone_retention: Duration::from_secs(
                settings.lifecycle.tombstone_retention_seconds,
            ),
            tombstone_purge_batch_size: settings.lifecycle.tombstone_purge_batch_size,
        },
    )
    .context("failed to initialize lifecycle service")?;
    let delivery_service = DerivativeDeliveryService::new(
        database.clone(),
        derivative_store,
        DerivativeDeliveryPolicy {
            signed_url_ttl: Duration::from_secs(settings.delivery.signed_url_ttl_seconds),
            manifest_cache_ttl: Duration::from_secs(settings.delivery.manifest_cache_ttl_seconds),
            manifest_cache_max_bytes: settings.delivery.manifest_cache_max_bytes,
        },
    )
    .context("failed to initialize derivative delivery")?;
    let allowed_skew_seconds = i64::try_from(settings.auth.allowed_skew_seconds)
        .context("auth clock skew exceeds signed range")?;
    let auth = ApiAuth::new(
        settings.auth.key_id.clone(),
        settings.auth.tenant_id.clone(),
        settings.auth.hmac_secret.clone(),
        allowed_skew_seconds,
    );
    let metrics = g7mb_telemetry::install_metrics()?;
    let listener = tokio::net::TcpListener::bind(settings.server.bind_addr)
        .await
        .context("failed to bind API listener")?;
    tracing::info!(bind_addr = %settings.server.bind_addr, "g7mb API ready");
    axum::serve(
        listener,
        router(
            ApiState::new(true, Some(metrics))
                .with_rate_limit_policy(ApiRateLimitPolicy {
                    requests_per_second: settings.server.rate_limit_requests_per_second,
                    burst: settings.server.rate_limit_burst,
                    max_in_flight: settings.server.max_in_flight_requests,
                })
                .context("failed to initialize API rate limits")?
                .with_operational_observer(database.clone())
                .with_capabilities(capabilities)
                .with_derivative_delivery(delivery_service)
                .with_lifecycle(lifecycle_service)
                .with_upload_control(upload_service, database.clone(), auth)
                .with_site_policy(SitePolicyService::new(
                    database,
                    settings.auth.allowed_skew_seconds,
                )),
            settings.server.request_body_limit_bytes,
        ),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("API server failed")?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = wait_for_shutdown_signal().await {
        tracing::error!(%error, "failed to install shutdown signal handler");
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}
