use adapter_core::AgentEvent;
use mission_domain::{EventId, EventKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::native::{ClaudeNativeEvent, ClaudeParseError, parse_stream_line};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaudeUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaudeNormalizedEvent {
    pub native_type: String,
    pub adapter_version: String,
    pub confidence: String,
    pub event: AgentEvent,
    pub terminal: bool,
    pub usage: Option<ClaudeUsage>,
}

#[derive(Clone, Debug)]
pub struct ClaudeNormalizer {
    pub adapter_version: String,
}

impl Default for ClaudeNormalizer {
    fn default() -> Self {
        Self {
            adapter_version: "claude-stream-json-v1".to_owned(),
        }
    }
}

impl ClaudeNormalizer {
    pub fn normalize_line(&self, line: &str) -> Result<ClaudeNormalizedEvent, ClaudeParseError> {
        let native = parse_stream_line(line)?;
        Ok(self.normalize(native))
    }

    pub fn normalize_line_lossless(&self, line: &str) -> ClaudeNormalizedEvent {
        match self.normalize_line(line) {
            Ok(event) => event,
            Err(error) => ClaudeNormalizedEvent {
                native_type: "adapter.protocol_warning".to_owned(),
                adapter_version: self.adapter_version.clone(),
                confidence: "observed".to_owned(),
                event: AgentEvent {
                    event_id: EventId::new(),
                    agent_run_id: None,
                    event_kind: EventKind::Unknown("adapter.protocol_warning".to_owned()),
                    payload: json!({"error": error.to_string(), "raw_line": line}),
                    requires_safe_pause: true,
                    raw_evidence: Some(json!({"raw_line": line})),
                },
                terminal: false,
                usage: None,
            },
        }
    }

    pub fn normalize(&self, native: ClaudeNativeEvent) -> ClaudeNormalizedEvent {
        normalize_with_version(native, &self.adapter_version)
    }
}

pub fn normalize_stream_line(line: &str) -> ClaudeNormalizedEvent {
    ClaudeNormalizer::default().normalize_line_lossless(line)
}

fn normalize_with_version(
    native: ClaudeNativeEvent,
    adapter_version: &str,
) -> ClaudeNormalizedEvent {
    let event_type = native.event_type.clone();
    let raw = native.raw;
    let (kind, payload, terminal, requires_safe_pause) = match event_type.as_str() {
        "system" => {
            let subtype = raw
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if subtype == "init" {
                (
                    EventKind::AgentRunStarted,
                    json!({"native_type": event_type, "data": raw}),
                    false,
                    false,
                )
            } else {
                (
                    EventKind::Unknown(format!("claude.system.{subtype}")),
                    json!({"native_type": event_type, "data": raw}),
                    false,
                    false,
                )
            }
        }
        "assistant" => normalize_assistant(&event_type, &raw),
        "user" => normalize_user(&event_type, &raw),
        "stream_event" => normalize_stream_event(&event_type, &raw),
        "message_delta" | "content_block_delta" | "message_start" | "message_stop" => (
            EventKind::Unknown(format!("claude.{event_type}")),
            json!({"native_type": event_type, "data": raw}),
            false,
            false,
        ),
        "result" => (
            EventKind::EvidenceRecorded,
            json!({"native_type": event_type, "data": raw}),
            true,
            false,
        ),
        "error" => (
            EventKind::Unknown("claude.error".to_owned()),
            json!({"native_type": event_type, "data": raw}),
            false,
            true,
        ),
        _ => {
            let requires_safe_pause = is_high_risk_type(&event_type, &raw);
            (
                EventKind::Unknown(event_type.clone()),
                json!({"native_type": event_type, "data": raw}),
                false,
                requires_safe_pause,
            )
        }
    };
    let usage = extract_usage(&raw);
    ClaudeNormalizedEvent {
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
        terminal,
        usage,
    }
}

fn normalize_assistant(event_type: &str, raw: &Value) -> (EventKind, Value, bool, bool) {
    let content = raw
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array);
    let has_tool_use = content.is_some_and(|items| {
        items
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
    });
    if has_tool_use {
        (
            EventKind::Unknown("tool.request".to_owned()),
            json!({"native_type": event_type, "data": raw}),
            false,
            true,
        )
    } else {
        (
            EventKind::AgentMessage,
            json!({"native_type": event_type, "data": raw}),
            false,
            false,
        )
    }
}

fn normalize_user(event_type: &str, raw: &Value) -> (EventKind, Value, bool, bool) {
    let content = raw
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array);
    let has_tool_result = content.is_some_and(|items| {
        items
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"))
    });
    (
        if has_tool_result {
            EventKind::EvidenceRecorded
        } else {
            EventKind::AgentMessage
        },
        json!({"native_type": event_type, "data": raw}),
        false,
        false,
    )
}

fn normalize_stream_event(event_type: &str, raw: &Value) -> (EventKind, Value, bool, bool) {
    let nested_type = raw
        .get("event")
        .and_then(|event| event.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let requires_safe_pause = nested_type.contains("tool") || nested_type.contains("permission");
    (
        if requires_safe_pause {
            EventKind::Unknown("tool.request".to_owned())
        } else {
            EventKind::AgentMessage
        },
        json!({"native_type": event_type, "nested_type": nested_type, "data": raw}),
        false,
        requires_safe_pause,
    )
}

fn is_high_risk_type(event_type: &str, raw: &Value) -> bool {
    let event_type = event_type.to_ascii_lowercase();
    event_type.contains("tool")
        || event_type.contains("permission")
        || event_type.contains("approval")
        || event_type.contains("error")
        || raw.get("tool_name").is_some()
        || raw.get("tool_use_id").is_some()
}

fn extract_usage(raw: &Value) -> Option<ClaudeUsage> {
    let usage = raw.get("usage")?;
    Some(ClaudeUsage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_read_input_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        cache_creation_input_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    })
}
