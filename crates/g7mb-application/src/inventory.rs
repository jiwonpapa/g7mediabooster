//! Bounded, race-safe provider inventory reconciliation for orphan objects.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use g7mb_domain::ObjectKey;
use thiserror::Error;
use time::OffsetDateTime;

use crate::{ListObjectsRequest, ListedObject, ObjectStore, ObjectStoreError};

/// Storage namespace reconciled against its matching durable key table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageNamespace {
    /// Private upload source objects under `raw/`.
    Raw,
    /// Immutable published derivatives under `media/`.
    Derivative,
}

impl StorageNamespace {
    /// Stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Derivative => "derivative",
        }
    }

    const fn prefix(self) -> &'static str {
        match self {
            Self::Raw => "raw/",
            Self::Derivative => "media/",
        }
    }
}

/// A validated provider object supplied to the persistence reconciliation port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryObject {
    /// Validated server-owned key.
    pub key: ObjectKey,
    /// Provider-reported byte length.
    pub content_length: u64,
}

/// An orphan old enough to delete only after one final durable ownership check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanCandidate {
    /// Validated server-owned key.
    pub key: ObjectKey,
    /// Provider-reported byte length.
    pub content_length: u64,
}

/// Durable classification for one provider page.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InventoryReconcileResult {
    /// Objects still owned by a durable upload or derivative row.
    pub known: usize,
    /// Newly or recently suspected orphan objects.
    pub suspected: usize,
    /// Suspects older than the configured grace period.
    pub eligible: Vec<OrphanCandidate>,
}

/// Persistence failure without SQL details.
#[derive(Debug, Error)]
#[error("inventory repository failed: {0}")]
pub struct InventoryRepositoryError(pub String);

/// Durable inventory cursor, orphan observation, and race-check boundary.
#[async_trait]
pub trait InventoryRepository: Send + Sync {
    /// Loads the last provider key scanned for one namespace.
    async fn inventory_cursor(
        &self,
        namespace: StorageNamespace,
    ) -> Result<Option<String>, InventoryRepositoryError>;

    /// Compares one bounded provider page with durable application ownership.
    async fn reconcile_inventory_page(
        &self,
        namespace: StorageNamespace,
        objects: &[InventoryObject],
        observed_at: OffsetDateTime,
        eligible_before: OffsetDateTime,
    ) -> Result<InventoryReconcileResult, InventoryRepositoryError>;

    /// Rechecks ownership immediately before an irreversible provider delete.
    async fn is_inventory_key_known(
        &self,
        namespace: StorageNamespace,
        key: &ObjectKey,
    ) -> Result<bool, InventoryRepositoryError>;

    /// Removes a stale suspicion after the final ownership check finds a row.
    async fn forget_orphan(
        &self,
        namespace: StorageNamespace,
        key: &ObjectKey,
    ) -> Result<(), InventoryRepositoryError>;

    /// Records successful idempotent provider deletion as an audit tombstone.
    async fn complete_orphan_deletion(
        &self,
        namespace: StorageNamespace,
        key: &ObjectKey,
        deleted_at: OffsetDateTime,
    ) -> Result<(), InventoryRepositoryError>;

    /// Records a stable provider deletion failure for the next scan.
    async fn fail_orphan_deletion(
        &self,
        namespace: StorageNamespace,
        key: &ObjectKey,
        error_code: &str,
        failed_at: OffsetDateTime,
    ) -> Result<(), InventoryRepositoryError>;

    /// Persists progress after a page; `None` starts a new full cycle next run.
    async fn save_inventory_cursor(
        &self,
        namespace: StorageNamespace,
        start_after: Option<&str>,
        updated_at: OffsetDateTime,
    ) -> Result<(), InventoryRepositoryError>;
}

/// Hard operator-owned limits for periodic provider scans.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InventoryPolicy {
    /// Minimum observation age before prune mode may delete an orphan.
    pub orphan_grace_period: Duration,
    /// Provider objects requested per page.
    pub page_size: u16,
    /// Pages scanned per namespace and invocation.
    pub max_pages_per_namespace: usize,
}

impl Default for InventoryPolicy {
    fn default() -> Self {
        Self {
            orphan_grace_period: Duration::from_secs(48 * 60 * 60),
            page_size: 1000,
            max_pages_per_namespace: 10,
        }
    }
}

impl InventoryPolicy {
    fn is_valid(self) -> bool {
        self.orphan_grace_period >= Duration::from_secs(60 * 60)
            && self.orphan_grace_period <= Duration::from_secs(30 * 24 * 60 * 60)
            && (1..=1000).contains(&self.page_size)
            && (1..=100).contains(&self.max_pages_per_namespace)
    }
}

/// Safe default audit mode or explicit grace-gated prune mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryMode {
    /// Observe and report without deleting provider objects.
    Audit,
    /// Delete only old suspects after a final database ownership check.
    Prune,
}

/// Bounded inventory outcome suitable for structured logs and metrics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InventoryRunSummary {
    /// Provider pages scanned.
    pub pages: usize,
    /// Provider objects returned before validation.
    pub listed: usize,
    /// Provider keys rejected before persistence or deletion.
    pub invalid_keys: usize,
    /// Objects matched by durable ownership.
    pub known: usize,
    /// Objects still inside the observation grace period.
    pub suspected: usize,
    /// Old orphan candidates found.
    pub eligible: usize,
    /// Provider objects deleted and audit-tombstoned.
    pub deleted: usize,
    /// Provider deletes left for retry.
    pub delete_failed: usize,
    /// Namespaces whose lexicographic cycle reached the end.
    pub completed_namespaces: usize,
}

/// Inventory orchestration failure. Per-object delete failures remain resumable.
#[derive(Debug, Error)]
pub enum InventoryRunError {
    /// Policy disables safety bounds.
    #[error("inventory policy violates page or grace bounds")]
    InvalidPolicy,
    /// Grace duration cannot be represented by the wall-clock type.
    #[error("inventory grace duration is outside the supported range")]
    InvalidDuration,
    /// Provider page order or cursor is inconsistent.
    #[error("provider returned an invalid inventory page")]
    InvalidPage,
    /// Provider listing failed.
    #[error(transparent)]
    ObjectStore(#[from] ObjectStoreError),
    /// Durable inventory state failed.
    #[error(transparent)]
    Repository(#[from] InventoryRepositoryError),
}

/// Single-node provider inventory reconciler with durable cursors and grace-gated pruning.
#[derive(Clone)]
pub struct InventoryService {
    raw_store: Arc<dyn ObjectStore>,
    derivative_store: Arc<dyn ObjectStore>,
    repository: Arc<dyn InventoryRepository>,
    policy: InventoryPolicy,
}

impl InventoryService {
    /// Creates a reconciler only when every scan bound is explicit and safe.
    pub fn new(
        raw_store: Arc<dyn ObjectStore>,
        derivative_store: Arc<dyn ObjectStore>,
        repository: Arc<dyn InventoryRepository>,
        policy: InventoryPolicy,
    ) -> Result<Self, InventoryRunError> {
        if !policy.is_valid() {
            return Err(InventoryRunError::InvalidPolicy);
        }
        Ok(Self {
            raw_store,
            derivative_store,
            repository,
            policy,
        })
    }

    /// Scans both fixed namespaces once, defaulting to non-destructive audit behavior.
    pub async fn run_once(
        &self,
        mode: InventoryMode,
    ) -> Result<InventoryRunSummary, InventoryRunError> {
        let now = OffsetDateTime::now_utc();
        let grace = time::Duration::try_from(self.policy.orphan_grace_period)
            .map_err(|_| InventoryRunError::InvalidDuration)?;
        let eligible_before = now - grace;
        let mut summary = InventoryRunSummary::default();
        for namespace in [StorageNamespace::Raw, StorageNamespace::Derivative] {
            let store = match namespace {
                StorageNamespace::Raw => &self.raw_store,
                StorageNamespace::Derivative => &self.derivative_store,
            };
            let mut cursor = self.repository.inventory_cursor(namespace).await?;
            validate_cursor(namespace, cursor.as_deref())?;
            for _ in 0..self.policy.max_pages_per_namespace {
                let page = store
                    .list_objects(ListObjectsRequest {
                        prefix: namespace.prefix().to_owned(),
                        start_after: cursor.clone(),
                        max_keys: self.policy.page_size,
                    })
                    .await?;
                validate_page(
                    namespace,
                    cursor.as_deref(),
                    &page.objects,
                    page.next_start_after.as_deref(),
                )?;
                summary.pages += 1;
                summary.listed += page.objects.len();
                let mut valid = Vec::with_capacity(page.objects.len());
                for object in &page.objects {
                    match ObjectKey::new(object.key.clone()) {
                        Ok(key) if object.key.starts_with(namespace.prefix()) => {
                            valid.push(InventoryObject {
                                key,
                                content_length: object.content_length,
                            });
                        }
                        _ => summary.invalid_keys += 1,
                    }
                }
                let reconciled = self
                    .repository
                    .reconcile_inventory_page(namespace, &valid, now, eligible_before)
                    .await?;
                summary.known += reconciled.known;
                summary.suspected += reconciled.suspected;
                summary.eligible += reconciled.eligible.len();
                if mode == InventoryMode::Prune {
                    for candidate in reconciled.eligible {
                        if self
                            .repository
                            .is_inventory_key_known(namespace, &candidate.key)
                            .await?
                        {
                            self.repository
                                .forget_orphan(namespace, &candidate.key)
                                .await?;
                            summary.known += 1;
                            summary.eligible = summary.eligible.saturating_sub(1);
                            continue;
                        }
                        match store.delete(&candidate.key).await {
                            Ok(()) => {
                                self.repository
                                    .complete_orphan_deletion(
                                        namespace,
                                        &candidate.key,
                                        OffsetDateTime::now_utc(),
                                    )
                                    .await?;
                                summary.deleted += 1;
                            }
                            Err(_) => {
                                self.repository
                                    .fail_orphan_deletion(
                                        namespace,
                                        &candidate.key,
                                        "PROVIDER_DELETE_FAILED",
                                        OffsetDateTime::now_utc(),
                                    )
                                    .await?;
                                summary.delete_failed += 1;
                            }
                        }
                    }
                }
                self.repository
                    .save_inventory_cursor(
                        namespace,
                        page.next_start_after.as_deref(),
                        OffsetDateTime::now_utc(),
                    )
                    .await?;
                cursor = page.next_start_after;
                if cursor.is_none() {
                    summary.completed_namespaces += 1;
                    break;
                }
            }
        }
        Ok(summary)
    }
}

fn validate_cursor(
    namespace: StorageNamespace,
    cursor: Option<&str>,
) -> Result<(), InventoryRunError> {
    if cursor.is_some_and(|value| value.len() > 1024 || !value.starts_with(namespace.prefix())) {
        return Err(InventoryRunError::InvalidPage);
    }
    Ok(())
}

fn validate_page(
    namespace: StorageNamespace,
    previous: Option<&str>,
    objects: &[ListedObject],
    next: Option<&str>,
) -> Result<(), InventoryRunError> {
    if objects.len() > 1000
        || objects.windows(2).any(|pair| pair[0].key >= pair[1].key)
        || objects.iter().any(|object| {
            object.key.len() > 1024
                || !object.key.starts_with(namespace.prefix())
                || previous.is_some_and(|cursor| object.key.as_str() <= cursor)
        })
        || next
            .is_some_and(|cursor| objects.last().map(|object| object.key.as_str()) != Some(cursor))
    {
        return Err(InventoryRunError::InvalidPage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use g7mb_domain::ObjectKey;
    use time::OffsetDateTime;

    use super::{
        InventoryMode, InventoryObject, InventoryPolicy, InventoryReconcileResult,
        InventoryRepository, InventoryRepositoryError, InventoryService, OrphanCandidate,
        StorageNamespace,
    };
    use crate::{
        AbortMultipartRequest, CompleteMultipartRequest, CreateMultipartRequest,
        DownloadObjectRequest, ListObjectsRequest, ListedObject, ListedObjectsPage,
        MultipartSession, ObjectMetadata, ObjectStore, ObjectStoreError, PresignPartRequest,
        PresignPutRequest, PresignedUpload, PutFileRequest,
    };

    struct FakeStore {
        objects: Vec<ListedObject>,
        deletes: AtomicUsize,
    }

    impl FakeStore {
        fn unsupported<T>() -> Result<T, ObjectStoreError> {
            Err(ObjectStoreError::InvalidRequest("not used".to_owned()))
        }
    }

    #[async_trait]
    impl ObjectStore for FakeStore {
        async fn presign_put(
            &self,
            _request: PresignPutRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Self::unsupported()
        }

        async fn create_multipart(
            &self,
            _request: CreateMultipartRequest,
        ) -> Result<MultipartSession, ObjectStoreError> {
            Self::unsupported()
        }

        async fn presign_part(
            &self,
            _request: PresignPartRequest,
        ) -> Result<PresignedUpload, ObjectStoreError> {
            Self::unsupported()
        }

        async fn complete_multipart(
            &self,
            _request: CompleteMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            Self::unsupported()
        }

        async fn abort_multipart(
            &self,
            _request: AbortMultipartRequest,
        ) -> Result<(), ObjectStoreError> {
            Self::unsupported()
        }

        async fn head(&self, _key: &ObjectKey) -> Result<ObjectMetadata, ObjectStoreError> {
            Self::unsupported()
        }

        async fn download_to(
            &self,
            _request: DownloadObjectRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Self::unsupported()
        }

        async fn put_file(
            &self,
            _request: PutFileRequest,
        ) -> Result<ObjectMetadata, ObjectStoreError> {
            Self::unsupported()
        }

        async fn list_objects(
            &self,
            _request: ListObjectsRequest,
        ) -> Result<ListedObjectsPage, ObjectStoreError> {
            Ok(ListedObjectsPage {
                objects: self.objects.clone(),
                next_start_after: None,
            })
        }

        async fn delete(&self, _key: &ObjectKey) -> Result<(), ObjectStoreError> {
            self.deletes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeRepository {
        known_on_recheck: AtomicBool,
        completed: AtomicUsize,
    }

    #[async_trait]
    impl InventoryRepository for FakeRepository {
        async fn inventory_cursor(
            &self,
            _namespace: StorageNamespace,
        ) -> Result<Option<String>, InventoryRepositoryError> {
            Ok(None)
        }

        async fn reconcile_inventory_page(
            &self,
            _namespace: StorageNamespace,
            objects: &[InventoryObject],
            _observed_at: OffsetDateTime,
            _eligible_before: OffsetDateTime,
        ) -> Result<InventoryReconcileResult, InventoryRepositoryError> {
            Ok(InventoryReconcileResult {
                known: 0,
                suspected: 0,
                eligible: objects
                    .iter()
                    .map(|object| OrphanCandidate {
                        key: object.key.clone(),
                        content_length: object.content_length,
                    })
                    .collect(),
            })
        }

        async fn is_inventory_key_known(
            &self,
            _namespace: StorageNamespace,
            _key: &ObjectKey,
        ) -> Result<bool, InventoryRepositoryError> {
            Ok(self.known_on_recheck.load(Ordering::Relaxed))
        }

        async fn forget_orphan(
            &self,
            _namespace: StorageNamespace,
            _key: &ObjectKey,
        ) -> Result<(), InventoryRepositoryError> {
            Ok(())
        }

        async fn complete_orphan_deletion(
            &self,
            _namespace: StorageNamespace,
            _key: &ObjectKey,
            _deleted_at: OffsetDateTime,
        ) -> Result<(), InventoryRepositoryError> {
            self.completed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn fail_orphan_deletion(
            &self,
            _namespace: StorageNamespace,
            _key: &ObjectKey,
            _error_code: &str,
            _failed_at: OffsetDateTime,
        ) -> Result<(), InventoryRepositoryError> {
            Ok(())
        }

        async fn save_inventory_cursor(
            &self,
            _namespace: StorageNamespace,
            _start_after: Option<&str>,
            _updated_at: OffsetDateTime,
        ) -> Result<(), InventoryRepositoryError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn audit_never_deletes_and_prune_rechecks_ownership()
    -> Result<(), Box<dyn std::error::Error>> {
        let raw_store = Arc::new(FakeStore {
            objects: vec![ListedObject {
                key: "raw/site-a/orphan/source".to_owned(),
                content_length: 1024,
            }],
            deletes: AtomicUsize::new(0),
        });
        let derivative_store = Arc::new(FakeStore {
            objects: Vec::new(),
            deletes: AtomicUsize::new(0),
        });
        let repository = Arc::new(FakeRepository::default());
        let service = InventoryService::new(
            raw_store.clone(),
            derivative_store,
            repository.clone(),
            InventoryPolicy::default(),
        )?;

        let audit = service.run_once(InventoryMode::Audit).await?;
        assert_eq!(audit.eligible, 1);
        assert_eq!(audit.deleted, 0);
        assert_eq!(raw_store.deletes.load(Ordering::Relaxed), 0);

        repository.known_on_recheck.store(true, Ordering::Relaxed);
        let raced = service.run_once(InventoryMode::Prune).await?;
        assert_eq!(raced.eligible, 0);
        assert_eq!(raced.known, 1);
        assert_eq!(raw_store.deletes.load(Ordering::Relaxed), 0);

        repository.known_on_recheck.store(false, Ordering::Relaxed);
        let pruned = service.run_once(InventoryMode::Prune).await?;
        assert_eq!(pruned.deleted, 1);
        assert_eq!(raw_store.deletes.load(Ordering::Relaxed), 1);
        assert_eq!(repository.completed.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn inventory_policy_cannot_disable_grace_or_page_bounds() {
        let repository = Arc::new(FakeRepository::default());
        let store = Arc::new(FakeStore {
            objects: Vec::new(),
            deletes: AtomicUsize::new(0),
        });
        let result = InventoryService::new(
            store.clone(),
            store,
            repository,
            InventoryPolicy {
                orphan_grace_period: std::time::Duration::ZERO,
                page_size: 1001,
                max_pages_per_namespace: 0,
            },
        );
        assert!(result.is_err());
    }
}
