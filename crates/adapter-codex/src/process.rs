use adapter_core::{
    AdapterError, AgentAdapter, AgentCapabilityReport, AgentEvent, AgentHandle, EventSink,
    StartAgentRequest,
};
use async_trait::async_trait;
use mission_domain::EventId;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::app_server::spawn_app_server;

#[derive(Clone, Debug)]
pub struct CodexAdapterOptions {
    pub executable: PathBuf,
}

#[derive(Clone)]
pub struct CodexAdapter {
    options: CodexAdapterOptions,
    _marker: Arc<()>,
}

impl CodexAdapter {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            options: CodexAdapterOptions {
                executable: executable.into(),
            },
            _marker: Arc::new(()),
        }
    }
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    async fn probe(&self) -> Result<AgentCapabilityReport, AdapterError> {
        Ok(AgentCapabilityReport {
            agent: "codex".to_owned(),
            version: None,
            install_state: adapter_core::InstallState::Installed,
            capability: adapter_core::Capability {
                structured_events: true,
                resume: true,
                approval: true,
                safe_pause: true,
                terminal_fallback: true,
            },
            executable_hash: None,
            configuration_source: Some("codex-cli".to_owned()),
        })
    }

    async fn start(
        &self,
        request: StartAgentRequest,
        sink: EventSink,
    ) -> Result<AgentHandle, AdapterError> {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let run_id = uuid::Uuid::now_v7().to_string();
        let event_run_id = run_id.clone();
        let executable = self.options.executable.clone();
        tokio::spawn(async move {
            let normalized = match spawn_app_server(&executable, &request.route_workspace).await {
                Ok(_) => AgentEvent {
                    event_id: EventId::new(),
                    event_kind: mission_domain::EventKind::AgentRunStarted,
                    payload: json!({"transport":"app-server","run_id":event_run_id}),
                    requires_safe_pause: false,
                    raw_evidence: None,
                },
                Err(error) => AgentEvent {
                    event_id: EventId::new(),
                    event_kind: mission_domain::EventKind::Unknown(
                        "adapter.protocol_error".to_owned(),
                    ),
                    payload: json!({"error":error.to_string()}),
                    requires_safe_pause: true,
                    raw_evidence: None,
                },
            };
            let _ = sink.send(normalized.clone());
            let _ = events_tx.send(normalized);
        });
        Ok(AgentHandle::detached(run_id, events_rx))
    }
}
