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
INSERT OR IGNORE INTO ledger_meta(key, value) VALUES ('schema_sentinel', 'mission-ledger-sqlcipher-v1');
