PRAGMA foreign_keys = ON;

CREATE TABLE uploads (
    id TEXT PRIMARY KEY NOT NULL,
    tenant_id TEXT NOT NULL,
    object_key TEXT NOT NULL UNIQUE,
    media_kind TEXT,
    state TEXT NOT NULL,
    expected_size_bytes INTEGER NOT NULL,
    actual_size_bytes INTEGER,
    content_type_hint TEXT NOT NULL,
    detected_content_type TEXT,
    source_sha256 TEXT,
    error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (state IN ('created', 'uploaded', 'quarantined', 'processing', 'ready', 'rejected', 'failed', 'deleted')),
    CHECK (expected_size_bytes >= 0),
    CHECK (actual_size_bytes IS NULL OR actual_size_bytes >= 0)
);

CREATE INDEX uploads_tenant_state_idx ON uploads (tenant_id, state, updated_at);

CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    upload_id TEXT NOT NULL REFERENCES uploads(id) ON DELETE CASCADE,
    preset_id TEXT NOT NULL,
    state TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at INTEGER NOT NULL,
    lease_owner TEXT,
    lease_until INTEGER,
    last_error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (upload_id, preset_id),
    CHECK (state IN ('queued', 'leased', 'completed', 'dead_letter')),
    CHECK (attempts >= 0)
);

CREATE INDEX jobs_lease_idx ON jobs (state, available_at, lease_until);

CREATE TABLE request_nonces (
    key_id TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (key_id, nonce)
);

CREATE INDEX request_nonces_expiry_idx ON request_nonces (expires_at);
