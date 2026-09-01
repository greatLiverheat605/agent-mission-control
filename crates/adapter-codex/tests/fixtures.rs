use adapter_codex::{CodexNormalizer, NativeParseError};

#[test]
fn normalizer_preserves_unknown_native_evidence_and_safe_pause() {
    let normalizer = CodexNormalizer::default();
    let event = normalizer
        .normalize_line(
            r#"{"type":"item/commandExecution/requestApproval","id":"r1","reason":"write file","extra":{"x":1}}"#,
        )
        .expect("parse fixture");
    assert!(event.event.requires_safe_pause);
    assert!(event.event.raw_evidence.as_ref().unwrap()["extra"]["x"] == 1);
    assert!(matches!(
        event.event.event_kind,
        mission_domain::EventKind::ApprovalRequested
    ));
    assert!(matches!(
        normalizer.normalize_line("[]"),
        Err(NativeParseError::NotObject)
    ));
}

#[test]
fn high_risk_unknown_events_fail_closed_and_protocol_limits_are_bounded() {
    let normalizer = CodexNormalizer::default();
    let event = normalizer
        .normalize_line(r#"{"type":"tool/unknown","token":"sk-1234567890abcdefghijklmnop"}"#)
        .expect("parse high-risk event");
    assert!(event.event.requires_safe_pause);
    let serialized = serde_json::to_string(&event.event).expect("serialize event");
    assert!(!serialized.contains("sk-1234567890abcdefghijklmnop"));

    let too_deep = format!("{}0{}", "[".repeat(70), "]".repeat(70));
    assert!(matches!(
        adapter_codex::parse_native_line(&too_deep),
        Err(NativeParseError::LimitExceeded)
    ));
}

#[test]
fn malformed_lines_are_safe_pause_and_redacted() {
    let event = CodexNormalizer::default()
        .normalize_line_lossless("not-json sk-1234567890abcdefghijklmnop");
    assert!(event.event.requires_safe_pause);
    let serialized = serde_json::to_string(&event.event).expect("serialize warning");
    assert!(!serialized.contains("sk-1234567890abcdefghijklmnop"));
}

#[test]
fn completion_and_abort_events_are_terminal_run_facts() {
    let normalizer = CodexNormalizer::default();
    let completed = normalizer
        .normalize_line(r#"{"type":"turn/completed","id":"turn-1"}"#)
        .expect("completion event");
    assert_eq!(
        completed.event.event_kind,
        mission_domain::EventKind::AgentRunCompleted
    );
    assert!(!completed.event.requires_safe_pause);

    let aborted = normalizer
        .normalize_line(r#"{"method":"error","params":{"message":"fixture failure"}}"#)
        .expect("abort event");
    assert_eq!(
        aborted.event.event_kind,
        mission_domain::EventKind::AgentRunAborted
    );
    assert!(aborted.event.requires_safe_pause);
}

#[test]
fn interrupted_turn_completion_preserves_status_for_recovery() {
    let event = CodexNormalizer::default()
        .normalize_line(
            r#"{"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}}"#,
        )
        .expect("interrupted completion event");
    assert_eq!(
        event.event.event_kind,
        mission_domain::EventKind::AgentRunCompleted
    );
    assert_eq!(event.event.payload["status"], "interrupted");
}

#[test]
fn turn_notifications_promote_nested_turn_id_for_interrupt_tracking() {
    let event = CodexNormalizer::default()
        .normalize_line(
            r#"{"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"inProgress"}}}"#,
        )
        .expect("turn started event");
    assert_eq!(event.event.payload["threadId"], "thread-1");
    assert_eq!(event.event.payload["turnId"], "turn-1");
}

#[test]
fn legacy_approval_requests_use_the_same_safe_pause_mapping() {
    let normalizer = CodexNormalizer::default();
    for method in ["execCommandApproval", "applyPatchApproval"] {
        let line = format!(r#"{{"method":"{method}","params":{{}}}}"#);
        let event = normalizer.normalize_line(&line).expect("approval request");
        assert!(event.event.requires_safe_pause);
        assert_eq!(
            event.event.event_kind,
            mission_domain::EventKind::ApprovalRequested
        );
    }
}

#[test]
fn redaction_replaces_every_inline_credential() {
    let event = CodexNormalizer::default()
        .normalize_line(
            r#"{"type":"item/completed","message":"Bearer first Bearer second Basic third Basic fourth"}"#,
        )
        .expect("parse event");
    assert_eq!(
        event.event.payload["data"]["message"],
        "[REDACTED:bearer:5431204c6713] [REDACTED:bearer:afa7c16c1d91] [REDACTED:basic:0c3c2b6f3278] [REDACTED:basic:286cd59111dc]"
    );
}

#[test]
fn redaction_normalizes_camel_case_and_plural_sensitive_keys() {
    let event = CodexNormalizer::default()
        .normalize_line(
            r#"{"type":"item/completed","accessToken":"access-secret","privateKey":"private-secret","credentials":"credential-secret","apiKeys":["key-secret"],"APIKeys":["acronym-secret"]}"#,
        )
        .expect("parse event");
    let data = &event.event.payload["data"];
    for key in [
        "accessToken",
        "privateKey",
        "credentials",
        "apiKeys",
        "APIKeys",
    ] {
        assert!(
            data[key]
                .as_str()
                .is_some_and(|value| value.starts_with("[REDACTED:")),
            "key {key}"
        );
    }
}

#[test]
fn adapter_and_ledger_redactors_are_cross_layer_identical() {
    let payload = serde_json::json!({
        "accessToken": "access-secret",
        "privateKey": "private-secret",
        "credentials": "credential-secret",
        "message": "Bearer first Bearer second sk-ant-abcdefghijklmnop",
        "nested": [{"cookie": "session-secret"}],
    });
    let line = serde_json::json!({"type":"item/completed", "data": payload}).to_string();
    let normalized = CodexNormalizer::default()
        .normalize_line(&line)
        .expect("parse event");
    let expected = mission_ledger::Redactor::default()
        .redact_event(serde_json::json!({"type":"item/completed", "data": payload}))
        .expect("redact payload")
        .value;
    assert_eq!(normalized.event.payload["data"], expected);
}
