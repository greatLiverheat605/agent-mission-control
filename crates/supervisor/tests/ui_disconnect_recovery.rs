use mission_domain::{EventKind, MissionId, RouteId};
use mission_ledger::{EncryptedLedger, InMemoryKeyStore};
use mission_supervisor::mission_actor::MissionActor;

#[test]
fn ui_disconnect_is_persisted_and_restart_does_not_resume_agent() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("mission.db");
    let keys = InMemoryKeyStore::default();
    let mission_id = MissionId::new();
    let route_id = RouteId::new();
    let mut actor = MissionActor::new(
        mission_id,
        route_id,
        EncryptedLedger::open(&path, "ui-disconnect-fixture", keys.clone()).expect("ledger"),
    );
    actor
        .record_event(
            EventKind::AgentRunStarted,
            serde_json::json!({"read_only":true}),
        )
        .expect("agent start");
    actor.set_ui_connected(false).expect("safe pause");
    assert!(matches!(
        actor.state(),
        mission_supervisor::pause::PauseState::PauseRequested { .. }
    ));
    drop(actor);

    let recovered = MissionActor::new(
        mission_id,
        route_id,
        EncryptedLedger::open(&path, "ui-disconnect-fixture", keys).expect("reopen ledger"),
    );
    assert_eq!(recovered.sequence(), 2);
    assert!(!recovered.ui_connected());
    assert!(
        matches!(recovered.state(), mission_supervisor::pause::PauseState::PauseRequested { reason } if reason == "ui disconnected")
    );
    assert_eq!(recovered.replay_after(1).len(), 1);
}
