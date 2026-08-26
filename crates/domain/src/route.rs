use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::evidence::{Approval, EvidenceMatrix};
use crate::ids::{RouteId, Timestamp};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RouteState {
    Draft,
    ReadOnlyExploration,
    AwaitingPlanApproval,
    Executing,
    Verifying,
    AwaitingAcceptance,
    Completed,
    Paused,
    Blocked,
    Abandoned,
}

impl RouteState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Abandoned)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub route_id: RouteId,
    pub state: RouteState,
    pub version: u64,
    pub derived_from: Option<RouteId>,
    pub abandonment: Option<RouteAbandonment>,
    pub final_approval: Option<Approval>,
    pub evidence_matrix: EvidenceMatrix,
    pub updated_at: Timestamp,
}

impl Route {
    pub fn new(route_id: RouteId) -> Self {
        Self {
            route_id,
            state: RouteState::Draft,
            version: 0,
            derived_from: None,
            abandonment: None,
            final_approval: None,
            evidence_matrix: EvidenceMatrix::default(),
            updated_at: Timestamp::now(),
        }
    }

    pub fn derived(route_id: RouteId, source: &Route) -> Result<Self, InvalidDerivation> {
        if source.state != RouteState::Abandoned || source.abandonment.is_none() {
            return Err(InvalidDerivation);
        }
        let mut route = Self::new(route_id);
        route.derived_from = Some(source.route_id);
        Ok(route)
    }

    /// Return an appendable domain event. The route is changed only by applying that event.
    pub fn transition(&self, target: RouteState) -> Result<RouteTransitioned, InvalidTransition> {
        if target == RouteState::Completed || !allowed_transition(self.state, target) {
            return Err(InvalidTransition {
                from: self.state,
                to: target,
            });
        }
        Ok(RouteTransitioned {
            route_id: self.route_id,
            from: self.state,
            to: target,
            expected_version: self.version,
            acceptance: None,
        })
    }

    pub fn complete_with_evidence(
        &self,
        approval: Approval,
        evidence_matrix: EvidenceMatrix,
    ) -> Result<RouteTransitioned, InvalidTransition> {
        if self.state != RouteState::AwaitingAcceptance
            || !approval.is_acceptance()
            || !evidence_matrix.is_complete()
        {
            return Err(InvalidTransition {
                from: self.state,
                to: RouteState::Completed,
            });
        }
        Ok(RouteTransitioned {
            route_id: self.route_id,
            from: self.state,
            to: RouteState::Completed,
            expected_version: self.version,
            acceptance: Some(RouteAcceptance {
                approval,
                evidence_matrix,
            }),
        })
    }

    pub fn abandon(
        &self,
        metadata: RouteAbandonment,
    ) -> Result<RouteAbandoned, InvalidAbandonment> {
        if self.state.is_terminal()
            || metadata.last_checkpoint_id.trim().is_empty()
            || metadata.reason.trim().is_empty()
            || metadata.failure_evidence_ids.is_empty()
        {
            return Err(InvalidAbandonment);
        }
        Ok(RouteAbandoned {
            route_id: self.route_id,
            from: self.state,
            expected_version: self.version,
            metadata,
        })
    }

    pub fn apply_abandonment(&mut self, event: RouteAbandoned) -> Result<(), InvalidAbandonment> {
        if event.route_id != self.route_id
            || event.from != self.state
            || event.expected_version != self.version
            || self.state.is_terminal()
        {
            return Err(InvalidAbandonment);
        }
        self.state = RouteState::Abandoned;
        self.abandonment = Some(event.metadata);
        self.version += 1;
        self.updated_at = Timestamp::now();
        Ok(())
    }

    pub fn apply_transition(&mut self, event: RouteTransitioned) -> Result<(), InvalidTransition> {
        if event.route_id != self.route_id
            || event.from != self.state
            || event.expected_version != self.version
            || (event.to == RouteState::Completed) != event.acceptance.is_some()
        {
            return Err(InvalidTransition {
                from: self.state,
                to: event.to,
            });
        }
        if let Some(acceptance) = event.acceptance {
            if !acceptance.approval.is_acceptance() || !acceptance.evidence_matrix.is_complete() {
                return Err(InvalidTransition {
                    from: self.state,
                    to: event.to,
                });
            }
            self.final_approval = Some(acceptance.approval);
            self.evidence_matrix = acceptance.evidence_matrix;
        }
        self.state = event.to;
        self.version += 1;
        self.updated_at = Timestamp::now();
        Ok(())
    }
}

fn allowed_transition(from: RouteState, to: RouteState) -> bool {
    matches!(
        (from, to),
        (RouteState::Draft, RouteState::ReadOnlyExploration)
            | (
                RouteState::ReadOnlyExploration,
                RouteState::AwaitingPlanApproval
            )
            | (RouteState::AwaitingPlanApproval, RouteState::Executing)
            | (RouteState::Executing, RouteState::Verifying)
            | (RouteState::Verifying, RouteState::AwaitingAcceptance)
            | (RouteState::AwaitingAcceptance, RouteState::Completed)
            | (RouteState::Draft, RouteState::Paused)
            | (RouteState::ReadOnlyExploration, RouteState::Paused)
            | (RouteState::AwaitingPlanApproval, RouteState::Paused)
            | (RouteState::Executing, RouteState::Paused)
            | (RouteState::Verifying, RouteState::Paused)
            | (RouteState::AwaitingAcceptance, RouteState::Paused)
            | (RouteState::Paused, RouteState::Executing)
            | (RouteState::Executing, RouteState::Blocked)
            | (RouteState::Verifying, RouteState::Blocked)
            | (RouteState::Blocked, RouteState::Executing)
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteAbandonment {
    pub last_checkpoint_id: String,
    pub reason: String,
    pub failure_evidence_ids: Vec<String>,
    pub reusable_artifacts: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteAbandoned {
    pub route_id: RouteId,
    pub from: RouteState,
    pub expected_version: u64,
    pub metadata: RouteAbandonment,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("route abandonment requires a final checkpoint, reason, and failure evidence")]
pub struct InvalidAbandonment;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("only an abandoned route with retained metadata can be derived")]
pub struct InvalidDerivation;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("invalid route transition from {from:?} to {to:?}")]
pub struct InvalidTransition {
    pub from: RouteState,
    pub to: RouteState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteTransitioned {
    pub route_id: RouteId,
    pub from: RouteState,
    pub to: RouteState,
    pub expected_version: u64,
    pub acceptance: Option<RouteAcceptance>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteAcceptance {
    pub approval: Approval,
    pub evidence_matrix: EvidenceMatrix,
}
