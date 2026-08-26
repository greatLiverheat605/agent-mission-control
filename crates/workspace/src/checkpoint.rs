use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{GitError, GitRunner};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointTrigger {
    BeforeLaunch,
    PlanPhaseEnded,
    BeforeCompaction,
    BeforeProviderHandoff,
    Paused,
    Blocked,
    Failed,
    AutopilotDisengaged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointRequest {
    pub checkpoint_id: String,
    pub trigger: CheckpointTrigger,
    pub contract_version: u64,
    pub loadout_fingerprint: String,
    pub ledger_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointMetadata {
    pub checkpoint_id: String,
    pub trigger: CheckpointTrigger,
    pub file_hashes: BTreeMap<String, String>,
    pub contract_version: u64,
    pub loadout_fingerprint: String,
    pub ledger_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCheckpoint {
    pub commit: String,
    pub metadata: CheckpointMetadata,
    pub committed: bool,
}

impl GitCheckpoint {
    pub fn create(runner: &GitRunner, request: CheckpointRequest) -> Result<Self, CheckpointError> {
        validate_request(&request)?;
        let file_hashes = hash_files(runner.cwd())?;
        if file_hashes.is_empty() {
            return Err(CheckpointError::IncompleteMetadata);
        }
        let metadata = CheckpointMetadata {
            checkpoint_id: request.checkpoint_id,
            trigger: request.trigger,
            file_hashes,
            contract_version: request.contract_version,
            loadout_fingerprint: request.loadout_fingerprint,
            ledger_sequence: request.ledger_sequence,
        };
        runner.run_text(&["add", "-A"])?;
        let message = format!("Mission Control checkpoint {}", metadata.checkpoint_id);
        runner.run(&[
            OsString::from("-c"),
            OsString::from("user.name=Agent Mission Control"),
            OsString::from("-c"),
            OsString::from("user.email=checkpoint@mission-control.invalid"),
            OsString::from("commit"),
            OsString::from("--allow-empty"),
            OsString::from("-m"),
            OsString::from(message),
        ])?;
        let commit = runner
            .run_text(&["rev-parse", "HEAD"])?
            .stdout
            .trim()
            .to_owned();
        Ok(Self {
            commit,
            metadata,
            committed: true,
        })
    }
}

fn validate_request(request: &CheckpointRequest) -> Result<(), CheckpointError> {
    if request.checkpoint_id.is_empty()
        || request.checkpoint_id.len() > 64
        || !request
            .checkpoint_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || request.loadout_fingerprint.trim().is_empty()
    {
        return Err(CheckpointError::IncompleteMetadata);
    }
    Ok(())
}

fn hash_files(root: &Path) -> Result<BTreeMap<String, String>, CheckpointError> {
    let mut hashes = BTreeMap::new();
    visit_files(root, root, &mut hashes)?;
    Ok(hashes)
}

fn visit_files(
    root: &Path,
    current: &Path,
    hashes: &mut BTreeMap<String, String>,
) -> Result<(), CheckpointError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            visit_files(root, &path, hashes)?;
        } else if metadata.is_file() {
            let relative = normalize_relative(
                path.strip_prefix(root)
                    .map_err(|_| CheckpointError::OutsideRoot)?,
            );
            let digest = Sha256::digest(fs::read(path)?);
            hashes.insert(relative, hex(&digest));
        }
    }
    Ok(())
}

fn normalize_relative(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("checkpoint I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("checkpoint metadata is incomplete")]
    IncompleteMetadata,
    #[error("checkpoint file escaped route root")]
    OutsideRoot,
}
