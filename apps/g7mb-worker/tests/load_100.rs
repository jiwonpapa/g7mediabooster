//! Ignored operational gate for 100 real image jobs and expired-lease recovery.

use std::{
    env,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use g7mb_application::{
    AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest, DownloadObjectRequest,
    JobQueue as _, MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError,
    PresignPartRequest, PresignPutRequest, PresignedUpload, ProcessingJob, PutFileRequest,
};
use g7mb_domain::{ObjectKey, UploadId};
use g7mb_persistence_sqlite::SqliteStore;
use g7mb_worker::{ProcessSandboxProbe, RunOutcome, SourceValidationWorker, WorkerPolicy};
use sqlx::Row as _;
use time::OffsetDateTime;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt as _,
    sync::Mutex,
    time::{Instant, sleep},
};

const JOB_COUNT: usize = 100;
const CRASHED_LEASE_COUNT: usize = 10;
const SOURCE_VALIDATION_PRESET: &str = "source-validation-v1";

struct FixtureObjectStore {
    source: Arc<Vec<u8>>,
    derivatives: AtomicUsize,
}

impl FixtureObjectStore {
    fn unused<T>() -> Result<T, ObjectStoreError> {
        Err(ObjectStoreError::InvalidRequest(
            "operation is outside the load harness".to_owned(),
        ))
    }
}

#[async_trait]
impl ObjectStore for FixtureObjectStore {
    async fn presign_put(
        &self,
        _request: PresignPutRequest,
    ) -> Result<PresignedUpload, ObjectStoreError> {
        Self::unused()
    }

    async fn create_multipart(
        &self,
        _request: CreateMultipartRequest,
    ) -> Result<MultipartSession, ObjectStoreError> {
        Self::unused()
    }

    async fn presign_part(
        &self,
        _request: PresignPartRequest,
    ) -> Result<PresignedUpload, ObjectStoreError> {
        Self::unused()
    }

    async fn complete_multipart(
        &self,
        _request: CompleteMultipartRequest,
    ) -> Result<(), ObjectStoreError> {
        Self::unused()
    }

    async fn abort_multipart(
        &self,
        _request: AbortMultipartRequest,
    ) -> Result<(), ObjectStoreError> {
        Self::unused()
    }

    async fn head(&self, _key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError> {
        Ok(ObjectMetadata {
            content_length: self.source.len() as u64,
            content_type: Some("image/jpeg".to_owned()),
            etag: None,
        })
    }

    async fn download_to(
        &self,
        request: DownloadObjectRequest,
    ) -> Result<ObjectMetadata, ObjectStoreError> {
        let length = self.source.len() as u64;
        if length != request.expected_length || length > request.max_length {
            return Err(ObjectStoreError::ContentLengthMismatch);
        }
        let mut destination = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(request.destination)
            .await
            .map_err(storage_error)?;
        destination
            .write_all(self.source.as_slice())
            .await
            .map_err(storage_error)?;
        destination.flush().await.map_err(storage_error)?;
        Ok(ObjectMetadata {
            content_length: length,
            content_type: Some("image/jpeg".to_owned()),
            etag: None,
        })
    }

    async fn put_file(&self, request: PutFileRequest) -> Result<ObjectMetadata, ObjectStoreError> {
        let metadata = tokio::fs::metadata(request.source)
            .await
            .map_err(storage_error)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(ObjectStoreError::Backend(
                "sandbox produced an empty derivative".to_owned(),
            ));
        }
        self.derivatives.fetch_add(1, Ordering::Relaxed);
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

fn storage_error(error: std::io::Error) -> ObjectStoreError {
    ObjectStoreError::Backend(error.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "operational gate: requires libvips and a real JPEG fixture"]
async fn load_100_real_jpeg_recovers_expired_leases() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = required_path("G7MB_LOAD_FIXTURE")?;
    let sandbox_binary = required_path("G7MB_SANDBOX_BIN")?;
    let concurrency = bounded_env_usize("G7MB_LOAD_CONCURRENCY", 4, 1, 8)?;
    let source = Arc::new(tokio::fs::read(&fixture).await?);
    if source.is_empty() || source.len() > 64 * 1024 * 1024 {
        return Err("fixture must be between 1 byte and 64 MiB".into());
    }

    let runtime = match env::var_os("G7MB_LOAD_RUNTIME_PARENT") {
        Some(parent) => tempfile::Builder::new()
            .prefix("g7mb-load-runtime-")
            .tempdir_in(PathBuf::from(parent))?,
        None => tempfile::tempdir()?,
    };
    let database_path = runtime.path().join("load.sqlite3");
    let database_url = format!("sqlite://{}", database_path.display());
    let database = Arc::new(SqliteStore::connect(&database_url, 8).await?);
    insert_jobs(&database, source.len()).await?;

    let crash_claim_time = OffsetDateTime::now_utc();
    for index in 0..CRASHED_LEASE_COUNT {
        let lease = database
            .claim_next(
                &format!("crashed-slot-{index}"),
                crash_claim_time,
                Duration::from_secs(2),
            )
            .await?;
        if lease.is_none() {
            return Err("failed to reserve a simulated crashed lease".into());
        }
    }

    let storage = Arc::new(FixtureObjectStore {
        source,
        derivatives: AtomicUsize::new(0),
    });
    let sandbox = Arc::new(ProcessSandboxProbe::new(
        sandbox_binary,
        Duration::from_secs(30),
        Duration::from_secs(30),
        1,
        64 * 1024,
    )?);
    let worker = SourceValidationWorker::new(
        storage.clone(),
        storage.clone(),
        database.clone(),
        database.clone(),
        database.clone(),
        sandbox,
        WorkerPolicy {
            lease_for: Duration::from_secs(6),
            heartbeat_every: Duration::from_secs(1),
            retry_delay: Duration::from_secs(1),
            max_attempts: 3,
            max_concurrent_heavy_images: 1,
            max_concurrent_videos: 1,
            max_image_bytes: 64 * 1024 * 1024,
            max_video_bytes: 256 * 1024 * 1024,
            max_temp_disk_bytes: 1024 * 1024 * 1024,
            temp_directory: runtime.path().join("jobs"),
            watermark: None,
        },
    )?;

    let completed = Arc::new(AtomicUsize::new(0));
    let processing_times_ms = Arc::new(Mutex::new(Vec::with_capacity(JOB_COUNT)));
    let started = Instant::now();
    let deadline = started + Duration::from_secs(180);
    let mut handles = Vec::with_capacity(concurrency);
    for index in 0..concurrency {
        let worker = worker.clone();
        let completed = completed.clone();
        let processing_times_ms = processing_times_ms.clone();
        handles.push(tokio::spawn(async move {
            let worker_id = format!("load-worker-{index}");
            loop {
                if completed.load(Ordering::Acquire) >= JOB_COUNT {
                    return Ok::<(), String>(());
                }
                if Instant::now() >= deadline {
                    return Err("100-job worker deadline exceeded".to_owned());
                }
                let job_started = Instant::now();
                match worker
                    .run_one(&worker_id)
                    .await
                    .map_err(|error| error.to_string())?
                {
                    RunOutcome::Completed => {
                        processing_times_ms
                            .lock()
                            .await
                            .push(job_started.elapsed().as_millis());
                        completed.fetch_add(1, Ordering::AcqRel);
                    }
                    RunOutcome::Idle => sleep(Duration::from_millis(25)).await,
                    outcome => return Err(format!("unexpected worker outcome: {outcome:?}")),
                }
            }
        }));
    }
    for handle in handles {
        handle.await??;
    }

    let elapsed = started.elapsed();
    let ready = count(
        &database,
        "SELECT COUNT(*) FROM uploads WHERE state = 'ready'",
    )
    .await?;
    let completed_jobs = count(
        &database,
        "SELECT COUNT(*) FROM jobs WHERE state = 'completed'",
    )
    .await?;
    let derivatives = count(&database, "SELECT COUNT(*) FROM derivatives").await?;
    let recovered = count(
        &database,
        "SELECT COUNT(*) FROM jobs WHERE state = 'completed' AND attempts = 2",
    )
    .await?;
    let dead_letter = count(
        &database,
        "SELECT COUNT(*) FROM jobs WHERE state = 'dead_letter'",
    )
    .await?;
    let unexpected_attempts = count(
        &database,
        "SELECT COUNT(*) FROM jobs WHERE attempts NOT IN (1, 2)",
    )
    .await?;

    assert_eq!(ready, JOB_COUNT as i64);
    assert_eq!(completed_jobs, JOB_COUNT as i64);
    assert_eq!(derivatives, (JOB_COUNT * 2) as i64);
    assert_eq!(storage.derivatives.load(Ordering::Relaxed), JOB_COUNT * 2);
    assert_eq!(recovered, CRASHED_LEASE_COUNT as i64);
    assert_eq!(dead_letter, 0);
    assert_eq!(unexpected_attempts, 0);

    let elapsed_ms = elapsed.as_millis();
    let throughput = JOB_COUNT as f64 / elapsed.as_secs_f64();
    let mut processing_times_ms = processing_times_ms.lock().await.clone();
    processing_times_ms.sort_unstable();
    if processing_times_ms.len() != JOB_COUNT {
        return Err("per-job timing sample count differs from completed jobs".into());
    }
    let p50_ms = percentile(&processing_times_ms, 50)?;
    let p95_ms = percentile(&processing_times_ms, 95)?;
    let p99_ms = percentile(&processing_times_ms, 99)?;
    println!(
        "G7MB_LOAD_RESULT jobs={JOB_COUNT} concurrency={concurrency} elapsed_ms={elapsed_ms} throughput_per_second={throughput:.2} p50_ms={p50_ms} p95_ms={p95_ms} p99_ms={p99_ms} ready={ready} completed={completed_jobs} derivatives={derivatives} recovered={recovered} dead_letter={dead_letter}"
    );
    Ok(())
}

fn percentile(
    sorted_samples: &[u128],
    percentile: usize,
) -> Result<u128, Box<dyn std::error::Error>> {
    if sorted_samples.is_empty() || !(1..=100).contains(&percentile) {
        return Err("invalid percentile request".into());
    }
    let rank = sorted_samples
        .len()
        .saturating_mul(percentile)
        .div_ceil(100);
    sorted_samples
        .get(rank.saturating_sub(1))
        .copied()
        .ok_or_else(|| "percentile rank exceeds samples".into())
}

async fn insert_jobs(
    database: &SqliteStore,
    fixture_size: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture_size = i64::try_from(fixture_size)?;
    let now = OffsetDateTime::now_utc().unix_timestamp();
    for _ in 0..JOB_COUNT {
        let upload_id = UploadId::new();
        sqlx::query(
            "INSERT INTO uploads
                (id, tenant_id, object_key, declared_kind, state, expected_size_bytes,
                 actual_size_bytes, content_type_hint, created_at, updated_at)
             VALUES (?, 'load-site', ?, 'image', 'quarantined', ?, ?, 'image/jpeg', ?, ?)",
        )
        .bind(upload_id.to_string())
        .bind(format!("raw/load-site/{upload_id}/source"))
        .bind(fixture_size)
        .bind(fixture_size)
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await?;
        database
            .enqueue(ProcessingJob {
                upload_id,
                preset_id: SOURCE_VALIDATION_PRESET.to_owned(),
                site_policy_revision: None,
            })
            .await?;
    }
    Ok(())
}

async fn count(
    database: &SqliteStore,
    query: &'static str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let row = sqlx::query(query).fetch_one(database.pool()).await?;
    Ok(row.try_get(0)?)
}

fn required_path(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let value = env::var_os(name).ok_or_else(|| format!("{name} is required"))?;
    let path = Path::new(&value).canonicalize()?;
    if !path.is_file() {
        return Err(format!("{name} must name a regular file").into());
    }
    Ok(path)
}

fn bounded_env_usize(
    name: &str,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let value = env::var(name)
        .ok()
        .map(|raw| raw.parse::<usize>())
        .transpose()?
        .unwrap_or(default);
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}").into());
    }
    Ok(value)
}
