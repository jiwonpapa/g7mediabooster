CREATE TABLE storage_usage_global (
    singleton INTEGER PRIMARY KEY NOT NULL DEFAULT 1,
    reserved_bytes INTEGER NOT NULL,
    CHECK (singleton = 1),
    CHECK (reserved_bytes >= 0)
);

INSERT INTO storage_usage_global (singleton, reserved_bytes)
SELECT 1, COALESCE(SUM(expected_size_bytes), 0)
FROM uploads
WHERE state <> 'deleted';

CREATE TABLE tenant_storage_usage (
    tenant_id TEXT PRIMARY KEY NOT NULL,
    reserved_bytes INTEGER NOT NULL,
    CHECK (length(tenant_id) BETWEEN 1 AND 64),
    CHECK (reserved_bytes >= 0)
);

INSERT INTO tenant_storage_usage (tenant_id, reserved_bytes)
SELECT tenant_id, SUM(expected_size_bytes)
FROM uploads
WHERE state <> 'deleted'
GROUP BY tenant_id;

CREATE TRIGGER uploads_storage_usage_after_insert
AFTER INSERT ON uploads
WHEN NEW.state <> 'deleted'
BEGIN
    UPDATE storage_usage_global
    SET reserved_bytes = reserved_bytes + NEW.expected_size_bytes
    WHERE singleton = 1;

    INSERT INTO tenant_storage_usage (tenant_id, reserved_bytes)
    VALUES (NEW.tenant_id, NEW.expected_size_bytes)
    ON CONFLICT (tenant_id) DO UPDATE
    SET reserved_bytes = reserved_bytes + excluded.reserved_bytes;
END;

CREATE TRIGGER uploads_storage_usage_after_update
AFTER UPDATE OF tenant_id, state, expected_size_bytes ON uploads
WHEN OLD.tenant_id <> NEW.tenant_id
  OR OLD.state <> NEW.state
  OR OLD.expected_size_bytes <> NEW.expected_size_bytes
BEGIN
    UPDATE storage_usage_global
    SET reserved_bytes = reserved_bytes
        - CASE WHEN OLD.state <> 'deleted' THEN OLD.expected_size_bytes ELSE 0 END
        + CASE WHEN NEW.state <> 'deleted' THEN NEW.expected_size_bytes ELSE 0 END
    WHERE singleton = 1;

    UPDATE tenant_storage_usage
    SET reserved_bytes = reserved_bytes - OLD.expected_size_bytes
    WHERE tenant_id = OLD.tenant_id
      AND OLD.state <> 'deleted';

    DELETE FROM tenant_storage_usage
    WHERE tenant_id = OLD.tenant_id
      AND reserved_bytes = 0;

    INSERT INTO tenant_storage_usage (tenant_id, reserved_bytes)
    SELECT NEW.tenant_id, NEW.expected_size_bytes
    WHERE NEW.state <> 'deleted'
    ON CONFLICT (tenant_id) DO UPDATE
    SET reserved_bytes = reserved_bytes + excluded.reserved_bytes;
END;

CREATE TRIGGER uploads_storage_usage_after_delete
AFTER DELETE ON uploads
WHEN OLD.state <> 'deleted'
BEGIN
    UPDATE storage_usage_global
    SET reserved_bytes = reserved_bytes - OLD.expected_size_bytes
    WHERE singleton = 1;

    UPDATE tenant_storage_usage
    SET reserved_bytes = reserved_bytes - OLD.expected_size_bytes
    WHERE tenant_id = OLD.tenant_id;

    DELETE FROM tenant_storage_usage
    WHERE tenant_id = OLD.tenant_id
      AND reserved_bytes = 0;
END;
