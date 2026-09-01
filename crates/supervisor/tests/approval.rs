use mission_domain::{DrivingMode, MissionId, RouteId};
use mission_policy::{
    ActionClass, ActionEvidence, ActionIntent, ApprovalActor, ApprovalRequest, ApprovalScope,
    ApprovalState, ApprovalSubject, Deviation, IntentOrigin, PolicyContext,
};
use mission_protocol::command::{Actor, ApprovalDecision, ApprovalGrantScope, ResolveApproval};
use mission_supervisor::mission_actor::{ActorError, DispatchGate, MissionActor};

fn intent() -> ActionIntent {
    ActionIntent {
        class: ActionClass::Write,
        origin: IntentOrigin::AdapterStructured,
        planned: true,
        within_allowed_paths: true,
        deviation: Deviation::None,
        evidence: ActionEvidence {
            event_id: "event-write".to_owned(),
            action_digest: "sha256:write".to_owned(),
        },
    }
}

#[test]
fn approval_resolution_is_revision_checked_before_dispatch() {
    let mission_id = MissionId::new();
    let route_id = RouteId::new();
    let subject = ApprovalSubject {
        mission_id,
        route_id,
        action_digest: "sha256:write".to_owned(),
        action_class: ActionClass::Write,
        contract_version: 4,
        loadout_fingerprint: "loadout-v4".to_owned(),
    };
    let request = ApprovalRequest::new(
        "approval-write",
        subject.clone(),
        ApprovalScope::Once,
        ApprovalActor::Supervisor,
        1_000,
    )
    .expect("request");
    let mut actor = MissionActor::new(mission_id, route_id, Vec::new());
    let gate = actor
        .gate_action(
            &PolicyContext {
                driving_mode: DrivingMode::Manual,
                autopilot_allowed_actions: Vec::new(),
            },
            &intent(),
            Some(request),
        )
        .expect("gate action");
    assert_eq!(
        gate,
        DispatchGate::WaitingForApproval("approval-write".to_owned())
    );
    assert_eq!(actor.ledger()[0].kind.as_str(), "approval_requested");

    let command = ResolveApproval {
        approval_id: "approval-write".to_owned(),
        expected_revision: 0,
        decision: ApprovalDecision::Approve,
        mission_id: mission_id.to_string(),
        route_id: route_id.to_string(),
        contract_version: 4,
        loadout_fingerprint: "loadout-v4".to_owned(),
        action_digest: "sha256:write".to_owned(),
        now_ms: 100,
        scope: Some(ApprovalGrantScope::Once),
    };
    assert!(matches!(
        actor.resolve_approval(Actor::Agent, command.clone()),
        Err(ActorError::Approval(_))
    ));
    actor
        .resolve_approval(Actor::User, command.clone())
        .expect("user resolves approval");
    assert_eq!(
        actor
            .pending_approval("approval-write")
            .map(ApprovalRequest::state),
        Some(ApprovalState::Approved)
    );
    assert!(matches!(
        actor.resolve_approval(Actor::User, command),
        Err(ActorError::Approval(_))
    ));

    actor
        .authorize_approved_action("approval-write", &subject, 101)
        .expect("approved action dispatch");
    assert_eq!(
        actor
            .pending_approval("approval-write")
            .map(ApprovalRequest::state),
        Some(ApprovalState::Consumed)
    );
    assert_eq!(
        actor.ledger().last().expect("event").kind.as_str(),
        "approval_consumed"
    );
}

#[test]
fn route_approval_resolution_preserves_route_action_scope() {
    let mission_id = MissionId::new();
    let route_id = RouteId::new();
    let subject = ApprovalSubject {
        mission_id,
        route_id,
        action_digest: "sha256:write".to_owned(),
        action_class: ActionClass::Write,
        contract_version: 2,
        loadout_fingerprint: "loadout-v2".to_owned(),
    };
    let request = ApprovalRequest::new(
        "approval-route",
        subject.clone(),
        ApprovalScope::Once,
        ApprovalActor::Supervisor,
        2_000,
    )
    .expect("request");
    let mut actor = MissionActor::new(mission_id, route_id, Vec::new());
    assert_eq!(
        actor
            .gate_action(
                &PolicyContext {
                    driving_mode: DrivingMode::Manual,
                    autopilot_allowed_actions: Vec::new(),
                },
                &intent(),
                Some(request),
            )
            .expect("gate"),
        DispatchGate::WaitingForApproval("approval-route".to_owned())
    );
    actor
        .resolve_approval(
            Actor::User,
            ResolveApproval {
                approval_id: "approval-route".to_owned(),
                expected_revision: 0,
                decision: ApprovalDecision::Approve,
                mission_id: mission_id.to_string(),
                route_id: route_id.to_string(),
                contract_version: 2,
                loadout_fingerprint: "loadout-v2".to_owned(),
                action_digest: "sha256:write".to_owned(),
                now_ms: 1_000,
                scope: Some(ApprovalGrantScope::RouteActionClass),
            },
        )
        .expect("resolve");
    assert_eq!(
        actor
            .pending_approval("approval-route")
            .expect("approval")
            .scope(),
        ApprovalScope::RouteActionClass(ActionClass::Write)
    );
}
