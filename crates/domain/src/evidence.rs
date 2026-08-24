use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Test,
    Build,
    Screenshot,
    Diff,
    UserAcceptance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceEntry {
    pub id: String,
    pub kind: EvidenceKind,
    pub summary: String,
    pub verified: bool,
    pub source_event_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceMatrix {
    pub entries: Vec<EvidenceEntry>,
}

impl EvidenceMatrix {
    pub fn is_complete(&self) -> bool {
        !self.entries.is_empty() && self.entries.iter().all(|entry| entry.verified)
    }

    pub fn verified_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.verified).count()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub actor: String,
    pub decision: String,
    pub evidence_event_ids: Vec<String>,
}

impl Approval {
    pub fn is_acceptance(&self) -> bool {
        self.decision == "accept"
            && !self.actor.trim().is_empty()
            && !self.evidence_event_ids.is_empty()
    }
}
