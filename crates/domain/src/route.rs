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
            final_approval: None,
            evidence_matrix: EvidenceMatrix::default(),
            updated_at: Timestamp::now(),
        }
    }

    pub fn derived(route_id: RouteId, derived_from: RouteId) -> Self {
        let mut route = Self::new(route_id);
        route.derived_from = Some(derived_from);
        route
    }

    /// Return an appendable domain event. The route is changed only by applying that event.
    pub fn transition(&self, target: RouteState) -> Result<RouteTransitioned, InvalidTransition> {
        if !allowed_transition(self.state, target) {
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
        })
    }

    pub fn apply_transition(&mut self, event: RouteTransitioned) -> Result<(), InvalidTransition> {
        if event.route_id != self.route_id
            || event.from != self.state
            || event.expected_version != self.version
        {
            return Err(InvalidTransition {
                from: self.state,
                to: event.to,
            });
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
            | (RouteState::Paused, RouteState::Abandoned)
            | (RouteState::Executing, RouteState::Blocked)
            | (RouteState::Verifying, RouteState::Blocked)
            | (RouteState::Blocked, RouteState::Executing)
            | (RouteState::Blocked, RouteState::Abandoned)
            | (RouteState::Draft, RouteState::Abandoned)
            | (RouteState::ReadOnlyExploration, RouteState::Abandoned)
            | (RouteState::AwaitingPlanApproval, RouteState::Abandoned)
            | (RouteState::Verifying, RouteState::Abandoned)
    )
}

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
}
