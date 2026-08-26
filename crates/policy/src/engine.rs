use mission_domain::DrivingMode;
use serde::{Deserialize, Serialize};

use crate::{ActionClass, ActionEvidence, ActionIntent, Deviation};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyContext {
    pub driving_mode: DrivingMode,
    pub autopilot_allowed_actions: Vec<ActionClass>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    MissingActionEvidence,
    UntrustedIntentSource,
    UnknownAction,
    OutsideAllowedPaths,
    ConfirmedContractDeviation,
    SuspectedContractDeviation,
    ManualApprovalRequired,
    AssistedApprovalRequired,
    OutsideAutopilotEnvelope,
    UserBoundaryRequired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    RequireApproval {
        reason_code: ReasonCode,
        action_evidence: ActionEvidence,
    },
    RequireUserJudgment {
        reason_code: ReasonCode,
        action_evidence: ActionEvidence,
    },
    DenyAndPause {
        contract_clause_id: String,
        reason_code: ReasonCode,
        action_evidence: ActionEvidence,
    },
}

pub fn evaluate(context: &PolicyContext, intent: &ActionIntent) -> PolicyDecision {
    if !intent.evidence.is_complete() {
        return deny(
            "action_evidence_required",
            ReasonCode::MissingActionEvidence,
            intent,
        );
    }
    if !intent.origin.is_trusted() {
        return deny(
            "trusted_intent_source",
            ReasonCode::UntrustedIntentSource,
            intent,
        );
    }
    if intent.class == ActionClass::Unknown {
        return deny("known_action_required", ReasonCode::UnknownAction, intent);
    }
    if !intent.within_allowed_paths {
        return deny("allowed_paths", ReasonCode::OutsideAllowedPaths, intent);
    }
    match intent.deviation {
        Deviation::Confirmed => {
            return deny(
                "goal_and_non_goals",
                ReasonCode::ConfirmedContractDeviation,
                intent,
            );
        }
        Deviation::Suspected => {
            return PolicyDecision::RequireUserJudgment {
                reason_code: ReasonCode::SuspectedContractDeviation,
                action_evidence: intent.evidence.clone(),
            };
        }
        Deviation::None => {}
    }
    if intent.class.always_requires_user() {
        return approval(ReasonCode::UserBoundaryRequired, intent);
    }

    match context.driving_mode {
        DrivingMode::Manual if intent.class != ActionClass::Read => {
            approval(ReasonCode::ManualApprovalRequired, intent)
        }
        DrivingMode::Assisted if !intent.planned || !intent.class.is_assisted_low_risk() => {
            approval(ReasonCode::AssistedApprovalRequired, intent)
        }
        DrivingMode::Autopilot
            if !intent.planned || !context.autopilot_allowed_actions.contains(&intent.class) =>
        {
            approval(ReasonCode::OutsideAutopilotEnvelope, intent)
        }
        _ => PolicyDecision::Allow,
    }
}

fn approval(reason_code: ReasonCode, intent: &ActionIntent) -> PolicyDecision {
    PolicyDecision::RequireApproval {
        reason_code,
        action_evidence: intent.evidence.clone(),
    }
}

fn deny(
    contract_clause_id: &str,
    reason_code: ReasonCode,
    intent: &ActionIntent,
) -> PolicyDecision {
    PolicyDecision::DenyAndPause {
        contract_clause_id: contract_clause_id.to_owned(),
        reason_code,
        action_evidence: intent.evidence.clone(),
    }
}
