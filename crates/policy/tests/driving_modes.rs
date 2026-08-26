use mission_domain::DrivingMode;
use mission_policy::{
    ActionClass, ActionEvidence, ActionIntent, Deviation, IntentOrigin, PolicyContext,
    PolicyDecision, ReasonCode, evaluate,
};

fn intent(class: ActionClass, planned: bool) -> ActionIntent {
    ActionIntent {
        class,
        origin: IntentOrigin::AdapterStructured,
        planned,
        within_allowed_paths: true,
        deviation: Deviation::None,
        evidence: ActionEvidence {
            event_id: "event-1".to_owned(),
            action_digest: "sha256:action".to_owned(),
        },
    }
}

fn context(driving_mode: DrivingMode) -> PolicyContext {
    PolicyContext {
        driving_mode,
        autopilot_allowed_actions: vec![
            ActionClass::Read,
            ActionClass::Write,
            ActionClass::Test,
            ActionClass::Build,
            ActionClass::DependencyInstall,
            ActionClass::NetworkAccess,
        ],
    }
}

#[test]
fn driving_modes_follow_the_decision_table() {
    use ActionClass::*;
    use DrivingMode::*;

    let cases = [
        (Manual, Read, true, None),
        (
            Manual,
            Write,
            true,
            Some(ReasonCode::ManualApprovalRequired),
        ),
        (Manual, Test, true, Some(ReasonCode::ManualApprovalRequired)),
        (Assisted, Write, true, None),
        (
            Assisted,
            Write,
            false,
            Some(ReasonCode::AssistedApprovalRequired),
        ),
        (
            Assisted,
            NetworkAccess,
            true,
            Some(ReasonCode::AssistedApprovalRequired),
        ),
        (Autopilot, DependencyInstall, true, None),
        (Autopilot, NetworkAccess, true, None),
        (
            Autopilot,
            Write,
            false,
            Some(ReasonCode::OutsideAutopilotEnvelope),
        ),
    ];

    for (mode, class, planned, approval_reason) in cases {
        let decision = evaluate(&context(mode), &intent(class, planned));
        match approval_reason {
            None => assert_eq!(decision, PolicyDecision::Allow, "{mode:?} {class:?}"),
            Some(reason_code) => assert!(
                matches!(decision, PolicyDecision::RequireApproval { reason_code: actual, .. } if actual == reason_code),
                "{mode:?} {class:?}: {decision:?}"
            ),
        }
    }
}

#[test]
fn permanent_user_boundaries_never_run_autonomously() {
    use ActionClass::*;
    let classes = [
        CredentialAccess,
        ContractChange,
        ProviderChange,
        GitPush,
        GitMerge,
        Deploy,
        PermanentDelete,
    ];

    for mode in [
        DrivingMode::Manual,
        DrivingMode::Assisted,
        DrivingMode::Autopilot,
    ] {
        for class in classes {
            assert!(matches!(
                evaluate(&context(mode), &intent(class, true)),
                PolicyDecision::RequireApproval {
                    reason_code: ReasonCode::UserBoundaryRequired,
                    ..
                }
            ));
        }
    }
}

#[test]
fn hard_and_soft_deviations_have_distinct_fail_closed_results() {
    let mut action = intent(ActionClass::Write, true);
    action.deviation = Deviation::Suspected;
    assert!(matches!(
        evaluate(&context(DrivingMode::Autopilot), &action),
        PolicyDecision::RequireUserJudgment {
            reason_code: ReasonCode::SuspectedContractDeviation,
            ..
        }
    ));

    action.deviation = Deviation::Confirmed;
    assert!(matches!(
        evaluate(&context(DrivingMode::Autopilot), &action),
        PolicyDecision::DenyAndPause { ref contract_clause_id, reason_code: ReasonCode::ConfirmedContractDeviation, .. }
            if contract_clause_id == "goal_and_non_goals"
    ));
}
