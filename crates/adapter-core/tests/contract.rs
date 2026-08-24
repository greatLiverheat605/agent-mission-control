use adapter_core::{AgentAdapter, AgentEvent, FakeAdapter, StartAgentRequest};
use mission_domain::{EventKind, MissionId, RouteId};
use serde_json::json;
use tokio::sync::mpsc;

#[tokio::test]
async fn fake_adapter_emits_ordered_events_and_exposes_capabilities() {
    let adapter = FakeAdapter::default();
    let report = adapter.probe().await.expect("probe");
    assert!(report.capability.structured_events);
    let request = StartAgentRequest {
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        project_root: "C:/repo".to_owned(),
        route_workspace: "C:/route".to_owned(),
        read_only: true,
        approved_environment: Vec::new(),
        model: None,
        loadout_fingerprint: "fixture".to_owned(),
        resume_token: None,
    };
    let (sink, _rx) = mpsc::unbounded_channel::<AgentEvent>();
    let handle = adapter.start(request, sink).await.expect("start");
    let mut seen = 0;
    while let Some(event) = handle.next_event().await {
        seen += 1;
        assert!(!event.event_id.to_string().is_empty());
        if matches!(event.event_kind, EventKind::Unknown(_)) {
            assert!(event.requires_safe_pause);
        }
    }
    assert_eq!(seen, 5);
    assert!(handle.request_safe_pause().await.is_ok());
    assert!(handle.request_safe_pause().await.is_err());
    assert!(handle.terminate_owned_tree().await.is_ok());
}

#[test]
fn event_envelope_keeps_raw_evidence_separate_from_normalized_payload() {
    let event = AgentEvent {
        event_id: mission_domain::EventId::new(),
        event_kind: EventKind::Unknown("native.unknown".to_owned()),
        payload: json!({"safe":"summary"}),
        requires_safe_pause: true,
        raw_evidence: Some(json!({"secret":"[REDACTED:token:abc]"})),
    };
    assert!(event.raw_evidence.is_some());
}
