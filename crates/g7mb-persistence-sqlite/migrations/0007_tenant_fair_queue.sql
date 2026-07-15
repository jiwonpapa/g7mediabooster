CREATE TABLE queue_sequence (
    singleton INTEGER PRIMARY KEY NOT NULL DEFAULT 1,
    next_claim_sequence INTEGER NOT NULL,
    CHECK (singleton = 1),
    CHECK (next_claim_sequence > 0)
);

INSERT INTO queue_sequence (singleton, next_claim_sequence) VALUES (1, 1);

CREATE TABLE tenant_queue_state (
    tenant_id TEXT PRIMARY KEY NOT NULL,
    last_claim_sequence INTEGER NOT NULL,
    CHECK (length(tenant_id) BETWEEN 1 AND 64),
    CHECK (last_claim_sequence > 0)
);

CREATE INDEX tenant_queue_state_sequence_idx
    ON tenant_queue_state (last_claim_sequence, tenant_id);
