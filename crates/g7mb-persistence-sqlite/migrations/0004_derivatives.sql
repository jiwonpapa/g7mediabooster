CREATE TABLE derivatives (
    upload_id TEXT NOT NULL REFERENCES uploads(id) ON DELETE CASCADE,
    preset_id TEXT NOT NULL,
    variant TEXT NOT NULL,
    object_key TEXT NOT NULL UNIQUE,
    content_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (upload_id, preset_id, variant),
    CHECK (length(preset_id) BETWEEN 1 AND 128),
    CHECK (length(variant) BETWEEN 1 AND 64),
    CHECK (byte_len > 0),
    CHECK (length(sha256) = 64)
);

CREATE INDEX derivatives_upload_idx ON derivatives (upload_id, created_at);
