CREATE TABLE inventory_cursors (
    namespace TEXT PRIMARY KEY NOT NULL,
    start_after TEXT,
    updated_at INTEGER NOT NULL,
    CHECK (namespace IN ('raw', 'derivative')),
    CHECK (start_after IS NULL OR length(start_after) BETWEEN 1 AND 1024)
);

CREATE TABLE orphan_objects (
    namespace TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_length INTEGER NOT NULL,
    first_seen_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    state TEXT NOT NULL,
    delete_attempts INTEGER NOT NULL DEFAULT 0,
    last_error_code TEXT,
    deleted_at INTEGER,
    PRIMARY KEY (namespace, object_key),
    CHECK (namespace IN ('raw', 'derivative')),
    CHECK (length(object_key) BETWEEN 1 AND 1024),
    CHECK (
        (namespace = 'raw' AND substr(object_key, 1, 4) = 'raw/')
        OR (namespace = 'derivative' AND substr(object_key, 1, 6) = 'media/')
    ),
    CHECK (content_length >= 0),
    CHECK (last_seen_at >= first_seen_at),
    CHECK (state IN ('suspected', 'deleted')),
    CHECK (delete_attempts >= 0),
    CHECK (last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 64),
    CHECK (
        (state = 'suspected' AND deleted_at IS NULL)
        OR (state = 'deleted' AND deleted_at IS NOT NULL)
    )
);

CREATE INDEX orphan_objects_eligible_idx
    ON orphan_objects (namespace, state, first_seen_at);
