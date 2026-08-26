use mission_domain::{
    Approval, CriterionEvidence, CriterionStatus, EvidenceEntry, EvidenceKind, EvidenceMatrix,
    Route, RouteAbandonment, RouteId, RouteState,
};

fn complete_evidence() -> EvidenceMatrix {
    EvidenceMatrix {
        criteria: vec![CriterionEvidence {
            criterion_id: "tests".to_owned(),
            description: "route tests".to_owned(),
            status: CriterionStatus::Verified,
            evidence_ids: vec!["test-1".to_owned()],
        }],
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
    assert!(route.transition(RouteState::Completed).is_err());
    assert!(
        route
            .complete_with_evidence(
                Approval {
                    actor: "user".to_owned(),
                    decision: "accept".to_owned(),
                    evidence_event_ids: vec!["event-1".to_owned()],
                },
                EvidenceMatrix::from_criteria([CriterionEvidence::new("tests", "route tests")]),
            )
            .is_err()
    );
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
    assert!(
        route
            .final_approval
            .as_ref()
            .is_some_and(Approval::is_acceptance)
    );
    assert!(route.evidence_matrix.is_complete());
    assert!(route.transition(RouteState::Executing).is_err());
}

#[test]
fn abandoned_route_can_only_be_recovered_as_a_new_derived_route() {
    let mut route = Route::new(RouteId::new());
    let event = route
        .abandon(RouteAbandonment {
            last_checkpoint_id: "cp-final".to_owned(),
            reason: "verification failed".to_owned(),
            failure_evidence_ids: vec!["evidence-failure".to_owned()],
            reusable_artifacts: vec!["src/lib.rs".to_owned()],
        })
        .expect("abandon");
    route.apply_abandonment(event).expect("apply abandon");
    assert!(route.transition(RouteState::Executing).is_err());
    let recovered = Route::derived(RouteId::new(), &route).expect("derive abandoned route");
    assert_eq!(recovered.derived_from, Some(route.route_id));
    assert_eq!(recovered.state, RouteState::Draft);
    assert_eq!(
        route
            .abandonment
            .as_ref()
            .expect("metadata")
            .last_checkpoint_id,
        "cp-final"
    );
}
