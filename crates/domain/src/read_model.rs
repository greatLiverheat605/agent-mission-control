use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::event::{EventEnvelope, EventKind};
use crate::evidence::{EvidenceEntry, EvidenceMatrix};
use crate::ids::{EventId, MissionId, RouteId};
use crate::route::RouteState;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SequenceRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadModel {
    pub mission_id: Option<MissionId>,
    pub route_id: Option<RouteId>,
    pub route_state: Option<RouteState>,
    pub route_version: u64,
    pub contract_version: u64,
    pub evidence_matrix: EvidenceMatrix,
    pub incomplete: bool,
    pub missing_sequences: Vec<SequenceRange>,
    pub compatibility_warnings: Vec<String>,
    pub applied_event_ids: BTreeSet<EventId>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProjectionError {
    #[error("event payload hash does not match event {0}")]
    PayloadHashMismatch(EventId),
    #[error("event {0} belongs to a different mission")]
    MissionMismatch(EventId),
    #[error("sequence {sequence} is already occupied by another event")]
    SequenceConflict { sequence: u64 },
    #[error("event {0} contains malformed payload: {1}")]
    MalformedPayload(EventId, String),
}

pub fn reduce(current: &ReadModel, event: &EventEnvelope) -> Result<ReadModel, ProjectionError> {
    if !event.has_valid_payload_hash() {
        return Err(ProjectionError::PayloadHashMismatch(event.event_id));
    }
    if current
        .mission_id
        .is_some_and(|mission_id| mission_id != event.mission_id)
    {
        return Err(ProjectionError::MissionMismatch(event.event_id));
    }
    if current.applied_event_ids.contains(&event.event_id) {
        return Ok(current.clone());
    }
    let mut next = current.clone();
    next.mission_id = Some(event.mission_id);
    next.applied_event_ids.insert(event.event_id);
    match &event.kind {
        EventKind::MissionCreated => {}
        EventKind::ContractUpdated => {
            next.contract_version = number(&event.payload, "version", event)?;
        }
        EventKind::RouteCreated => {
            next.route_id = Some(event.route_id);
            next.route_state = Some(RouteState::Draft);
            next.route_version = 0;
        }
        EventKind::RouteStateChanged => {
            next.route_id = Some(event.route_id);
            let state = string(&event.payload, "state", event)?;
            next.route_state = Some(parse_state(state).ok_or_else(|| {
                ProjectionError::MalformedPayload(event.event_id, "unknown route state".to_owned())
            })?);
            next.route_version = number(&event.payload, "version", event)?;
        }
        EventKind::EvidenceRecorded => {
            let entry: EvidenceEntry =
                serde_json::from_value(event.payload.clone()).map_err(|error| {
                    ProjectionError::MalformedPayload(event.event_id, error.to_string())
                })?;
            next.evidence_matrix.entries.push(entry);
        }
        EventKind::Unknown(kind) => {
            next.compatibility_warnings
                .push(format!("unknown event kind: {kind}"));
        }
        EventKind::AgentMessage => {
            next.compatibility_warnings
                .push("agent message cannot verify evidence".to_owned());
        }
        EventKind::ExplorationStarted | EventKind::AgentRunStarted | EventKind::PauseRequested => {}
    }
    Ok(next)
}

pub fn replay<I>(events: I) -> Result<ReadModel, ProjectionError>
where
    I: IntoIterator<Item = EventEnvelope>,
{
    let mut events: Vec<_> = events.into_iter().collect();
    events.sort_by_key(|event| event.sequence);
    let mut model = ReadModel::default();
    let mut expected_sequence = 1_u64;
    let mut seen_sequences = BTreeSet::new();
    for event in events {
        if !seen_sequences.insert(event.sequence)
            && !model.applied_event_ids.contains(&event.event_id)
        {
            return Err(ProjectionError::SequenceConflict {
                sequence: event.sequence,
            });
        }
        if model.applied_event_ids.contains(&event.event_id) {
            continue;
        }
        if event.sequence > expected_sequence {
            model.incomplete = true;
            model.missing_sequences.push(SequenceRange {
                start: expected_sequence,
                end: event.sequence - 1,
            });
        }
        expected_sequence = event.sequence.saturating_add(1);
        model = reduce(&model, &event)?;
    }
    Ok(model)
}

fn number(payload: &Value, field: &str, event: &EventEnvelope) -> Result<u64, ProjectionError> {
    payload.get(field).and_then(Value::as_u64).ok_or_else(|| {
        ProjectionError::MalformedPayload(event.event_id, format!("missing {field}"))
    })
}

fn string<'a>(
    payload: &'a Value,
    field: &str,
    event: &EventEnvelope,
) -> Result<&'a str, ProjectionError> {
    payload.get(field).and_then(Value::as_str).ok_or_else(|| {
        ProjectionError::MalformedPayload(event.event_id, format!("missing {field}"))
    })
}

fn parse_state(value: &str) -> Option<RouteState> {
    serde_json::from_value(Value::String(value.to_owned())).ok()
}
