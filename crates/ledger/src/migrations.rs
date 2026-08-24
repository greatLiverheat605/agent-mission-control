use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::LedgerError;

pub const SCHEMA: &str = include_str!("../migrations/0001_event_ledger.sql");

pub fn backup_path(path: &Path) -> PathBuf {
    let bytes = fs::read(path).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    let suffix: String = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    path.with_extension(format!("backup-{suffix}"))
}

pub fn backup_database(path: &Path) -> Result<PathBuf, LedgerError> {
    let backup = backup_path(path);
    fs::copy(path, &backup).map_err(LedgerError::Io)?;
    Ok(backup)
}

pub fn restore_database(backup: &Path, path: &Path) -> Result<(), LedgerError> {
    fs::copy(backup, path).map_err(LedgerError::Io)?;
    Ok(())
}
