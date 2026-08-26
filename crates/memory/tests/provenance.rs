use mission_domain::{
    EventConfidence, EventEnvelope, EventId, EventKind, EventSource, MissionId, RouteId,
};
use mission_memory::{
    MemoryAuthor, MemoryError, MemoryFreshness, MemoryKind, MemoryScope, MemoryStatus, MemoryStore,
    extract_candidates,
};
use serde_json::json;

fn event(
    kind: EventKind,
    payload: serde_json::Value,
    source: EventSource,
    confidence: EventConfidence,
) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        EventId::new(),
        MissionId::new(),
        RouteId::new(),
        1,
        kind,
        payload,
    );
    event.source = source;
    event.confidence = confidence;
    event
}

#[test]
fn candidates_keep_provenance_scope_freshness_version_status_and_author() {
    let contract = event(
        EventKind::ContractUpdated,
        json!({"goal": "ship recovery", "version": 2}),
        EventSource::Supervisor,
        EventConfidence::Observed,
    );
    let approval = event(
        EventKind::ApprovalResolved,
        json!({"decision": "approve", "action": "write"}),
        EventSource::User,
        EventConfidence::Confirmed,
    );
    let evidence = event(
        EventKind::EvidenceRecorded,
        json!({"summary": "test passed"}),
        EventSource::Supervisor,
        EventConfidence::Confirmed,
    );
    let preference = event(
        EventKind::Unknown("user_preference".to_owned()),
        json!({"preference": "concise output"}),
        EventSource::User,
        EventConfidence::Confirmed,
    );
    let ignored = event(
        EventKind::AgentMessage,
        json!({"summary": "model guess"}),
        EventSource::Agent,
        EventConfidence::Inferred,
    );

    let candidates =
        extract_candidates(&[contract.clone(), approval, evidence, preference, ignored])
            .expect("candidate extraction");
    assert_eq!(candidates.len(), 4);

    let contract_item = candidates
        .iter()
        .find(|item| item.kind == MemoryKind::Constraint)
        .expect("constraint");
    assert_eq!(contract_item.source_event_ids, vec![contract.event_id]);
    assert_eq!(contract_item.scope, MemoryScope::Mission);
    assert_eq!(contract_item.freshness, MemoryFreshness::Fresh);
    assert_eq!(contract_item.version, 1);
    assert_eq!(contract_item.status, MemoryStatus::Candidate);
    assert_eq!(contract_item.author, MemoryAuthor::Supervisor);
    assert_eq!(contract_item.mission_id, contract.mission_id);
    assert_eq!(contract_item.route_id, contract.route_id);
}

#[test]
fn inferred_candidates_cannot_be_confirmed_and_source_is_required() {
    let inferred = event(
        EventKind::EvidenceRecorded,
        json!({"summary": "unverified clue"}),
        EventSource::Supervisor,
        EventConfidence::Inferred,
    );
    let candidate = extract_candidates(&[inferred])
        .expect("candidate extraction")
        .pop()
        .expect("inference");
    assert_eq!(candidate.kind, MemoryKind::Inference);

    let mut store = MemoryStore::default();
    store.insert(candidate.clone()).expect("insert");
    assert_eq!(
        store.confirm(&candidate.id, MemoryAuthor::User),
        Err(MemoryError::InferenceCannotBeConfirmed)
    );

    let mut source_less = candidate;
    source_less.source_event_ids.clear();
    assert_eq!(store.insert(source_less), Err(MemoryError::SourceRequired));
}

#[test]
fn store_rejects_preconfirmed_items() {
    let event = event(
        EventKind::ContractUpdated,
        json!({"goal": "ship recovery", "version": 2}),
        EventSource::Supervisor,
        EventConfidence::Observed,
    );
    let mut item = extract_candidates(&[event])
        .expect("candidate extraction")
        .pop()
        .expect("candidate");
    item.status = MemoryStatus::Confirmed;
    assert_eq!(
        MemoryStore::default().insert(item),
        Err(MemoryError::InvalidTransition)
    );
}
