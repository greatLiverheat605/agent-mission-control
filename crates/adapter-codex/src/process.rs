use adapter_core::{
    AdapterError, AgentAdapter, AgentCapabilityReport, AgentControl, AgentEvent, AgentHandle,
    EventSink, StartAgentRequest,
};
use async_trait::async_trait;
use mission_domain::EventId;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use crate::app_server::spawn_app_server;

#[derive(Clone, Debug)]
pub struct CodexAdapterOptions {
    pub executable: PathBuf,
}

#[derive(Clone)]
pub struct CodexAdapter {
    options: CodexAdapterOptions,
    runs: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<AgentControl>>>>,
}

impl CodexAdapter {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            options: CodexAdapterOptions {
                executable: executable.into(),
            },
            runs: Arc::new(Mutex::new(HashMap::new())),
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
        let (control_tx, mut control_rx) = mpsc::unbounded_channel();
        let run_id = uuid::Uuid::now_v7().to_string();
        self.runs
            .lock()
            .await
            .insert(run_id.clone(), control_tx.clone());
        let event_run_id = run_id.clone();
        let executable = self.options.executable.clone();
        let runs = Arc::clone(&self.runs);
        tokio::spawn(async move {
            let mut client = match spawn_app_server(&executable, &request.route_workspace).await {
                Ok(client) => client,
                Err(error) => {
                    emit(
                        &sink,
                        &events_tx,
                        AgentEvent {
                            event_id: EventId::new(),
                            agent_run_id: Some(event_run_id.clone()),
                            event_kind: mission_domain::EventKind::Unknown(
                                "adapter.protocol_error".to_owned(),
                            ),
                            payload: json!({"error": error.to_string()}),
                            requires_safe_pause: true,
                            raw_evidence: Some(json!({"error": error.to_string()})),
                        },
                    );
                    runs.lock().await.remove(&event_run_id);
                    return;
                }
            };
            let route_workspace = request.route_workspace.clone();
            let handshake_timeout = std::time::Duration::from_secs(5);
            if let Err(error) = client
                .request(
                    "initialize",
                    json!({"readOnly": request.read_only, "cwd": route_workspace, "model": request.model}),
                    handshake_timeout,
                )
                .await
            {
                emit(&sink, &events_tx, AgentEvent {
                    event_id: EventId::new(),
                    agent_run_id: Some(event_run_id.clone()),
                    event_kind: mission_domain::EventKind::Unknown("adapter.protocol_error".to_owned()),
                    payload: json!({"method":"initialize", "error": error.to_string()}),
                    requires_safe_pause: true,
                    raw_evidence: Some(json!({"method":"initialize", "error": error.to_string()})),
                });
                let _ = client.shutdown().await;
                runs.lock().await.remove(&event_run_id);
                return;
            }
            if let Err(error) = client
                .request(
                    "thread/start",
                    json!({"cwd": request.route_workspace, "resumeToken": request.resume_token}),
                    handshake_timeout,
                )
                .await
            {
                emit(
                    &sink,
                    &events_tx,
                    AgentEvent {
                        event_id: EventId::new(),
                        agent_run_id: Some(event_run_id.clone()),
                        event_kind: mission_domain::EventKind::Unknown(
                            "adapter.protocol_error".to_owned(),
                        ),
                        payload: json!({"method":"thread/start", "error": error.to_string()}),
                        requires_safe_pause: true,
                        raw_evidence: Some(
                            json!({"method":"thread/start", "error": error.to_string()}),
                        ),
                    },
                );
                let _ = client.shutdown().await;
                runs.lock().await.remove(&event_run_id);
                return;
            }
            emit(
                &sink,
                &events_tx,
                AgentEvent {
                    event_id: EventId::new(),
                    agent_run_id: Some(event_run_id.clone()),
                    event_kind: mission_domain::EventKind::AgentRunStarted,
                    payload: json!({"transport":"app-server","run_id":event_run_id}),
                    requires_safe_pause: false,
                    raw_evidence: None,
                },
            );
            let normalizer = crate::normalize::CodexNormalizer::default();
            let mut paused = false;
            loop {
                tokio::select! {
                    control = control_rx.recv() => match control {
                        Some(AgentControl::SafePause { reason }) if !paused => {
                            paused = true;
                            let _ = client.notify("turn/interrupt", json!({"reason": reason})).await;
                            emit(&sink, &events_tx, AgentEvent { event_id: EventId::new(), agent_run_id: Some(event_run_id.clone()), event_kind: mission_domain::EventKind::PauseRequested, payload: json!({"reason":"safe pause requested"}), requires_safe_pause: false, raw_evidence: None });
                        }
                        Some(AgentControl::Terminate) => {
                            let _ = client.shutdown().await;
                            break;
                        }
                        Some(AgentControl::SafePause { .. }) | None => {}
                    },
                    line = client.next_line() => match line {
                        Ok(Some(line)) => {
                            let mut event = normalizer.normalize_line_lossless(&line).event;
                            event.agent_run_id = Some(event_run_id.clone());
                            emit(&sink, &events_tx, event);
                        }
                        Ok(None) => {
                            emit(&sink, &events_tx, AgentEvent {
                                event_id: EventId::new(),
                                agent_run_id: Some(event_run_id.clone()),
                                event_kind: mission_domain::EventKind::Unknown("adapter.stdout_eof".to_owned()),
                                payload: json!({"run_id": event_run_id}),
                                requires_safe_pause: true,
                                raw_evidence: Some(json!({"reason":"stdout_eof"})),
                            });
                            break;
                        }
                        Err(error) => {
                            emit(&sink, &events_tx, AgentEvent {
                                event_id: EventId::new(),
                                agent_run_id: Some(event_run_id.clone()),
                                event_kind: mission_domain::EventKind::Unknown("adapter.protocol_error".to_owned()),
                                payload: json!({"error": error.to_string(), "run_id": event_run_id}),
                                requires_safe_pause: true,
                                raw_evidence: Some(json!({"error": error.to_string()})),
                            });
                            break;
                        }
                    }
                }
            }
            let _ = client.shutdown().await;
            runs.lock().await.remove(&event_run_id);
        });
        Ok(AgentHandle::with_control(run_id, events_rx, control_tx))
    }

    async fn request_safe_pause(&self, run_id: &str) -> Result<(), AdapterError> {
        let sender = self
            .runs
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| AdapterError::Unavailable("unknown run".to_owned()))?;
        sender
            .send(AgentControl::SafePause {
                reason: "adapter request".to_owned(),
            })
            .map_err(|_| AdapterError::Unavailable("agent run ended".to_owned()))
    }

    async fn terminate_owned_tree(&self, run_id: &str) -> Result<(), AdapterError> {
        let sender = self
            .runs
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| AdapterError::Unavailable("unknown run".to_owned()))?;
        sender
            .send(AgentControl::Terminate)
            .map_err(|_| AdapterError::Unavailable("agent run ended".to_owned()))
    }
}

fn emit(sink: &EventSink, events: &mpsc::UnboundedSender<AgentEvent>, event: AgentEvent) {
    let _ = sink.send(event.clone());
    let _ = events.send(event);
}
