use adapter_core::AgentEvent;
use mission_domain::{EventId, EventKind};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::native::{NativeEvent, NativeParseError, parse_native_line};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    pub native_type: String,
    pub adapter_version: String,
    pub confidence: String,
    pub event: AgentEvent,
}

#[derive(Clone, Debug)]
pub struct CodexNormalizer {
    pub adapter_version: String,
}

impl Default for CodexNormalizer {
    fn default() -> Self {
        Self {
            adapter_version: "codex-app-server-v1".to_owned(),
        }
    }
}

impl CodexNormalizer {
    pub fn normalize_line(&self, line: &str) -> Result<NormalizedEvent, NativeParseError> {
        let native = parse_native_line(line)?;
        Ok(self.normalize(native))
    }

    pub fn normalize(&self, native: NativeEvent) -> NormalizedEvent {
        normalize_with_version(native, &self.adapter_version)
    }

    pub fn normalize_line_lossless(&self, line: &str) -> NormalizedEvent {
        match self.normalize_line(line) {
            Ok(event) => event,
            Err(error) => NormalizedEvent {
                native_type: "adapter.protocol_warning".to_owned(),
                adapter_version: self.adapter_version.clone(),
                confidence: "observed".to_owned(),
                event: AgentEvent {
                    event_id: EventId::new(),
                    agent_run_id: None,
                    event_kind: EventKind::Unknown("adapter.protocol_warning".to_owned()),
                    payload: json!({"error": error.to_string(), "raw_line": redact_text(line)}),
                    requires_safe_pause: true,
                    raw_evidence: Some(json!({"raw_line": redact_text(line)})),
                },
            },
        }
    }
}

pub fn normalize_native(native: NativeEvent) -> NormalizedEvent {
    CodexNormalizer::default().normalize(native)
}

fn normalize_with_version(native: NativeEvent, adapter_version: &str) -> NormalizedEvent {
    let raw = redact_value(native.raw_value());
    let event_type = native.event_type.clone();
    let requires_safe_pause = matches!(
        event_type.as_str(),
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "item/tool/requestUserInput"
            | "mcpServer/elicitation/request"
            | "execCommandApproval"
            | "applyPatchApproval"
            | "error"
    ) || event_type.ends_with("/aborted")
        || event_type.ends_with("/failed")
        || (matches!(event_type.as_str(), value if value.starts_with("unknown/") || value.starts_with("tool/") || value.starts_with("command/") || value.starts_with("exec/") || value.starts_with("write/")));
    let mut payload = json!({"native_type": event_type, "data": raw.clone()});
    if let Some(object) = payload
        .get_mut("data")
        .and_then(|value| value.as_object().cloned())
    {
        for key in ["threadId", "turnId", "itemId", "tokenUsage", "delta"] {
            let nested_turn_id = if key == "turnId" {
                object
                    .get("turn")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|turn| turn.get("id"))
                    .cloned()
                    .or_else(|| {
                        object
                            .get("params")
                            .and_then(serde_json::Value::as_object)
                            .and_then(|params| params.get("turn"))
                            .and_then(serde_json::Value::as_object)
                            .and_then(|turn| turn.get("id"))
                            .cloned()
                    })
            } else {
                None
            };
            if let Some(value) = object
                .get(key)
                .cloned()
                .or_else(|| {
                    object
                        .get("params")
                        .and_then(serde_json::Value::as_object)
                        .and_then(|params| params.get(key))
                        .cloned()
                })
                .or(nested_turn_id)
            {
                payload[key] = value;
            }
        }
        if let Some(status) = object.get("status").cloned().or_else(|| {
            object
                .get("params")
                .and_then(serde_json::Value::as_object)
                .and_then(|params| params.get("turn"))
                .and_then(serde_json::Value::as_object)
                .and_then(|turn| turn.get("status"))
                .cloned()
        }) {
            payload["status"] = status;
        }
    }
    let (kind, payload) = match event_type.as_str() {
        "thread/started" => (EventKind::AgentRunStarted, payload),
        "turn/started" => (EventKind::AgentRunStarted, payload),
        "turn/completed" => (EventKind::AgentRunCompleted, payload),
        "error" => (EventKind::AgentRunAborted, payload),
        "item/started" | "item/completed" | "item/agentMessage/delta" => {
            (EventKind::AgentMessage, payload)
        }
        "turn/diff/updated" => (EventKind::EvidenceRecorded, payload),
        "thread/tokenUsage/updated" => (EventKind::AgentMessage, payload),
        "warning" => (EventKind::Unknown("warning".to_owned()), payload),
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval"
        | "item/tool/requestUserInput"
        | "mcpServer/elicitation/request"
        | "execCommandApproval"
        | "applyPatchApproval" => (EventKind::ApprovalRequested, payload),
        _ => (EventKind::Unknown(event_type.clone()), payload),
    };
    NormalizedEvent {
        native_type: native.event_type,
        adapter_version: adapter_version.to_owned(),
        confidence: "observed".to_owned(),
        event: AgentEvent {
            event_id: EventId::new(),
            agent_run_id: None,
            event_kind: kind,
            payload,
            requires_safe_pause,
            raw_evidence: Some(raw),
        },
    }
}

fn redact_value(value: serde_json::Value) -> serde_json::Value {
    mission_ledger::Redactor::default()
        .redact_event(value)
        .map(|result| result.value)
        .unwrap_or_else(|_| json!("[REDACTED:adapter-limit]"))
}

fn redact_text(text: &str) -> String {
    mission_ledger::Redactor::default()
        .redact_event(serde_json::Value::String(text.to_owned()))
        .ok()
        .and_then(|result| result.value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "[REDACTED:adapter-limit]".to_owned())
}
