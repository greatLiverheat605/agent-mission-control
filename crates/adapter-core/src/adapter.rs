use async_trait::async_trait;
use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};

use crate::capability::AgentCapabilityReport;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StartAgentRequest {
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub project_root: String,
    pub route_workspace: String,
    pub read_only: bool,
    pub approved_environment: Vec<(String, String)>,
    pub model: Option<String>,
    pub loadout_fingerprint: String,
    pub resume_token: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub event_id: EventId,
    pub event_kind: EventKind,
    pub payload: Value,
    pub requires_safe_pause: bool,
    pub raw_evidence: Option<Value>,
}

impl AgentEvent {
    pub fn into_envelope(
        self,
        mission_id: MissionId,
        route_id: RouteId,
        sequence: u64,
    ) -> EventEnvelope {
        let mut event = EventEnvelope::new(
            self.event_id,
            mission_id,
            route_id,
            sequence,
            self.event_kind,
            self.payload,
        );
        event.agent_run_id = None;
        event
    }
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("adapter unavailable: {0}")]
    Unavailable(String),
    #[error("adapter protocol error: {0}")]
    Protocol(String),
    #[error("adapter operation timed out")]
    Timeout,
    #[error("adapter is already paused")]
    AlreadyPaused,
    #[error("adapter is already terminated")]
    AlreadyTerminated,
    #[error("adapter operation is unsupported")]
    Unsupported,
}

pub type EventSink = mpsc::UnboundedSender<AgentEvent>;

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    async fn probe(&self) -> Result<AgentCapabilityReport, AdapterError>;
    async fn start(
        &self,
        request: StartAgentRequest,
        sink: EventSink,
    ) -> Result<AgentHandle, AdapterError>;

    async fn request_safe_pause(&self, _run_id: &str) -> Result<(), AdapterError> {
        Err(AdapterError::Unsupported)
    }

    async fn terminate_owned_tree(&self, _run_id: &str) -> Result<(), AdapterError> {
        Err(AdapterError::Unsupported)
    }
}

#[derive(Clone)]
pub struct AgentHandle {
    run_id: String,
    control: Arc<Mutex<ControlState>>,
    events: Arc<Mutex<Option<mpsc::UnboundedReceiver<AgentEvent>>>>,
}

#[derive(Default)]
pub(crate) struct ControlState {
    pub(crate) paused: bool,
    pub(crate) terminated: bool,
}

impl AgentHandle {
    pub(crate) fn new(
        run_id: String,
        events: mpsc::UnboundedReceiver<AgentEvent>,
        control: Arc<Mutex<ControlState>>,
    ) -> Self {
        Self {
            run_id,
            control,
            events: Arc::new(Mutex::new(Some(events))),
        }
    }

    pub fn detached(
        run_id: impl Into<String>,
        events: mpsc::UnboundedReceiver<AgentEvent>,
    ) -> Self {
        Self::new(
            run_id.into(),
            events,
            Arc::new(Mutex::new(ControlState::default())),
        )
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub async fn next_event(&self) -> Option<AgentEvent> {
        let mut receiver = self.events.lock().await;
        receiver.as_mut()?.recv().await
    }

    pub async fn request_safe_pause(&self) -> Result<(), AdapterError> {
        let mut state = self.control.lock().await;
        if state.terminated {
            return Err(AdapterError::AlreadyTerminated);
        }
        if state.paused {
            return Err(AdapterError::AlreadyPaused);
        }
        state.paused = true;
        Ok(())
    }

    pub async fn terminate_owned_tree(&self) -> Result<(), AdapterError> {
        let mut state = self.control.lock().await;
        if state.terminated {
            return Err(AdapterError::AlreadyTerminated);
        }
        state.terminated = true;
        Ok(())
    }
}
