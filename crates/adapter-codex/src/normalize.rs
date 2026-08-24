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
                    event_kind: EventKind::Unknown("adapter.protocol_warning".to_owned()),
                    payload: json!({"error": error.to_string(), "raw_line": line}),
                    requires_safe_pause: true,
                    raw_evidence: Some(json!({"raw_line": line})),
                },
            },
        }
    }
}

pub fn normalize_native(native: NativeEvent) -> NormalizedEvent {
    CodexNormalizer::default().normalize(native)
}

fn normalize_with_version(native: NativeEvent, adapter_version: &str) -> NormalizedEvent {
    let raw = native.raw_value();
    let event_type = native.event_type.clone();
    let requires_safe_pause = matches!(
        event_type.as_str(),
        "approval.requested" | "item.tool_call" | "tool.request" | "error"
    ) || event_type.starts_with("approval.");
    let (kind, payload) = match event_type.as_str() {
        "thread.started" | "turn.started" | "item.started" => (
            EventKind::AgentRunStarted,
            json!({"native_type": event_type, "data": raw}),
        ),
        "message" | "item.completed" | "turn.completed" | "thread.completed" => (
            EventKind::AgentMessage,
            json!({"native_type": event_type, "data": raw}),
        ),
        "file.diff" | "item.file_change" | "tool.result" => (
            EventKind::EvidenceRecorded,
            json!({"native_type": event_type, "data": raw}),
        ),
        _ => (
            EventKind::Unknown(event_type.clone()),
            json!({"native_type": event_type, "data": raw}),
        ),
    };
    NormalizedEvent {
        native_type: native.event_type,
        adapter_version: adapter_version.to_owned(),
        confidence: "observed".to_owned(),
        event: AgentEvent {
            event_id: EventId::new(),
            event_kind: kind,
            payload,
            requires_safe_pause,
            raw_evidence: Some(raw),
        },
    }
}
