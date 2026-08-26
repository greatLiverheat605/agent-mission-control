use mission_protocol::command::{
    ContinuityErrorCode, IpcCommand, ProviderHandoffRequest, ReviewMemoryRequest,
};

#[test]
fn continuity_commands_and_errors_are_explicit() {
    assert!(IpcCommand::allowlisted().contains(&IpcCommand::ReviewMemory));
    assert!(IpcCommand::allowlisted().contains(&IpcCommand::BuildContextPack));
    assert!(IpcCommand::allowlisted().contains(&IpcCommand::BuildRecoveryPackage));
    assert!(IpcCommand::allowlisted().contains(&IpcCommand::HandoffProvider));
    assert_eq!(
        serde_json::to_string(&IpcCommand::HandoffProvider).unwrap(),
        "\"HandoffProvider\""
    );
    assert_eq!(
        ContinuityErrorCode::ProviderUnavailable.as_str(),
        "PROVIDER_UNAVAILABLE"
    );
}

#[test]
fn continuity_requests_require_explicit_hashes_and_scopes() {
    let memory = ReviewMemoryRequest {
        mission_id: "mission-1".to_owned(),
        candidate_ids: vec!["memory-1".to_owned()],
        expected_revision: 4,
    };
    let handoff = ProviderHandoffRequest {
        mission_id: "mission-1".to_owned(),
        route_id: "route-1".to_owned(),
        target_provider: "claude".to_owned(),
        context_pack_hash: "ctx".to_owned(),
        pending_approval_hash: Some("approval".to_owned()),
    };
    assert_eq!(memory.expected_revision, 4);
    assert_eq!(handoff.target_provider, "claude");
}
