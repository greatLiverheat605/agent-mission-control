use async_trait::async_trait;
use mission_domain::{EventId, EventKind};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use crate::adapter::{
    AdapterError, AgentAdapter, AgentEvent, AgentHandle, EventSink, StartAgentRequest,
};
use crate::capability::{AgentCapabilityReport, Capability, InstallState};

#[derive(Clone)]
pub struct FakeAdapter {
    events: Arc<Vec<AgentEvent>>,
    controls: Arc<Mutex<HashMap<String, Arc<Mutex<crate::adapter::ControlState>>>>>,
}

impl Default for FakeAdapter {
    fn default() -> Self {
        Self::new(vec![
            AgentEvent {
                event_id: EventId::new(),
                agent_run_id: None,
                event_kind: EventKind::AgentRunStarted,
                payload: json!({"phase":"started"}),
                requires_safe_pause: false,
                raw_evidence: None,
            },
            AgentEvent {
                event_id: EventId::new(),
                agent_run_id: None,
                event_kind: EventKind::AgentMessage,
                payload: json!({"phase":"analysis"}),
                requires_safe_pause: false,
                raw_evidence: None,
            },
            AgentEvent {
                event_id: EventId::new(),
                agent_run_id: None,
                event_kind: EventKind::Unknown("tool.request".to_owned()),
                payload: json!({"tool":"read_file"}),
                requires_safe_pause: true,
                raw_evidence: Some(json!({"type":"tool.request"})),
            },
            AgentEvent {
                event_id: EventId::new(),
                agent_run_id: None,
                event_kind: EventKind::EvidenceRecorded,
                payload: json!({"evidence_id":"fake-tool-result","kind":"test","status":"verified","source":"agent"}),
                requires_safe_pause: false,
                raw_evidence: None,
            },
            AgentEvent {
                event_id: EventId::new(),
                agent_run_id: None,
                event_kind: EventKind::AgentMessage,
                payload: json!({"usage":{"input":10,"output":5}}),
                requires_safe_pause: false,
                raw_evidence: None,
            },
        ])
    }
}

impl FakeAdapter {
    pub fn new(events: Vec<AgentEvent>) -> Self {
        Self {
            events: Arc::new(events),
            controls: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl AgentAdapter for FakeAdapter {
    async fn probe(&self) -> Result<AgentCapabilityReport, AdapterError> {
        Ok(AgentCapabilityReport {
            provider: crate::ProviderId::Codex,
            agent: "fake-codex".to_owned(),
            version: Some("fixture-1".to_owned()),
            install_state: InstallState::Installed,
            capability: Capability {
                structured_events: true,
                resume: true,
                approval: true,
                safe_pause: true,
                terminal_fallback: true,
            },
            unavailable_reason: None,
            executable_hash: Some("fixture".to_owned()),
            configuration_source: Some("test".to_owned()),
        })
    }

    async fn start(
        &self,
        _request: StartAgentRequest,
        sink: EventSink,
    ) -> Result<AgentHandle, AdapterError> {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let control = Arc::new(Mutex::new(Default::default()));
        let run_id = uuid::Uuid::now_v7().to_string();
        self.controls
            .lock()
            .await
            .insert(run_id.clone(), control.clone());
        let events = self.events.clone();
        tokio::spawn(async move {
            for event in events.iter().cloned() {
                if sink.send(event.clone()).is_err() {
                    break;
                }
                let _ = events_tx.send(event);
            }
        });
        Ok(AgentHandle::new(run_id, events_rx, control))
    }

    async fn request_safe_pause(&self, run_id: &str) -> Result<(), AdapterError> {
        let control = self
            .controls
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| AdapterError::Unavailable("unknown run".to_owned()))?;
        let mut state = control.lock().await;
        if state.terminated {
            return Err(AdapterError::AlreadyTerminated);
        }
        if state.paused {
            return Err(AdapterError::AlreadyPaused);
        }
        state.paused = true;
        Ok(())
    }

    async fn terminate_owned_tree(&self, run_id: &str) -> Result<(), AdapterError> {
        let control = self
            .controls
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| AdapterError::Unavailable("unknown run".to_owned()))?;
        let mut state = control.lock().await;
        if state.terminated {
            return Err(AdapterError::AlreadyTerminated);
        }
        state.terminated = true;
        Ok(())
    }
}
