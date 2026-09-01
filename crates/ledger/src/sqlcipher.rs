use std::fs;
use std::path::{Path, PathBuf};

use mission_domain::{EventEnvelope, EventId, MissionId, RouteId, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::key_store::{KeyStore, KeyStoreError};
use crate::migrations::{SCHEMA, backup_database, restore_database};
use crate::redaction::{RedactionError, Redactor};

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("KEY_MISMATCH")]
    KeyMismatch,
    #[error("KEY_UNAVAILABLE")]
    KeyUnavailable,
    #[error("MIGRATION_FAILED_READ_ONLY")]
    MigrationFailedReadOnly,
    #[error("REDACTION_LIMIT_EXCEEDED")]
    RedactionLimitExceeded,
    #[error("duplicate event is not equivalent")]
    DuplicateConflict,
    #[error("sequence must be monotonic")]
    SequenceViolation,
    #[error("database I/O failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid event payload: {0}")]
    InvalidPayload(String),
    #[error("LEDGER_INTEGRITY_FAILED: {0}")]
    IntegrityFailed(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedgerIntegrityReport {
    pub event_count: u64,
    pub last_committed_sequence: u64,
    pub recovery_required: bool,
    pub reason: Option<String>,
}

impl From<KeyStoreError> for LedgerError {
    fn from(_: KeyStoreError) -> Self {
        Self::KeyUnavailable
    }
}

impl From<RedactionError> for LedgerError {
    fn from(_: RedactionError) -> Self {
        Self::RedactionLimitExceeded
    }
}

pub struct EncryptedLedger {
    path: PathBuf,
    connection: Connection,
    redactor: Redactor,
}

impl EncryptedLedger {
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

impl EncryptedLedger {
    pub fn open(
        path: impl AsRef<Path>,
        install_id: impl Into<String>,
        key_store: impl KeyStore,
    ) -> Result<Self, LedgerError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let install_id = install_id.into();
        let is_new = !path.exists() || fs::metadata(&path)?.len() == 0;
        let key = if is_new {
            key_store.load_or_create_database_key(&install_id)?
        } else {
            key_store.load_database_key(&install_id)?
        };
        let backup = if is_new {
            None
        } else {
            Some(backup_database(&path)?)
        };
        let connection = Connection::open(&path)?;
        apply_key(&connection, &key)?;
        connection.pragma_update(None, "cipher_compatibility", 4_i64)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        let readable = connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        });
        if readable.is_err() {
            return Err(LedgerError::KeyMismatch);
        }
        let result = connection.execute_batch(SCHEMA);
        if let Err(error) = result {
            if let Some(backup) = backup {
                drop(connection);
                restore_database(&backup, &path)?;
            }
            return Err(if is_new {
                LedgerError::Database(error)
            } else {
                LedgerError::MigrationFailedReadOnly
            });
        }
        ensure_raw_evidence_column(&connection)?;
        let sentinel: Option<String> = connection
            .query_row(
                "SELECT value FROM ledger_meta WHERE key = 'schema_sentinel'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if sentinel.as_deref() != Some("mission-ledger-sqlcipher-v1") {
            return Err(LedgerError::KeyMismatch);
        }
        let ledger = Self {
            path,
            connection,
            redactor: Redactor::default(),
        };
        ledger.integrity_report()?;
        Ok(ledger)
    }

    pub fn append(&mut self, event: &EventEnvelope) -> Result<(), LedgerError> {
        if !event.has_valid_payload_hash() {
            return Err(LedgerError::InvalidPayload(
                "event payload hash does not match payload".to_owned(),
            ));
        }
        let sequence = i64::try_from(event.sequence).map_err(|_| LedgerError::SequenceViolation)?;
        let redacted = self.redactor.redact_event(event.payload.clone())?;
        let payload = serde_json::to_string(&redacted.value)
            .map_err(|error| LedgerError::InvalidPayload(error.to_string()))?;
        let persisted_payload_hash = persisted_payload_hash(&redacted.value);
        let persisted_raw_evidence = event
            .raw_evidence
            .as_ref()
            .map(|value| self.redactor.redact_event(value.clone()))
            .transpose()?
            .map(|redacted| {
                serde_json::to_string(&redacted.value)
                    .map_err(|error| LedgerError::InvalidPayload(error.to_string()))
            })
            .transpose()?;
        let tx = self.connection.transaction()?;
        let last: Option<i64> = tx
            .query_row(
                "SELECT sequence FROM events WHERE mission_id = ?1 ORDER BY sequence DESC LIMIT 1",
                [event.mission_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = tx
            .query_row(
                "SELECT mission_id, sequence, payload_hash, raw_evidence, schema_version, route_id, kind, occurred_at FROM events WHERE event_id = ?1",
                [event.event_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?
        {
            return if existing.0 == event.mission_id.to_string()
                && existing.1 == sequence
                && existing.2 == persisted_payload_hash
                && existing.3 == persisted_raw_evidence
                && existing.4 == i64::from(event.schema_version)
                && existing.5 == event.route_id.to_string()
                && existing.6 == event.kind.as_str()
                && existing.7 == event.occurred_at.as_str()
            {
                Ok(())
            } else {
                Err(LedgerError::DuplicateConflict)
            };
        }
        if last.map_or(sequence != 1, |previous| sequence != previous + 1) {
            return Err(LedgerError::SequenceViolation);
        }
        tx.execute(
            "INSERT INTO events(mission_id, route_id, sequence, event_id, schema_version, kind, occurred_at, payload, raw_evidence, payload_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![event.mission_id.to_string(), event.route_id.to_string(), sequence, event.event_id.to_string(), event.schema_version as i64, event.kind.as_str(), event.occurred_at.as_str(), payload, persisted_raw_evidence, persisted_payload_hash],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Append a group of events in one transaction.
    ///
    /// All validation and redaction is performed before the transaction is
    /// committed.  Any sequence, duplicate, or database error therefore
    /// rolls the entire group back, which is required for multi-event
    /// lifecycle operations such as mission creation.
    pub fn append_batch(&mut self, events: &[EventEnvelope]) -> Result<(), LedgerError> {
        if events.is_empty() {
            return Ok(());
        }

        struct PreparedEvent<'a> {
            event: &'a EventEnvelope,
            sequence: i64,
            payload: String,
            raw_evidence: Option<String>,
            payload_hash: String,
        }

        let mut prepared = Vec::with_capacity(events.len());
        for event in events {
            if !event.has_valid_payload_hash() {
                return Err(LedgerError::InvalidPayload(
                    "event payload hash does not match payload".to_owned(),
                ));
            }
            let sequence =
                i64::try_from(event.sequence).map_err(|_| LedgerError::SequenceViolation)?;
            let redacted = self.redactor.redact_event(event.payload.clone())?;
            let payload = serde_json::to_string(&redacted.value)
                .map_err(|error| LedgerError::InvalidPayload(error.to_string()))?;
            let payload_hash = persisted_payload_hash(&redacted.value);
            let raw_evidence = event
                .raw_evidence
                .as_ref()
                .map(|value| self.redactor.redact_event(value.clone()))
                .transpose()?
                .map(|redacted| {
                    serde_json::to_string(&redacted.value)
                        .map_err(|error| LedgerError::InvalidPayload(error.to_string()))
                })
                .transpose()?;
            prepared.push(PreparedEvent {
                event,
                sequence,
                payload,
                raw_evidence,
                payload_hash,
            });
        }

        let tx = self.connection.transaction()?;
        for item in prepared {
            let event = item.event;
            let mission_id = event.mission_id.to_string();
            let last: Option<i64> = tx
                .query_row(
                    "SELECT sequence FROM events WHERE mission_id = ?1 ORDER BY sequence DESC LIMIT 1",
                    [&mission_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(existing) = tx
                .query_row(
                    "SELECT mission_id, sequence, payload_hash, raw_evidence, schema_version, route_id, kind, occurred_at FROM events WHERE event_id = ?1",
                    [event.event_id.to_string()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                        ))
                    },
                )
                .optional()?
            {
                if existing.0 == mission_id
                    && existing.1 == item.sequence
                    && existing.2 == item.payload_hash
                    && existing.3 == item.raw_evidence
                    && existing.4 == i64::from(event.schema_version)
                    && existing.5 == event.route_id.to_string()
                    && existing.6 == event.kind.as_str()
                    && existing.7 == event.occurred_at.as_str()
                {
                    continue;
                }
                return Err(LedgerError::DuplicateConflict);
            }
            if last.map_or(item.sequence != 1, |previous| item.sequence != previous + 1) {
                return Err(LedgerError::SequenceViolation);
            }
            tx.execute(
                "INSERT INTO events(mission_id, route_id, sequence, event_id, schema_version, kind, occurred_at, payload, raw_evidence, payload_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    mission_id,
                    event.route_id.to_string(),
                    item.sequence,
                    event.event_id.to_string(),
                    event.schema_version as i64,
                    event.kind.as_str(),
                    event.occurred_at.as_str(),
                    item.payload,
                    item.raw_evidence,
                    item.payload_hash,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn replay(&self, mission_id: &str) -> Result<Vec<(u64, String, String)>, LedgerError> {
        let mut statement = self.connection.prepare(
            "SELECT sequence, kind, payload FROM events WHERE mission_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([mission_id], |row| {
            Ok((row.get::<_, i64>(0)? as u64, row.get(1)?, row.get(2)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn replay_events(&self, mission_id: &MissionId) -> Result<Vec<EventEnvelope>, LedgerError> {
        let mut statement = self.connection.prepare(
            "SELECT route_id, sequence, event_id, schema_version, kind, occurred_at, payload, raw_evidence, payload_hash FROM events WHERE mission_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([mission_id.to_string()], |row| {
            let route_id: String = row.get(0)?;
            let sequence: i64 = row.get(1)?;
            let event_id: String = row.get(2)?;
            let schema_version: i64 = row.get(3)?;
            let kind: String = row.get(4)?;
            let occurred_at: String = row.get(5)?;
            let payload: String = row.get(6)?;
            let raw_evidence: Option<String> = row.get(7)?;
            let payload_hash: String = row.get(8)?;
            let payload = serde_json::from_str(&payload)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let mut event =
                EventEnvelope::new(
                    EventId::from_uuid(uuid::Uuid::parse_str(&event_id).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?),
                    *mission_id,
                    RouteId::from_uuid(uuid::Uuid::parse_str(&route_id).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?),
                    u64::try_from(sequence).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?,
                    serde_json::from_value(serde_json::Value::String(kind)).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?,
                    payload,
                );
            event.schema_version = u16::try_from(schema_version)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            event.occurred_at = Timestamp::parse(occurred_at)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
            event.payload_hash = payload_hash;
            event.raw_evidence = raw_evidence
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            Ok(event)
        })?;
        let events = rows.collect::<Result<Vec<_>, _>>()?;
        for event in &events {
            if !event.has_valid_payload_hash() {
                return Err(LedgerError::IntegrityFailed(format!(
                    "payload hash mismatch for mission {} sequence {}",
                    mission_id, event.sequence
                )));
            }
        }
        Ok(events)
    }

    pub fn mission_ids(&self) -> Result<Vec<MissionId>, LedgerError> {
        let mut statement = self
            .connection
            .prepare("SELECT DISTINCT mission_id FROM events ORDER BY mission_id")?;
        let rows = statement.query_map([], |row| {
            let value: String = row.get(0)?;
            value
                .parse()
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Validate the committed event stream without mutating the database.
    /// A sequence gap is recovery-required evidence, never something to repair silently.
    pub fn integrity_report(&self) -> Result<LedgerIntegrityReport, LedgerError> {
        let quick_check: String = self
            .connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if !quick_check.eq_ignore_ascii_case("ok") {
            return Err(LedgerError::IntegrityFailed(quick_check));
        }
        let mut statement = self.connection.prepare(
            "SELECT mission_id, sequence, payload, payload_hash FROM events ORDER BY mission_id, sequence",
        )?;
        let mut rows = statement.query([])?;
        let mut current_mission = String::new();
        let mut expected = 1_u64;
        let mut count = 0_u64;
        let mut last = 0_u64;
        while let Some(row) = rows.next()? {
            let mission: String = row.get(0)?;
            let sequence: i64 = row.get(1)?;
            let sequence = u64::try_from(sequence)
                .map_err(|_| LedgerError::IntegrityFailed("negative sequence".to_owned()))?;
            let payload: String = row.get(2)?;
            let stored_hash: String = row.get(3)?;
            let payload: serde_json::Value = serde_json::from_str(&payload).map_err(|error| {
                LedgerError::IntegrityFailed(format!(
                    "invalid payload for mission {mission} sequence {sequence}: {error}"
                ))
            })?;
            let actual_hash = persisted_payload_hash(&payload);
            if actual_hash != stored_hash {
                return Err(LedgerError::IntegrityFailed(format!(
                    "payload hash mismatch for mission {mission} sequence {sequence}"
                )));
            }
            if mission != current_mission {
                current_mission = mission;
                expected = 1;
            }
            if sequence != expected {
                return Err(LedgerError::IntegrityFailed(format!(
                    "sequence gap for mission at expected {expected}, found {sequence}"
                )));
            }
            expected = expected.saturating_add(1);
            count = count.saturating_add(1);
            last = last.max(sequence);
        }
        Ok(LedgerIntegrityReport {
            event_count: count,
            last_committed_sequence: last,
            recovery_required: false,
            reason: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn backup(&self) -> Result<PathBuf, LedgerError> {
        backup_database(&self.path)
    }
}

fn apply_key(connection: &Connection, key: &[u8; 32]) -> Result<(), LedgerError> {
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    connection.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))?;
    Ok(())
}

fn ensure_raw_evidence_column(connection: &Connection) -> Result<(), LedgerError> {
    let mut statement = connection.prepare("PRAGMA table_info(events)")?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    let has_column = columns
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "raw_evidence");
    if !has_column {
        connection.execute("ALTER TABLE events ADD COLUMN raw_evidence TEXT", [])?;
    }
    Ok(())
}

fn persisted_payload_hash(payload: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(serde_json::to_vec(payload).expect("JSON values are serializable"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryKeyStore;
    use mission_domain::{EventId, EventKind};
    use serde_json::json;

    #[test]
    fn integrity_and_replay_reject_payload_tampering_with_location() {
        let path = std::env::temp_dir().join(format!(
            "mission-ledger-integrity-hash-{}.db",
            uuid::Uuid::new_v4()
        ));
        let store = InMemoryKeyStore::default();
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let mut ledger = EncryptedLedger::open(&path, "install", store).expect("open ledger");
        let event = EventEnvelope::new(
            EventId::new(),
            mission_id,
            route_id,
            1,
            EventKind::MissionCreated,
            json!({"value":"original"}),
        );
        ledger.append(&event).expect("append");
        ledger
            .connection_mut()
            .execute(
                "UPDATE events SET payload = ?1 WHERE mission_id = ?2 AND sequence = 1",
                rusqlite::params![r#"{"value":"tampered"}"#, mission_id.to_string()],
            )
            .expect("tamper payload");

        let report = ledger.integrity_report();
        let report_error = report.expect_err("tampering must fail integrity report");
        let message = report_error.to_string();
        assert!(
            message.contains(&mission_id.to_string()),
            "missing mission location: {message}"
        );
        assert!(
            message.contains("sequence 1"),
            "missing sequence location: {message}"
        );

        let replay_error = ledger
            .replay_events(&mission_id)
            .expect_err("tampering must fail replay");
        let replay_message = replay_error.to_string();
        assert!(
            replay_message.contains(&mission_id.to_string()),
            "missing mission location: {replay_message}"
        );
        assert!(
            replay_message.contains("sequence 1"),
            "missing sequence location: {replay_message}"
        );
        drop(ledger);
        let _ = std::fs::remove_file(path);
    }
}
