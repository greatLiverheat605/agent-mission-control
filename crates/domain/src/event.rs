use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

use crate::ids::{EventId, MissionId, RouteId, Timestamp};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    Supervisor,
    Agent,
    User,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventConfidence {
    Observed,
    Inferred,
    Confirmed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventKind {
    MissionCreated,
    ContractUpdated,
    RouteCreated,
    RouteStateChanged,
    ExplorationStarted,
    AgentRunStarted,
    AgentMessage,
    EvidenceRecorded,
    PauseRequested,
    Unknown(String),
}

impl EventKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::MissionCreated => "mission_created",
            Self::ContractUpdated => "contract_updated",
            Self::RouteCreated => "route_created",
            Self::RouteStateChanged => "route_state_changed",
            Self::ExplorationStarted => "exploration_started",
            Self::AgentRunStarted => "agent_run_started",
            Self::AgentMessage => "agent_message",
            Self::EvidenceRecorded => "evidence_recorded",
            Self::PauseRequested => "pause_requested",
            Self::Unknown(value) => value,
        }
    }
}

impl Serialize for EventKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EventKindVisitor;

        impl<'de> Visitor<'de> for EventKindVisitor {
            type Value = EventKind;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an event kind string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(match value {
                    "mission_created" => EventKind::MissionCreated,
                    "contract_updated" => EventKind::ContractUpdated,
                    "route_created" => EventKind::RouteCreated,
                    "route_state_changed" => EventKind::RouteStateChanged,
                    "exploration_started" => EventKind::ExplorationStarted,
                    "agent_run_started" => EventKind::AgentRunStarted,
                    "agent_message" => EventKind::AgentMessage,
                    "evidence_recorded" => EventKind::EvidenceRecorded,
                    "pause_requested" => EventKind::PauseRequested,
                    other => EventKind::Unknown(other.to_owned()),
                })
            }
        }

        deserializer.deserialize_str(EventKindVisitor)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventLinks {
    pub parent_event_id: Option<EventId>,
    pub source_event_ids: Vec<EventId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub schema_version: u16,
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub agent_run_id: Option<String>,
    pub sequence: u64,
    pub occurred_at: Timestamp,
    pub source: EventSource,
    pub confidence: EventConfidence,
    pub kind: EventKind,
    pub payload: Value,
    pub payload_hash: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub links: EventLinks,
}

impl EventEnvelope {
    pub fn new(
        event_id: EventId,
        mission_id: MissionId,
        route_id: RouteId,
        sequence: u64,
        kind: EventKind,
        payload: Value,
    ) -> Self {
        let payload_hash = payload_hash(&payload);
        Self {
            event_id,
            schema_version: 1,
            mission_id,
            route_id,
            agent_run_id: None,
            sequence,
            occurred_at: Timestamp::now(),
            source: EventSource::Supervisor,
            confidence: EventConfidence::Observed,
            kind,
            payload,
            payload_hash,
            correlation_id: None,
            causation_id: None,
            links: EventLinks::default(),
        }
    }

    pub fn has_valid_payload_hash(&self) -> bool {
        self.payload_hash == payload_hash(&self.payload)
    }
}

pub fn payload_hash(payload: &Value) -> String {
    let bytes = serde_json::to_vec(payload).expect("JSON values are serializable");
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
