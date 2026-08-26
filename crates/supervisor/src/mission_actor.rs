use adapter_core::AgentEvent;
use mission_domain::{
    EventConfidence, EventEnvelope, EventId, EventKind, EventSource, MissionId, RouteId,
};
use mission_policy::{
    ActionIntent, ApprovalAction, ApprovalActor, ApprovalError, ApprovalRequest,
    ApprovalResolution, ApprovalState, ApprovalSubject, BudgetSignal, EnvelopeDecision,
    FlightEnvelope, FlightIdentity, PolicyContext, PolicyDecision, evaluate,
};
use mission_protocol::command::{Actor, ApprovalDecision, ResolveApproval};
use std::collections::HashMap;
use std::str::FromStr;

use crate::pause::{PauseController, PauseError, PauseState};
use crate::process_tree::OwnedProcessTree;

pub trait ActorLedger {
    type Error: std::fmt::Display;
    fn append_event(&mut self, event: &EventEnvelope) -> Result<(), Self::Error>;
    fn replay_after(&self, mission_id: &MissionId, after_sequence: u64) -> Vec<EventEnvelope>;
    fn latest_sequence(&self, mission_id: &MissionId) -> u64 {
        self.replay_after(mission_id, 0)
            .iter()
            .map(|event| event.sequence)
            .max()
            .unwrap_or(0)
    }
}

impl ActorLedger for Vec<EventEnvelope> {
    type Error = std::convert::Infallible;
    fn append_event(&mut self, event: &EventEnvelope) -> Result<(), Self::Error> {
        self.push(event.clone());
        Ok(())
    }
    fn replay_after(&self, _mission_id: &MissionId, after_sequence: u64) -> Vec<EventEnvelope> {
        self.iter()
            .filter(|event| event.sequence > after_sequence)
            .cloned()
            .collect()
    }
}

impl ActorLedger for mission_ledger::EncryptedLedger {
    type Error = mission_ledger::LedgerError;

    fn append_event(&mut self, event: &EventEnvelope) -> Result<(), Self::Error> {
        self.append(event)
    }

    fn replay_after(&self, mission_id: &MissionId, after_sequence: u64) -> Vec<EventEnvelope> {
        self.replay_events(mission_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|event| event.sequence > after_sequence)
            .collect()
    }
}

#[derive(Debug)]
pub struct MissionActor<L> {
    mission_id: MissionId,
    route_id: RouteId,
    ledger: L,
    sequence: u64,
    pause: PauseController,
    process_tree: OwnedProcessTree,
    ui_connected: bool,
    approvals: HashMap<String, ApprovalRequest>,
}

#[derive(Debug)]
pub enum ActorError {
    Ledger(String),
    Pause(PauseError),
    Approval(ApprovalError),
    MissingApprovalRequest,
    ApprovalBindingMismatch,
    InvalidApprovalSubject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchGate {
    Ready,
    WaitingForApproval(String),
    Paused,
}

impl<L: ActorLedger> MissionActor<L> {
    pub fn new(mission_id: MissionId, route_id: RouteId, ledger: L) -> Self {
        let sequence = ledger.latest_sequence(&mission_id);
        let replayed = ledger.replay_after(&mission_id, 0);
        let pause = PauseController::from_events(&replayed);
        let approvals = replay_approvals(&replayed);
        let ui_connected = matches!(pause.state(), PauseState::Running);
        Self {
            mission_id,
            route_id,
            ledger,
            sequence,
            pause,
            process_tree: OwnedProcessTree::new(),
            ui_connected,
            approvals,
        }
    }

    pub fn state(&self) -> &PauseState {
        self.pause.state()
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn ledger(&self) -> &L {
        &self.ledger
    }
    pub fn ledger_mut(&mut self) -> &mut L {
        &mut self.ledger
    }
    pub fn process_tree_mut(&mut self) -> &mut OwnedProcessTree {
        &mut self.process_tree
    }
    pub fn ui_connected(&self) -> bool {
        self.ui_connected
    }
    pub fn replay_after(&self, after_sequence: u64) -> Vec<EventEnvelope> {
        self.ledger.replay_after(&self.mission_id, after_sequence)
    }

    pub fn pending_approval(&self, approval_id: &str) -> Option<&ApprovalRequest> {
        self.approvals.get(approval_id)
    }

    pub fn gate_action(
        &mut self,
        context: &PolicyContext,
        intent: &ActionIntent,
        request: Option<ApprovalRequest>,
    ) -> Result<DispatchGate, ActorError> {
        match evaluate(context, intent) {
            PolicyDecision::Allow => Ok(DispatchGate::Ready),
            PolicyDecision::RequireApproval {
                action_evidence, ..
            } => {
                let request = request.ok_or(ActorError::MissingApprovalRequest)?;
                if request.subject().mission_id != self.mission_id
                    || request.subject().route_id != self.route_id
                    || request.subject().action_digest != action_evidence.action_digest
                    || request.subject().action_class != intent.class
                {
                    return Err(ActorError::ApprovalBindingMismatch);
                }
                let approval_id = request.id().to_owned();
                self.append(
                    EventKind::ApprovalRequested,
                    serde_json::json!({"approval": request}),
                )?;
                self.approvals.insert(approval_id.clone(), request);
                Ok(DispatchGate::WaitingForApproval(approval_id))
            }
            PolicyDecision::RequireUserJudgment { .. } | PolicyDecision::DenyAndPause { .. } => {
                self.request_safe_pause("policy denied action dispatch")?;
                Ok(DispatchGate::Paused)
            }
        }
    }

    pub fn resolve_approval(
        &mut self,
        actor: Actor,
        command: ResolveApproval,
    ) -> Result<ApprovalState, ActorError> {
        let current = self
            .approvals
            .get(&command.approval_id)
            .cloned()
            .ok_or(ActorError::InvalidApprovalSubject)?;
        let mut next = current.clone();
        let subject = ApprovalSubject {
            mission_id: MissionId::from_str(&command.mission_id)
                .map_err(|_| ActorError::InvalidApprovalSubject)?,
            route_id: RouteId::from_str(&command.route_id)
                .map_err(|_| ActorError::InvalidApprovalSubject)?,
            action_digest: command.action_digest,
            action_class: current.subject().action_class,
            contract_version: command.contract_version,
            loadout_fingerprint: command.loadout_fingerprint,
        };
        let approval_actor = protocol_actor(actor);
        let event_kind = match command.decision {
            ApprovalDecision::Approve | ApprovalDecision::Deny => {
                let decision = if command.decision == ApprovalDecision::Approve {
                    ApprovalAction::Approve
                } else {
                    ApprovalAction::Deny
                };
                next.resolve(ApprovalResolution {
                    approval_id: command.approval_id.clone(),
                    expected_revision: command.expected_revision,
                    actor: approval_actor,
                    decision,
                    subject,
                    now_ms: command.now_ms,
                })
                .map_err(ActorError::Approval)?;
                EventKind::ApprovalResolved
            }
            ApprovalDecision::Revoke => {
                if command.expected_revision != next.revision() {
                    return Err(ActorError::Approval(ApprovalError::RevisionConflict {
                        expected: command.expected_revision,
                        actual: next.revision(),
                    }));
                }
                next.revoke(approval_actor).map_err(ActorError::Approval)?;
                EventKind::ApprovalRevoked
            }
        };
        self.append(event_kind, serde_json::json!({"approval": next}))?;
        let state = next.state();
        self.approvals.insert(command.approval_id, next);
        Ok(state)
    }

    pub fn authorize_approved_action(
        &mut self,
        approval_id: &str,
        subject: &ApprovalSubject,
        now_ms: u64,
    ) -> Result<(), ActorError> {
        let mut next = self
            .approvals
            .get(approval_id)
            .cloned()
            .ok_or(ActorError::InvalidApprovalSubject)?;
        let before = next.state();
        next.authorize(subject, now_ms)
            .map_err(ActorError::Approval)?;
        if before != next.state() {
            self.append(
                EventKind::ApprovalConsumed,
                serde_json::json!({"approval": next}),
            )?;
        }
        self.approvals.insert(approval_id.to_owned(), next);
        Ok(())
    }

    pub fn apply_budget_signals(&mut self, signals: &[BudgetSignal]) -> Result<bool, ActorError> {
        let mut pause = false;
        for signal in signals {
            let (kind, dimension) = match signal {
                BudgetSignal::Warning(dimension) => (EventKind::BudgetWarning, dimension),
                BudgetSignal::RequireApproval(dimension) => {
                    (EventKind::BudgetApprovalRequired, dimension)
                }
                BudgetSignal::PauseAtSafeBoundary(dimension) => {
                    pause = true;
                    (EventKind::BudgetExceeded, dimension)
                }
            };
            self.append(
                kind,
                serde_json::json!({"dimension": format!("{dimension:?}")}),
            )?;
        }
        if pause {
            self.request_safe_pause("mission budget reached at safe boundary")?;
        }
        Ok(pause)
    }

    pub fn check_flight_identity(
        &mut self,
        envelope: &FlightEnvelope,
        current: &FlightIdentity,
    ) -> Result<EnvelopeDecision, ActorError> {
        let decision = envelope.check_before_model_request(current);
        if decision == EnvelopeDecision::PauseIdentityChanged {
            self.append(
                EventKind::FlightEnvelopeChanged,
                serde_json::json!({
                    "provider": current.provider,
                    "model": current.model,
                    "loadout_fingerprint": current.loadout_fingerprint,
                }),
            )?;
            self.request_safe_pause("flight envelope identity changed")?;
        }
        Ok(decision)
    }

    pub fn check_loadout_change(
        &mut self,
        previous_fingerprint: &str,
        next_fingerprint: &str,
    ) -> Result<bool, ActorError> {
        if previous_fingerprint.trim().is_empty()
            || next_fingerprint.trim().is_empty()
            || previous_fingerprint == next_fingerprint
        {
            return Ok(false);
        }
        let approval_ids = self
            .approvals
            .values()
            .filter(|approval| {
                matches!(
                    approval.state(),
                    ApprovalState::Pending | ApprovalState::Approved
                )
            })
            .map(|approval| approval.id().to_owned())
            .collect::<Vec<_>>();
        for approval_id in approval_ids {
            let mut approval = self
                .approvals
                .get(&approval_id)
                .cloned()
                .ok_or(ActorError::InvalidApprovalSubject)?;
            approval
                .revoke_for_loadout_change()
                .map_err(ActorError::Approval)?;
            self.append(
                EventKind::ApprovalRevoked,
                serde_json::json!({"approval": approval, "reason": "loadout_changed"}),
            )?;
            self.approvals.insert(approval_id, approval);
        }
        self.append(
            EventKind::LoadoutChanged,
            serde_json::json!({
                "previous_fingerprint": previous_fingerprint,
                "next_fingerprint": next_fingerprint,
            }),
        )?;
        self.request_safe_pause("provider loadout changed")?;
        Ok(true)
    }

    pub fn record_agent_event(&mut self, event: AgentEvent) -> Result<(), ActorError> {
        let next = self.sequence + 1;
        let mut envelope = event.into_envelope(self.mission_id, self.route_id, next);
        envelope.source = EventSource::Agent;
        envelope.confidence = EventConfidence::Observed;
        self.append_envelope(envelope)
    }

    pub fn record_event(
        &mut self,
        kind: EventKind,
        payload: serde_json::Value,
    ) -> Result<(), ActorError> {
        self.append(kind, payload)
    }

    pub fn request_safe_pause(&mut self, reason: impl Into<String>) -> Result<(), ActorError> {
        let reason = reason.into();
        let transition = self
            .pause
            .request(reason.clone())
            .map_err(ActorError::Pause)?;
        self.append(
            EventKind::PauseRequested,
            serde_json::json!({"reason": reason, "state": format!("{:?}", transition.to)}),
        )
    }

    pub fn acknowledge_pause(&mut self) -> Result<(), ActorError> {
        let transition = self.pause.acknowledge().map_err(ActorError::Pause)?;
        self.append(
            EventKind::Unknown("pause_acknowledged".to_owned()),
            serde_json::json!({"state": format!("{:?}", transition.to)}),
        )
    }

    pub fn pause_timeout(&mut self) -> Result<String, ActorError> {
        let transition = self.pause.timeout().map_err(ActorError::Pause)?;
        let token = transition
            .confirmation_token
            .clone()
            .expect("timeout creates token");
        self.append(EventKind::Unknown("pause_timed_out".to_owned()), serde_json::json!({"state": format!("{:?}", transition.to), "force_token": "[REDACTED:confirmation_token]"}))?;
        Ok(token)
    }

    pub fn force_terminate(&mut self, token: &str) -> Result<(), ActorError> {
        let transition = self
            .pause
            .force_terminate(token)
            .map_err(ActorError::Pause)?;
        self.process_tree
            .terminate()
            .map_err(|error| ActorError::Ledger(error.to_string()))?;
        self.append(
            EventKind::Unknown("force_terminated".to_owned()),
            serde_json::json!({"state": format!("{:?}", transition.to)}),
        )
    }

    pub fn can_force_terminate(&self, token: &str) -> bool {
        self.pause.can_force_terminate(token)
    }

    pub fn set_ui_connected(&mut self, connected: bool) -> Result<(), ActorError> {
        self.ui_connected = connected;
        if !connected && matches!(self.pause.state(), PauseState::Running) {
            self.request_safe_pause("ui disconnected")?;
        }
        Ok(())
    }

    fn append(&mut self, kind: EventKind, payload: serde_json::Value) -> Result<(), ActorError> {
        let next = self.sequence + 1;
        let event = EventEnvelope::new(
            EventId::new(),
            self.mission_id,
            self.route_id,
            next,
            kind,
            payload,
        );
        self.append_envelope(event)
    }

    fn append_envelope(&mut self, event: EventEnvelope) -> Result<(), ActorError> {
        self.ledger
            .append_event(&event)
            .map_err(|error| ActorError::Ledger(error.to_string()))?;
        self.sequence = event.sequence;
        Ok(())
    }
}

fn protocol_actor(actor: Actor) -> ApprovalActor {
    match actor {
        Actor::User => ApprovalActor::User,
        Actor::Supervisor => ApprovalActor::Supervisor,
        Actor::Agent | Actor::Renderer => ApprovalActor::Agent,
    }
}

fn replay_approvals(events: &[EventEnvelope]) -> HashMap<String, ApprovalRequest> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::ApprovalRequested
                    | EventKind::ApprovalResolved
                    | EventKind::ApprovalRevoked
                    | EventKind::ApprovalExpired
                    | EventKind::ApprovalConsumed
            )
        })
        .filter_map(|event| {
            serde_json::from_value::<ApprovalRequest>(event.payload.get("approval")?.clone()).ok()
        })
        .map(|approval| (approval.id().to_owned(), approval))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::MissionActor;
    use mission_domain::{MissionId, RouteId};
    use mission_policy::{
        ActionClass, ApprovalActor, ApprovalRequest, ApprovalScope, ApprovalState, ApprovalSubject,
    };

    #[test]
    fn append_first_keeps_sequence_and_disconnect_is_safe_pause() {
        let mut actor = MissionActor::new(MissionId::new(), RouteId::new(), Vec::new());
        actor.set_ui_connected(false).expect("pause");
        assert_eq!(actor.sequence(), 1);
        assert!(!actor.ui_connected());
        assert_eq!(actor.ledger().len(), 1);
        assert_eq!(actor.replay_after(0).len(), 1);
        actor.set_ui_connected(true).expect("reconnect");
        assert_eq!(
            actor.state(),
            &crate::pause::PauseState::PauseRequested {
                reason: "ui disconnected".to_owned()
            }
        );
    }

    #[test]
    fn loadout_change_revokes_bound_approvals_before_pausing() {
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let mut actor = MissionActor::new(mission_id, route_id, Vec::new());
        let approval = ApprovalRequest::new(
            "approval-loadout",
            ApprovalSubject {
                mission_id,
                route_id,
                action_digest: "sha256:action".to_owned(),
                action_class: ActionClass::Write,
                contract_version: 1,
                loadout_fingerprint: "loadout-v1".to_owned(),
            },
            ApprovalScope::Once,
            ApprovalActor::Supervisor,
            u64::MAX,
        )
        .expect("approval");
        actor.approvals.insert(approval.id().to_owned(), approval);

        assert!(
            actor
                .check_loadout_change("loadout-v1", "loadout-v2")
                .expect("loadout change")
        );
        assert_eq!(
            actor
                .pending_approval("approval-loadout")
                .expect("approval")
                .state(),
            ApprovalState::Revoked
        );
        assert!(
            actor
                .ledger()
                .iter()
                .any(|event| event.kind == mission_domain::EventKind::LoadoutChanged)
        );
        assert!(matches!(
            actor.state(),
            crate::pause::PauseState::PauseRequested { .. }
        ));
    }
}
