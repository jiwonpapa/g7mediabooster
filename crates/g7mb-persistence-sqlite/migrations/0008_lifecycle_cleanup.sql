ALTER TABLE uploads ADD COLUMN delete_requested_at INTEGER;
ALTER TABLE uploads ADD COLUMN deleted_at INTEGER;
ALTER TABLE uploads ADD COLUMN cleanup_lease_owner TEXT;
ALTER TABLE uploads ADD COLUMN cleanup_lease_until INTEGER;
ALTER TABLE uploads ADD COLUMN cleanup_retry_at INTEGER;
ALTER TABLE uploads ADD COLUMN cleanup_attempts INTEGER NOT NULL DEFAULT 0
    CHECK (cleanup_attempts >= 0);
ALTER TABLE uploads ADD COLUMN cleanup_error_code TEXT;

CREATE INDEX uploads_cleanup_eligibility_idx
    ON uploads (state, delete_requested_at, cleanup_retry_at, cleanup_lease_until, updated_at);
