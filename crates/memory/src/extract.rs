use std::collections::BTreeSet;

use mission_domain::{EventEnvelope, EventKind, EventSource};

use crate::{MemoryError, MemoryItem};

pub fn extract_candidates(events: &[EventEnvelope]) -> Result<Vec<MemoryItem>, MemoryError> {
    let mut seen = BTreeSet::new();
    events
        .iter()
        .filter(|event| eligible(event))
        .filter(|event| seen.insert(event.event_id))
        .map(MemoryItem::from_event)
        .collect()
}

fn eligible(event: &EventEnvelope) -> bool {
    matches!(
        event.kind,
        EventKind::ContractUpdated | EventKind::ApprovalResolved | EventKind::EvidenceRecorded
    ) || event.source == EventSource::User
}
