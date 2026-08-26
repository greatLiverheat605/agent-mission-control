use thiserror::Error;

use crate::{ActionClass, BudgetLimits};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlightIdentity {
    pub provider: String,
    pub model: String,
    pub loadout_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvelopeDecision {
    Allow,
    PauseIdentityChanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlightEnvelope {
    allowed_action_classes: Vec<ActionClass>,
    path_roots: Vec<String>,
    network_allowlist: Vec<String>,
    dependency_sources: Vec<String>,
    identity: FlightIdentity,
    budget: BudgetLimits,
}

impl FlightEnvelope {
    pub fn new(
        allowed_action_classes: Vec<ActionClass>,
        path_roots: Vec<String>,
        network_allowlist: Vec<String>,
        dependency_sources: Vec<String>,
        identity: FlightIdentity,
        budget: BudgetLimits,
    ) -> Result<Self, EnvelopeError> {
        if allowed_action_classes.is_empty()
            || path_roots.iter().any(|value| value.trim().is_empty())
            || path_roots.is_empty()
            || identity.provider.trim().is_empty()
            || identity.model.trim().is_empty()
            || identity.loadout_fingerprint.trim().is_empty()
        {
            return Err(EnvelopeError::Incomplete);
        }
        Ok(Self {
            allowed_action_classes,
            path_roots,
            network_allowlist,
            dependency_sources,
            identity,
            budget,
        })
    }

    pub fn allows_action(&self, action: ActionClass) -> bool {
        self.allowed_action_classes.contains(&action)
    }

    pub fn check_before_model_request(&self, current: &FlightIdentity) -> EnvelopeDecision {
        if &self.identity == current {
            EnvelopeDecision::Allow
        } else {
            EnvelopeDecision::PauseIdentityChanged
        }
    }

    pub fn path_roots(&self) -> &[String] {
        &self.path_roots
    }

    pub fn network_allowlist(&self) -> &[String] {
        &self.network_allowlist
    }

    pub fn dependency_sources(&self) -> &[String] {
        &self.dependency_sources
    }

    pub fn identity(&self) -> &FlightIdentity {
        &self.identity
    }

    pub fn budget(&self) -> &BudgetLimits {
        &self.budget
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EnvelopeError {
    #[error("flight envelope is missing required launch bindings")]
    Incomplete,
}
