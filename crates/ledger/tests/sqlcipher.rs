use std::fs;

use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId};
use mission_ledger::{EncryptedLedger, InMemoryKeyStore, LedgerError};
use serde_json::json;

fn event(sequence: u64, payload: serde_json::Value) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(),
        MissionId::new(),
        RouteId::new(),
        sequence,
        EventKind::MissionCreated,
        payload,
    )
}

#[test]
fn encrypted_database_reopens_and_wrong_key_fails_without_overwrite() {
    let root = std::env::temp_dir().join(format!("mission-ledger-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("create temp root");
    let path = root.join("ledger.db");
    let store = InMemoryKeyStore::default();
    let mission = MissionId::new();
    let mut first =
        EncryptedLedger::open(&path, "install", store.clone()).expect("open new ledger");
    let mut item = event(1, json!({"message": "hello"}));
    item.mission_id = mission;
    first.append(&item).expect("append event");
    drop(first);
    let reopened = EncryptedLedger::open(&path, "install", store.clone()).expect("reopen ledger");
    assert_eq!(
        reopened.replay(&mission.to_string()).expect("replay").len(),
        1
    );
    let wrong = InMemoryKeyStore::default();
    wrong.insert("install", [7_u8; 32]);
    assert!(matches!(
        EncryptedLedger::open(&path, "install", wrong),
        Err(LedgerError::KeyMismatch)
    ));
    drop(reopened);
    let bytes = fs::read(&path).expect("read encrypted file");
    assert!(!bytes.windows(5).any(|window| window == b"hello"));
    fs::remove_dir_all(root).expect("remove temp root");
}

#[test]
fn append_is_idempotent_and_sequence_is_monotonic() {
    let path = std::env::temp_dir().join(format!("mission-ledger-{}.db", uuid::Uuid::new_v4()));
    let store = InMemoryKeyStore::default();
    let mission = MissionId::new();
    let mut ledger = EncryptedLedger::open(&path, "install", store).expect("open ledger");
    let mut first = event(1, json!({}));
    first.mission_id = mission;
    ledger.append(&first).expect("append first");
    ledger.append(&first).expect("duplicate is idempotent");
    let mut gap = event(3, json!({}));
    gap.mission_id = mission;
    assert!(matches!(
        ledger.append(&gap),
        Err(LedgerError::SequenceViolation)
    ));
    drop(ledger);
    let _ = fs::remove_file(path);
}

#[test]
fn duplicate_event_id_with_changed_envelope_is_rejected() {
    let path = std::env::temp_dir().join(format!("mission-ledger-{}.db", uuid::Uuid::new_v4()));
    let store = InMemoryKeyStore::default();
    let mission = MissionId::new();
    let mut ledger = EncryptedLedger::open(&path, "install", store).expect("open ledger");
    let mut first = event(1, json!({"value": 1}));
    first.mission_id = mission;
    ledger.append(&first).expect("append first");
    let mut changed = event(1, json!({"value": 1}));
    changed.event_id = first.event_id;
    changed.mission_id = mission;
    changed.route_id = RouteId::new();
    assert!(matches!(
        ledger.append(&changed),
        Err(LedgerError::DuplicateConflict)
    ));
    drop(ledger);
    let _ = fs::remove_file(path);
}

#[test]
fn missing_key_fails_closed_without_initializing_a_database() {
    let root =
        std::env::temp_dir().join(format!("mission-ledger-missing-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("create temp root");
    let path = root.join("ledger.db");
    let store = InMemoryKeyStore::default();
    let ledger = EncryptedLedger::open(&path, "missing-install", store).expect("create ledger");
    drop(ledger);
    let store = InMemoryKeyStore::default();
    assert!(matches!(
        EncryptedLedger::open(&path, "missing-install", store),
        Err(LedgerError::KeyUnavailable)
    ));
    assert!(path.exists());
    fs::remove_dir_all(root).expect("remove temp root");
}

#[test]
fn replayed_redacted_event_keeps_a_valid_persisted_hash() {
    let root =
        std::env::temp_dir().join(format!("mission-ledger-redacted-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("create temp root");
    let path = root.join("ledger.db");
    let store = InMemoryKeyStore::default();
    let mission = MissionId::new();
    let mut ledger = EncryptedLedger::open(&path, "install", store).expect("open ledger");
    let mut item = event(
        1,
        json!({"token": "sk-1234567890abcdefghijklmnop", "message": "keep context"}),
    );
    item.mission_id = mission;
    ledger.append(&item).expect("append redacted event");
    let replayed = ledger.replay_events(&mission).expect("replay events");
    assert_eq!(replayed.len(), 1);
    assert!(replayed[0].has_valid_payload_hash());
    assert!(
        !replayed[0]
            .payload
            .to_string()
            .contains("sk-1234567890abcdefghijklmnop")
    );
    drop(ledger);
    fs::remove_dir_all(root).expect("remove temp root");
}
