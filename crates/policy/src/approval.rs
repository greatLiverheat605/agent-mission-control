use mission_domain::{MissionId, RouteId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ActionClass;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    Expired,
    Revoked,
    Consumed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalActor {
    User,
    Supervisor,
    Agent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAction {
    Approve,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Once,
    RouteActionClass(ActionClass),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalSubject {
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub action_digest: String,
    pub action_class: ActionClass,
    pub contract_version: u64,
    pub loadout_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalResolution {
    pub approval_id: String,
    pub expected_revision: u64,
    pub actor: ApprovalActor,
    pub decision: ApprovalAction,
    pub subject: ApprovalSubject,
    pub now_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    id: String,
    subject: ApprovalSubject,
    scope: ApprovalScope,
    requested_by: ApprovalActor,
    resolved_by: Option<ApprovalActor>,
    expires_at_ms: u64,
    state: ApprovalState,
    revision: u64,
}

impl ApprovalRequest {
    pub fn new(
        id: impl Into<String>,
        subject: ApprovalSubject,
        scope: ApprovalScope,
        requested_by: ApprovalActor,
        expires_at_ms: u64,
    ) -> Result<Self, ApprovalError> {
        if requested_by != ApprovalActor::Supervisor {
            return Err(ApprovalError::ForbiddenActor);
        }
        let id = id.into();
        if id.trim().is_empty()
            || subject.action_digest.trim().is_empty()
            || subject.loadout_fingerprint.trim().is_empty()
        {
            return Err(ApprovalError::InvalidRequest);
        }
        Ok(Self {
            id,
            subject,
            scope,
            requested_by,
            resolved_by: None,
            expires_at_ms,
            state: ApprovalState::Pending,
            revision: 0,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn subject(&self) -> &ApprovalSubject {
        &self.subject
    }

    pub const fn scope(&self) -> ApprovalScope {
        self.scope
    }

    pub const fn state(&self) -> ApprovalState {
        self.state
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    pub const fn requested_by(&self) -> ApprovalActor {
        self.requested_by
    }

    pub const fn resolved_by(&self) -> Option<ApprovalActor> {
        self.resolved_by
    }

    pub fn resolve(&mut self, resolution: ApprovalResolution) -> Result<(), ApprovalError> {
        if resolution.approval_id != self.id {
            return Err(ApprovalError::ApprovalMismatch);
        }
        if resolution.expected_revision != self.revision {
            return Err(ApprovalError::RevisionConflict {
                expected: resolution.expected_revision,
                actual: self.revision,
            });
        }
        if resolution.actor != ApprovalActor::User {
            return Err(ApprovalError::ForbiddenActor);
        }
        if self.state != ApprovalState::Pending {
            return Err(ApprovalError::NotPending);
        }
        if resolution.now_ms >= self.expires_at_ms {
            self.transition(ApprovalState::Expired, None);
            return Err(ApprovalError::Expired);
        }
        if !self.matches_authority_context(&resolution.subject)
            || resolution.subject.action_digest != self.subject.action_digest
        {
            return Err(ApprovalError::ContextMismatch);
        }

        let state = match resolution.decision {
            ApprovalAction::Approve => ApprovalState::Approved,
            ApprovalAction::Deny => ApprovalState::Denied,
        };
        self.transition(state, Some(resolution.actor));
        Ok(())
    }

    pub fn authorize(
        &mut self,
        subject: &ApprovalSubject,
        now_ms: u64,
    ) -> Result<(), ApprovalError> {
        if self.state != ApprovalState::Approved {
            return Err(ApprovalError::NotApproved);
        }
        if now_ms >= self.expires_at_ms {
            self.transition(ApprovalState::Expired, self.resolved_by);
            return Err(ApprovalError::Expired);
        }
        if !self.matches_authority_context(subject) {
            return Err(ApprovalError::ContextMismatch);
        }
        match self.scope {
            ApprovalScope::Once if subject.action_digest != self.subject.action_digest => {
                Err(ApprovalError::ContextMismatch)
            }
            ApprovalScope::Once => {
                self.transition(ApprovalState::Consumed, self.resolved_by);
                Ok(())
            }
            ApprovalScope::RouteActionClass(class) if subject.action_class == class => Ok(()),
            ApprovalScope::RouteActionClass(_) => Err(ApprovalError::ContextMismatch),
        }
    }

    pub fn revoke(&mut self, actor: ApprovalActor) -> Result<(), ApprovalError> {
        if actor != ApprovalActor::User {
            return Err(ApprovalError::ForbiddenActor);
        }
        if !matches!(self.state, ApprovalState::Pending | ApprovalState::Approved) {
            return Err(ApprovalError::NotRevocable);
        }
        self.transition(ApprovalState::Revoked, Some(actor));
        Ok(())
    }

    pub fn revoke_for_loadout_change(&mut self) -> Result<(), ApprovalError> {
        if !matches!(self.state, ApprovalState::Pending | ApprovalState::Approved) {
            return Err(ApprovalError::NotRevocable);
        }
        self.transition(ApprovalState::Revoked, Some(ApprovalActor::Supervisor));
        Ok(())
    }

    pub fn expire(&mut self, now_ms: u64) -> bool {
        if matches!(self.state, ApprovalState::Pending | ApprovalState::Approved)
            && now_ms >= self.expires_at_ms
        {
            self.transition(ApprovalState::Expired, self.resolved_by);
            true
        } else {
            false
        }
    }

    fn matches_authority_context(&self, subject: &ApprovalSubject) -> bool {
        self.subject.mission_id == subject.mission_id
            && self.subject.route_id == subject.route_id
            && self.subject.contract_version == subject.contract_version
            && self.subject.loadout_fingerprint == subject.loadout_fingerprint
            && self.subject.action_class == subject.action_class
    }

    fn transition(&mut self, state: ApprovalState, actor: Option<ApprovalActor>) {
        self.state = state;
        self.resolved_by = actor;
        self.revision += 1;
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ApprovalError {
    #[error("only the supervisor can request and only the user can resolve approvals")]
    ForbiddenActor,
    #[error("approval request is missing required binding data")]
    InvalidRequest,
    #[error("approval id does not match")]
    ApprovalMismatch,
    #[error("approval revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("approval is not pending")]
    NotPending,
    #[error("approval has expired")]
    Expired,
    #[error("approval authority context does not match")]
    ContextMismatch,
    #[error("approval is not approved")]
    NotApproved,
    #[error("approval cannot be revoked in its current state")]
    NotRevocable,
}
