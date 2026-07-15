UPDATE uploads
SET deleted_at = updated_at
WHERE state = 'deleted' AND deleted_at IS NULL;

CREATE TABLE operational_counters (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    queued_jobs INTEGER NOT NULL CHECK (queued_jobs >= 0),
    leased_jobs INTEGER NOT NULL CHECK (leased_jobs >= 0),
    dead_letter_jobs INTEGER NOT NULL CHECK (dead_letter_jobs >= 0),
    processing_uploads INTEGER NOT NULL CHECK (processing_uploads >= 0),
    cleanup_pending_uploads INTEGER NOT NULL CHECK (cleanup_pending_uploads >= 0),
    upload_tombstones INTEGER NOT NULL CHECK (upload_tombstones >= 0),
    orphan_suspects INTEGER NOT NULL CHECK (orphan_suspects >= 0),
    orphan_delete_failures INTEGER NOT NULL CHECK (orphan_delete_failures >= 0)
);

INSERT INTO operational_counters (
    singleton,
    queued_jobs,
    leased_jobs,
    dead_letter_jobs,
    processing_uploads,
    cleanup_pending_uploads,
    upload_tombstones,
    orphan_suspects,
    orphan_delete_failures
)
VALUES (
    1,
    (SELECT COUNT(*) FROM jobs WHERE state = 'queued'),
    (SELECT COUNT(*) FROM jobs WHERE state = 'leased'),
    (SELECT COUNT(*) FROM jobs WHERE state = 'dead_letter'),
    (SELECT COUNT(*) FROM uploads WHERE state = 'processing'),
    (SELECT COUNT(*) FROM uploads WHERE state <> 'deleted' AND delete_requested_at IS NOT NULL),
    (SELECT COUNT(*) FROM uploads WHERE state = 'deleted'),
    (SELECT COUNT(*) FROM orphan_objects WHERE state = 'suspected'),
    (SELECT COUNT(*) FROM orphan_objects
        WHERE state = 'suspected' AND last_error_code IS NOT NULL)
);

CREATE INDEX jobs_state_created_at_idx ON jobs (state, created_at);

CREATE TRIGGER operational_jobs_insert
AFTER INSERT ON jobs
BEGIN
    UPDATE operational_counters SET
        queued_jobs = queued_jobs + (NEW.state = 'queued'),
        leased_jobs = leased_jobs + (NEW.state = 'leased'),
        dead_letter_jobs = dead_letter_jobs + (NEW.state = 'dead_letter')
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_jobs_update
AFTER UPDATE OF state ON jobs
BEGIN
    UPDATE operational_counters SET
        queued_jobs = queued_jobs + (NEW.state = 'queued') - (OLD.state = 'queued'),
        leased_jobs = leased_jobs + (NEW.state = 'leased') - (OLD.state = 'leased'),
        dead_letter_jobs = dead_letter_jobs
            + (NEW.state = 'dead_letter') - (OLD.state = 'dead_letter')
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_jobs_delete
AFTER DELETE ON jobs
BEGIN
    UPDATE operational_counters SET
        queued_jobs = queued_jobs - (OLD.state = 'queued'),
        leased_jobs = leased_jobs - (OLD.state = 'leased'),
        dead_letter_jobs = dead_letter_jobs - (OLD.state = 'dead_letter')
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_uploads_insert
AFTER INSERT ON uploads
BEGIN
    UPDATE operational_counters SET
        processing_uploads = processing_uploads + (NEW.state = 'processing'),
        cleanup_pending_uploads = cleanup_pending_uploads
            + (NEW.state <> 'deleted' AND NEW.delete_requested_at IS NOT NULL),
        upload_tombstones = upload_tombstones + (NEW.state = 'deleted')
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_uploads_update
AFTER UPDATE OF state, delete_requested_at ON uploads
BEGIN
    UPDATE operational_counters SET
        processing_uploads = processing_uploads
            + (NEW.state = 'processing') - (OLD.state = 'processing'),
        cleanup_pending_uploads = cleanup_pending_uploads
            + (NEW.state <> 'deleted' AND NEW.delete_requested_at IS NOT NULL)
            - (OLD.state <> 'deleted' AND OLD.delete_requested_at IS NOT NULL),
        upload_tombstones = upload_tombstones
            + (NEW.state = 'deleted') - (OLD.state = 'deleted')
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_uploads_delete
AFTER DELETE ON uploads
BEGIN
    UPDATE operational_counters SET
        processing_uploads = processing_uploads - (OLD.state = 'processing'),
        cleanup_pending_uploads = cleanup_pending_uploads
            - (OLD.state <> 'deleted' AND OLD.delete_requested_at IS NOT NULL),
        upload_tombstones = upload_tombstones - (OLD.state = 'deleted')
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_orphans_insert
AFTER INSERT ON orphan_objects
BEGIN
    UPDATE operational_counters SET
        orphan_suspects = orphan_suspects + (NEW.state = 'suspected'),
        orphan_delete_failures = orphan_delete_failures
            + (NEW.state = 'suspected' AND NEW.last_error_code IS NOT NULL)
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_orphans_update
AFTER UPDATE OF state, last_error_code ON orphan_objects
BEGIN
    UPDATE operational_counters SET
        orphan_suspects = orphan_suspects
            + (NEW.state = 'suspected') - (OLD.state = 'suspected'),
        orphan_delete_failures = orphan_delete_failures
            + (NEW.state = 'suspected' AND NEW.last_error_code IS NOT NULL)
            - (OLD.state = 'suspected' AND OLD.last_error_code IS NOT NULL)
    WHERE singleton = 1;
END;

CREATE TRIGGER operational_orphans_delete
AFTER DELETE ON orphan_objects
BEGIN
    UPDATE operational_counters SET
        orphan_suspects = orphan_suspects - (OLD.state = 'suspected'),
        orphan_delete_failures = orphan_delete_failures
            - (OLD.state = 'suspected' AND OLD.last_error_code IS NOT NULL)
    WHERE singleton = 1;
END;
