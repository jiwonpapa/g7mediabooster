//! Read-only durable operational snapshot exposed only through bounded metrics scraping.

use async_trait::async_trait;
use thiserror::Error;
use time::OffsetDateTime;

/// Low-cardinality queue, lifecycle, inventory, and quota gauges.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OperationalSnapshot {
    /// Jobs currently eligible or delayed in the durable queue.
    pub queued_jobs: u64,
    /// Jobs currently protected by a worker lease.
    pub leased_jobs: u64,
    /// Jobs at the retry ceiling.
    pub dead_letter_jobs: u64,
    /// Age of the oldest queued job in seconds, clamped to zero.
    pub oldest_queued_age_seconds: u64,
    /// Uploads currently validating or transforming.
    pub processing_uploads: u64,
    /// Uploads with durable cleanup requested but not tombstoned.
    pub cleanup_pending_uploads: u64,
    /// Completed upload tombstones retained for audit.
    pub upload_tombstones: u64,
    /// Provider objects under orphan observation grace.
    pub orphan_suspects: u64,
    /// Orphan delete attempts whose latest attempt failed.
    pub orphan_delete_failures: u64,
    /// Retained source reservation bytes.
    pub reserved_source_bytes: u64,
}

/// Durable operational snapshot failure without SQL details.
#[derive(Debug, Error)]
#[error("operational snapshot failed: {0}")]
pub struct OperationalSnapshotError(pub String);

/// Read-only scrape-time snapshot boundary; never polled by worker hot paths.
#[async_trait]
pub trait OperationalObserver: Send + Sync {
    /// Reads one internally consistent low-cardinality snapshot.
    async fn operational_snapshot(
        &self,
        now: OffsetDateTime,
    ) -> Result<OperationalSnapshot, OperationalSnapshotError>;
}
