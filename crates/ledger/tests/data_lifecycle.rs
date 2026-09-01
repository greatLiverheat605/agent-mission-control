use std::fs;

use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId};
use mission_ledger::{
    EncryptedBlobStore, EncryptedLedger, InMemoryKeyStore, KeyStore, LifecycleError, StorageBudget,
};
use serde_json::json;

fn event(mission_id: MissionId, sequence: u64, payload: serde_json::Value) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(),
        mission_id,
        RouteId::new(),
        sequence,
        EventKind::AgentMessage,
        payload,
    )
}

fn ledger() -> (std::path::PathBuf, EncryptedLedger, MissionId) {
    let root = std::env::temp_dir().join(format!("mission-lifecycle-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("create test root");
    let path = root.join("ledger.db");
    let mission = MissionId::new();
    let ledger = EncryptedLedger::open(&path, "lifecycle-test", InMemoryKeyStore::default())
        .expect("open ledger");
    (root, ledger, mission)
}

#[test]
fn retention_plan_reports_pressure_without_automatic_deletion() {
    let (root, mut ledger, mission) = ledger();
    ledger
        .append(&event(mission, 1, json!({"message": "x".repeat(128)})))
        .expect("append event");

    let plan = ledger
        .retention_plan(&StorageBudget::new(Some(8), None))
        .expect("build retention plan");

    assert!(plan.over_budget);
    assert!(!plan.automatic_deletion);
    assert!(!plan.impact_hash.is_empty());
    assert_eq!(
        ledger.replay(&mission.to_string()).expect("replay").len(),
        1
    );
    drop(ledger);
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn retention_project_usage_groups_missions_by_project_root() {
    let (root, mut ledger, first) = ledger();
    let second = MissionId::new();
    let first_created = EventEnvelope::new(
        EventId::new(),
        first,
        RouteId::new(),
        1,
        EventKind::MissionCreated,
        json!({"project_root": "C:/projects/alpha"}),
    );
    ledger.append(&first_created).expect("append first mission");
    let second_created = EventEnvelope::new(
        EventId::new(),
        second,
        RouteId::new(),
        1,
        EventKind::MissionCreated,
        json!({"project_root": "C:/projects/beta"}),
    );
    ledger
        .append(&second_created)
        .expect("append second mission");
    ledger
        .append(&event(first, 2, json!({"message": "alpha"})))
        .expect("append alpha payload");
    ledger
        .append(&event(second, 2, json!({"message": "beta"})))
        .expect("append beta payload");

    let alpha = ledger
        .retention_plan_for_project(&StorageBudget::default(), Some("C:/projects/alpha"))
        .expect("alpha retention plan");
    let missing = ledger
        .retention_plan_for_project(&StorageBudget::default(), Some("C:/projects/missing"))
        .expect("missing retention plan");
    assert_eq!(alpha.project_usage.event_count, 2);
    assert_eq!(missing.project_usage.event_count, 0);
    assert_eq!(alpha.candidate_missions.len(), 1);
    assert_eq!(alpha.candidate_missions[0].mission_id, first);
    drop(ledger);
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn archive_requires_matching_impact_hash_and_writes_audit_receipt() {
    let (root, mut ledger, mission) = ledger();
    ledger
        .append(&event(mission, 1, json!({"message": "archive me"})))
        .expect("append event");
    let plan = ledger.archive_plan(mission).expect("build archive plan");
    assert_ne!(plan.created_at, "1970-01-01T00:00:00Z");
    assert!(plan.created_at.ends_with('Z'));
    let receipt = ledger.archive(&plan).expect("archive mission");
    assert_eq!(receipt.operation, "archive");
    assert_ne!(receipt.created_at, "1970-01-01T00:00:00Z");
    assert!(receipt.created_at.ends_with('Z'));
    assert!(ledger.is_archived(&mission).expect("archive state"));
    assert!(matches!(
        ledger.archive(&plan),
        Err(LifecycleError::PlanAlreadyApplied)
    ));
    drop(ledger);
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn export_preview_and_materialization_are_redacted_and_hash_bound() {
    let (root, mut ledger, mission) = ledger();
    let secret = "sk-1234567890abcdefghijklmnop";
    ledger
        .append(&event(
            mission,
            1,
            json!({"token": secret, "message": "safe"}),
        ))
        .expect("append event");
    let preview = ledger.export_preview(&mission).expect("export preview");
    assert_eq!(preview.event_count, 1);
    assert!(!preview.content_hash.is_empty());
    assert!(!preview.categories.is_empty());
    assert!(!preview.contains_raw_provider_payload);
    let export = ledger
        .materialize_export(&mission)
        .expect("materialize export");
    assert_eq!(export.content_hash, preview.content_hash);
    let text = String::from_utf8(export.bytes).expect("utf8 export");
    assert!(!text.contains(secret));
    assert!(text.contains("REDACTED"));
    drop(ledger);
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn delete_requires_impact_plan_and_keeps_source_workspace_outside_ledger() {
    let (root, mut ledger, mission) = ledger();
    let workspace = root.join("workspace.txt");
    fs::write(&workspace, b"source remains unchanged").expect("write source");
    ledger
        .append(&event(mission, 1, json!({"message": "delete me"})))
        .expect("append event");
    let plan = ledger.delete_impact(&mission).expect("delete impact");
    assert_ne!(plan.created_at, "1970-01-01T00:00:00Z");
    assert!(plan.created_at.ends_with('Z'));
    let receipt = ledger.delete_mission(&plan).expect("delete mission");
    assert_eq!(receipt.operation, "delete");
    assert_ne!(receipt.created_at, "1970-01-01T00:00:00Z");
    assert!(
        ledger
            .replay(&mission.to_string())
            .expect("replay")
            .is_empty()
    );
    assert_eq!(
        fs::read(&workspace).expect("read source"),
        b"source remains unchanged"
    );
    assert!(matches!(
        ledger.delete_mission(&plan),
        Err(LifecycleError::PlanAlreadyApplied)
    ));
    drop(ledger);
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn deleting_one_mission_does_not_remove_a_shared_blob() {
    let root =
        std::env::temp_dir().join(format!("mission-lifecycle-shared-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("create test root");
    let ledger_path = root.join("ledger.db");
    let keys = InMemoryKeyStore::default();
    let mut ledger =
        EncryptedLedger::open(&ledger_path, "shared-test", keys.clone()).expect("open ledger");
    let blob_store = EncryptedBlobStore::open_for_ledger(
        root.join("blobs"),
        &ledger_path,
        keys.load_database_key("shared-test").expect("key"),
    )
    .expect("open blob store");
    let first = MissionId::new();
    let second = MissionId::new();
    ledger
        .append(&event(first, 1, json!({"message": "first"})))
        .expect("append first");
    ledger
        .append(&event(second, 1, json!({"message": "second"})))
        .expect("append second");
    let reference = blob_store
        .put(b"shared evidence", "text/plain")
        .expect("put blob");
    blob_store
        .retain_for_mission(&first, &reference)
        .expect("retain first");
    blob_store
        .retain_for_mission(&second, &reference)
        .expect("retain second");

    let plan = ledger.delete_impact(&first).expect("delete impact");
    assert_eq!(plan.blob_refs[0].ref_count, 2);
    assert!(!plan.blob_refs[0].will_remove);
    ledger.delete_mission(&plan).expect("delete first");
    assert_eq!(
        blob_store
            .mission_references(&second)
            .expect("second refs")
            .len(),
        1
    );
    assert_eq!(
        blob_store.read(&reference).expect("read shared blob"),
        b"shared evidence"
    );

    assert!(
        !blob_store
            .delete_if_unreferenced(&reference)
            .expect("keep shared blob physically")
    );
    let second_plan = ledger.delete_impact(&second).expect("delete second impact");
    ledger
        .delete_mission(&second_plan)
        .expect("delete second mission");
    assert!(
        blob_store
            .delete_if_unreferenced(&reference)
            .expect("remove unreferenced blob physically")
    );
    assert!(!blob_store.path_for(&reference).expect("blob path").exists());

    drop(blob_store);
    drop(ledger);
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn delete_rejects_a_stale_impact_plan_without_mutating_events() {
    let (root, mut ledger, mission) = ledger();
    ledger
        .append(&event(mission, 1, json!({"message": "before plan"})))
        .expect("append first event");
    let mut plan = ledger.delete_impact(&mission).expect("build delete impact");
    ledger
        .append(&event(mission, 2, json!({"message": "after plan"})))
        .expect("append second event");
    plan.event_count = 1;
    assert!(matches!(
        ledger.delete_mission(&plan),
        Err(LifecycleError::PlanMismatch)
    ));
    assert_eq!(
        ledger.replay(&mission.to_string()).expect("replay").len(),
        2
    );
    drop(ledger);
    fs::remove_dir_all(root).expect("remove test root");
}
