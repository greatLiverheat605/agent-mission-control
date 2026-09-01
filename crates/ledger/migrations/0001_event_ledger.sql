CREATE TABLE IF NOT EXISTS ledger_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
    mission_id TEXT NOT NULL,
    route_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    event_id TEXT NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL,
    kind TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    raw_evidence TEXT,
    payload_hash TEXT NOT NULL,
    PRIMARY KEY (mission_id, sequence)
);
CREATE INDEX IF NOT EXISTS events_mission_sequence ON events (mission_id, sequence);
CREATE TABLE IF NOT EXISTS blob_refs (
    blob_hash TEXT PRIMARY KEY NOT NULL,
    size INTEGER NOT NULL,
    media_type TEXT NOT NULL,
    ref_count INTEGER NOT NULL DEFAULT 0 CHECK (ref_count >= 0)
);
CREATE TABLE IF NOT EXISTS mission_blob_refs (
    mission_id TEXT NOT NULL,
    blob_hash TEXT NOT NULL,
    size INTEGER NOT NULL,
    media_type TEXT NOT NULL,
    PRIMARY KEY (mission_id, blob_hash),
    FOREIGN KEY (blob_hash) REFERENCES blob_refs(blob_hash) ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS mission_lifecycle (
    mission_id TEXT PRIMARY KEY NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    archived_at TEXT,
    archive_plan_hash TEXT
);
CREATE TABLE IF NOT EXISTS lifecycle_audit (
    receipt_id TEXT PRIMARY KEY NOT NULL,
    operation TEXT NOT NULL,
    mission_id TEXT NOT NULL,
    plan_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    receipt_json TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS lifecycle_audit_plan ON lifecycle_audit(operation, plan_hash);
CREATE TRIGGER IF NOT EXISTS lifecycle_audit_no_update
BEFORE UPDATE ON lifecycle_audit
BEGIN
    SELECT RAISE(ABORT, 'lifecycle audit receipts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS lifecycle_audit_no_delete
BEFORE DELETE ON lifecycle_audit
BEGIN
    SELECT RAISE(ABORT, 'lifecycle audit receipts are immutable');
END;
INSERT OR IGNORE INTO ledger_meta(key, value) VALUES ('schema_sentinel', 'mission-ledger-sqlcipher-v1');
