use adapter_core::{
    AgentAdapter, AgentCapabilityReport, AgentEvent, FakeAdapter, LoadoutSnapshot, ProviderId,
    StartAgentRequest,
};
use mission_domain::{EventKind, MissionId, RouteId};
use serde_json::json;
use tokio::sync::mpsc;

#[tokio::test]
async fn fake_adapter_emits_ordered_events_and_exposes_capabilities() {
    let adapter = FakeAdapter::default();
    let report = adapter.probe().await.expect("probe");
    assert!(report.capability.structured_events);
    let request = StartAgentRequest {
        provider: ProviderId::Codex,
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        project_root: "C:/repo".to_owned(),
        route_workspace: "C:/route".to_owned(),
        read_only: true,
        approved_environment: Vec::new(),
        model: None,
        loadout_fingerprint: "fixture".to_owned(),
        resume_token: None,
        loadout: None,
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
fn provider_and_loadout_contracts_are_stable_and_self_describing() {
    let loadout = LoadoutSnapshot {
        provider: ProviderId::Claude,
        model: Some("claude-sonnet".to_owned()),
        config_fingerprint: "config".to_owned(),
        hooks_fingerprint: "hooks".to_owned(),
        skills_fingerprint: "skills".to_owned(),
        plugins_fingerprint: "plugins".to_owned(),
        mcp_fingerprint: "mcp".to_owned(),
    };
    let encoded = serde_json::to_value(&loadout).expect("loadout serializes");
    assert_eq!(encoded["provider"], "claude");
    assert_eq!(encoded["model"], "claude-sonnet");
    assert_eq!(loadout.fingerprint_material().len(), 7);

    let report = AgentCapabilityReport {
        provider: ProviderId::Claude,
        agent: "claude".to_owned(),
        version: Some("2.1.220".to_owned()),
        install_state: adapter_core::InstallState::Installed,
        capability: adapter_core::Capability {
            structured_events: true,
            resume: false,
            approval: true,
            safe_pause: true,
            terminal_fallback: true,
        },
        unavailable_reason: None,
        executable_hash: None,
        configuration_source: Some("local_cli".to_owned()),
    };
    assert!(report.is_available());
}

#[test]
fn event_envelope_keeps_raw_evidence_separate_from_normalized_payload() {
    let event = AgentEvent {
        event_id: mission_domain::EventId::new(),
        agent_run_id: None,
        event_kind: EventKind::Unknown("native.unknown".to_owned()),
        payload: json!({"safe":"summary"}),
        requires_safe_pause: true,
        raw_evidence: Some(json!({"secret":"[REDACTED:token:abc]"})),
    };
    assert!(event.raw_evidence.is_some());
}
