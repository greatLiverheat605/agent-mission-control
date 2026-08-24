use mission_ledger::{RedactionError, Redactor};
use serde_json::json;

#[test]
fn corpus_secrets_are_redacted_in_nested_values_and_context_survives() {
    let input = json!({
        "message": "call https://user:password@example.test with Bearer eyJhbGciOiJIUzI1NiJ9.example.signature",
        "nested": [{"api_key": "sk-1234567890abcdefghijklmnop"}],
        "stderr": "ghp_123456789012345678901234567890123456 and AKIAIOSFODNN7EXAMPLE",
        "pem": "-----BEGIN PRIVATE KEY-----\nsecret-key-material\n-----END PRIVATE KEY-----",
        "tail": "keep this diagnostic context"
    });
    let result = Redactor::default()
        .redact_event(input)
        .expect("redact corpus");
    let serialized = serde_json::to_string(&result.value).expect("serialize redacted value");
    assert!(!serialized.contains("sk-1234567890abcdefghijklmnop"));
    assert!(!serialized.contains("Bearer eyJhbGciOiJIUzI1NiJ9"));
    assert!(!serialized.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(!serialized.contains("PRIVATE KEY"));
    assert!(serialized.contains("keep this diagnostic context"));
    assert!(result.audit.replacement_count >= 5);
}

#[test]
fn sensitive_field_names_are_redacted_without_removing_the_field() {
    let result = Redactor::default()
        .redact_event(json!({"token": "do-not-persist", "normal": "visible"}))
        .expect("redact fields");
    assert!(
        result
            .value
            .get("token")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("[REDACTED:token:")
    );
    assert_eq!(result.value["normal"], "visible");
}

#[test]
fn oversized_or_deep_input_fails_closed() {
    let oversized = Redactor::new(8, 32).redact_event(json!({"value": "too large"}));
    assert_eq!(oversized, Err(RedactionError::LimitExceeded));
    let mut deep = json!(0);
    for _ in 0..10 {
        deep = json!([deep]);
    }
    assert_eq!(
        Redactor::new(1024, 4).redact_event(deep),
        Err(RedactionError::LimitExceeded)
    );
}
