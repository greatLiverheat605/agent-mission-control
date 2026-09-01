use mission_domain::MissionId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::EncryptedLedger;
use crate::delete::LifecycleError;
use crate::redaction::Redactor;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExportPreview {
    pub mission_id: MissionId,
    pub event_count: u64,
    pub size_bytes: u64,
    pub content_hash: String,
    pub categories: Vec<String>,
    pub contains_raw_provider_payload: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExportArtifact {
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub size_bytes: u64,
    pub content_hash: String,
}

fn document(
    ledger: &EncryptedLedger,
    mission_id: &MissionId,
) -> Result<(Vec<u8>, Vec<String>), LifecycleError> {
    let redactor = Redactor::default();
    let mut categories = Vec::new();
    let events = ledger.replay_events(mission_id)?;
    let entries = events
        .into_iter()
        .map(|event| {
            let redacted = redactor
                .redact_event(event.payload)
                .map_err(|error| LifecycleError::Redaction(error.to_string()))?;
            for category in redacted.audit.categories {
                if !categories.iter().any(|item| item == &category) {
                    categories.push(category);
                }
            }
            Ok(serde_json::json!({
                "sequence": event.sequence,
                "kind": event.kind.as_str(),
                "occurred_at": event.occurred_at,
                "payload": redacted.value,
            }))
        })
        .collect::<Result<Vec<Value>, LifecycleError>>()?;
    let value = serde_json::json!({ "schema": "mission-export-v1", "mission_id": mission_id, "events": entries });
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| LifecycleError::Serialization(error.to_string()))?;
    Ok((bytes, categories))
}

impl EncryptedLedger {
    pub fn export_preview(&self, mission_id: &MissionId) -> Result<ExportPreview, LifecycleError> {
        let (bytes, categories) = document(self, mission_id)?;
        Ok(ExportPreview {
            mission_id: *mission_id,
            event_count: self.replay_events(mission_id)?.len() as u64,
            size_bytes: bytes.len() as u64,
            content_hash: digest(&bytes),
            categories,
            contains_raw_provider_payload: false,
        })
    }

    pub fn materialize_export(
        &self,
        mission_id: &MissionId,
    ) -> Result<ExportArtifact, LifecycleError> {
        let (bytes, _) = document(self, mission_id)?;
        Ok(ExportArtifact {
            size_bytes: bytes.len() as u64,
            content_hash: digest(&bytes),
            bytes,
        })
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
