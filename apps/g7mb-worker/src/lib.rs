//! Bounded durable source-validation worker orchestration.

use std::{
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use g7mb_application::{
    DownloadObjectRequest, JobFailureDisposition, JobQueue, ObjectStore, ObjectStoreError,
    PutFileRequest, WatermarkPosition,
    policies::{SitePolicyRepository, StoredWatermarkPolicy},
    processing::{ProcessingRepository, ProcessingSource, PublishedDerivative},
};
use g7mb_domain::{ImageWorkClass, MediaKind, ObjectKey, UploadState};
use g7mb_media::{MediaFormat, MediaInspection, detect_file};
use metrics::{counter, gauge, histogram};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    process::Command,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{MissedTickBehavior, interval},
};

const SOURCE_VALIDATION_PRESET: &str = "source-validation-v1";
const DEFAULT_DERIVATIVE_PRESET: &str = "board-default-v1";
const SANITIZED_MASTER_MAX_EDGE: u32 = 8192;
const MAX_WATERMARK_BYTES: u64 = 16 * 1024 * 1024;
const TEMP_DISK_PERMIT_BYTES: u64 = 1024 * 1024;
const MAX_TEMP_DISK_BYTES: u64 = 1024 * 1024 * 1024 * 1024;

fn required_temp_disk_bytes(max_image_bytes: u64, max_video_bytes: u64) -> Option<u64> {
    max_image_bytes
        .checked_mul(2)?
        .checked_add(max_image_bytes.max(max_video_bytes))?
        .checked_add(MAX_WATERMARK_BYTES)
}

/// Stable result of attempting one queue claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    /// No eligible work existed.
    Idle,
    /// Validation passed and durable facts were recorded.
    Completed,
    /// Deterministic unsafe or unsupported media was rejected.
    Rejected,
    /// A transient failure was returned to the queue.
    RetryScheduled,
    /// The transient attempt cap was reached.
    DeadLetter,
}

/// Worker resource and retry policy.
#[derive(Clone, Debug)]
pub struct WorkerPolicy {
    /// Job lease duration.
    pub lease_for: Duration,
    /// Lease renewal period.
    pub heartbeat_every: Duration,
    /// Retry delay after transient failure.
    pub retry_delay: Duration,
    /// Maximum claims before dead-letter.
    pub max_attempts: u32,
    /// Maximum simultaneous full-pixel transformations for heavy images.
    pub max_concurrent_heavy_images: usize,
    /// Maximum simultaneous video frame transformations.
    pub max_concurrent_videos: usize,
    /// Largest image download.
    pub max_image_bytes: u64,
    /// Largest video download.
    pub max_video_bytes: u64,
    /// Total temporary-disk reservation shared by jobs in this worker process.
    pub max_temp_disk_bytes: u64,
    /// Parent of private per-job directories.
    pub temp_directory: PathBuf,
    /// Optional versioned and digest-pinned watermark preset.
    pub watermark: Option<WatermarkPolicy>,
}

/// Trusted operational watermark policy. Browser requests cannot alter these values.
#[derive(Clone, Debug)]
pub struct WatermarkPolicy {
    /// Local registered asset read by the worker and copied into each private job directory.
    pub asset_path: PathBuf,
    /// Lowercase SHA-256 pin of the registered asset.
    pub asset_sha256: String,
    /// Safe identifier included in immutable derivative keys.
    pub preset_revision: String,
    /// Allowlisted overlay anchor.
    pub position: WatermarkPosition,
    /// Safe edge margin.
    pub margin_px: u32,
    /// Maximum derivative width percentage.
    pub max_width_percent: u8,
    /// Final alpha multiplier percentage.
    pub opacity_percent: u8,
}

impl WorkerPolicy {
    /// Rejects unsafe or internally inconsistent worker limits.
    pub fn validate(&self) -> Result<(), WorkerError> {
        if self.lease_for.is_zero()
            || self.heartbeat_every.is_zero()
            || self.lease_for <= self.heartbeat_every.saturating_mul(2)
            || self.retry_delay.is_zero()
            || self.max_attempts == 0
            || self.max_concurrent_heavy_images == 0
            || self.max_concurrent_heavy_images > 32
            || self.max_concurrent_videos == 0
            || self.max_concurrent_videos > 32
            || self.max_image_bytes == 0
            || self.max_video_bytes == 0
            || required_temp_disk_bytes(self.max_image_bytes, self.max_video_bytes)
                .is_none_or(|required| self.max_temp_disk_bytes < required)
            || self.max_temp_disk_bytes > MAX_TEMP_DISK_BYTES
            || temp_disk_permits(self.max_temp_disk_bytes).is_none()
            || self.temp_directory.as_os_str().is_empty()
            || self
                .watermark
                .as_ref()
                .is_some_and(|watermark| !watermark.is_valid())
        {
            return Err(WorkerError::InvalidPolicy);
        }
        Ok(())
    }
}

impl WatermarkPolicy {
    fn is_valid(&self) -> bool {
        !self.asset_path.as_os_str().is_empty()
            && self.asset_sha256.len() == 64
            && self
                .asset_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && (1..=32).contains(&self.preset_revision.len())
            && self
                .preset_revision
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            && self.margin_px <= 1024
            && (1..=50).contains(&self.max_width_percent)
            && (1..=100).contains(&self.opacity_percent)
    }

    fn preset_id(&self) -> String {
        format!(
            "{DEFAULT_DERIVATIVE_PRESET}-wm-{}-{}",
            self.preset_revision, self.asset_sha256
        )
    }
}

/// Credential-free media probe boundary.
#[async_trait]
pub trait SandboxProbe: Send + Sync {
    /// Returns validated JSON facts for one worker-owned local file.
    async fn inspect(
        &self,
        source: &std::path::Path,
        declared_kind: MediaKind,
        byte_len: u64,
    ) -> Result<MediaInspection, SandboxProbeError>;

    /// Produces one metadata-stripped JPEG master for a validated image.
    async fn image_master(
        &self,
        source: &std::path::Path,
        output: &std::path::Path,
        watermark: Option<&WatermarkPolicy>,
    ) -> Result<(), SandboxProbeError>;

    /// Produces one sanitized default JPEG thumbnail at a worker-owned path.
    async fn thumbnail(
        &self,
        source: &std::path::Path,
        inspection: &MediaInspection,
        output: &std::path::Path,
        watermark: Option<&WatermarkPolicy>,
    ) -> Result<(), SandboxProbeError>;
}

/// Subprocess sandbox implementation with bounded output and timeout.
#[derive(Clone, Debug)]
pub struct ProcessSandboxProbe {
    binary: PathBuf,
    image_timeout: Duration,
    video_timeout: Duration,
    native_threads: usize,
    max_output_bytes: usize,
}

impl ProcessSandboxProbe {
    /// Creates a process sandbox adapter with hard request bounds.
    pub fn new(
        binary: PathBuf,
        image_timeout: Duration,
        video_timeout: Duration,
        native_threads: usize,
        max_output_bytes: usize,
    ) -> Result<Self, SandboxProbeError> {
        if binary.as_os_str().is_empty()
            || image_timeout.is_zero()
            || video_timeout.is_zero()
            || native_threads == 0
            || native_threads > 16
            || !(1024..=1_048_576).contains(&max_output_bytes)
        {
            return Err(SandboxProbeError::InvalidConfiguration);
        }
        Ok(Self {
            binary,
            image_timeout,
            video_timeout,
            native_threads,
            max_output_bytes,
        })
    }

    async fn render_image(
        &self,
        source: &std::path::Path,
        output: &std::path::Path,
        max_edge: u32,
        watermark: Option<&WatermarkPolicy>,
    ) -> Result<(), SandboxProbeError> {
        let mut args = vec![
            "--max-edge".to_owned(),
            max_edge.to_string(),
            "--format".to_owned(),
            "jpeg".to_owned(),
        ];
        if let Some(watermark) = watermark {
            args.extend([
                "--watermark".to_owned(),
                watermark.asset_path.to_string_lossy().into_owned(),
                "--watermark-position".to_owned(),
                watermark_position_name(watermark.position).to_owned(),
                "--watermark-margin-px".to_owned(),
                watermark.margin_px.to_string(),
                "--watermark-max-width-percent".to_owned(),
                watermark.max_width_percent.to_string(),
                "--watermark-opacity-percent".to_owned(),
                watermark.opacity_percent.to_string(),
            ]);
        }
        let process_output = tokio::time::timeout(
            self.image_timeout,
            Command::new(&self.binary)
                .arg("image-thumbnail")
                .arg("--input")
                .arg(source)
                .arg("--output")
                .arg(output)
                .args(args)
                .arg("--threads")
                .arg(self.native_threads.to_string())
                .env_clear()
                .env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| SandboxProbeError::Timeout)?
        .map_err(SandboxProbeError::Spawn)?;
        if !process_output.status.success() {
            return Err(if watermark.is_some() {
                SandboxProbeError::Operational
            } else {
                SandboxProbeError::Rejected
            });
        }
        if process_output.stdout.len() > self.max_output_bytes
            || process_output.stderr.len() > self.max_output_bytes
        {
            return Err(SandboxProbeError::OutputTooLarge);
        }
        let metadata = tokio::fs::metadata(output)
            .await
            .map_err(SandboxProbeError::Output)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(if watermark.is_some() {
                SandboxProbeError::Operational
            } else {
                SandboxProbeError::InvalidOutput
            });
        }
        Ok(())
    }
}

#[async_trait]
impl SandboxProbe for ProcessSandboxProbe {
    async fn inspect(
        &self,
        source: &std::path::Path,
        declared_kind: MediaKind,
        byte_len: u64,
    ) -> Result<MediaInspection, SandboxProbeError> {
        let timeout = match declared_kind {
            MediaKind::Image => self.image_timeout,
            MediaKind::Video => self.video_timeout,
        };
        let kind = match declared_kind {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
        };
        let output = tokio::time::timeout(
            timeout,
            Command::new(&self.binary)
                .arg("probe")
                .arg("--input")
                .arg(source)
                .arg("--declared-kind")
                .arg(kind)
                .arg("--byte-len")
                .arg(byte_len.to_string())
                .arg("--timeout-seconds")
                .arg(timeout.as_secs().to_string())
                .arg("--threads")
                .arg(self.native_threads.to_string())
                .env_clear()
                .env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| SandboxProbeError::Timeout)?
        .map_err(SandboxProbeError::Spawn)?;
        if !output.status.success() {
            return Err(SandboxProbeError::Rejected);
        }
        if output.stdout.len() > self.max_output_bytes
            || output.stderr.len() > self.max_output_bytes
        {
            return Err(SandboxProbeError::OutputTooLarge);
        }
        serde_json::from_slice(&output.stdout).map_err(SandboxProbeError::InvalidJson)
    }

    async fn image_master(
        &self,
        source: &std::path::Path,
        output: &std::path::Path,
        watermark: Option<&WatermarkPolicy>,
    ) -> Result<(), SandboxProbeError> {
        self.render_image(source, output, SANITIZED_MASTER_MAX_EDGE, watermark)
            .await
    }

    async fn thumbnail(
        &self,
        source: &std::path::Path,
        inspection: &MediaInspection,
        output: &std::path::Path,
        watermark: Option<&WatermarkPolicy>,
    ) -> Result<(), SandboxProbeError> {
        let uses_watermark = watermark.is_some();
        let is_video = matches!(inspection, MediaInspection::Video { .. });
        let (timeout, command, extra_args) = match inspection {
            MediaInspection::Image { .. } => {
                return self.render_image(source, output, 1280, watermark).await;
            }
            MediaInspection::Video {
                format, inspection, ..
            } => {
                let mut args = vec![
                    "--timestamp-ms".to_owned(),
                    (inspection.probe.duration_ms / 10).min(1000).to_string(),
                    "--duration-ms".to_owned(),
                    inspection.probe.duration_ms.to_string(),
                    "--max-width".to_owned(),
                    "1280".to_owned(),
                    "--timeout-seconds".to_owned(),
                    self.video_timeout.as_secs().max(1).to_string(),
                ];
                if allows_openh264_fallback(*format, &inspection.codec) {
                    args.push("--allow-openh264-fallback".to_owned());
                }
                if let Some(watermark) = watermark {
                    args.extend([
                        "--watermark".to_owned(),
                        watermark.asset_path.to_string_lossy().into_owned(),
                        "--watermark-position".to_owned(),
                        watermark_position_name(watermark.position).to_owned(),
                        "--watermark-margin-px".to_owned(),
                        watermark.margin_px.to_string(),
                        "--watermark-max-width-percent".to_owned(),
                        watermark.max_width_percent.to_string(),
                        "--watermark-opacity-percent".to_owned(),
                        watermark.opacity_percent.to_string(),
                    ]);
                }
                (self.video_timeout, "video-thumbnail", args)
            }
        };
        let process_output = tokio::time::timeout(
            timeout,
            Command::new(&self.binary)
                .arg(command)
                .arg("--input")
                .arg(source)
                .arg("--output")
                .arg(output)
                .args(extra_args)
                .arg("--threads")
                .arg(self.native_threads.to_string())
                .env_clear()
                .env("PATH", "/usr/bin:/usr/local/bin:/opt/homebrew/bin")
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| SandboxProbeError::Timeout)?
        .map_err(SandboxProbeError::Spawn)?;
        if !process_output.status.success() {
            return Err(if uses_watermark || is_video {
                SandboxProbeError::Operational
            } else {
                SandboxProbeError::Rejected
            });
        }
        if process_output.stdout.len() > self.max_output_bytes
            || process_output.stderr.len() > self.max_output_bytes
        {
            return Err(SandboxProbeError::OutputTooLarge);
        }
        let metadata = tokio::fs::metadata(output)
            .await
            .map_err(SandboxProbeError::Output)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(if uses_watermark || is_video {
                SandboxProbeError::Operational
            } else {
                SandboxProbeError::InvalidOutput
            });
        }
        Ok(())
    }
}

fn allows_openh264_fallback(format: MediaFormat, codec: &str) -> bool {
    format == MediaFormat::Mp4 && codec == "h264"
}

const fn video_master_extension(format: MediaFormat) -> Option<&'static str> {
    match format {
        MediaFormat::Mp4 => Some("mp4"),
        MediaFormat::QuickTime => Some("mov"),
        MediaFormat::Webm => Some("webm"),
        MediaFormat::Jpeg
        | MediaFormat::Png
        | MediaFormat::Gif
        | MediaFormat::Webp
        | MediaFormat::Avif
        | MediaFormat::Heif => None,
    }
}

const fn watermark_position_name(position: WatermarkPosition) -> &'static str {
    match position {
        WatermarkPosition::Center => "center",
        WatermarkPosition::TopLeft => "top-left",
        WatermarkPosition::TopRight => "top-right",
        WatermarkPosition::BottomLeft => "bottom-left",
        WatermarkPosition::BottomRight => "bottom-right",
    }
}

/// Durable single-job worker. Multiple clones may run with distinct worker IDs.
#[derive(Clone)]
pub struct SourceValidationWorker {
    raw_store: Arc<dyn ObjectStore>,
    derivative_store: Arc<dyn ObjectStore>,
    queue: Arc<dyn JobQueue>,
    repository: Arc<dyn ProcessingRepository>,
    policy_repository: Arc<dyn SitePolicyRepository>,
    sandbox: Arc<dyn SandboxProbe>,
    resource_gates: Arc<ResourceGates>,
    policy: WorkerPolicy,
}

struct ResourceGates {
    heavy_images: Arc<Semaphore>,
    videos: Arc<Semaphore>,
    temp_disk: Arc<Semaphore>,
}

fn temp_disk_permits(bytes: u64) -> Option<u32> {
    u32::try_from(bytes.div_ceil(TEMP_DISK_PERMIT_BYTES)).ok()
}

struct ActiveJobMetric;

impl ActiveJobMetric {
    fn new() -> Self {
        gauge!("g7mb_worker_active_jobs").increment(1.0);
        Self
    }
}

impl Drop for ActiveJobMetric {
    fn drop(&mut self) {
        gauge!("g7mb_worker_active_jobs").decrement(1.0);
    }
}

struct WorkerStageMetric {
    stage: &'static str,
    started: Instant,
}

impl WorkerStageMetric {
    fn new(stage: &'static str) -> Self {
        Self {
            stage,
            started: Instant::now(),
        }
    }
}

impl Drop for WorkerStageMetric {
    fn drop(&mut self) {
        histogram!("g7mb_worker_stage_duration_seconds", "stage" => self.stage)
            .record(self.started.elapsed().as_secs_f64());
    }
}

fn observe_worker_outcome(outcome: RunOutcome, elapsed: Duration) {
    let outcome = match outcome {
        RunOutcome::Idle => "idle",
        RunOutcome::Completed => "completed",
        RunOutcome::Rejected => "rejected",
        RunOutcome::RetryScheduled => "retry_scheduled",
        RunOutcome::DeadLetter => "dead_letter",
    };
    counter!("g7mb_worker_jobs_total", "outcome" => outcome).increment(1);
    histogram!("g7mb_worker_job_duration_seconds", "outcome" => outcome)
        .record(elapsed.as_secs_f64());
}

impl SourceValidationWorker {
    /// Creates a worker after validating all resource limits.
    pub fn new(
        raw_store: Arc<dyn ObjectStore>,
        derivative_store: Arc<dyn ObjectStore>,
        queue: Arc<dyn JobQueue>,
        repository: Arc<dyn ProcessingRepository>,
        policy_repository: Arc<dyn SitePolicyRepository>,
        sandbox: Arc<dyn SandboxProbe>,
        policy: WorkerPolicy,
    ) -> Result<Self, WorkerError> {
        policy.validate()?;
        let temp_disk_permits =
            temp_disk_permits(policy.max_temp_disk_bytes).ok_or(WorkerError::InvalidPolicy)?;
        let resource_gates = Arc::new(ResourceGates {
            heavy_images: Arc::new(Semaphore::new(policy.max_concurrent_heavy_images)),
            videos: Arc::new(Semaphore::new(policy.max_concurrent_videos)),
            temp_disk: Arc::new(Semaphore::new(
                usize::try_from(temp_disk_permits).map_err(|_| WorkerError::InvalidPolicy)?,
            )),
        });
        Ok(Self {
            raw_store,
            derivative_store,
            queue,
            repository,
            policy_repository,
            sandbox,
            resource_gates,
            policy,
        })
    }

    /// Claims and processes at most one source-validation job.
    pub async fn run_one(&self, worker_id: &str) -> Result<RunOutcome, WorkerError> {
        let leased = match self
            .queue
            .claim_next(worker_id, OffsetDateTime::now_utc(), self.policy.lease_for)
            .await
        {
            Ok(leased) => leased,
            Err(error) => {
                counter!("g7mb_worker_claims_total", "result" => "error").increment(1);
                return Err(WorkerError::queue(error));
            }
        };
        let Some(leased) = leased else {
            counter!("g7mb_worker_claims_total", "result" => "idle").increment(1);
            return Ok(RunOutcome::Idle);
        };
        counter!("g7mb_worker_claims_total", "result" => "leased").increment(1);
        histogram!("g7mb_worker_job_attempts").record(f64::from(leased.attempts));
        let queue_age = (OffsetDateTime::now_utc() - leased.enqueued_at)
            .as_seconds_f64()
            .max(0.0);
        histogram!("g7mb_worker_job_age_at_claim_seconds").record(queue_age);
        let _active_job = ActiveJobMetric::new();
        let job_started = Instant::now();
        if leased.job.preset_id != SOURCE_VALIDATION_PRESET {
            let disposition = self
                .queue
                .fail(
                    &leased.job_id,
                    worker_id,
                    OffsetDateTime::now_utc(),
                    OffsetDateTime::now_utc(),
                    1,
                    "UNSUPPORTED_PRESET",
                )
                .await
                .map_err(WorkerError::queue)?;
            let outcome = disposition.into();
            observe_worker_outcome(outcome, job_started.elapsed());
            return Ok(outcome);
        }

        let process = self.validate_source(leased.job.upload_id, leased.job.site_policy_revision);
        tokio::pin!(process);
        let mut heartbeat = interval(self.policy.heartbeat_every);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        heartbeat.tick().await;
        let result = loop {
            tokio::select! {
                result = &mut process => break result,
                _ = heartbeat.tick() => {
                    self.queue
                        .renew(
                            &leased.job_id,
                            worker_id,
                            OffsetDateTime::now_utc(),
                            self.policy.lease_for,
                        )
                        .await
                        .map_err(WorkerError::queue)?;
                }
            }
        };

        let outcome = match result {
            Ok(()) => {
                self.queue
                    .complete(&leased.job_id, worker_id, OffsetDateTime::now_utc())
                    .await
                    .map_err(WorkerError::queue)?;
                Ok(RunOutcome::Completed)
            }
            Err(ProcessingFailure::Permanent(code)) => {
                counter!(
                    "g7mb_worker_processing_failures_total",
                    "class" => "permanent",
                    "code" => code
                )
                .increment(1);
                self.repository
                    .mark_rejected(leased.job.upload_id, code, OffsetDateTime::now_utc())
                    .await
                    .map_err(WorkerError::repository)?;
                self.queue
                    .complete(&leased.job_id, worker_id, OffsetDateTime::now_utc())
                    .await
                    .map_err(WorkerError::queue)?;
                Ok(RunOutcome::Rejected)
            }
            Err(ProcessingFailure::Transient(code)) => {
                counter!(
                    "g7mb_worker_processing_failures_total",
                    "class" => "transient",
                    "code" => code
                )
                .increment(1);
                if leased.attempts >= self.policy.max_attempts {
                    self.repository
                        .mark_rejected(leased.job.upload_id, code, OffsetDateTime::now_utc())
                        .await
                        .map_err(WorkerError::repository)?;
                }
                let now = OffsetDateTime::now_utc();
                let disposition = self
                    .queue
                    .fail(
                        &leased.job_id,
                        worker_id,
                        now,
                        now + time::Duration::try_from(self.policy.retry_delay)
                            .map_err(|_| WorkerError::InvalidPolicy)?,
                        self.policy.max_attempts,
                        code,
                    )
                    .await
                    .map_err(WorkerError::queue)?;
                Ok(disposition.into())
            }
            Err(ProcessingFailure::Operational(code)) => {
                counter!(
                    "g7mb_worker_processing_failures_total",
                    "class" => "operational",
                    "code" => code
                )
                .increment(1);
                Err(WorkerError::Operational(code))
            }
        };
        match outcome {
            Ok(value) => {
                observe_worker_outcome(value, job_started.elapsed());
                Ok(value)
            }
            Err(error) => {
                counter!("g7mb_worker_jobs_total", "outcome" => "error").increment(1);
                histogram!(
                    "g7mb_worker_job_duration_seconds",
                    "outcome" => "error"
                )
                .record(job_started.elapsed().as_secs_f64());
                Err(error)
            }
        }
    }

    async fn validate_source(
        &self,
        upload_id: g7mb_domain::UploadId,
        site_policy_revision: Option<u64>,
    ) -> Result<(), ProcessingFailure> {
        let source = self
            .repository
            .find_processing_source(upload_id)
            .await
            .map_err(|_| ProcessingFailure::Transient("DATABASE_UNAVAILABLE"))?
            .ok_or(ProcessingFailure::Transient("SOURCE_NOT_FOUND"))?;
        if matches!(source.state, UploadState::Rejected | UploadState::Ready) {
            return Ok(());
        }
        if !matches!(
            source.state,
            UploadState::Quarantined | UploadState::Processing
        ) {
            return Err(ProcessingFailure::Transient("SOURCE_STATE_CONFLICT"));
        }
        self.validate_downloaded_source(&source, site_policy_revision)
            .await
    }

    async fn validate_downloaded_source(
        &self,
        source: &ProcessingSource,
        site_policy_revision: Option<u64>,
    ) -> Result<(), ProcessingFailure> {
        let max_length = match source.declared_kind {
            MediaKind::Image => self.policy.max_image_bytes,
            MediaKind::Video => self.policy.max_video_bytes,
        };
        if source.expected_size_bytes > max_length {
            return Err(ProcessingFailure::Permanent("MEDIA_SIZE_EXCEEDED"));
        }
        let _temp_disk_permit = self
            .acquire_temp_disk_permit(source.expected_size_bytes)
            .await?;
        tokio::fs::create_dir_all(&self.policy.temp_directory)
            .await
            .map_err(|_| ProcessingFailure::Transient("TEMP_STORAGE_UNAVAILABLE"))?;
        let directory = tempfile::Builder::new()
            .prefix("g7mb-job-")
            .tempdir_in(&self.policy.temp_directory)
            .map_err(|_| ProcessingFailure::Transient("TEMP_STORAGE_UNAVAILABLE"))?;
        let local_source = directory.path().join("source");
        {
            let _metric = WorkerStageMetric::new("download");
            self.raw_store
                .download_to(DownloadObjectRequest {
                    key: source.object_key.clone(),
                    destination: local_source.clone(),
                    expected_length: source.expected_size_bytes,
                    max_length,
                })
                .await
                .map_err(classify_storage_error)?;
        }
        let _inspection_metric = WorkerStageMetric::new("inspect");
        detect_file(&local_source, source.declared_kind)
            .await
            .map_err(|_| ProcessingFailure::Permanent("MEDIA_SIGNATURE_REJECTED"))?;
        let digest = sha256_file(&local_source)
            .await
            .map_err(|_| ProcessingFailure::Transient("TEMP_STORAGE_UNAVAILABLE"))?;
        let inspection = self
            .sandbox
            .inspect(
                &local_source,
                source.declared_kind,
                source.expected_size_bytes,
            )
            .await
            .map_err(classify_sandbox_error)?;
        drop(_inspection_metric);
        let detected_content_type = match &inspection {
            MediaInspection::Image { content_type, .. }
            | MediaInspection::Video { content_type, .. } => content_type.clone(),
        };
        let _resource_permit = self.acquire_transform_permit(&inspection).await?;
        if source.state == UploadState::Quarantined {
            self.repository
                .start_processing(
                    source.upload_id,
                    &detected_content_type,
                    &digest,
                    OffsetDateTime::now_utc(),
                )
                .await
                .map_err(|_| ProcessingFailure::Transient("DATABASE_UNAVAILABLE"))?;
        } else if source.source_sha256.as_deref() != Some(digest.as_str())
            || source.detected_content_type.as_deref() != Some(detected_content_type.as_str())
        {
            return Err(ProcessingFailure::Permanent("SOURCE_DIGEST_CHANGED"));
        }

        let watermark = self
            .prepare_watermark(directory.path(), &source.tenant_id, site_policy_revision)
            .await?;
        let preset_id = watermark.as_ref().map_or_else(
            || DEFAULT_DERIVATIVE_PRESET.to_owned(),
            WatermarkPolicy::preset_id,
        );
        let mut derivatives = Vec::with_capacity(2);
        match &inspection {
            MediaInspection::Image { .. } => {
                let master = directory.path().join("master.jpg");
                {
                    let _metric = WorkerStageMetric::new("transform");
                    self.sandbox
                        .image_master(&local_source, &master, watermark.as_ref())
                        .await
                        .map_err(classify_sandbox_error)?;
                }
                let master_key = ObjectKey::new(format!(
                    "media/{}/{}/{}/{}/master.jpg",
                    source.tenant_id, source.upload_id, digest, preset_id
                ))
                .map_err(|_| ProcessingFailure::Transient("DERIVATIVE_KEY_INVALID"))?;
                derivatives.push(
                    self.upload_generated_derivative(
                        master,
                        master_key,
                        &preset_id,
                        "master",
                        "image/jpeg",
                    )
                    .await?,
                );
            }
            MediaInspection::Video { format, .. } => {
                let extension = video_master_extension(*format)
                    .ok_or(ProcessingFailure::Permanent("VIDEO_CONTAINER_UNSUPPORTED"))?;
                let master_key = ObjectKey::new(format!(
                    "media/{}/{}/{}/{}/master.{extension}",
                    source.tenant_id, source.upload_id, digest, preset_id
                ))
                .map_err(|_| ProcessingFailure::Transient("DERIVATIVE_KEY_INVALID"))?;
                let stored = {
                    let _metric = WorkerStageMetric::new("upload");
                    self.derivative_store
                        .put_file(PutFileRequest {
                            key: master_key.clone(),
                            source: local_source.clone(),
                            content_type: detected_content_type.clone(),
                        })
                        .await
                        .map_err(classify_storage_error)?
                };
                if stored.content_length != source.expected_size_bytes {
                    return Err(ProcessingFailure::Transient("DERIVATIVE_LENGTH_MISMATCH"));
                }
                derivatives.push(PublishedDerivative {
                    object_key: master_key,
                    preset_id: preset_id.clone(),
                    variant: "master".to_owned(),
                    content_type: detected_content_type.clone(),
                    byte_len: source.expected_size_bytes,
                    sha256: digest.clone(),
                });
            }
        }

        let thumbnail = directory.path().join("thumbnail.jpg");
        {
            let _metric = WorkerStageMetric::new("transform");
            self.sandbox
                .thumbnail(&local_source, &inspection, &thumbnail, watermark.as_ref())
                .await
                .map_err(classify_sandbox_error)?;
        }
        let thumbnail_key = ObjectKey::new(format!(
            "media/{}/{}/{}/{}/thumbnail.jpg",
            source.tenant_id, source.upload_id, digest, preset_id
        ))
        .map_err(|_| ProcessingFailure::Transient("DERIVATIVE_KEY_INVALID"))?;
        derivatives.push(
            self.upload_generated_derivative(
                thumbnail,
                thumbnail_key,
                &preset_id,
                "thumbnail",
                "image/jpeg",
            )
            .await?,
        );
        let _metric = WorkerStageMetric::new("commit");
        self.repository
            .publish_derivatives(source.upload_id, &derivatives, OffsetDateTime::now_utc())
            .await
            .map_err(|_| ProcessingFailure::Transient("DATABASE_UNAVAILABLE"))
    }

    async fn upload_generated_derivative(
        &self,
        source: PathBuf,
        object_key: ObjectKey,
        preset_id: &str,
        variant: &str,
        content_type: &str,
    ) -> Result<PublishedDerivative, ProcessingFailure> {
        let byte_len = tokio::fs::metadata(&source)
            .await
            .map_err(|_| ProcessingFailure::Transient("TEMP_STORAGE_UNAVAILABLE"))?
            .len();
        if byte_len == 0 || byte_len > self.policy.max_image_bytes {
            return Err(ProcessingFailure::Permanent("DERIVATIVE_SIZE_EXCEEDED"));
        }
        let sha256 = sha256_file(&source)
            .await
            .map_err(|_| ProcessingFailure::Transient("TEMP_STORAGE_UNAVAILABLE"))?;
        let stored = {
            let _metric = WorkerStageMetric::new("upload");
            self.derivative_store
                .put_file(PutFileRequest {
                    key: object_key.clone(),
                    source,
                    content_type: content_type.to_owned(),
                })
                .await
                .map_err(classify_storage_error)?
        };
        if stored.content_length != byte_len {
            return Err(ProcessingFailure::Transient("DERIVATIVE_LENGTH_MISMATCH"));
        }
        Ok(PublishedDerivative {
            object_key,
            preset_id: preset_id.to_owned(),
            variant: variant.to_owned(),
            content_type: content_type.to_owned(),
            byte_len,
            sha256,
        })
    }

    async fn acquire_transform_permit(
        &self,
        inspection: &MediaInspection,
    ) -> Result<Option<OwnedSemaphorePermit>, ProcessingFailure> {
        let gate = match inspection {
            MediaInspection::Image { probe, .. } if probe.work_class() == ImageWorkClass::Heavy => {
                Some(("heavy_image", self.resource_gates.heavy_images.clone()))
            }
            MediaInspection::Video { .. } => Some(("video", self.resource_gates.videos.clone())),
            MediaInspection::Image { .. } => None,
        };
        match gate {
            Some((lane, gate)) => {
                let started = Instant::now();
                let permit = gate
                    .acquire_owned()
                    .await
                    .map(Some)
                    .map_err(|_| ProcessingFailure::Operational("RESOURCE_GATE_CLOSED"));
                histogram!("g7mb_worker_resource_wait_seconds", "lane" => lane)
                    .record(started.elapsed().as_secs_f64());
                permit
            }
            None => Ok(None),
        }
    }

    async fn acquire_temp_disk_permit(
        &self,
        source_bytes: u64,
    ) -> Result<OwnedSemaphorePermit, ProcessingFailure> {
        let reservation = source_bytes
            .checked_add(self.policy.max_image_bytes.checked_mul(2).ok_or(
                ProcessingFailure::Operational("TEMP_DISK_RESERVATION_INVALID"),
            )?)
            .and_then(|bytes| bytes.checked_add(MAX_WATERMARK_BYTES))
            .ok_or(ProcessingFailure::Operational(
                "TEMP_DISK_RESERVATION_INVALID",
            ))?;
        let permits = temp_disk_permits(reservation).ok_or(ProcessingFailure::Operational(
            "TEMP_DISK_RESERVATION_INVALID",
        ))?;
        let started = Instant::now();
        let permit = self
            .resource_gates
            .temp_disk
            .clone()
            .acquire_many_owned(permits)
            .await
            .map_err(|_| ProcessingFailure::Operational("RESOURCE_GATE_CLOSED"));
        histogram!("g7mb_worker_resource_wait_seconds", "lane" => "temp_disk")
            .record(started.elapsed().as_secs_f64());
        permit
    }

    async fn prepare_watermark(
        &self,
        job_directory: &std::path::Path,
        tenant_id: &str,
        site_policy_revision: Option<u64>,
    ) -> Result<Option<WatermarkPolicy>, ProcessingFailure> {
        if let Some(revision) = site_policy_revision {
            let snapshot = self
                .policy_repository
                .find_site_policy_revision(tenant_id, revision)
                .await
                .map_err(|_| ProcessingFailure::Operational("SITE_POLICY_UNAVAILABLE"))?
                .ok_or(ProcessingFailure::Operational("SITE_POLICY_UNAVAILABLE"))?;
            if snapshot.revision != revision || snapshot.tenant_id != tenant_id {
                return Err(ProcessingFailure::Operational("SITE_POLICY_INVALID"));
            }
            return match snapshot.watermark {
                Some(watermark) => self
                    .prepare_object_watermark(job_directory, revision, watermark)
                    .await
                    .map(Some),
                None => Ok(None),
            };
        }
        let Some(configured) = self.policy.watermark.as_ref() else {
            return Ok(None);
        };
        let source = tokio::fs::File::open(&configured.asset_path)
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        let metadata = source
            .metadata()
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_WATERMARK_BYTES {
            return Err(ProcessingFailure::Operational("WATERMARK_ASSET_INVALID"));
        }
        let local_path = job_directory.join("registered-watermark");
        let mut destination = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&local_path)
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        let copied = tokio::io::copy(&mut source.take(MAX_WATERMARK_BYTES + 1), &mut destination)
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        destination
            .flush()
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        if copied == 0 || copied > MAX_WATERMARK_BYTES {
            return Err(ProcessingFailure::Operational("WATERMARK_ASSET_INVALID"));
        }
        let digest = sha256_file(&local_path)
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        if digest != configured.asset_sha256 {
            return Err(ProcessingFailure::Operational("WATERMARK_DIGEST_MISMATCH"));
        }
        let mut prepared = configured.clone();
        prepared.asset_path = local_path;
        Ok(Some(prepared))
    }

    async fn prepare_object_watermark(
        &self,
        job_directory: &std::path::Path,
        revision: u64,
        stored: StoredWatermarkPolicy,
    ) -> Result<WatermarkPolicy, ProcessingFailure> {
        let local_path = job_directory.join("registered-watermark");
        self.raw_store
            .download_to(DownloadObjectRequest {
                key: stored.object_key,
                destination: local_path.clone(),
                expected_length: stored.byte_len,
                max_length: MAX_WATERMARK_BYTES,
            })
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        let digest = sha256_file(&local_path)
            .await
            .map_err(|_| ProcessingFailure::Operational("WATERMARK_ASSET_UNAVAILABLE"))?;
        if digest != stored.asset_sha256 {
            return Err(ProcessingFailure::Operational("WATERMARK_DIGEST_MISMATCH"));
        }
        Ok(WatermarkPolicy {
            asset_path: local_path,
            asset_sha256: stored.asset_sha256,
            preset_revision: format!("g7-r{revision}"),
            position: stored.position,
            margin_px: stored.margin_px,
            max_width_percent: stored.max_width_percent,
            opacity_percent: stored.opacity_percent,
        })
    }
}

async fn sha256_file(path: &std::path::Path) -> Result<String, std::io::Error> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn classify_storage_error(error: ObjectStoreError) -> ProcessingFailure {
    match error {
        ObjectStoreError::ContentLengthMismatch | ObjectStoreError::InvalidRequest(_) => {
            ProcessingFailure::Permanent("SOURCE_LENGTH_REJECTED")
        }
        ObjectStoreError::NotFound => ProcessingFailure::Transient("SOURCE_NOT_FOUND"),
        ObjectStoreError::Backend(_) => ProcessingFailure::Transient("STORAGE_UNAVAILABLE"),
    }
}

fn classify_sandbox_error(error: SandboxProbeError) -> ProcessingFailure {
    match error {
        SandboxProbeError::Operational => {
            ProcessingFailure::Operational("MEDIA_RENDERER_UNAVAILABLE")
        }
        SandboxProbeError::Rejected => ProcessingFailure::Permanent("MEDIA_DECODER_REJECTED"),
        SandboxProbeError::Timeout => ProcessingFailure::Transient("MEDIA_PROBE_TIMEOUT"),
        SandboxProbeError::InvalidConfiguration
        | SandboxProbeError::Spawn(_)
        | SandboxProbeError::OutputTooLarge
        | SandboxProbeError::InvalidJson(_)
        | SandboxProbeError::Output(_) => ProcessingFailure::Transient("SANDBOX_UNAVAILABLE"),
        SandboxProbeError::InvalidOutput => ProcessingFailure::Permanent("THUMBNAIL_REJECTED"),
    }
}

enum ProcessingFailure {
    Permanent(&'static str),
    Transient(&'static str),
    Operational(&'static str),
}

impl From<JobFailureDisposition> for RunOutcome {
    fn from(value: JobFailureDisposition) -> Self {
        match value {
            JobFailureDisposition::RetryScheduled => Self::RetryScheduled,
            JobFailureDisposition::DeadLetter => Self::DeadLetter,
        }
    }
}

/// Sandbox process failure classified by worker retry policy.
#[derive(Debug, Error)]
pub enum SandboxProbeError {
    /// Adapter configuration violates hard bounds.
    #[error("sandbox probe configuration is invalid")]
    InvalidConfiguration,
    /// Trusted operational policy or renderer failed; do not blame the uploaded source.
    #[error("sandbox operational media policy failed")]
    Operational,
    /// Process could not start.
    #[error("sandbox process could not start: {0}")]
    Spawn(std::io::Error),
    /// Process exceeded its wall-clock budget.
    #[error("sandbox process timed out")]
    Timeout,
    /// Decoder or policy rejected the media.
    #[error("sandbox rejected the media")]
    Rejected,
    /// Child output exceeded its protocol budget.
    #[error("sandbox output exceeded its byte limit")]
    OutputTooLarge,
    /// Child returned malformed protocol JSON.
    #[error("sandbox returned malformed JSON: {0}")]
    InvalidJson(serde_json::Error),
    /// Generated output metadata could not be read.
    #[error("sandbox output file could not be inspected: {0}")]
    Output(std::io::Error),
    /// Child returned success without a usable derivative.
    #[error("sandbox produced no usable thumbnail")]
    InvalidOutput,
}

/// Worker orchestration failure that prevents safe lease completion.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// Resource or lease settings are invalid.
    #[error("worker policy is invalid")]
    InvalidPolicy,
    /// Durable queue operation failed.
    #[error("worker queue operation failed: {0}")]
    Queue(String),
    /// Durable source-state operation failed.
    #[error("worker source repository operation failed: {0}")]
    Repository(String),
    /// Fail-closed operational configuration issue; the lease is left for recovery.
    #[error("worker operational dependency is invalid: {0}")]
    Operational(&'static str),
}

impl WorkerError {
    fn queue(error: impl std::fmt::Display) -> Self {
        Self::Queue(error.to_string())
    }

    fn repository(error: impl std::fmt::Display) -> Self {
        Self::Repository(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use g7mb_application::WatermarkPosition;
    use g7mb_application::{
        AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest,
        DownloadObjectRequest, JobQueue as _, MultipartSession, ObjectMetadata, ObjectStore,
        ObjectStoreError, PresignPartRequest, PresignPutRequest, PresignedUpload, ProcessingJob,
        PutFileRequest,
        policies::{
            PublishPolicyOutcome, SitePolicyRepository as _, SitePolicySnapshot,
            StoredWatermarkPolicy,
        },
    };
    use g7mb_domain::{ImageProbe, MediaKind, ObjectKey, UploadId, VideoProbe};
    use g7mb_media::{MediaFormat, MediaInspection, VideoInspection};
    use g7mb_persistence_sqlite::SqliteStore;
    use sha2::{Digest as _, Sha256};
    use sqlx::Row as _;
    use time::OffsetDateTime;

    use super::{
        ProcessingFailure, RunOutcome, SandboxProbe, SandboxProbeError, SourceValidationWorker,
        WatermarkPolicy, WorkerError, WorkerPolicy, allows_openh264_fallback,
        classify_sandbox_error,
    };

    struct FakeRawStore {
        bytes: Vec<u8>,
    }

    #[async_trait]
    impl ObjectStore for FakeRawStore {
        async fn presign_put(
            &self,
            _request: PresignPutRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "unused in test".to_owned(),
            ))
        }

        async fn create_multipart(
            &self,
            _request: CreateMultipartRequest,
        ) -> Result<MultipartSession, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "unused in test".to_owned(),
            ))
        }

        async fn presign_part(
            &self,
            _request: PresignPartRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "unused in test".to_owned(),
            ))
        }

        async fn complete_multipart(
            &self,
            _request: CompleteMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "unused in test".to_owned(),
            ))
        }

        async fn abort_multipart(
            &self,
            _request: AbortMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest(
                "unused in test".to_owned(),
            ))
        }

        async fn head(&self, _key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError> {
            Ok(ObjectMetadata {
                content_length: self.bytes.len() as u64,
                content_type: None,
                etag: None,
            })
        }

        async fn download_to(
            &self,
            request: DownloadObjectRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            let length = u64::try_from(self.bytes.len())
                .map_err(|_| ObjectStoreError::ContentLengthMismatch)?;
            if length != request.expected_length || length > request.max_length {
                return Err(ObjectStoreError::ContentLengthMismatch);
            }
            tokio::fs::write(request.destination, &self.bytes)
                .await
                .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
            Ok(ObjectMetadata {
                content_length: length,
                content_type: None,
                etag: None,
            })
        }

        async fn put_file(
            &self,
            request: PutFileRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            let metadata = tokio::fs::metadata(request.source)
                .await
                .map_err(|error| ObjectStoreError::Backend(error.to_string()))?;
            Ok(ObjectMetadata {
                content_length: metadata.len(),
                content_type: Some(request.content_type),
                etag: None,
            })
        }

        async fn delete(&self, _key: &ObjectKey) -> Result<(), ObjectStoreError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeSandbox {
        calls: AtomicUsize,
        watermark_calls: AtomicUsize,
        probe: Option<ImageProbe>,
        video_format: Option<MediaFormat>,
        thumbnail_delay: Duration,
        active_thumbnails: AtomicUsize,
        max_active_thumbnails: AtomicUsize,
    }

    #[async_trait]
    impl SandboxProbe for FakeSandbox {
        async fn inspect(
            &self,
            _source: &std::path::Path,
            declared_kind: MediaKind,
            byte_len: u64,
        ) -> Result<MediaInspection, SandboxProbeError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if declared_kind == MediaKind::Video {
                let format = self.video_format.ok_or(SandboxProbeError::Rejected)?;
                return Ok(MediaInspection::Video {
                    format,
                    content_type: match format {
                        MediaFormat::Mp4 => "video/mp4",
                        MediaFormat::QuickTime => "video/quicktime",
                        MediaFormat::Webm => "video/webm",
                        _ => return Err(SandboxProbeError::Rejected),
                    }
                    .to_owned(),
                    inspection: VideoInspection {
                        probe: VideoProbe {
                            byte_len,
                            duration_ms: 1_000,
                            width: 1_920,
                            height: 1_080,
                            video_streams: 1,
                            total_streams: 1,
                        },
                        codec: "h264".to_owned(),
                        container: "mov,mp4,m4a,3gp,3g2,mj2".to_owned(),
                    },
                });
            }
            Ok(MediaInspection::Image {
                format: MediaFormat::Jpeg,
                content_type: "image/jpeg".to_owned(),
                probe: self.probe.unwrap_or(ImageProbe {
                    byte_len,
                    width: 8,
                    height: 8,
                    frames: 1,
                }),
            })
        }

        async fn image_master(
            &self,
            _source: &std::path::Path,
            output: &std::path::Path,
            watermark: Option<&WatermarkPolicy>,
        ) -> Result<(), SandboxProbeError> {
            if let Some(watermark) = watermark {
                let bytes = tokio::fs::read(&watermark.asset_path)
                    .await
                    .map_err(SandboxProbeError::Output)?;
                if bytes.is_empty() {
                    return Err(SandboxProbeError::InvalidOutput);
                }
                self.watermark_calls.fetch_add(1, Ordering::Relaxed);
            }
            tokio::fs::write(output, b"\xff\xd8\xff\xe0master")
                .await
                .map_err(SandboxProbeError::Output)
        }

        async fn thumbnail(
            &self,
            _source: &std::path::Path,
            _inspection: &MediaInspection,
            output: &std::path::Path,
            watermark: Option<&WatermarkPolicy>,
        ) -> Result<(), SandboxProbeError> {
            if let Some(watermark) = watermark {
                let bytes = tokio::fs::read(&watermark.asset_path)
                    .await
                    .map_err(SandboxProbeError::Output)?;
                if bytes.is_empty() {
                    return Err(SandboxProbeError::InvalidOutput);
                }
                self.watermark_calls.fetch_add(1, Ordering::Relaxed);
            }
            let active = self.active_thumbnails.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_active_thumbnails
                .fetch_max(active, Ordering::AcqRel);
            if !self.thumbnail_delay.is_zero() {
                tokio::time::sleep(self.thumbnail_delay).await;
            }
            let result = tokio::fs::write(output, b"\xff\xd8\xff\xe0thumbnail")
                .await
                .map_err(SandboxProbeError::Output);
            self.active_thumbnails.fetch_sub(1, Ordering::AcqRel);
            result
        }
    }

    #[tokio::test]
    async fn streams_valid_source_records_digest_and_completes_job()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"\xff\xd8\xff\xe0valid-jpeg".to_vec();
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let upload_id = insert_quarantined_job(&database, &bytes).await?;
        let sandbox = Arc::new(FakeSandbox::default());
        let temp = tempfile::tempdir()?;
        let storage = Arc::new(FakeRawStore {
            bytes: bytes.clone(),
        });
        let worker = SourceValidationWorker::new(
            storage.clone(),
            storage,
            database.clone(),
            database.clone(),
            database.clone(),
            sandbox.clone(),
            test_policy(temp.path().to_path_buf()),
        )?;

        assert_eq!(worker.run_one("worker-a").await?, RunOutcome::Completed);
        assert_eq!(sandbox.calls.load(Ordering::Relaxed), 1);
        let row = sqlx::query(
            "SELECT state, detected_content_type, source_sha256 FROM uploads WHERE id = ?",
        )
        .bind(upload_id.to_string())
        .fetch_one(database.pool())
        .await?;
        assert_eq!(row.try_get::<String, _>("state")?, "ready");
        assert_eq!(
            row.try_get::<String, _>("detected_content_type")?,
            "image/jpeg"
        );
        assert_eq!(row.try_get::<String, _>("source_sha256")?.len(), 64);
        let completed = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM jobs WHERE upload_id = ? AND state = 'completed'",
        )
        .bind(upload_id.to_string())
        .fetch_one(database.pool())
        .await?;
        assert_eq!(completed, 1);
        let derivatives = sqlx::query_scalar::<_, String>(
            "SELECT variant FROM derivatives WHERE upload_id = ? AND preset_id = 'board-default-v1' ORDER BY variant",
        )
        .bind(upload_id.to_string())
        .fetch_all(database.pool())
        .await?;
        assert_eq!(derivatives, vec!["master", "thumbnail"]);
        Ok(())
    }

    #[tokio::test]
    async fn publishes_validated_video_master_and_poster_as_one_ready_set()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = 16_u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"ftypisom\0\0\0\0");
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let upload_id = insert_quarantined_video_job(&database, &bytes).await?;
        let sandbox = Arc::new(FakeSandbox {
            video_format: Some(MediaFormat::Mp4),
            ..FakeSandbox::default()
        });
        let temp = tempfile::tempdir()?;
        let storage = Arc::new(FakeRawStore {
            bytes: bytes.clone(),
        });
        let worker = SourceValidationWorker::new(
            storage.clone(),
            storage,
            database.clone(),
            database.clone(),
            database.clone(),
            sandbox,
            test_policy(temp.path().to_path_buf()),
        )?;

        assert_eq!(worker.run_one("video-worker").await?, RunOutcome::Completed);
        let rows = sqlx::query(
            "SELECT variant, object_key, content_type, byte_len, sha256
             FROM derivatives WHERE upload_id = ? ORDER BY variant",
        )
        .bind(upload_id.to_string())
        .fetch_all(database.pool())
        .await?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].try_get::<String, _>("variant")?, "master");
        assert!(
            rows[0]
                .try_get::<String, _>("object_key")?
                .ends_with("/master.mp4")
        );
        assert_eq!(rows[0].try_get::<String, _>("content_type")?, "video/mp4");
        assert_eq!(rows[0].try_get::<i64, _>("byte_len")?, bytes.len() as i64);
        assert_eq!(
            rows[0].try_get::<String, _>("sha256")?,
            hex::encode(Sha256::digest(&bytes))
        );
        assert_eq!(rows[1].try_get::<String, _>("variant")?, "thumbnail");
        assert!(
            rows[1]
                .try_get::<String, _>("object_key")?
                .ends_with("/thumbnail.jpg")
        );
        assert_eq!(rows[1].try_get::<String, _>("content_type")?, "image/jpeg");
        Ok(())
    }

    #[tokio::test]
    async fn heavy_image_lane_serializes_full_pixel_transforms()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"\xff\xd8\xff\xe0valid-jpeg".to_vec();
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        insert_quarantined_job(&database, &bytes).await?;
        insert_quarantined_job(&database, &bytes).await?;
        let sandbox = Arc::new(FakeSandbox {
            probe: Some(ImageProbe {
                byte_len: bytes.len() as u64,
                width: 25_000,
                height: 4_000,
                frames: 1,
            }),
            thumbnail_delay: Duration::from_millis(100),
            ..FakeSandbox::default()
        });
        let temp = tempfile::tempdir()?;
        let storage = Arc::new(FakeRawStore { bytes });
        let worker = SourceValidationWorker::new(
            storage.clone(),
            storage,
            database.clone(),
            database.clone(),
            database,
            sandbox.clone(),
            test_policy(temp.path().to_path_buf()),
        )?;

        let (first, second) = tokio::join!(
            worker.run_one("heavy-worker-a"),
            worker.run_one("heavy-worker-b")
        );
        assert_eq!(first?, RunOutcome::Completed);
        assert_eq!(second?, RunOutcome::Completed);
        assert_eq!(sandbox.max_active_thumbnails.load(Ordering::Acquire), 1);
        Ok(())
    }

    #[tokio::test]
    async fn temp_disk_reservation_serializes_jobs_that_cannot_both_fit()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"\xff\xd8\xff\xe0valid-jpeg".to_vec();
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        insert_quarantined_job(&database, &bytes).await?;
        insert_quarantined_job(&database, &bytes).await?;
        let sandbox = Arc::new(FakeSandbox {
            thumbnail_delay: Duration::from_millis(100),
            ..FakeSandbox::default()
        });
        let temp = tempfile::tempdir()?;
        let storage = Arc::new(FakeRawStore { bytes });
        let worker = SourceValidationWorker::new(
            storage.clone(),
            storage,
            database.clone(),
            database.clone(),
            database,
            sandbox.clone(),
            test_policy(temp.path().to_path_buf()),
        )?;

        let (first, second) = tokio::join!(
            worker.run_one("temp-worker-a"),
            worker.run_one("temp-worker-b")
        );
        assert_eq!(first?, RunOutcome::Completed);
        assert_eq!(second?, RunOutcome::Completed);
        assert_eq!(sandbox.max_active_thumbnails.load(Ordering::Acquire), 1);
        Ok(())
    }

    #[tokio::test]
    async fn pins_watermark_digest_and_revision_into_the_immutable_derivative()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"\xff\xd8\xff\xe0valid-jpeg".to_vec();
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let upload_id = insert_quarantined_job(&database, &bytes).await?;
        let sandbox = Arc::new(FakeSandbox::default());
        let temp = tempfile::tempdir()?;
        let watermark_bytes = b"registered-watermark";
        let watermark_path = temp.path().join("watermark.png");
        tokio::fs::write(&watermark_path, watermark_bytes).await?;
        let watermark_digest = hex::encode(Sha256::digest(watermark_bytes));
        let mut policy = test_policy(temp.path().to_path_buf());
        policy.watermark = Some(WatermarkPolicy {
            asset_path: watermark_path,
            asset_sha256: watermark_digest.clone(),
            preset_revision: "site-v2".to_owned(),
            position: WatermarkPosition::BottomRight,
            margin_px: 24,
            max_width_percent: 20,
            opacity_percent: 80,
        });
        let storage = Arc::new(FakeRawStore {
            bytes: bytes.clone(),
        });
        let worker = SourceValidationWorker::new(
            storage.clone(),
            storage,
            database.clone(),
            database.clone(),
            database.clone(),
            sandbox.clone(),
            policy,
        )?;

        assert_eq!(worker.run_one("worker-a").await?, RunOutcome::Completed);
        assert_eq!(sandbox.watermark_calls.load(Ordering::Relaxed), 2);
        let rows = sqlx::query(
            "SELECT preset_id, variant, object_key FROM derivatives WHERE upload_id = ? ORDER BY variant",
        )
        .bind(upload_id.to_string())
        .fetch_all(database.pool())
        .await?;
        assert_eq!(rows.len(), 2);
        let expected_preset = format!("board-default-v1-wm-site-v2-{watermark_digest}");
        for row in rows {
            assert_eq!(row.try_get::<String, _>("preset_id")?, expected_preset);
            assert!(
                row.try_get::<String, _>("object_key")?
                    .contains(&expected_preset)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn consumes_the_exact_job_pinned_site_policy_revision()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"\xff\xd8\xff\xe0valid-jpeg".to_vec();
        let digest = hex::encode(Sha256::digest(&bytes));
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let watermark_upload_id = UploadId::new();
        let watermark_key = ObjectKey::new(format!("raw/site-a/{watermark_upload_id}/source"))?;
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, detected_content_type, source_sha256,
                 created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'ready', ?, ?, 'image/jpeg',
                     'image/jpeg', ?, ?, ?)",
        )
        .bind(watermark_upload_id.to_string())
        .bind(watermark_key.as_str())
        .bind(i64::try_from(bytes.len())?)
        .bind(i64::try_from(bytes.len())?)
        .bind(&digest)
        .bind(now.unix_timestamp())
        .bind(now.unix_timestamp())
        .execute(database.pool())
        .await?;
        let snapshot = SitePolicySnapshot {
            tenant_id: "site-a".to_owned(),
            schema_version: 1,
            revision: 1,
            issued_at: now,
            settings_sha256: "c".repeat(64),
            watermark: Some(StoredWatermarkPolicy {
                asset_upload_id: watermark_upload_id,
                object_key: watermark_key,
                byte_len: bytes.len() as u64,
                asset_sha256: digest.clone(),
                position: WatermarkPosition::BottomRight,
                margin_px: 12,
                max_width_percent: 25,
                opacity_percent: 70,
            }),
        };
        assert_eq!(
            database.publish_site_policy(&snapshot).await?,
            PublishPolicyOutcome::Published
        );
        let upload_id =
            insert_quarantined_job_with_policy_revision(&database, &bytes, Some(1)).await?;
        let sandbox = Arc::new(FakeSandbox::default());
        let temp = tempfile::tempdir()?;
        let storage = Arc::new(FakeRawStore {
            bytes: bytes.clone(),
        });
        let worker = SourceValidationWorker::new(
            storage.clone(),
            storage,
            database.clone(),
            database.clone(),
            database.clone(),
            sandbox.clone(),
            test_policy(temp.path().to_path_buf()),
        )?;

        assert_eq!(worker.run_one("worker-a").await?, RunOutcome::Completed);
        assert_eq!(sandbox.watermark_calls.load(Ordering::Relaxed), 2);
        let preset_ids = sqlx::query_scalar::<_, String>(
            "SELECT preset_id FROM derivatives WHERE upload_id = ? ORDER BY variant",
        )
        .bind(upload_id.to_string())
        .fetch_all(database.pool())
        .await?;
        assert_eq!(
            preset_ids,
            vec![
                format!("board-default-v1-wm-g7-r1-{digest}"),
                format!("board-default-v1-wm-g7-r1-{digest}"),
            ]
        );
        Ok(())
    }

    #[test]
    fn rejects_unpinned_watermark_configuration() {
        let mut policy = test_policy(std::path::PathBuf::from("/tmp/g7mb-test"));
        policy.watermark = Some(WatermarkPolicy {
            asset_path: std::path::PathBuf::from("/etc/g7mb/watermark.png"),
            asset_sha256: "not-a-digest".to_owned(),
            preset_revision: "v1".to_owned(),
            position: WatermarkPosition::Center,
            margin_px: 0,
            max_width_percent: 20,
            opacity_percent: 80,
        });
        assert!(matches!(policy.validate(), Err(WorkerError::InvalidPolicy)));
    }

    #[test]
    fn rejects_temp_disk_cap_below_one_worst_case_job() {
        let mut policy = test_policy(std::path::PathBuf::from("/tmp/g7mb-test"));
        policy.max_image_bytes = 64 * 1024 * 1024;
        policy.max_video_bytes = 32 * 1024 * 1024;
        policy.max_temp_disk_bytes = 207 * 1024 * 1024;
        assert!(matches!(policy.validate(), Err(WorkerError::InvalidPolicy)));
    }

    #[test]
    fn media_renderer_failure_never_blames_the_uploaded_source() {
        assert!(matches!(
            classify_sandbox_error(SandboxProbeError::Operational),
            ProcessingFailure::Operational("MEDIA_RENDERER_UNAVAILABLE")
        ));
    }

    #[test]
    fn openh264_fallback_is_allowlisted_only_for_mp4_h264() {
        assert!(allows_openh264_fallback(MediaFormat::Mp4, "h264"));
        assert!(!allows_openh264_fallback(MediaFormat::Mp4, "hevc"));
        assert!(!allows_openh264_fallback(MediaFormat::QuickTime, "h264"));
        assert!(!allows_openh264_fallback(MediaFormat::Webm, "h264"));
    }

    #[tokio::test]
    async fn rejects_disguised_php_before_native_decoder() -> Result<(), Box<dyn std::error::Error>>
    {
        let bytes = b"<?php echo 'not image';".to_vec();
        let database = Arc::new(SqliteStore::connect("sqlite::memory:", 1).await?);
        let upload_id = insert_quarantined_job(&database, &bytes).await?;
        let sandbox = Arc::new(FakeSandbox::default());
        let temp = tempfile::tempdir()?;
        let storage = Arc::new(FakeRawStore { bytes });
        let worker = SourceValidationWorker::new(
            storage.clone(),
            storage,
            database.clone(),
            database.clone(),
            database.clone(),
            sandbox.clone(),
            test_policy(temp.path().to_path_buf()),
        )?;

        assert_eq!(worker.run_one("worker-a").await?, RunOutcome::Rejected);
        assert_eq!(sandbox.calls.load(Ordering::Relaxed), 0);
        let row = sqlx::query("SELECT state, error_code FROM uploads WHERE id = ?")
            .bind(upload_id.to_string())
            .fetch_one(database.pool())
            .await?;
        assert_eq!(row.try_get::<String, _>("state")?, "rejected");
        assert_eq!(
            row.try_get::<String, _>("error_code")?,
            "MEDIA_SIGNATURE_REJECTED"
        );
        Ok(())
    }

    async fn insert_quarantined_job(
        database: &SqliteStore,
        bytes: &[u8],
    ) -> Result<UploadId, Box<dyn std::error::Error>> {
        insert_quarantined_job_with_policy_revision(database, bytes, None).await
    }

    async fn insert_quarantined_job_with_policy_revision(
        database: &SqliteStore,
        bytes: &[u8],
        site_policy_revision: Option<u64>,
    ) -> Result<UploadId, Box<dyn std::error::Error>> {
        let upload_id = UploadId::new();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, created_at, updated_at)
             VALUES (?, 'site-a', ?, 'image', 'quarantined', ?, ?, 'image/jpeg', ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(format!("raw/site-a/{upload_id}/source"))
        .bind(i64::try_from(bytes.len())?)
        .bind(i64::try_from(bytes.len())?)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await?;
        database
            .enqueue(ProcessingJob {
                upload_id,
                preset_id: super::SOURCE_VALIDATION_PRESET.to_owned(),
                site_policy_revision,
            })
            .await?;
        Ok(upload_id)
    }

    async fn insert_quarantined_video_job(
        database: &SqliteStore,
        bytes: &[u8],
    ) -> Result<UploadId, Box<dyn std::error::Error>> {
        let upload_id = UploadId::new();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, created_at, updated_at)
             VALUES (?, 'site-a', ?, 'video', 'quarantined', ?, ?, 'video/mp4', ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(format!("raw/site-a/{upload_id}/source"))
        .bind(i64::try_from(bytes.len())?)
        .bind(i64::try_from(bytes.len())?)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await?;
        database
            .enqueue(ProcessingJob {
                upload_id,
                preset_id: super::SOURCE_VALIDATION_PRESET.to_owned(),
                site_policy_revision: None,
            })
            .await?;
        Ok(upload_id)
    }

    fn test_policy(temp_directory: std::path::PathBuf) -> WorkerPolicy {
        WorkerPolicy {
            lease_for: Duration::from_secs(3),
            heartbeat_every: Duration::from_secs(1),
            retry_delay: Duration::from_secs(1),
            max_attempts: 2,
            max_concurrent_heavy_images: 1,
            max_concurrent_videos: 1,
            max_image_bytes: 1024,
            max_video_bytes: 1024,
            max_temp_disk_bytes: 32 * 1024 * 1024,
            temp_directory,
            watermark: None,
        }
    }
}
