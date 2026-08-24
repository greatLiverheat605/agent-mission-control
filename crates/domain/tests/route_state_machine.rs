use mission_domain::{
    Approval, EvidenceEntry, EvidenceKind, EvidenceMatrix, Route, RouteId, RouteState,
};

fn complete_evidence() -> EvidenceMatrix {
    EvidenceMatrix {
        entries: vec![EvidenceEntry {
            id: "test-1".to_owned(),
            kind: EvidenceKind::Test,
            summary: "route test".to_owned(),
            verified: true,
            source_event_ids: vec!["event-1".to_owned()],
        }],
    }
}

#[test]
fn valid_route_path_is_append_only_and_invalid_jump_is_rejected() {
    let mut route = Route::new(RouteId::new());
    assert!(route.transition(RouteState::Executing).is_err());
    assert_eq!(route.state, RouteState::Draft);

    for target in [
        RouteState::ReadOnlyExploration,
        RouteState::AwaitingPlanApproval,
        RouteState::Executing,
        RouteState::Verifying,
        RouteState::AwaitingAcceptance,
    ] {
        let event = route.transition(target).expect("valid transition");
        route.apply_transition(event).expect("apply transition");
    }
    let event = route
        .complete_with_evidence(
            Approval {
                actor: "user".to_owned(),
                decision: "accept".to_owned(),
                evidence_event_ids: vec!["event-1".to_owned()],
            },
            complete_evidence(),
        )
        .expect("complete route");
    route.apply_transition(event).expect("apply completion");
    assert_eq!(route.state, RouteState::Completed);
    assert!(route.transition(RouteState::Executing).is_err());
}

#[test]
fn abandoned_route_can_only_be_recovered_as_a_new_derived_route() {
    let mut route = Route::new(RouteId::new());
    let event = route.transition(RouteState::Abandoned).expect("abandon");
    route.apply_transition(event).expect("apply abandon");
    assert!(route.transition(RouteState::Executing).is_err());
    let recovered = Route::derived(RouteId::new(), route.route_id);
    assert_eq!(recovered.derived_from, Some(route.route_id));
    assert_eq!(recovered.state, RouteState::Draft);
}
