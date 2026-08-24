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
