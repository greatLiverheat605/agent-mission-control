use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId};

use crate::pause::{PauseController, PauseError, PauseState};
use crate::process_tree::OwnedProcessTree;

pub trait ActorLedger {
    type Error: std::fmt::Display;
    fn append_event(&mut self, event: &EventEnvelope) -> Result<(), Self::Error>;
}

impl ActorLedger for Vec<EventEnvelope> {
    type Error = std::convert::Infallible;
    fn append_event(&mut self, event: &EventEnvelope) -> Result<(), Self::Error> {
        self.push(event.clone());
        Ok(())
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
    events: Vec<EventEnvelope>,
}

#[derive(Debug)]
pub enum ActorError {
    Ledger(String),
    Pause(PauseError),
}

impl<L: ActorLedger> MissionActor<L> {
    pub fn new(mission_id: MissionId, route_id: RouteId, ledger: L) -> Self {
        Self {
            mission_id,
            route_id,
            ledger,
            sequence: 0,
            pause: PauseController::default(),
            process_tree: OwnedProcessTree::new(),
            ui_connected: true,
            events: Vec::new(),
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
        self.events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .cloned()
            .collect()
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
        self.ledger
            .append_event(&event)
            .map_err(|error| ActorError::Ledger(error.to_string()))?;
        self.events.push(event);
        self.sequence = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MissionActor;
    use mission_domain::{MissionId, RouteId};

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
}
