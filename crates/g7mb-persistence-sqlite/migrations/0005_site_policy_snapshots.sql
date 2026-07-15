CREATE TABLE site_policy_snapshots (
    tenant_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    issued_at INTEGER NOT NULL,
    settings_sha256 TEXT NOT NULL,
    watermark_enabled INTEGER NOT NULL,
    watermark_upload_id TEXT REFERENCES uploads(id) ON DELETE RESTRICT,
    watermark_object_key TEXT,
    watermark_byte_len INTEGER,
    watermark_sha256 TEXT,
    watermark_position TEXT,
    watermark_margin_px INTEGER,
    watermark_max_width_percent INTEGER,
    watermark_opacity_percent INTEGER,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (tenant_id, revision),
    CHECK (length(tenant_id) BETWEEN 1 AND 64),
    CHECK (revision > 0),
    CHECK (schema_version = 1),
    CHECK (length(settings_sha256) = 64),
    CHECK (watermark_enabled IN (0, 1)),
    CHECK (
        (watermark_enabled = 0
            AND watermark_upload_id IS NULL
            AND watermark_object_key IS NULL
            AND watermark_byte_len IS NULL
            AND watermark_sha256 IS NULL
            AND watermark_position IS NULL
            AND watermark_margin_px IS NULL
            AND watermark_max_width_percent IS NULL
            AND watermark_opacity_percent IS NULL)
        OR
        (watermark_enabled = 1
            AND watermark_upload_id IS NOT NULL
            AND watermark_object_key IS NOT NULL
            AND watermark_byte_len BETWEEN 1 AND 16777216
            AND length(watermark_sha256) = 64
            AND watermark_position IN ('center', 'top_left', 'top_right', 'bottom_left', 'bottom_right')
            AND watermark_margin_px BETWEEN 0 AND 1024
            AND watermark_max_width_percent BETWEEN 1 AND 50
            AND watermark_opacity_percent BETWEEN 1 AND 100)
    )
);

CREATE INDEX site_policy_snapshots_active_idx
    ON site_policy_snapshots (tenant_id, revision DESC);
