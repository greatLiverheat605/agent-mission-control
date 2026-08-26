use mission_domain::{
    EventConfidence, EventEnvelope, EventId, EventKind, EventSource, MissionId, RouteId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    ConfirmedDecision,
    Constraint,
    Fact,
    Preference,
    Risk,
    Inference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Mission,
    Route,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFreshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Candidate,
    Confirmed,
    Deferred,
    Invalidated,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAuthor {
    User,
    Supervisor,
    Agent,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub kind: MemoryKind,
    pub content: String,
    pub source_event_ids: Vec<EventId>,
    pub scope: MemoryScope,
    pub freshness: MemoryFreshness,
    pub version: u64,
    pub status: MemoryStatus,
    pub author: MemoryAuthor,
}

impl MemoryItem {
    pub fn from_event(event: &EventEnvelope) -> Result<Self, MemoryError> {
        let content = content_from_payload(&event.payload)?;
        let kind = if event.confidence == EventConfidence::Inferred {
            MemoryKind::Inference
        } else {
            match event.kind {
                EventKind::ContractUpdated => MemoryKind::Constraint,
                EventKind::ApprovalResolved => MemoryKind::ConfirmedDecision,
                EventKind::EvidenceRecorded => MemoryKind::Fact,
                EventKind::Unknown(_) if event.source == EventSource::User => {
                    MemoryKind::Preference
                }
                _ => return Err(MemoryError::UnsupportedSource),
            }
        };
        let item = Self {
            id: format!("memory:{}", event.event_id),
            mission_id: event.mission_id,
            route_id: event.route_id,
            kind,
            content,
            source_event_ids: vec![event.event_id],
            scope: if matches!(event.kind, EventKind::ContractUpdated) {
                MemoryScope::Mission
            } else {
                MemoryScope::Route
            },
            freshness: MemoryFreshness::Fresh,
            version: 1,
            status: MemoryStatus::Candidate,
            author: author_for(event.source.clone()),
        };
        item.validate()?;
        Ok(item)
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.id.trim().is_empty() || self.id.len() > 512 || self.content.trim().is_empty() {
            return Err(MemoryError::InvalidItem);
        }
        if self.source_event_ids.is_empty() {
            return Err(MemoryError::SourceRequired);
        }
        if self.version == 0 {
            return Err(MemoryError::InvalidItem);
        }
        Ok(())
    }
}

fn author_for(source: EventSource) -> MemoryAuthor {
    match source {
        EventSource::User => MemoryAuthor::User,
        EventSource::Supervisor => MemoryAuthor::Supervisor,
        EventSource::Agent => MemoryAuthor::Agent,
        EventSource::System => MemoryAuthor::System,
    }
}

fn content_from_payload(payload: &serde_json::Value) -> Result<String, MemoryError> {
    for field in ["summary", "goal", "decision", "preference", "reason"] {
        if let Some(value) = payload.get(field).and_then(serde_json::Value::as_str)
            && !value.trim().is_empty()
        {
            return Ok(value.to_owned());
        }
    }
    serde_json::to_string(payload).map_err(|_| MemoryError::InvalidItem)
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MemoryError {
    #[error("memory item requires at least one source event")]
    SourceRequired,
    #[error("memory item is invalid")]
    InvalidItem,
    #[error("memory item source is not eligible for extraction")]
    UnsupportedSource,
    #[error("inference cannot be confirmed directly")]
    InferenceCannotBeConfirmed,
    #[error("only the user may mutate memory lifecycle state")]
    ForbiddenActor,
    #[error("memory item was not found")]
    NotFound,
    #[error("memory item already exists")]
    DuplicateId,
    #[error("memory item cannot transition from its current state")]
    InvalidTransition,
}
