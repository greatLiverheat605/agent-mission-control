use std::fs;
use std::path::{Path, PathBuf};

use mission_domain::{EventEnvelope, EventId, MissionId, RouteId, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};
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
        Ok(Self {
            path,
            connection,
            redactor: Redactor::default(),
        })
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
                "SELECT mission_id, sequence, payload_hash, schema_version, route_id, kind, occurred_at FROM events WHERE event_id = ?1",
                [event.event_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?
        {
            return if existing.0 == event.mission_id.to_string()
                && existing.1 == sequence
                && existing.2 == persisted_payload_hash
                && existing.3 == i64::from(event.schema_version)
                && existing.4 == event.route_id.to_string()
                && existing.5 == event.kind.as_str()
                && existing.6 == event.occurred_at.as_str()
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
            "INSERT INTO events(mission_id, route_id, sequence, event_id, schema_version, kind, occurred_at, payload, payload_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![event.mission_id.to_string(), event.route_id.to_string(), sequence, event.event_id.to_string(), event.schema_version as i64, event.kind.as_str(), event.occurred_at.as_str(), payload, persisted_payload_hash],
        )?;
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
            "SELECT route_id, sequence, event_id, schema_version, kind, occurred_at, payload, payload_hash FROM events WHERE mission_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([mission_id.to_string()], |row| {
            let route_id: String = row.get(0)?;
            let sequence: i64 = row.get(1)?;
            let event_id: String = row.get(2)?;
            let schema_version: i64 = row.get(3)?;
            let kind: String = row.get(4)?;
            let occurred_at: String = row.get(5)?;
            let payload: String = row.get(6)?;
            let payload_hash: String = row.get(7)?;
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
            Ok(event)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

fn persisted_payload_hash(payload: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(serde_json::to_vec(payload).expect("JSON values are serializable"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
