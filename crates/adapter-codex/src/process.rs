use adapter_core::{
    AdapterError, AgentAdapter, AgentCapabilityReport, AgentControl, AgentEvent, AgentHandle,
    Capability, EventSink, InstallState, StartAgentRequest,
};
use adapter_detect::{
    ProbeError, ProbeOptions, VersionProbe, probe_executable, resolve_executable,
};
use async_trait::async_trait;
use mission_domain::EventId;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use crate::app_server::spawn_app_server;
use crate::exec_probe::run_exec_probe;

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

const VERSION_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const EXEC_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn unavailable_report(
    install_state: InstallState,
    reason: impl Into<String>,
    version: Option<String>,
    executable_hash: Option<String>,
    configuration_source: Option<String>,
) -> AgentCapabilityReport {
    AgentCapabilityReport {
        provider: adapter_core::ProviderId::Codex,
        agent: "codex".to_owned(),
        version,
        install_state,
        capability: Capability {
            structured_events: false,
            resume: false,
            approval: false,
            safe_pause: false,
            terminal_fallback: false,
        },
        unavailable_reason: Some(reason.into()),
        executable_hash,
        configuration_source,
    }
}

fn resolve_probe_path(executable: &Path) -> Option<PathBuf> {
    if executable.is_absolute() || executable.components().count() > 1 {
        return executable.is_file().then(|| executable.to_owned());
    }
    executable
        .to_str()
        .and_then(|name| resolve_executable(name, None))
}

fn probe_failure_report(
    probe: &VersionProbe,
    install_state: InstallState,
    reason: &'static str,
) -> AgentCapabilityReport {
    unavailable_report(
        install_state,
        reason,
        probe.version.clone(),
        Some(probe.executable_hash.clone()),
        Some("codex-cli".to_owned()),
    )
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    async fn probe(&self) -> Result<AgentCapabilityReport, AdapterError> {
        let configured_executable = self.options.executable.clone();
        let Some(executable) = resolve_probe_path(&configured_executable) else {
            return Ok(unavailable_report(
                InstallState::Missing,
                "codex executable not found",
                None,
                None,
                None,
            ));
        };

        let version_path = executable.clone();
        let version_probe = tokio::task::spawn_blocking(move || {
            probe_executable(
                version_path,
                &ProbeOptions {
                    timeout: VERSION_PROBE_TIMEOUT,
                },
            )
        })
        .await;
        let version_probe = match version_probe {
            Ok(Ok(probe)) => probe,
            Ok(Err(ProbeError::Missing)) => {
                return Ok(unavailable_report(
                    InstallState::Missing,
                    "codex executable disappeared during probe",
                    None,
                    None,
                    Some("codex-cli".to_owned()),
                ));
            }
            Ok(Err(ProbeError::Start(_))) => {
                return Ok(unavailable_report(
                    InstallState::DetectedNotRunnable,
                    "codex version probe could not start",
                    None,
                    None,
                    Some("codex-cli".to_owned()),
                ));
            }
            Ok(Err(ProbeError::Metadata(_))) => {
                return Ok(unavailable_report(
                    InstallState::DetectedNotRunnable,
                    "codex executable metadata unavailable",
                    None,
                    None,
                    Some("codex-cli".to_owned()),
                ));
            }
            Err(_) => {
                return Ok(unavailable_report(
                    InstallState::DetectedNotRunnable,
                    "codex version probe worker failed",
                    None,
                    None,
                    Some("codex-cli".to_owned()),
                ));
            }
        };
        if version_probe.timed_out {
            return Ok(probe_failure_report(
                &version_probe,
                InstallState::Unknown,
                "codex version probe timed out",
            ));
        }
        if version_probe.version.is_none() {
            return Ok(probe_failure_report(
                &version_probe,
                InstallState::Unknown,
                "codex version output is unknown",
            ));
        }

        let project_root = match std::env::current_dir() {
            Ok(path) => path,
            Err(_) => {
                return Ok(probe_failure_report(
                    &version_probe,
                    InstallState::DetectedNotRunnable,
                    "codex probe workspace is unavailable",
                ));
            }
        };
        let exec_probe = run_exec_probe(
            &executable,
            &project_root,
            "mission-control capability probe",
            EXEC_PROBE_TIMEOUT,
        )
        .await;
        let exec_probe = match exec_probe {
            Ok(result) => result,
            Err(crate::exec_probe::ExecProbeError::Timeout) => {
                return Ok(probe_failure_report(
                    &version_probe,
                    InstallState::DetectedNotRunnable,
                    "codex exec probe timed out",
                ));
            }
            Err(crate::exec_probe::ExecProbeError::Start(_)) => {
                return Ok(probe_failure_report(
                    &version_probe,
                    InstallState::DetectedNotRunnable,
                    "codex exec probe could not start",
                ));
            }
        };
        if exec_probe.exit_code != Some(0) {
            return Ok(probe_failure_report(
                &version_probe,
                InstallState::DetectedNotRunnable,
                "codex exec probe exited unsuccessfully",
            ));
        }
        if exec_probe.events.is_empty() {
            return Ok(probe_failure_report(
                &version_probe,
                InstallState::Unknown,
                "codex exec probe returned no structured events",
            ));
        }

        Ok(AgentCapabilityReport {
            provider: adapter_core::ProviderId::Codex,
            agent: "codex".to_owned(),
            version: version_probe.version,
            install_state: InstallState::Installed,
            capability: Capability {
                structured_events: true,
                resume: true,
                approval: true,
                safe_pause: true,
                terminal_fallback: true,
            },
            unavailable_reason: None,
            executable_hash: Some(version_probe.executable_hash),
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
        let route_workspace = request.route_workspace.clone();
        let goal = request.goal.clone().unwrap_or_default();
        let model = request.model.clone();
        let resume_thread_id = request.resume_thread_id.clone();
        let loadout_fingerprint = request.loadout_fingerprint.clone();
        let contract_version = request.contract_version;
        let read_only = request.read_only;
        let control_tx_for_timeouts = control_tx.clone();
        tokio::spawn(async move {
            let mut client = match spawn_app_server(&executable, &route_workspace).await {
                Ok(client) => client,
                Err(error) => {
                    emit_protocol_error(&sink, &events_tx, &event_run_id, &error.to_string());
                    runs.lock().await.remove(&event_run_id);
                    return;
                }
            };
            let handshake_timeout = std::time::Duration::from_secs(5);
            if let Err(error) = client
                .request(
                    "initialize",
                    json!({"clientInfo":{"name":"agent-mission-control","title":"Agent Mission Control","version":"0.1.0"},"capabilities":{}}),
                    handshake_timeout,
                )
                .await
            {
                emit_protocol_error(&sink, &events_tx, &event_run_id, &format!("initialize: {error}"));
                let _ = client.shutdown().await;
                runs.lock().await.remove(&event_run_id);
                return;
            }
            let thread_response = if let Some(thread_id) = resume_thread_id.clone() {
                client
                    .request(
                        "thread/resume",
                        json!({"threadId":thread_id,"cwd":route_workspace,"model":model.clone()}),
                        handshake_timeout,
                    )
                    .await
            } else {
                client.request("thread/start", json!({"cwd":route_workspace,"model":model.clone(),"sandbox":if read_only {"read-only"} else {"workspace-write"}}), handshake_timeout).await
            };
            let thread_response = match thread_response {
                Ok(response) => response,
                Err(error) => {
                    let method = if resume_thread_id.is_some() {
                        "thread/resume"
                    } else {
                        "thread/start"
                    };
                    emit_protocol_error(
                        &sink,
                        &events_tx,
                        &event_run_id,
                        &format!("{method}: {error}"),
                    );
                    let _ = client.shutdown().await;
                    runs.lock().await.remove(&event_run_id);
                    return;
                }
            };
            let mut thread_id = thread_response
                .result
                .as_ref()
                .and_then(|result| result.get("thread"))
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if thread_id.is_none() {
                let mut deferred_lines = Vec::new();
                loop {
                    let line =
                        match tokio::time::timeout(handshake_timeout, client.next_line()).await {
                            Ok(Ok(Some(line))) => line,
                            Ok(Ok(None)) => break,
                            Ok(Err(error)) => {
                                emit_protocol_error(
                                    &sink,
                                    &events_tx,
                                    &event_run_id,
                                    &format!("thread/started: {error}"),
                                );
                                break;
                            }
                            Err(_) => break,
                        };
                    let value: Value = match serde_json::from_str(&line) {
                        Ok(value) => value,
                        Err(_) => {
                            deferred_lines.push(line);
                            continue;
                        }
                    };
                    if value.get("method").and_then(Value::as_str) == Some("thread/started") {
                        thread_id = value
                            .get("params")
                            .and_then(Value::as_object)
                            .and_then(|params| params.get("thread"))
                            .and_then(Value::as_object)
                            .and_then(|thread| thread.get("id"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                        if thread_id.is_some() {
                            break;
                        }
                    }
                    deferred_lines.push(line);
                }
                for line in deferred_lines.into_iter().rev() {
                    client.requeue_line(line);
                }
            }
            let Some(thread_id) = thread_id else {
                emit_protocol_error(
                    &sink,
                    &events_tx,
                    &event_run_id,
                    "thread response missing thread.id",
                );
                let _ = client.shutdown().await;
                runs.lock().await.remove(&event_run_id);
                return;
            };
            let turn_response = match client
                .request(
                    "turn/start",
                    json!({"threadId":thread_id,"input":[{"type":"text","text":goal}]}),
                    handshake_timeout,
                )
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    emit_protocol_error(
                        &sink,
                        &events_tx,
                        &event_run_id,
                        &format!("turn/start: {error}"),
                    );
                    let _ = client.shutdown().await;
                    runs.lock().await.remove(&event_run_id);
                    return;
                }
            };
            let mut turn_id = turn_response
                .result
                .as_ref()
                .and_then(|result| result.get("turn"))
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            emit(
                &sink,
                &events_tx,
                AgentEvent {
                    event_id: EventId::new(),
                    agent_run_id: Some(event_run_id.clone()),
                    event_kind: mission_domain::EventKind::AgentRunStarted,
                    payload: json!({"transport":"app-server","run_id":event_run_id,"thread_id":thread_id,"turn_id":turn_id}),
                    requires_safe_pause: false,
                    raw_evidence: None,
                },
            );
            let normalizer = crate::normalize::CodexNormalizer::default();
            let mut paused = false;
            let mut pending_requests = HashSet::new();
            // Periodic tick prevents an idle provider read from starving control messages.
            let mut control_poll = tokio::time::interval(std::time::Duration::from_millis(25));
            loop {
                tokio::select! {
                    biased;
                    control = control_rx.recv() => match control {
                        Some(AgentControl::SafePause { reason }) if !paused => {
                            let Some(active_turn_id) = turn_id.clone() else { emit_protocol_error(&sink, &events_tx, &event_run_id, "turn/interrupt requested before turn/started"); continue; };
                            match client.request("turn/interrupt", json!({"threadId":thread_id,"turnId":active_turn_id}), handshake_timeout).await {
                                Ok(_) => { paused = true; emit(&sink, &events_tx, AgentEvent { event_id: EventId::new(), agent_run_id: Some(event_run_id.clone()), event_kind: mission_domain::EventKind::PauseRequested, payload: json!({"reason":reason,"thread_id":thread_id,"turn_id":turn_id}), requires_safe_pause: false, raw_evidence: None }); }
                                Err(error) => emit_protocol_error(&sink, &events_tx, &event_run_id, &format!("turn/interrupt: {error}")),
                            }
                        }
                        Some(AgentControl::RespondToServerRequest { request_id, decision }) => {
                            let key = rpc_id_key(&request_id);
                            if pending_requests.remove(&key) && client.respond(request_id, json!({"decision":decision})).await.is_err() { emit_protocol_error(&sink, &events_tx, &event_run_id, "approval response failed"); }
                        }
                        Some(AgentControl::Terminate) => { let _ = client.shutdown().await; break; }
                        Some(AgentControl::SafePause { .. }) | None => {}
                    },
                    _ = control_poll.tick() => {},
                    line = client.next_line() => match line {
                        Ok(Some(line)) => {
                            let value: Value = match serde_json::from_str(&line) { Ok(value) => value, Err(error) => { emit_protocol_error(&sink, &events_tx, &event_run_id, &format!("invalid inbound message: {error}")); continue; } };
                            if let (Some(request_id), Some(method)) = (value.get("id").cloned(), value.get("method").and_then(Value::as_str))
                                && is_approval_method(method) {
                                    let key = rpc_id_key(&request_id);
                                    pending_requests.insert(key.clone());
                                    let timeout_tx = control_tx_for_timeouts.clone();
                                    let timeout_id = request_id.clone();
                                    let timeout_sink = sink.clone();
                                    let timeout_events = events_tx.clone();
                                    let timeout_run_id = event_run_id.clone();
                                    tokio::spawn(async move {
                                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                                        if timeout_tx
                                            .send(AgentControl::RespondToServerRequest {
                                                request_id: timeout_id,
                                                decision: "decline".to_owned(),
                                            })
                                            .is_err()
                                        {
                                            emit_protocol_error(
                                                &timeout_sink,
                                                &timeout_events,
                                                &timeout_run_id,
                                                "approval timeout response dispatch failed",
                                            );
                                        }
                                    });
                                    emit(&sink, &events_tx, AgentEvent { event_id: EventId::new(), agent_run_id: Some(event_run_id.clone()), event_kind: mission_domain::EventKind::ApprovalRequested, payload: json!({"server_request_id":request_id,"approval_id":key,"method":method,"params":value.get("params").cloned().unwrap_or(Value::Null),"action_digest":format!("protocol:{method}:{key}"),"action_class":action_class_for_method(method),"contract_version":contract_version,"loadout_fingerprint":loadout_fingerprint}), requires_safe_pause: true, raw_evidence: Some(value) });
                                    continue;
                                }
                            let mut event = normalizer.normalize_line_lossless(&line).event;
                            if let Some(value) = event.payload.get("turnId").and_then(Value::as_str) { turn_id = Some(value.to_owned()); }
                            event.agent_run_id = Some(event_run_id.clone());
                            emit(&sink, &events_tx, event);
                        }
                        Ok(None) => { emit(&sink, &events_tx, AgentEvent { event_id: EventId::new(), agent_run_id: Some(event_run_id.clone()), event_kind: mission_domain::EventKind::Unknown("adapter.stdout_eof".to_owned()), payload: json!({"run_id":event_run_id}), requires_safe_pause: true, raw_evidence: Some(json!({"reason":"stdout_eof"})) }); break; }
                        Err(error) => { emit_protocol_error(&sink, &events_tx, &event_run_id, &error.to_string()); break; }
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

    async fn respond_to_server_request(
        &self,
        run_id: &str,
        request_id: Value,
        decision: &str,
    ) -> Result<(), AdapterError> {
        let sender = self
            .runs
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| AdapterError::Unavailable("unknown run".to_owned()))?;
        sender
            .send(AgentControl::RespondToServerRequest {
                request_id,
                decision: decision.to_owned(),
            })
            .map_err(|_| AdapterError::Unavailable("agent run ended".to_owned()))
    }
}

fn emit(sink: &EventSink, events: &mpsc::UnboundedSender<AgentEvent>, event: AgentEvent) {
    let _ = sink.send(event.clone());
    let _ = events.send(event);
}

fn rpc_id_key(id: &Value) -> String {
    id.as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| id.to_string())
}

fn is_approval_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "item/tool/requestUserInput"
            | "mcpServer/elicitation/request"
            | "execCommandApproval"
            | "applyPatchApproval"
    )
}

fn action_class_for_method(method: &str) -> &'static str {
    match method {
        "item/commandExecution/requestApproval" | "execCommandApproval" => "exec",
        "item/fileChange/requestApproval" | "applyPatchApproval" => "write",
        "item/permissions/requestApproval" => "permission",
        "item/tool/requestUserInput" | "mcpServer/elicitation/request" | "requestUserInput" => {
            "input"
        }
        _ => "write",
    }
}

fn emit_protocol_error(
    sink: &EventSink,
    events: &mpsc::UnboundedSender<AgentEvent>,
    run_id: &str,
    error: &str,
) {
    emit(
        sink,
        events,
        AgentEvent {
            event_id: EventId::new(),
            agent_run_id: Some(run_id.to_owned()),
            event_kind: mission_domain::EventKind::Unknown("adapter.protocol_error".to_owned()),
            payload: json!({"error": error}),
            requires_safe_pause: true,
            raw_evidence: Some(json!({"error": error})),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::action_class_for_method;

    #[test]
    fn approval_action_class_follows_protocol_method() {
        assert_eq!(
            action_class_for_method("item/commandExecution/requestApproval"),
            "exec"
        );
        assert_eq!(
            action_class_for_method("item/fileChange/requestApproval"),
            "write"
        );
        assert_eq!(
            action_class_for_method("item/permissions/requestApproval"),
            "permission"
        );
        assert_eq!(
            action_class_for_method("item/tool/requestUserInput"),
            "input"
        );
    }
}
