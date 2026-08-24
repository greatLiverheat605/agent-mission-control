use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId, RouteState, replay};
use serde_json::json;

fn event(sequence: u64, kind: EventKind, payload: serde_json::Value) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(),
        MissionId::new(),
        RouteId::new(),
        sequence,
        kind,
        payload,
    )
}

#[test]
fn replay_sorts_sequences_deduplicates_events_and_reports_gaps() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let created = EventEnvelope::new(
        EventId::new(),
        mission,
        route,
        1,
        EventKind::MissionCreated,
        json!({}),
    );
    let route_created = EventEnvelope::new(
        EventId::new(),
        mission,
        route,
        3,
        EventKind::RouteCreated,
        json!({}),
    );
    let state = EventEnvelope::new(
        EventId::new(),
        mission,
        route,
        4,
        EventKind::RouteStateChanged,
        json!({"state": "AwaitingPlanApproval", "version": 1}),
    );
    let model = replay([state.clone(), route_created, created.clone(), created]).expect("replay");
    assert_eq!(model.route_state, Some(RouteState::AwaitingPlanApproval));
    assert!(model.incomplete);
    assert_eq!(model.missing_sequences[0].start, 2);
    assert_eq!(model.applied_event_ids.len(), 3);
}

#[test]
fn unknown_events_are_preserved_as_warnings_and_agent_messages_cannot_verify() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let mut unknown = event(
        1,
        EventKind::Unknown("future_event".to_owned()),
        json!({"x": 1}),
    );
    unknown.mission_id = mission;
    unknown.route_id = route;
    let model = replay([
        unknown,
        EventEnvelope::new(
            EventId::new(),
            mission,
            route,
            2,
            EventKind::AgentMessage,
            json!({"verified": true}),
        ),
    ])
    .expect("unknown event is forward compatible");
    assert!(
        model
            .compatibility_warnings
            .iter()
            .any(|warning| warning.contains("future_event"))
    );
    assert!(
        model
            .compatibility_warnings
            .iter()
            .any(|warning| warning.contains("cannot verify"))
    );
    assert!(!model.evidence_matrix.is_complete());
}

#[test]
fn canonical_read_model_is_stable_across_replays() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let events = vec![EventEnvelope::new(
        EventId::new(),
        mission,
        route,
        1,
        EventKind::MissionCreated,
        json!({}),
    )];
    let first =
        serde_json::to_vec(&replay(events.clone()).expect("first replay")).expect("serialize");
    let second = serde_json::to_vec(&replay(events).expect("second replay")).expect("serialize");
    assert_eq!(first, second);
}
