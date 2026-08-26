use std::collections::BTreeSet;

use mission_domain::{MissionId, RouteId};
use mission_ledger::{BlobRef, BlobStoreError, EncryptedBlobStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const RECOVERY_SCHEMA_VERSION: u16 = 1;
const RECOVERY_MEDIA_TYPE: &str = "application/vnd.mission-control.recovery+json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryInput {
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub contract_version: u64,
    pub checkpoint_id: String,
    pub ledger_sequence: u64,
    pub loadout_fingerprint: String,
    pub context_pack_hash: String,
    pub pending_approval_hash: Option<String>,
    pub permissions: BTreeSet<String>,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryConstraints {
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub contract_version: u64,
    pub ledger_sequence: u64,
    pub loadout_fingerprint: String,
    pub context_pack_hash: String,
    pub pending_approval_hash: Option<String>,
    pub permissions: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryManifest {
    pub schema_version: u16,
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub contract_version: u64,
    pub checkpoint_id: String,
    pub ledger_sequence: u64,
    pub loadout_fingerprint: String,
    pub context_pack_hash: String,
    pub pending_approval_hash: Option<String>,
    pub permissions: BTreeSet<String>,
    pub entry_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryPackage {
    pub manifest: RecoveryManifest,
    pub blob: BlobRef,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RecoveryError {
    #[error("recovery input is invalid")]
    InvalidInput,
    #[error("recovery blob integrity check failed")]
    BlobIntegrity,
    #[error("recovery manifest hash does not match")]
    ManifestTampered,
    #[error("recovery package ledger sequence is from the future")]
    SequenceInvalid,
    #[error("recovery package would expand permissions")]
    PermissionExpansion,
    #[error("recovery package authority binding does not match")]
    BindingMismatch,
    #[error("pending approval binding does not match")]
    PendingApprovalMismatch,
    #[error("recovery blob store failed: {0}")]
    BlobStore(String),
}

impl From<BlobStoreError> for RecoveryError {
    fn from(error: BlobStoreError) -> Self {
        match error {
            BlobStoreError::IntegrityFailed => Self::BlobIntegrity,
            other => Self::BlobStore(other.to_string()),
        }
    }
}

pub fn build_recovery_package(
    store: &EncryptedBlobStore,
    input: RecoveryInput,
) -> Result<RecoveryPackage, RecoveryError> {
    validate_input(&input)?;
    let blob = store
        .put(&input.payload, RECOVERY_MEDIA_TYPE)
        .map_err(RecoveryError::from)?;
    let mut manifest = RecoveryManifest {
        schema_version: RECOVERY_SCHEMA_VERSION,
        mission_id: input.mission_id,
        route_id: input.route_id,
        contract_version: input.contract_version,
        checkpoint_id: input.checkpoint_id,
        ledger_sequence: input.ledger_sequence,
        loadout_fingerprint: input.loadout_fingerprint,
        context_pack_hash: input.context_pack_hash,
        pending_approval_hash: input.pending_approval_hash,
        permissions: input.permissions,
        entry_hash: String::new(),
    };
    manifest.entry_hash = digest_manifest(&manifest, &blob)?;
    Ok(RecoveryPackage { manifest, blob })
}

impl RecoveryPackage {
    pub fn verify(
        &self,
        store: &EncryptedBlobStore,
        constraints: &RecoveryConstraints,
    ) -> Result<Vec<u8>, RecoveryError> {
        if self.manifest.schema_version != RECOVERY_SCHEMA_VERSION
            || self.blob.media_type != RECOVERY_MEDIA_TYPE
            || self.manifest.entry_hash != digest_manifest(&self.manifest, &self.blob)?
        {
            return Err(RecoveryError::ManifestTampered);
        }
        if self.manifest.ledger_sequence > constraints.ledger_sequence {
            return Err(RecoveryError::SequenceInvalid);
        }
        if self.manifest.mission_id != constraints.mission_id
            || self.manifest.route_id != constraints.route_id
            || self.manifest.contract_version != constraints.contract_version
            || self.manifest.loadout_fingerprint != constraints.loadout_fingerprint
            || self.manifest.context_pack_hash != constraints.context_pack_hash
        {
            return Err(RecoveryError::BindingMismatch);
        }
        if self.manifest.pending_approval_hash != constraints.pending_approval_hash {
            return Err(RecoveryError::PendingApprovalMismatch);
        }
        if !self
            .manifest
            .permissions
            .is_subset(&constraints.permissions)
        {
            return Err(RecoveryError::PermissionExpansion);
        }
        store.read(&self.blob).map_err(RecoveryError::from)
    }
}

fn validate_input(input: &RecoveryInput) -> Result<(), RecoveryError> {
    if input.checkpoint_id.trim().is_empty()
        || input.checkpoint_id.len() > 512
        || input.loadout_fingerprint.trim().is_empty()
        || input.context_pack_hash.trim().is_empty()
        || input.payload.is_empty()
    {
        return Err(RecoveryError::InvalidInput);
    }
    if input
        .permissions
        .iter()
        .any(|permission| permission.trim().is_empty() || permission.len() > 128)
    {
        return Err(RecoveryError::InvalidInput);
    }
    Ok(())
}

fn digest_manifest(manifest: &RecoveryManifest, blob: &BlobRef) -> Result<String, RecoveryError> {
    let mut unsigned = manifest.clone();
    unsigned.entry_hash.clear();
    let bytes = serde_json::to_vec(&(unsigned, blob)).map_err(|_| RecoveryError::InvalidInput)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
