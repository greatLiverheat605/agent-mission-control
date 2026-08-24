use adapter_codex::{CodexNormalizer, NativeParseError};

#[test]
fn normalizer_preserves_unknown_native_evidence_and_safe_pause() {
    let normalizer = CodexNormalizer::default();
    let event = normalizer
        .normalize_line(
            r#"{"type":"approval.requested","id":"r1","reason":"write file","extra":{"x":1}}"#,
        )
        .expect("parse fixture");
    assert!(event.event.requires_safe_pause);
    assert!(event.event.raw_evidence.as_ref().unwrap()["extra"]["x"] == 1);
    assert!(matches!(
        event.event.event_kind,
        mission_domain::EventKind::Unknown(_)
    ));
    assert!(matches!(
        normalizer.normalize_line("[]"),
        Err(NativeParseError::NotObject)
    ));
}
