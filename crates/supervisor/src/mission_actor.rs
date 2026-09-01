use adapter_core::AgentEvent;
use mission_domain::{
    EventConfidence, EventEnvelope, EventId, EventKind, EventSource, MissionId, RouteId,
    payload_hash,
};
use mission_policy::{
    ActionClass, ActionIntent, ApprovalAction, ApprovalActor, ApprovalError, ApprovalRequest,
    ApprovalResolution, ApprovalScope, ApprovalState, ApprovalSubject, BudgetSignal,
    EnvelopeDecision, FlightEnvelope, FlightIdentity, PolicyContext, PolicyDecision, evaluate,
};
use mission_protocol::command::{Actor, ApprovalDecision, ApprovalGrantScope, ResolveApproval};
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
    requires_recovery: bool,
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
    RecoveryNotRequired,
    RecoveryDecisionInvalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchGate {
    Ready,
    WaitingForApproval(String),
    Paused,
}

impl<L: ActorLedger> MissionActor<L> {
    pub fn new(mission_id: MissionId, route_id: RouteId, ledger: L) -> Self {
        Self::try_new(mission_id, route_id, ledger)
            .unwrap_or_else(|error| panic!("mission actor recovery failed: {error}"))
    }

    pub fn try_new(mission_id: MissionId, route_id: RouteId, ledger: L) -> Result<Self, String> {
        let sequence = ledger.latest_sequence(&mission_id);
        let replayed = ledger.replay_after(&mission_id, 0);
        let pause = PauseController::from_events(&replayed);
        let approvals = replay_approvals(&replayed)?;
        let requires_recovery = recovery_required(&replayed);
        let ui_connected = matches!(pause.state(), PauseState::Running) && !requires_recovery;
        Ok(Self {
            mission_id,
            route_id,
            ledger,
            sequence,
            pause,
            process_tree: OwnedProcessTree::new(),
            ui_connected,
            requires_recovery,
            approvals,
        })
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
    pub fn requires_recovery(&self) -> bool {
        self.requires_recovery
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
                    approval_event_payload(&request),
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
                if command.decision == ApprovalDecision::Approve {
                    let scope = match command.scope {
                        Some(ApprovalGrantScope::RouteActionClass) => {
                            ApprovalScope::RouteActionClass(current.subject().action_class)
                        }
                        Some(ApprovalGrantScope::Once) | None => ApprovalScope::Once,
                    };
                    next.set_scope(scope).map_err(ActorError::Approval)?;
                }
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
        let approval = if event.event_kind == EventKind::ApprovalRequested {
            approval_from_agent_event(&event, self.mission_id, self.route_id)
        } else {
            None
        };
        let next = self.sequence + 1;
        let mut envelope = event.into_envelope(self.mission_id, self.route_id, next);
        if let Some(approval) = &approval {
            if let Some(payload) = envelope.payload.as_object_mut() {
                payload.insert("approval".to_owned(), serde_json::json!(approval));
                payload.insert(
                    "approval_id".to_owned(),
                    serde_json::Value::String(approval.id().to_owned()),
                );
                payload.insert(
                    "expected_revision".to_owned(),
                    serde_json::Value::from(approval.revision()),
                );
            }
            envelope.payload_hash = payload_hash(&envelope.payload);
        }
        envelope.source = EventSource::Agent;
        envelope.confidence = EventConfidence::Observed;
        self.append_envelope(envelope)?;
        if let Some(approval) = approval {
            self.approvals.insert(approval.id().to_owned(), approval);
        }
        Ok(())
    }

    pub fn record_event(
        &mut self,
        kind: EventKind,
        payload: serde_json::Value,
    ) -> Result<(), ActorError> {
        self.append(kind, payload)
    }

    pub fn resolve_recovery(
        &mut self,
        decision: &str,
        manifest: serde_json::Value,
    ) -> Result<(), ActorError> {
        if !self.requires_recovery {
            return Err(ActorError::RecoveryNotRequired);
        }
        let kind = match decision {
            "continue" => EventKind::RecoveryContinued,
            "abandon" => EventKind::RecoveryAbandoned,
            _ => return Err(ActorError::RecoveryDecisionInvalid),
        };
        self.append(
            kind,
            serde_json::json!({
                "decision": decision,
                "manifest": manifest,
            }),
        )
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
        self.record_force_token_transition(transition)
    }

    pub fn request_force_termination(&mut self) -> Result<String, ActorError> {
        if matches!(self.pause.state(), PauseState::Running) {
            self.request_safe_pause("user requested force termination")?;
        }
        let transition = self
            .pause
            .force_termination_token()
            .map_err(ActorError::Pause)?;
        self.record_force_token_transition(transition)
    }

    pub fn force_terminate(&mut self, token: &str) -> Result<(), ActorError> {
        if !self.pause.can_force_terminate(token) {
            return Err(ActorError::Pause(PauseError::InvalidConfirmationToken));
        }
        if let Err(error) = self.process_tree.terminate() {
            let message = error.to_string();
            let _ = self.append(
                EventKind::Unknown("termination_failed".to_owned()),
                serde_json::json!({"reason": message, "state": "degraded", "degraded": true}),
            );
            return Err(ActorError::Ledger(format!("termination failed: {message}")));
        }
        let transition = self
            .pause
            .force_terminate(token)
            .map_err(ActorError::Pause)?;
        self.append(
            EventKind::Unknown("force_terminated".to_owned()),
            serde_json::json!({"state": format!("{:?}", transition.to)}),
        )
    }

    pub fn can_force_terminate(&self, token: &str) -> bool {
        self.pause.can_force_terminate(token)
    }

    fn record_force_token_transition(
        &mut self,
        transition: crate::pause::PauseTransition,
    ) -> Result<String, ActorError> {
        let token = transition
            .confirmation_token
            .clone()
            .ok_or(ActorError::Pause(PauseError::InvalidState))?;
        if transition.from != transition.to {
            self.append(
                EventKind::Unknown("pause_timed_out".to_owned()),
                serde_json::json!({
                    "state": format!("{:?}", transition.to),
                    "force_token": "[REDACTED:confirmation_token]"
                }),
            )?;
        }
        Ok(token)
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
        match event.kind {
            EventKind::AgentRunStarted => self.requires_recovery = true,
            EventKind::AgentRunCompleted => {
                if event
                    .payload
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    != Some("interrupted")
                {
                    self.requires_recovery = false;
                }
            }
            EventKind::AgentRunAborted
            | EventKind::RecoveryContinued
            | EventKind::RecoveryAbandoned => self.requires_recovery = false,
            EventKind::Unknown(ref kind) if kind == "force_terminated" => {
                self.requires_recovery = false
            }
            _ => {}
        }
        self.sequence = event.sequence;
        Ok(())
    }
}

fn recovery_required(events: &[EventEnvelope]) -> bool {
    let mut active = false;
    for event in events {
        match &event.kind {
            EventKind::AgentRunStarted => active = true,
            EventKind::AgentRunCompleted => {
                if event
                    .payload
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    != Some("interrupted")
                {
                    active = false;
                }
            }
            EventKind::AgentRunAborted
            | EventKind::RecoveryContinued
            | EventKind::RecoveryAbandoned => active = false,
            EventKind::Unknown(kind) if kind == "force_terminated" => active = false,
            _ => {}
        }
    }
    active
}

fn protocol_actor(actor: Actor) -> ApprovalActor {
    match actor {
        Actor::User => ApprovalActor::User,
        Actor::Supervisor => ApprovalActor::Supervisor,
        Actor::Agent | Actor::Renderer => ApprovalActor::Agent,
    }
}

fn replay_approvals(events: &[EventEnvelope]) -> Result<HashMap<String, ApprovalRequest>, String> {
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
        .map(|event| {
            let approval = event.payload.get("approval").ok_or_else(|| {
                format!(
                    "approval event at sequence {} is missing approval payload",
                    event.sequence
                )
            })?;
            serde_json::from_value::<ApprovalRequest>(approval.clone()).map_err(|error| {
                format!(
                    "approval event at sequence {} is invalid: {error}",
                    event.sequence
                )
            })
        })
        .map(|result| result.map(|approval| (approval.id().to_owned(), approval)))
        .collect()
}

fn approval_event_payload(request: &ApprovalRequest) -> serde_json::Value {
    let scope = match request.scope() {
        ApprovalScope::Once => "once",
        ApprovalScope::RouteActionClass(_) => "route_action_class",
    };
    serde_json::json!({
        "approval": request,
        "approval_id": request.id(),
        "action": format!("{:?}", request.subject().action_class),
        "action_class": request.subject().action_class,
        "action_digest": request.subject().action_digest,
        "scope": scope,
        "expires_at_ms": request.expires_at_ms(),
        "expected_revision": request.revision(),
        "contract_version": request.subject().contract_version,
        "loadout_fingerprint": request.subject().loadout_fingerprint,
    })
}

fn approval_from_agent_event(
    event: &AgentEvent,
    mission_id: MissionId,
    route_id: RouteId,
) -> Option<ApprovalRequest> {
    let payload = event.payload.as_object()?;
    let id = payload
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            payload
                .get("server_request_id")
                .map(serde_json::Value::to_string)
        })?;
    let action_digest = payload
        .get("action_digest")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("protocol-request");
    let contract_version = payload
        .get("contract_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    let loadout_fingerprint = payload
        .get("loadout_fingerprint")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("protocol");
    ApprovalRequest::new(
        id,
        ApprovalSubject {
            mission_id,
            route_id,
            action_digest: action_digest.to_owned(),
            action_class: ActionClass::Write,
            contract_version,
            loadout_fingerprint: loadout_fingerprint.to_owned(),
        },
        ApprovalScope::Once,
        ApprovalActor::Supervisor,
        u64::MAX,
    )
    .ok()
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

    #[test]
    fn rehydrated_agent_run_requires_explicit_recovery() {
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let mut events = Vec::new();
        let event = mission_domain::EventEnvelope::new(
            mission_domain::EventId::new(),
            mission_id,
            route_id,
            1,
            mission_domain::EventKind::AgentRunStarted,
            serde_json::json!({"run_id":"fixture"}),
        );
        events.push(event);
        let actor = MissionActor::new(mission_id, route_id, events);
        assert!(actor.requires_recovery());
        assert!(!actor.ui_connected());
    }

    #[test]
    fn terminal_run_events_clear_recovery_and_decisions_are_audited() {
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let mut actor = MissionActor::new(mission_id, route_id, Vec::new());
        actor
            .record_event(
                mission_domain::EventKind::AgentRunStarted,
                serde_json::json!({"run_id":"fixture"}),
            )
            .expect("start");
        assert!(actor.requires_recovery());
        actor
            .record_event(
                mission_domain::EventKind::AgentRunCompleted,
                serde_json::json!({"run_id":"fixture"}),
            )
            .expect("complete");
        assert!(!actor.requires_recovery());

        actor
            .record_event(
                mission_domain::EventKind::AgentRunStarted,
                serde_json::json!({"run_id":"fixture-2"}),
            )
            .expect("start again");
        actor
            .resolve_recovery("continue", serde_json::json!({"entry_hash":"hash"}))
            .expect("continue recovery");
        assert!(!actor.requires_recovery());
        assert_eq!(
            actor.ledger().last().expect("decision").kind,
            mission_domain::EventKind::RecoveryContinued
        );
        assert!(
            actor
                .resolve_recovery("continue", serde_json::json!({}))
                .is_err()
        );
    }

    #[test]
    fn interrupted_completion_keeps_recovery_required() {
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let mut actor = MissionActor::new(mission_id, route_id, Vec::new());
        actor
            .record_event(
                mission_domain::EventKind::AgentRunStarted,
                serde_json::json!({"run_id":"fixture"}),
            )
            .expect("start");
        actor
            .record_event(
                mission_domain::EventKind::AgentRunCompleted,
                serde_json::json!({"run_id":"fixture","status":"interrupted"}),
            )
            .expect("interrupted completion");
        assert!(actor.requires_recovery());
    }

    #[test]
    fn failed_process_kill_keeps_force_state_and_records_failure() {
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let mut actor = MissionActor::new(mission_id, route_id, Vec::new());
        actor.process_tree_mut().register(42);
        actor
            .process_tree_mut()
            .fail_termination_for_test("injected kill failure");
        let token = actor.request_force_termination().expect("force token");

        let result = actor.force_terminate(&token);
        assert!(result.is_err());
        assert!(matches!(
            actor.state(),
            crate::pause::PauseState::ForceTerminationAvailable { .. }
        ));
        assert!(
            actor
                .ledger()
                .iter()
                .any(|event| event.kind.as_str() == "termination_failed")
        );
    }

    #[test]
    fn malformed_approval_event_fails_recovery_with_sequence() {
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let event = mission_domain::EventEnvelope::new(
            mission_domain::EventId::new(),
            mission_id,
            route_id,
            7,
            mission_domain::EventKind::ApprovalRequested,
            serde_json::json!({"approval": {"id": "broken"}}),
        );
        let error = MissionActor::try_new(mission_id, route_id, vec![event])
            .expect_err("malformed approval must fail closed");
        assert!(error.contains("sequence 7"));
    }
}
