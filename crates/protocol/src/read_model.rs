use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionReadModel {
    pub mission_id: String,
    pub revision: u64,
    pub route_state: String,
    pub evidence_verified: u64,
    pub evidence_required: u64,
    pub incomplete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionEventSummary {
    pub event_id: String,
    pub sequence: u64,
    pub kind: String,
    pub occurred_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilityReadModel {
    pub provider: String,
    pub install_state: String,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub structured_events: bool,
    pub resume: bool,
    pub approval: bool,
    pub safe_pause: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadoutSnapshot {
    pub provider: String,
    pub model: Option<String>,
    pub config_fingerprint: String,
    pub hooks_fingerprint: String,
    pub skills_fingerprint: String,
    pub plugins_fingerprint: String,
    pub mcp_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackSummary {
    pub hash: String,
    pub token_estimate: u32,
    pub included_ids: Vec<String>,
    pub excluded_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPackageSummary {
    pub package_id: String,
    pub blob_ref: String,
    pub manifest_hash: String,
}
