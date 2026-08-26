use mission_domain::DrivingMode;
use mission_policy::{
    ActionClass, ActionEvidence, ActionIntent, Deviation, IntentOrigin, PolicyContext,
    PolicyDecision, ReasonCode, evaluate,
};

fn context() -> PolicyContext {
    PolicyContext {
        driving_mode: DrivingMode::Autopilot,
        autopilot_allowed_actions: vec![ActionClass::Read, ActionClass::Write],
    }
}

fn intent(origin: IntentOrigin, class: ActionClass) -> ActionIntent {
    ActionIntent {
        class,
        origin,
        planned: true,
        within_allowed_paths: true,
        deviation: Deviation::None,
        evidence: ActionEvidence {
            event_id: "event-untrusted".to_owned(),
            action_digest: "sha256:untrusted".to_owned(),
        },
    }
}

#[test]
fn untrusted_content_cannot_create_actions_or_mutate_authority() {
    let origins = [
        IntentOrigin::TerminalText,
        IntentOrigin::RepositoryContent,
        IntentOrigin::WebContent,
        IntentOrigin::McpContent,
        IntentOrigin::Memory,
        IntentOrigin::ToolOutput,
    ];

    for origin in origins {
        for class in [ActionClass::Read, ActionClass::ContractChange] {
            assert!(matches!(
                evaluate(&context(), &intent(origin, class)),
                PolicyDecision::DenyAndPause {
                    ref contract_clause_id,
                    reason_code: ReasonCode::UntrustedIntentSource,
                    ..
                } if contract_clause_id == "trusted_intent_source"
            ));
        }
    }
}

#[test]
fn unknown_and_outside_path_actions_pause_before_mode_evaluation() {
    let unknown = intent(IntentOrigin::AdapterStructured, ActionClass::Unknown);
    assert!(matches!(
        evaluate(&context(), &unknown),
        PolicyDecision::DenyAndPause {
            reason_code: ReasonCode::UnknownAction,
            ..
        }
    ));

    let mut outside = intent(IntentOrigin::Supervisor, ActionClass::Read);
    outside.within_allowed_paths = false;
    assert!(matches!(
        evaluate(&context(), &outside),
        PolicyDecision::DenyAndPause {
            ref contract_clause_id,
            reason_code: ReasonCode::OutsideAllowedPaths,
            ..
        } if contract_clause_id == "allowed_paths"
    ));
}

#[test]
fn actions_without_auditable_evidence_pause() {
    let mut action = intent(IntentOrigin::Supervisor, ActionClass::Read);
    action.evidence.action_digest.clear();

    assert!(matches!(
        evaluate(&context(), &action),
        PolicyDecision::DenyAndPause {
            ref contract_clause_id,
            reason_code: ReasonCode::MissingActionEvidence,
            ..
        } if contract_clause_id == "action_evidence_required"
    ));
}
