CREATE TABLE upload_batches (
    id TEXT PRIMARY KEY NOT NULL,
    tenant_id TEXT NOT NULL,
    state TEXT NOT NULL,
    file_count INTEGER NOT NULL,
    expected_size_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (state IN ('created', 'uploading', 'completed', 'aborted', 'expired')),
    CHECK (file_count BETWEEN 1 AND 100),
    CHECK (expected_size_bytes > 0)
);

CREATE INDEX upload_batches_tenant_state_idx
    ON upload_batches (tenant_id, state, updated_at);

ALTER TABLE uploads ADD COLUMN batch_id TEXT REFERENCES upload_batches(id) ON DELETE CASCADE;
ALTER TABLE uploads ADD COLUMN declared_kind TEXT
    CHECK (declared_kind IN ('image', 'video'));
ALTER TABLE uploads ADD COLUMN transfer_kind TEXT NOT NULL DEFAULT 'single_put'
    CHECK (transfer_kind IN ('single_put', 'multipart'));
ALTER TABLE uploads ADD COLUMN multipart_upload_id TEXT;

CREATE INDEX uploads_batch_idx ON uploads (batch_id, created_at);

CREATE TABLE upload_parts (
    upload_id TEXT NOT NULL REFERENCES uploads(id) ON DELETE CASCADE,
    part_number INTEGER NOT NULL,
    content_length INTEGER NOT NULL,
    etag TEXT,
    completed_at INTEGER,
    PRIMARY KEY (upload_id, part_number),
    CHECK (part_number BETWEEN 1 AND 10000),
    CHECK (content_length > 0)
);
