use std::fs;
use std::path::{Component, Path, PathBuf};

use mission_ledger::{BlobRef, BlobStoreError, EncryptedBlobStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnapshotOptions {
    pub forbidden_scopes: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub relative_path: String,
    pub blob: BlobRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub snapshot_id: String,
    pub entries: Vec<SnapshotEntry>,
    pub root_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonGitSnapshot {
    pub manifest: SnapshotManifest,
    pub manifest_blob: BlobRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedRestore {
    pub path: PathBuf,
    pub manifest_hash: String,
}

pub struct NonGitSnapshotter<'a> {
    store: &'a EncryptedBlobStore,
}

impl<'a> NonGitSnapshotter<'a> {
    pub const fn new(store: &'a EncryptedBlobStore) -> Self {
        Self { store }
    }

    pub fn create(
        &self,
        root: impl AsRef<Path>,
        snapshot_id: impl Into<String>,
        options: SnapshotOptions,
    ) -> Result<NonGitSnapshot, SnapshotError> {
        let snapshot_id = snapshot_id.into();
        validate_id(&snapshot_id)?;
        let root = root.as_ref().canonicalize()?;
        let forbidden = validate_forbidden(&options.forbidden_scopes)?;
        let mut entries = Vec::new();
        if let Err(error) = collect_files(&root, &root, &forbidden, self.store, &mut entries) {
            release_entries(self.store, &entries);
            return Err(error);
        }
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let root_hash = match entries_hash(&entries) {
            Ok(hash) => hash,
            Err(error) => {
                release_entries(self.store, &entries);
                return Err(error);
            }
        };
        let manifest = SnapshotManifest {
            snapshot_id,
            entries,
            root_hash,
        };
        let encoded = match serde_json::to_vec(&manifest) {
            Ok(encoded) => encoded,
            Err(error) => {
                release_entries(self.store, &manifest.entries);
                return Err(error.into());
            }
        };
        match self
            .store
            .put(&encoded, "application/vnd.mission.snapshot+json")
        {
            Ok(manifest_blob) => {
                if let Err(error) = self.store.retain(&manifest_blob) {
                    release_entries(self.store, &manifest.entries);
                    return Err(error.into());
                }
                Ok(NonGitSnapshot {
                    manifest,
                    manifest_blob,
                })
            }
            Err(error) => {
                release_entries(self.store, &manifest.entries);
                Err(error.into())
            }
        }
    }

    pub fn prepare_restore(
        &self,
        snapshot: &NonGitSnapshot,
        parent: impl AsRef<Path>,
    ) -> Result<PreparedRestore, SnapshotError> {
        if entries_hash(&snapshot.manifest.entries)? != snapshot.manifest.root_hash {
            return Err(SnapshotError::ManifestIntegrity);
        }
        fs::create_dir_all(parent.as_ref())?;
        let parent = parent.as_ref().canonicalize()?;
        let target = parent.join(format!(".restore-{}", snapshot.manifest.snapshot_id));
        if target.exists() {
            return Err(SnapshotError::RestoreTargetExists(target));
        }
        fs::create_dir(&target)?;
        let restore = (|| {
            for entry in &snapshot.manifest.entries {
                let relative = safe_relative(&entry.relative_path)?;
                let destination = target.join(relative);
                if !destination.starts_with(&target) {
                    return Err(SnapshotError::OutsideRoot);
                }
                if let Some(directory) = destination.parent() {
                    fs::create_dir_all(directory)?;
                }
                fs::write(destination, self.store.read(&entry.blob)?)?;
            }
            Ok(PreparedRestore {
                path: target.clone(),
                manifest_hash: snapshot.manifest.root_hash.clone(),
            })
        })();
        if restore.is_err() {
            let _ = fs::remove_dir_all(&target);
        }
        restore
    }
}

fn collect_files(
    root: &Path,
    current: &Path,
    forbidden: &[PathBuf],
    store: &EncryptedBlobStore,
    entries: &mut Vec<SnapshotEntry>,
) -> Result<(), SnapshotError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| SnapshotError::OutsideRoot)?;
        if ignored(relative) || forbidden.iter().any(|scope| relative.starts_with(scope)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_files(root, &path, forbidden, store, entries)?;
        } else if metadata.is_file() {
            let blob = store.put(&fs::read(&path)?, "application/octet-stream")?;
            store.retain(&blob)?;
            entries.push(SnapshotEntry {
                relative_path: normalize_relative(relative),
                blob,
            });
        }
    }
    Ok(())
}

fn validate_forbidden(scopes: &[PathBuf]) -> Result<Vec<PathBuf>, SnapshotError> {
    scopes
        .iter()
        .map(|scope| {
            if scope.as_os_str().is_empty()
                || scope.is_absolute()
                || scope
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                Err(SnapshotError::InvalidScope)
            } else {
                Ok(scope.clone())
            }
        })
        .collect()
}

fn ignored(relative: &Path) -> bool {
    relative.components().any(|component| {
        matches!(component, Component::Normal(name) if matches!(name.to_str(), Some(".git" | ".codex" | "node_modules" | "target")))
    })
}

fn safe_relative(value: &str) -> Result<PathBuf, SnapshotError> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SnapshotError::OutsideRoot);
    }
    Ok(path)
}

fn normalize_relative(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn entries_hash(entries: &[SnapshotEntry]) -> Result<String, SnapshotError> {
    Ok(hex(&Sha256::digest(serde_json::to_vec(entries)?)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn release_entries(store: &EncryptedBlobStore, entries: &[SnapshotEntry]) {
    for entry in entries.iter().rev() {
        let _ = store.release(&entry.blob);
    }
}

fn validate_id(value: &str) -> Result<(), SnapshotError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SnapshotError::InvalidSnapshotId);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error(transparent)]
    Blob(#[from] BlobStoreError),
    #[error("snapshot I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("snapshot id is invalid")]
    InvalidSnapshotId,
    #[error("snapshot forbidden scope is invalid")]
    InvalidScope,
    #[error("snapshot path escaped root")]
    OutsideRoot,
    #[error("snapshot manifest integrity failed")]
    ManifestIntegrity,
    #[error("restore target already exists: {0}")]
    RestoreTargetExists(PathBuf),
}
