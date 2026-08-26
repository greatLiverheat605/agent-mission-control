use adapter_claude::{
    ClaudeNormalizer, ClaudeParseError, normalize_stream_line, parse_stream_line,
};
use mission_domain::EventKind;

#[test]
fn maps_system_assistant_tool_and_result_with_terminal_boundary() {
    let normalizer = ClaudeNormalizer::default();

    let started = normalizer.normalize_line(
        r#"{"type":"system","subtype":"init","session_id":"session-1","model":"claude-sonnet"}"#,
    ).expect("system event");
    assert!(matches!(
        started.event.event_kind,
        EventKind::AgentRunStarted
    ));
    assert!(!started.terminal);

    let message = normalizer.normalize_line(
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]},"session_id":"session-1"}"#,
    ).expect("assistant event");
    assert!(matches!(message.event.event_kind, EventKind::AgentMessage));
    assert!(!message.terminal);

    let tool = normalizer.normalize_line(
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-1","name":"Bash","input":{"command":"echo hi"}}]},"session_id":"session-1"}"#,
    ).expect("tool event");
    assert!(tool.event.requires_safe_pause);
    assert!(matches!(tool.event.event_kind, EventKind::Unknown(_)));
    assert!(!tool.terminal);

    let result = normalizer.normalize_line(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"session-1"}"#,
    ).expect("result event");
    assert!(matches!(
        result.event.event_kind,
        EventKind::EvidenceRecorded
    ));
    assert!(result.terminal);
}

#[test]
fn partial_and_usage_events_do_not_end_the_run_or_duplicate_costs() {
    let normalizer = ClaudeNormalizer::default();
    let partial = normalizer.normalize_line(
        r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}},"session_id":"session-1"}"#,
    ).expect("partial event");
    assert!(matches!(partial.event.event_kind, EventKind::AgentMessage));
    assert!(!partial.terminal);
    assert!(partial.usage.is_none());

    let usage = normalizer.normalize_line(
        r#"{"type":"message_delta","usage":{"input_tokens":12,"output_tokens":3},"session_id":"session-1"}"#,
    ).expect("usage event");
    assert!(matches!(usage.event.event_kind, EventKind::Unknown(_)));
    assert!(!usage.terminal);
    assert_eq!(usage.usage.unwrap().output_tokens, 3);
}

#[test]
fn malformed_and_unknown_high_risk_lines_fail_closed_with_raw_evidence() {
    let normalizer = ClaudeNormalizer::default();
    let unknown = normalizer
        .normalize_line(
            r#"{"type":"mystery.tool","tool_name":"delete_everything","session_id":"session-1"}"#,
        )
        .expect("unknown event");
    assert!(unknown.event.requires_safe_pause);
    assert!(matches!(unknown.event.event_kind, EventKind::Unknown(_)));
    assert!(unknown.event.raw_evidence.is_some());
    assert!(!unknown.terminal);

    let truncated =
        normalizer.normalize_line_lossless(r#"{"type":"assistant","message":{"role":"assistant"}"#);
    assert!(truncated.event.requires_safe_pause);
    assert!(!truncated.terminal);
    assert!(truncated.event.raw_evidence.is_some());

    assert!(matches!(
        parse_stream_line("[]"),
        Err(ClaudeParseError::NotObject)
    ));
    assert!(normalize_stream_line("not-json").event.requires_safe_pause);
}

#[test]
fn fixtures_are_jsonl_and_never_contain_real_secrets() {
    let fixture = include_str!("../../../fixtures/agents/claude/stream-json/v1/basic.jsonl");
    for line in fixture.lines() {
        let value: serde_json::Value = serde_json::from_str(line).expect("fixture JSONL");
        assert!(value.is_object());
        let serialized = value.to_string();
        assert!(!serialized.contains("sk-") && !serialized.contains("AKIA"));
    }
}
