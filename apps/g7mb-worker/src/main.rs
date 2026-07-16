//! Durable worker process entrypoint and dependency doctor.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand};
use g7mb_application::{
    WatermarkPosition,
    inventory::{InventoryMode, InventoryPolicy, InventoryService},
    lifecycle::{LifecyclePolicy, LifecycleService},
};
use g7mb_config::{Settings, WatermarkPositionSetting};
use g7mb_object_store_s3::{S3CompatibleStore, S3StorageAdmin};
use g7mb_persistence_sqlite::{SqliteStore, backup::verify_database_file};
use g7mb_worker::{
    ProcessSandboxProbe, RunOutcome, SourceValidationWorker, WatermarkPolicy, WorkerPolicy,
};
use sha2::{Digest as _, Sha256};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    sync::watch,
    task::JoinSet,
};

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
    /// Verify configuration, SQLite, live S3/R2 runtime operations, and native tools.
    Doctor {
        /// Only constructs storage clients; skips destructive-only-to-canary-key network checks.
        #[arg(long)]
        offline: bool,
    },
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
    /// Audits bounded raw/media provider pages or explicitly prunes old confirmed orphans.
    Inventory {
        /// Enables deletion after the grace period and a final database ownership recheck.
        #[arg(long)]
        prune: bool,
    },
    /// Creates, verifies, or rehearses restoration of SQLite snapshots.
    Database {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DatabaseCommand {
    /// Creates a verified online snapshot and SHA-256 manifest with bounded retention.
    Backup,
    /// Verifies an existing snapshot read-only against its expected SHA-256.
    Verify {
        /// Absolute snapshot path.
        #[arg(long)]
        input: PathBuf,
        /// Lowercase SHA-256 from the separately retained manifest.
        #[arg(long)]
        expected_sha256: String,
    },
    /// Copies a snapshot into an isolated directory and proves writable rollback plus invariants.
    RestoreRehearsal {
        /// Absolute snapshot path.
        #[arg(long)]
        input: PathBuf,
        /// Lowercase SHA-256 from the separately retained manifest.
        #[arg(long)]
        expected_sha256: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    g7mb_telemetry::init_tracing()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor { offline } => doctor(&cli.config, offline).await,
        Command::Run { worker_id } => run(&cli.config, &worker_id).await,
        Command::Once { worker_id } => once(&cli.config, &worker_id).await,
        Command::Cleanup { worker_id } => cleanup(&cli.config, &worker_id).await,
        Command::Inventory { prune } => inventory(&cli.config, prune).await,
        Command::Database { command } => database(&cli.config, command).await,
    }
}

async fn database(path: &std::path::Path, command: DatabaseCommand) -> anyhow::Result<()> {
    let settings = Settings::load(Some(path)).context("failed to load configuration")?;
    match command {
        DatabaseCommand::Backup => database_backup(&settings).await,
        DatabaseCommand::Verify {
            input,
            expected_sha256,
        } => database_verify(&input, &expected_sha256).await,
        DatabaseCommand::RestoreRehearsal {
            input,
            expected_sha256,
        } => database_restore_rehearsal(&input, &expected_sha256).await,
    }
}

async fn database_backup(settings: &Settings) -> anyhow::Result<()> {
    let directory = &settings.database.backup_directory;
    tokio::fs::create_dir_all(directory)
        .await
        .context("failed to create backup directory")?;
    let directory_metadata = tokio::fs::symlink_metadata(directory)
        .await
        .context("failed to inspect backup directory")?;
    if !directory_metadata.is_dir() || directory_metadata.file_type().is_symlink() {
        bail!("backup directory must be a real directory, not a symlink");
    }
    let created_at = time::OffsetDateTime::now_utc();
    let stem = format!("g7mb-{}", created_at.unix_timestamp_nanos());
    let snapshot = directory.join(format!("{stem}.db"));
    let partial_snapshot = directory.join(format!("{stem}.db.partial"));
    let manifest = directory.join(format!("{stem}.db.manifest.json"));
    let partial_manifest = directory.join(format!("{stem}.db.manifest.json.partial"));
    let store = SqliteStore::connect(&settings.database.url, settings.database.max_connections)
        .await
        .context("failed to initialize SQLite")?;
    let finalized: anyhow::Result<_> = async {
        let verification = store
            .backup_to(&partial_snapshot)
            .await
            .context("failed to create verified SQLite snapshot")?;
        secure_regular_file(&partial_snapshot).await?;
        let sha256 = sha256_file(&partial_snapshot).await?;
        let bytes = tokio::fs::metadata(&partial_snapshot).await?.len();
        let body = serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "created_at_unix": created_at.unix_timestamp(),
            "snapshot_file": snapshot.file_name().and_then(|value| value.to_str()),
            "sha256": sha256,
            "bytes": bytes,
            "database_schema_version": verification.schema_version,
            "uploads": verification.uploads,
            "derivatives": verification.derivatives,
            "jobs": verification.jobs,
            "orphan_suspects": verification.orphan_suspects,
            "reserved_source_bytes": verification.reserved_source_bytes,
        }))?;
        write_create_new(&partial_manifest, &body).await?;
        tokio::fs::hard_link(&partial_snapshot, &snapshot)
            .await
            .context("failed to publish create-new backup snapshot")?;
        tokio::fs::hard_link(&partial_manifest, &manifest)
            .await
            .context("failed to publish create-new backup manifest")?;
        tokio::fs::remove_file(&partial_snapshot).await?;
        tokio::fs::remove_file(&partial_manifest).await?;
        Ok((verification, sha256, bytes))
    }
    .await;
    let (verification, sha256, bytes) = match finalized {
        Ok(value) => value,
        Err(error) => {
            remove_if_present(&partial_snapshot).await;
            remove_if_present(&partial_manifest).await;
            remove_if_present(&snapshot).await;
            remove_if_present(&manifest).await;
            return Err(error);
        }
    };
    rotate_backups(directory, settings.database.backup_retention_count).await?;
    tracing::info!(
        path = %snapshot.display(),
        manifest = %manifest.display(),
        sha256,
        bytes,
        schema_version = verification.schema_version,
        uploads = verification.uploads,
        "verified database backup completed"
    );
    Ok(())
}

async fn remove_if_present(path: &std::path::Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "failed to remove partial backup output")
        }
    }
}

async fn database_verify(input: &std::path::Path, expected_sha256: &str) -> anyhow::Result<()> {
    validate_snapshot_input(input, expected_sha256).await?;
    let verification = verify_database_file(input)
        .await
        .context("database snapshot verification failed")?;
    tracing::info!(
        path = %input.display(),
        schema_version = verification.schema_version,
        uploads = verification.uploads,
        derivatives = verification.derivatives,
        jobs = verification.jobs,
        orphan_suspects = verification.orphan_suspects,
        reserved_source_bytes = verification.reserved_source_bytes,
        "database snapshot verification completed"
    );
    Ok(())
}

async fn database_restore_rehearsal(
    input: &std::path::Path,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    validate_snapshot_input(input, expected_sha256).await?;
    let expected = verify_database_file(input)
        .await
        .context("restore source verification failed")?;
    let directory = tempfile::tempdir().context("failed to create restore rehearsal directory")?;
    let candidate = directory.path().join("restored.db");
    copy_create_new(input, &candidate).await?;
    let candidate_url = format!("sqlite://{}", candidate.display());
    let restored = SqliteStore::connect(&candidate_url, 1)
        .await
        .context("failed to open restored candidate writable")?;
    let mut transaction = restored.pool().begin().await?;
    sqlx::query(
        "CREATE TABLE restore_rehearsal_canary (
            id INTEGER PRIMARY KEY NOT NULL,
            created_at INTEGER NOT NULL
         )",
    )
    .execute(&mut *transaction)
    .await?;
    transaction.rollback().await?;
    let actual = restored.verify_database().await?;
    restored.pool().close().await;
    if actual != expected {
        bail!("restored candidate counts differ from the verified snapshot");
    }
    tracing::info!(
        schema_version = actual.schema_version,
        uploads = actual.uploads,
        reserved_source_bytes = actual.reserved_source_bytes,
        "isolated database restore rehearsal completed"
    );
    Ok(())
}

async fn validate_snapshot_input(
    input: &std::path::Path,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    if !input.is_absolute()
        || expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("snapshot path or expected SHA-256 is invalid");
    }
    let metadata = tokio::fs::symlink_metadata(input)
        .await
        .context("failed to inspect snapshot")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        bail!("snapshot must be a non-empty regular file, not a symlink");
    }
    let actual = sha256_file(input).await?;
    if actual != expected_sha256 {
        bail!("snapshot SHA-256 does not match the expected manifest value");
    }
    Ok(())
}

async fn sha256_file(path: &std::path::Path) -> anyhow::Result<String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .context("failed to open file for SHA-256")?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .context("failed to hash file")?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

async fn write_create_new(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await
        .context("failed to create backup manifest")?;
    file.write_all(bytes)
        .await
        .context("failed to write backup manifest")?;
    file.sync_all()
        .await
        .context("failed to sync backup manifest")?;
    drop(file);
    secure_regular_file(path).await
}

async fn copy_create_new(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> anyhow::Result<()> {
    let mut source = tokio::fs::File::open(source)
        .await
        .context("failed to open restore source")?;
    let mut destination = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .await
        .context("failed to create restore candidate")?;
    tokio::io::copy(&mut source, &mut destination)
        .await
        .context("failed to copy restore candidate")?;
    destination
        .sync_all()
        .await
        .context("failed to sync restore candidate")?;
    Ok(())
}

async fn secure_regular_file(path: &std::path::Path) -> anyhow::Result<()> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .context("failed to inspect generated backup file")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("generated backup output is not a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .await
            .context("failed to restrict backup file permissions")?;
    }
    tokio::fs::File::open(path)
        .await
        .context("failed to reopen generated backup file")?
        .sync_all()
        .await
        .context("failed to sync generated backup file")?;
    Ok(())
}

async fn rotate_backups(directory: &std::path::Path, retain: usize) -> anyhow::Result<()> {
    let mut entries = tokio::fs::read_dir(directory)
        .await
        .context("failed to enumerate backup retention directory")?;
    let mut snapshots = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .context("failed to read backup retention entry")?
    {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("g7mb-") || !name.ends_with(".db") {
            continue;
        }
        let file_type = entry
            .file_type()
            .await
            .context("failed to inspect backup retention entry")?;
        if file_type.is_file() && !file_type.is_symlink() {
            snapshots.push(entry.path());
        }
    }
    snapshots.sort_unstable();
    let remove_count = snapshots.len().saturating_sub(retain);
    for snapshot in snapshots.into_iter().take(remove_count) {
        let manifest = snapshot.with_extension("db.manifest.json");
        tokio::fs::remove_file(&snapshot)
            .await
            .context("failed to remove expired local snapshot")?;
        match tokio::fs::remove_file(&manifest).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("failed to remove expired backup manifest"),
        }
    }
    Ok(())
}

async fn inventory(path: &std::path::Path, prune: bool) -> anyhow::Result<()> {
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
    let service = InventoryService::new(
        raw_store,
        derivative_store,
        database,
        InventoryPolicy {
            orphan_grace_period: Duration::from_secs(
                settings.lifecycle.orphan_grace_period_seconds,
            ),
            page_size: settings.lifecycle.inventory_page_size,
            max_pages_per_namespace: settings.lifecycle.inventory_max_pages_per_run,
        },
    )?;
    let mode = if prune {
        InventoryMode::Prune
    } else {
        InventoryMode::Audit
    };
    let summary = service
        .run_once(mode)
        .await
        .context("provider inventory failed")?;
    tracing::info!(
        ?mode,
        pages = summary.pages,
        listed = summary.listed,
        invalid_keys = summary.invalid_keys,
        known = summary.known,
        suspected = summary.suspected,
        eligible = summary.eligible,
        deleted = summary.deleted,
        delete_failed = summary.delete_failed,
        completed_namespaces = summary.completed_namespaces,
        "provider inventory completed"
    );
    Ok(())
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
        tombstone_retention: Duration::from_secs(settings.lifecycle.tombstone_retention_seconds),
        tombstone_purge_batch_size: settings.lifecycle.tombstone_purge_batch_size,
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
    g7mb_telemetry::install_metrics_http(settings.worker.metrics_bind_addr)
        .context("failed to start worker metrics listener")?;
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
            max_temp_disk_bytes: settings.worker.max_temp_disk_bytes,
            temp_directory: settings.worker.temp_directory.clone(),
            watermark,
        },
    )
    .context("worker policy is invalid")
}

async fn doctor(path: &std::path::Path, offline: bool) -> anyhow::Result<()> {
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
    if !offline {
        let report = S3StorageAdmin::new(&settings.storage)?
            .canary(&settings.storage)
            .await
            .context("live S3/R2 runtime canary failed")?;
        tracing::info!(
            buckets = report.buckets_checked,
            single_object = report.single_object,
            multipart = report.multipart,
            "storage runtime canary passed"
        );
    }
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

#[cfg(test)]
mod tests {
    use g7mb_config::Settings;
    use g7mb_persistence_sqlite::SqliteStore;

    use super::{database_backup, database_restore_rehearsal, database_verify};

    #[tokio::test]
    async fn backup_rotation_hash_verification_and_restore_rehearsal_are_end_to_end()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let database = directory.path().join("live.db");
        let backups = directory.path().join("backups");
        let config = directory.path().join("g7mb.toml");
        std::fs::write(
            &config,
            format!(
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
url = "sqlite://{}"
max_connections = 2
backup_directory = "{}"
backup_retention_count = 2
"#,
                database.display(),
                backups.display()
            ),
        )?;
        let settings = Settings::load(Some(&config))?;
        let store = SqliteStore::connect(&settings.database.url, 2).await?;
        store.pool().close().await;

        database_backup(&settings).await?;
        database_backup(&settings).await?;
        database_backup(&settings).await?;

        let mut snapshots = Vec::new();
        let mut manifests = Vec::new();
        for entry in std::fs::read_dir(&backups)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if name.ends_with(".db") {
                snapshots.push(path);
            } else if name.ends_with(".db.manifest.json") {
                manifests.push(path);
            }
        }
        snapshots.sort_unstable();
        manifests.sort_unstable();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(manifests.len(), 2);

        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(manifests.last().ok_or("manifest missing")?)?)?;
        let sha256 = manifest
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or("manifest SHA-256 missing")?;
        let snapshot = snapshots.last().ok_or("snapshot missing")?;
        database_verify(snapshot, sha256).await?;
        database_restore_rehearsal(snapshot, sha256).await?;
        Ok(())
    }
}
