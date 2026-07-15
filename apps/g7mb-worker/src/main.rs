//! Durable worker process entrypoint and dependency doctor.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand};
use g7mb_application::{
    WatermarkPosition,
    lifecycle::{LifecyclePolicy, LifecycleService},
};
use g7mb_config::{Settings, WatermarkPositionSetting};
use g7mb_object_store_s3::S3CompatibleStore;
use g7mb_persistence_sqlite::SqliteStore;
use g7mb_worker::{
    ProcessSandboxProbe, RunOutcome, SourceValidationWorker, WatermarkPolicy, WorkerPolicy,
};
use tokio::{sync::watch, task::JoinSet};

#[derive(Debug, Parser)]
#[command(version, about = "G7MediaBooster durable worker")]
struct Cli {
    /// TOML configuration file.
    #[arg(long, env = "G7MB_CONFIG")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Verify configuration, SQLite, S3/R2 client construction, and native tools.
    Doctor,
    /// Runs bounded source-validation slots until SIGINT/SIGTERM.
    Run {
        /// Stable process identifier; slot suffixes are added automatically.
        #[arg(long)]
        worker_id: String,
    },
    /// Claims and processes at most one job for smoke tests and operations.
    Once {
        /// Stable worker identifier.
        #[arg(long)]
        worker_id: String,
    },
    /// Cleans one bounded batch of expired, rejected, failed, or user-deleted objects.
    Cleanup {
        /// Stable cleanup lease owner identifier.
        #[arg(long)]
        worker_id: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    g7mb_telemetry::init_tracing()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor => doctor(&cli.config).await,
        Command::Run { worker_id } => run(&cli.config, &worker_id).await,
        Command::Once { worker_id } => once(&cli.config, &worker_id).await,
        Command::Cleanup { worker_id } => cleanup(&cli.config, &worker_id).await,
    }
}

async fn cleanup(path: &std::path::Path, worker_id: &str) -> anyhow::Result<()> {
    let settings = Settings::load(Some(path)).context("failed to load configuration")?;
    let database = Arc::new(
        SqliteStore::connect(&settings.database.url, settings.database.max_connections)
            .await
            .context("failed to initialize SQLite")?,
    );
    let raw_store = Arc::new(
        S3CompatibleStore::for_raw_bucket(&settings.storage)
            .await
            .context("failed to initialize raw object store")?,
    );
    let derivative_store = Arc::new(
        S3CompatibleStore::for_derivative_bucket(&settings.storage)
            .await
            .context("failed to initialize derivative object store")?,
    );
    let lifecycle = LifecycleService::new(
        raw_store,
        derivative_store,
        database,
        lifecycle_policy(&settings),
    )?;
    let summary = lifecycle
        .run_once(worker_id)
        .await
        .context("lifecycle cleanup failed")?;
    tracing::info!(
        claimed = summary.claimed,
        completed = summary.completed,
        failed = summary.failed,
        dead_lettered = summary.dead_lettered,
        "lifecycle cleanup completed"
    );
    Ok(())
}

fn lifecycle_policy(settings: &Settings) -> LifecyclePolicy {
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
    }
}

async fn once(path: &std::path::Path, worker_id: &str) -> anyhow::Result<()> {
    let settings = Settings::load(Some(path)).context("failed to load configuration")?;
    let worker = build_worker(&settings).await?;
    let outcome = worker
        .run_one(worker_id)
        .await
        .context("source-validation job failed")?;
    tracing::info!(?outcome, "worker once completed");
    Ok(())
}

async fn run(path: &std::path::Path, worker_id: &str) -> anyhow::Result<()> {
    if worker_id.is_empty() || worker_id.len() > 112 {
        bail!("worker_id must contain 1..=112 characters");
    }
    let settings = Settings::load(Some(path)).context("failed to load configuration")?;
    let worker = build_worker(&settings).await?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut slots = JoinSet::new();
    for slot in 0..settings.worker.max_concurrent_jobs {
        let worker = worker.clone();
        let slot_id = format!("{worker_id}-{slot}");
        let shutdown = shutdown_rx.clone();
        let poll_interval = Duration::from_millis(settings.worker.poll_interval_ms);
        slots.spawn(async move { run_slot(worker, slot_id, poll_interval, shutdown).await });
    }

    tokio::select! {
        signal = shutdown_signal() => {
            signal.context("failed to listen for shutdown signal")?;
            tracing::info!("worker shutdown requested");
        }
        result = slots.join_next() => {
            match result {
                Some(Ok(Ok(()))) => bail!("worker slot stopped unexpectedly"),
                Some(Ok(Err(error))) => return Err(error).context("worker slot failed"),
                Some(Err(error)) => return Err(error).context("worker slot panicked"),
                None => bail!("worker started no slots"),
            }
        }
    }
    let _send_result = shutdown_tx.send(true);
    while let Some(result) = slots.join_next().await {
        result.context("worker slot join failed")??;
    }
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}

async fn run_slot(
    worker: SourceValidationWorker,
    worker_id: String,
    poll_interval: Duration,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        match worker.run_one(&worker_id).await? {
            RunOutcome::Idle => {
                tokio::select! {
                    _ = tokio::time::sleep(poll_interval) => {}
                    changed = shutdown.changed() => {
                        changed.context("worker shutdown channel closed")?;
                    }
                }
            }
            outcome => tracing::info!(?outcome, worker_id, "worker processed queue job"),
        }
    }
}

async fn build_worker(settings: &Settings) -> anyhow::Result<SourceValidationWorker> {
    let database = Arc::new(
        SqliteStore::connect(&settings.database.url, settings.database.max_connections)
            .await
            .context("failed to initialize SQLite")?,
    );
    let raw_store = Arc::new(
        S3CompatibleStore::for_raw_bucket(&settings.storage)
            .await
            .context("failed to initialize raw object store")?,
    );
    let derivative_store = Arc::new(
        S3CompatibleStore::for_derivative_bucket(&settings.storage)
            .await
            .context("failed to initialize derivative object store")?,
    );
    let sandbox = Arc::new(ProcessSandboxProbe::new(
        settings.worker.sandbox_binary.clone(),
        Duration::from_secs(settings.worker.image_timeout_seconds),
        Duration::from_secs(settings.worker.video_timeout_seconds),
        settings.worker.native_threads_per_job,
        settings.worker.max_sandbox_output_bytes,
    )?);
    let watermark = settings.watermark.enabled.then(|| WatermarkPolicy {
        asset_path: settings.watermark.asset_path.clone(),
        asset_sha256: settings.watermark.asset_sha256.clone(),
        preset_revision: settings.watermark.preset_revision.clone(),
        position: match settings.watermark.position {
            WatermarkPositionSetting::Center => WatermarkPosition::Center,
            WatermarkPositionSetting::TopLeft => WatermarkPosition::TopLeft,
            WatermarkPositionSetting::TopRight => WatermarkPosition::TopRight,
            WatermarkPositionSetting::BottomLeft => WatermarkPosition::BottomLeft,
            WatermarkPositionSetting::BottomRight => WatermarkPosition::BottomRight,
        },
        margin_px: settings.watermark.margin_px,
        max_width_percent: settings.watermark.max_width_percent,
        opacity_percent: settings.watermark.opacity_percent,
    });
    SourceValidationWorker::new(
        raw_store,
        derivative_store,
        database.clone(),
        database.clone(),
        database,
        sandbox,
        WorkerPolicy {
            lease_for: Duration::from_secs(settings.worker.lease_seconds),
            heartbeat_every: Duration::from_secs(settings.worker.heartbeat_seconds),
            retry_delay: Duration::from_secs(settings.worker.retry_delay_seconds),
            max_attempts: settings.worker.max_attempts,
            max_concurrent_heavy_images: settings.worker.max_concurrent_heavy_images,
            max_concurrent_videos: settings.worker.max_concurrent_videos,
            max_image_bytes: settings.upload.max_image_bytes,
            max_video_bytes: settings.upload.max_video_bytes,
            temp_directory: settings.worker.temp_directory.clone(),
            watermark,
        },
    )
    .context("worker policy is invalid")
}

async fn doctor(path: &std::path::Path) -> anyhow::Result<()> {
    let settings = Settings::load(Some(path)).context("failed to load configuration")?;
    let _database = SqliteStore::connect(&settings.database.url, settings.database.max_connections)
        .await
        .context("failed to initialize SQLite")?;
    let _raw_store = S3CompatibleStore::for_raw_bucket(&settings.storage)
        .await
        .context("failed to initialize raw object store")?;
    let _derivative_store = S3CompatibleStore::for_derivative_bucket(&settings.storage)
        .await
        .context("failed to initialize derivative object store")?;
    let reports = g7mb_media::native_tool_report().await?;
    for report in &reports {
        tracing::info!(tool = %report.tool, version = %report.version, healthy = report.healthy, warning = ?report.warning, "native tool report");
    }
    if reports.iter().any(|report| !report.healthy) {
        bail!("native tool loader health check failed");
    }
    tracing::info!("worker dependency doctor passed");
    Ok(())
}
