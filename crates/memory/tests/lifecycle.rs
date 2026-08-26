use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId};
use mission_memory::{MemoryAuthor, MemoryError, MemoryStatus, MemoryStore, extract_candidates};
use serde_json::json;

fn candidate() -> mission_memory::MemoryItem {
    let event = EventEnvelope::new(
        EventId::new(),
        MissionId::new(),
        RouteId::new(),
        1,
        EventKind::ContractUpdated,
        json!({"goal": "ship recovery", "version": 2}),
    );
    extract_candidates(&[event])
        .expect("candidate extraction")
        .pop()
        .expect("candidate")
}

#[test]
fn lifecycle_mutations_increment_version_and_append_replayable_events() {
    let item = candidate();
    let source_ids = item.source_event_ids.clone();
    let mut store = MemoryStore::default();
    store.insert(item.clone()).expect("insert");

    let confirmed = store
        .confirm(&item.id, MemoryAuthor::User)
        .expect("confirm");
    assert_eq!(confirmed.status, MemoryStatus::Confirmed);
    assert_eq!(confirmed.version, 2);
    let edited = store
        .edit(&item.id, MemoryAuthor::User, "updated goal")
        .expect("edit");
    assert_eq!(edited.status, MemoryStatus::Candidate);
    assert_eq!(edited.version, 3);
    let narrowed = store
        .narrow(&item.id, MemoryAuthor::User, "updated goal only")
        .expect("narrow");
    assert_eq!(narrowed.status, MemoryStatus::Candidate);
    assert_eq!(narrowed.version, 4);
    let deferred = store.defer(&item.id, MemoryAuthor::User).expect("defer");
    assert_eq!(deferred.status, MemoryStatus::Deferred);
    assert_eq!(deferred.version, 5);
    let invalidated = store
        .invalidate(&item.id, MemoryAuthor::User)
        .expect("invalidate");
    assert_eq!(invalidated.status, MemoryStatus::Invalidated);
    assert_eq!(invalidated.version, 6);

    assert_eq!(store.history().len(), 6);
    assert!(store.history().iter().all(|mutation| {
        mutation.event.has_valid_payload_hash()
            && mutation.event.links.source_event_ids == source_ids
            && mutation.item.mission_id == item.mission_id
    }));
    assert_eq!(
        store.history()[1].event.kind.as_str(),
        "memory_item_changed"
    );
}

#[test]
fn lifecycle_rejects_forbidden_actor_and_terminal_mutations() {
    let item = candidate();
    let mut store = MemoryStore::default();
    store.insert(item.clone()).expect("insert");
    assert_eq!(
        store.confirm(&item.id, MemoryAuthor::Agent),
        Err(MemoryError::ForbiddenActor)
    );
    store.reject(&item.id, MemoryAuthor::User).expect("reject");
    assert_eq!(
        store.invalidate(&item.id, MemoryAuthor::User),
        Err(MemoryError::InvalidTransition)
    );
}
