use serde::{Deserialize, Serialize};
use thiserror::Error;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionStatus {
    Verified,
    PartiallyVerified,
    Unverified,
    NotApplicable,
}

impl CriterionStatus {
    const fn satisfies_completion(self) -> bool {
        matches!(self, Self::Verified | Self::NotApplicable)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CriterionEvidence {
    pub criterion_id: String,
    pub description: String,
    pub status: CriterionStatus,
    pub evidence_ids: Vec<String>,
}

impl CriterionEvidence {
    pub fn new(criterion_id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            criterion_id: criterion_id.into(),
            description: description.into(),
            status: CriterionStatus::Unverified,
            evidence_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceMatrix {
    #[serde(default)]
    pub criteria: Vec<CriterionEvidence>,
    #[serde(default)]
    pub entries: Vec<EvidenceEntry>,
}

impl EvidenceMatrix {
    pub fn from_criteria(criteria: impl IntoIterator<Item = CriterionEvidence>) -> Self {
        Self {
            criteria: criteria.into_iter().collect(),
            entries: Vec::new(),
        }
    }

    pub fn record<'a>(
        &mut self,
        criterion_id: &str,
        status: CriterionStatus,
        evidence_ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), EvidenceMatrixError> {
        let criterion = self
            .criteria
            .iter_mut()
            .find(|criterion| criterion.criterion_id == criterion_id)
            .ok_or(EvidenceMatrixError::UnknownCriterion)?;
        let evidence_ids: Vec<_> = evidence_ids.into_iter().map(str::to_owned).collect();
        if matches!(
            status,
            CriterionStatus::Verified | CriterionStatus::PartiallyVerified
        ) && evidence_ids.is_empty()
        {
            return Err(EvidenceMatrixError::EvidenceRequired);
        }
        criterion.status = status;
        criterion.evidence_ids = evidence_ids;
        Ok(())
    }

    pub fn can_await_acceptance(&self) -> bool {
        !self.criteria.is_empty()
    }

    pub fn is_complete(&self) -> bool {
        !self.criteria.is_empty()
            && self
                .criteria
                .iter()
                .all(|criterion| criterion.status.satisfies_completion())
    }

    pub fn verified_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.verified).count()
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum EvidenceMatrixError {
    #[error("criterion is not present in the evidence matrix")]
    UnknownCriterion,
    #[error("verified or partially verified criteria require evidence")]
    EvidenceRequired,
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
