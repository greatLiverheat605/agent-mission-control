use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use mission_domain::MissionId;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAGIC: &[u8; 8] = b"MCBLOB01";
const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = 8 + 1 + NONCE_LEN + 8;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlobRef {
    pub hash: String,
    pub size: u64,
    pub media_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlobRecoveryReport {
    pub temporary_files: Vec<String>,
    pub missing_blob_files: Vec<String>,
    pub recovery_required: bool,
}

#[derive(Debug, Error)]
pub enum BlobStoreError {
    #[error("ledger database is not initialized")]
    LedgerNotInitialized,
    #[error("BLOB_INTEGRITY_FAILED")]
    IntegrityFailed,
    #[error("invalid blob reference")]
    InvalidReference,
    #[error("blob encryption failed")]
    Encryption,
    #[error("blob I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("blob metadata I/O failed: {0}")]
    Database(#[from] rusqlite::Error),
}

pub struct EncryptedBlobStore {
    root: PathBuf,
    connection: Connection,
    cipher: Aes256Gcm,
}

impl EncryptedBlobStore {
    pub fn open(root: impl AsRef<Path>, database_key: [u8; 32]) -> Result<Self, BlobStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        let metadata_path = root.join("metadata.db");
        Self::open_metadata(root, metadata_path, database_key, false)
    }

    pub fn open_for_ledger(
        root: impl AsRef<Path>,
        ledger_path: impl AsRef<Path>,
        database_key: [u8; 32],
    ) -> Result<Self, BlobStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Self::open_metadata(root, ledger_path.as_ref().to_path_buf(), database_key, true)
    }

    fn open_metadata(
        root: PathBuf,
        metadata_path: PathBuf,
        database_key: [u8; 32],
        require_ledger: bool,
    ) -> Result<Self, BlobStoreError> {
        let connection = Connection::open(metadata_path)?;
        apply_key(&connection, &database_key)?;
        if require_ledger {
            let initialized: Option<String> = connection
                .query_row(
                    "SELECT value FROM ledger_meta WHERE key = 'schema_sentinel'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if initialized.as_deref() != Some("mission-ledger-sqlcipher-v1") {
                return Err(BlobStoreError::LedgerNotInitialized);
            }
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS blob_refs (
                blob_hash TEXT PRIMARY KEY NOT NULL,
                size INTEGER NOT NULL,
                media_type TEXT NOT NULL,
                ref_count INTEGER NOT NULL DEFAULT 0 CHECK (ref_count >= 0)
            );",
        )?;
        let cipher = derive_cipher(&database_key);
        Ok(Self {
            root,
            connection,
            cipher,
        })
    }

    pub fn put(
        &self,
        plaintext: &[u8],
        media_type: impl Into<String>,
    ) -> Result<BlobRef, BlobStoreError> {
        let hash = digest(plaintext);
        let media_type = media_type.into();
        let reference = BlobRef {
            hash: hash.clone(),
            size: plaintext.len() as u64,
            media_type: media_type.clone(),
        };
        let path = self.path_for_hash(&hash)?;
        if !path.exists() {
            let parent = path.parent().ok_or(BlobStoreError::InvalidReference)?;
            fs::create_dir_all(parent)?;
            let temp = parent.join(format!(".{}.{}.tmp", hash, uuid::Uuid::new_v4()));
            write_encrypted(&temp, plaintext, &hash, &self.cipher)?;
            match fs::rename(&temp, &path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&temp);
                }
                Err(error) => {
                    let _ = fs::remove_file(&temp);
                    return Err(error.into());
                }
            }
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO blob_refs(blob_hash, size, media_type, ref_count) VALUES (?1, ?2, ?3, 0)",
            params![reference.hash, reference.size as i64, reference.media_type],
        )?;
        Ok(reference)
    }

    pub fn read(&self, reference: &BlobRef) -> Result<Vec<u8>, BlobStoreError> {
        validate_hash(&reference.hash)?;
        let path = self.path_for_hash(&reference.hash)?;
        let mut file = File::open(path)?;
        let mut encoded = Vec::new();
        file.read_to_end(&mut encoded)?;
        let plaintext = decrypt_blob(&encoded, &reference.hash, &self.cipher)?;
        if plaintext.len() as u64 != reference.size || digest(&plaintext) != reference.hash {
            return Err(BlobStoreError::IntegrityFailed);
        }
        Ok(plaintext)
    }

    pub fn retain(&self, reference: &BlobRef) -> Result<(), BlobStoreError> {
        validate_hash(&reference.hash)?;
        let changed = self.connection.execute(
            "UPDATE blob_refs SET ref_count = ref_count + 1 WHERE blob_hash = ?1 AND size = ?2 AND media_type = ?3",
            params![reference.hash, reference.size as i64, reference.media_type],
        )?;
        if changed == 0 {
            return Err(BlobStoreError::InvalidReference);
        }
        Ok(())
    }

    /// Attach a blob to a mission exactly once and increment its shared reference count.
    pub fn retain_for_mission(
        &self,
        mission_id: &MissionId,
        reference: &BlobRef,
    ) -> Result<(), BlobStoreError> {
        validate_hash(&reference.hash)?;
        let tx = self.connection.unchecked_transaction()?;
        let known: Option<(i64, String)> = tx
            .query_row(
                "SELECT size, media_type FROM blob_refs WHERE blob_hash = ?1",
                [&reference.hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if known != Some((reference.size as i64, reference.media_type.clone())) {
            return Err(BlobStoreError::InvalidReference);
        }
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO mission_blob_refs(mission_id, blob_hash, size, media_type) VALUES (?1, ?2, ?3, ?4)",
            params![mission_id.to_string(), reference.hash, reference.size as i64, reference.media_type],
        )?;
        if inserted != 0 {
            tx.execute(
                "UPDATE blob_refs SET ref_count = ref_count + 1 WHERE blob_hash = ?1",
                [&reference.hash],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn mission_references(
        &self,
        mission_id: &MissionId,
    ) -> Result<Vec<BlobRef>, BlobStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT blob_hash, size, media_type FROM mission_blob_refs WHERE mission_id = ?1 ORDER BY blob_hash",
        )?;
        let rows = statement.query_map([mission_id.to_string()], |row| {
            Ok(BlobRef {
                hash: row.get(0)?,
                size: u64::try_from(row.get::<_, i64>(1)?)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
                media_type: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Remove a mission attachment and decrement the global count; physical cleanup remains explicit.
    pub fn release_for_mission(
        &self,
        mission_id: &MissionId,
        reference: &BlobRef,
    ) -> Result<bool, BlobStoreError> {
        validate_hash(&reference.hash)?;
        let tx = self.connection.unchecked_transaction()?;
        let removed = tx.execute(
            "DELETE FROM mission_blob_refs WHERE mission_id = ?1 AND blob_hash = ?2 AND size = ?3 AND media_type = ?4",
            params![mission_id.to_string(), reference.hash, reference.size as i64, reference.media_type],
        )?;
        if removed == 0 {
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "UPDATE blob_refs SET ref_count = max(ref_count - 1, 0) WHERE blob_hash = ?1",
            [&reference.hash],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn release(&self, reference: &BlobRef) -> Result<bool, BlobStoreError> {
        validate_hash(&reference.hash)?;
        let tx = self.connection.unchecked_transaction()?;
        let count: Option<i64> = tx
            .query_row(
                "SELECT ref_count FROM blob_refs WHERE blob_hash = ?1 AND size = ?2 AND media_type = ?3",
                params![reference.hash, reference.size as i64, reference.media_type],
                |row| row.get(0),
            )
            .optional()?;
        let Some(count) = count else {
            return Err(BlobStoreError::InvalidReference);
        };
        if count == 0 {
            return Ok(false);
        }
        tx.execute(
            "UPDATE blob_refs SET ref_count = ref_count - 1 WHERE blob_hash = ?1",
            [&reference.hash],
        )?;
        tx.commit()?;
        Ok(count == 1)
    }

    pub fn delete_if_unreferenced(&self, reference: &BlobRef) -> Result<bool, BlobStoreError> {
        validate_hash(&reference.hash)?;
        let tx = self.connection.unchecked_transaction()?;
        let count: Option<i64> = tx
            .query_row(
                "SELECT ref_count FROM blob_refs WHERE blob_hash = ?1 AND size = ?2 AND media_type = ?3",
                params![reference.hash, reference.size as i64, reference.media_type],
                |row| row.get(0),
            )
            .optional()?;
        if count != Some(0) {
            tx.commit()?;
            return Ok(false);
        }
        let path = self.path_for_hash(&reference.hash)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        tx.execute(
            "DELETE FROM blob_refs WHERE blob_hash = ?1 AND size = ?2 AND media_type = ?3",
            params![reference.hash, reference.size as i64, reference.media_type],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn path_for(&self, reference: &BlobRef) -> Result<PathBuf, BlobStoreError> {
        validate_hash(&reference.hash)?;
        self.path_for_hash(&reference.hash)
    }

    /// Inspect incomplete writes and metadata references without deleting anything.
    pub fn recovery_report(&self) -> Result<BlobRecoveryReport, BlobStoreError> {
        let mut temporary_files = Vec::new();
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "tmp") {
                    temporary_files.push(path.display().to_string());
                }
            }
        }
        let mut statement = self.connection.prepare("SELECT blob_hash FROM blob_refs")?;
        let missing_blob_files = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .filter_map(|hash| {
                self.path_for_hash(&hash)
                    .ok()
                    .filter(|path| !path.exists())
                    .map(|_| hash)
            })
            .collect::<Vec<_>>();
        Ok(BlobRecoveryReport {
            recovery_required: !temporary_files.is_empty() || !missing_blob_files.is_empty(),
            temporary_files,
            missing_blob_files,
        })
    }

    fn path_for_hash(&self, hash: &str) -> Result<PathBuf, BlobStoreError> {
        validate_hash(hash)?;
        Ok(self
            .root
            .join(&hash[..2])
            .join(&hash[2..4])
            .join(format!("{}.blob", hash)))
    }
}

fn apply_key(connection: &Connection, key: &[u8; 32]) -> Result<(), BlobStoreError> {
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    connection.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))?;
    Ok(())
}

fn derive_cipher(database_key: &[u8; 32]) -> Aes256Gcm {
    let hk = Hkdf::<Sha256>::new(Some(b"mission-ledger-blob-v1"), database_key);
    let mut key = [0_u8; 32];
    hk.expand(b"content-addressed-blob", &mut key)
        .expect("32-byte HKDF output is valid");
    Aes256Gcm::new_from_slice(&key).expect("AES-256 key length is fixed")
}

fn write_encrypted(
    path: &Path,
    plaintext: &[u8],
    hash: &str,
    cipher: &Aes256Gcm,
) -> Result<(), BlobStoreError> {
    let mut nonce_bytes = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes).map_err(|_| BlobStoreError::Encryption)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: plaintext,
                aad: hash.as_bytes(),
            },
        )
        .map_err(|_| BlobStoreError::Encryption)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(MAGIC)?;
    file.write_all(&[VERSION])?;
    file.write_all(&nonce_bytes)?;
    file.write_all(&(ciphertext.len() as u64).to_le_bytes())?;
    file.write_all(&ciphertext)?;
    file.sync_all()?;
    Ok(())
}

fn decrypt_blob(encoded: &[u8], hash: &str, cipher: &Aes256Gcm) -> Result<Vec<u8>, BlobStoreError> {
    if encoded.len() < HEADER_LEN || &encoded[..8] != MAGIC || encoded[8] != VERSION {
        return Err(BlobStoreError::IntegrityFailed);
    }
    let nonce = &encoded[9..9 + NONCE_LEN];
    let length = u64::from_le_bytes(encoded[21..29].try_into().expect("header length checked"));
    let length = usize::try_from(length).map_err(|_| BlobStoreError::IntegrityFailed)?;
    if encoded.len() != HEADER_LEN + length {
        return Err(BlobStoreError::IntegrityFailed);
    }
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &encoded[HEADER_LEN..],
                aad: hash.as_bytes(),
            },
        )
        .map_err(|_| BlobStoreError::IntegrityFailed)
}

fn validate_hash(hash: &str) -> Result<(), BlobStoreError> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BlobStoreError::InvalidReference);
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
