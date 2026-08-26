use mission_domain::{EventId, MissionId, RouteId};
use mission_memory::{
    ContextCandidate, ContextKind, ContextPackError, ExclusionReason, MemoryAuthor,
    MemoryFreshness, MemoryItem, MemoryKind, MemoryScope, MemoryStatus, build_context_pack,
};

fn candidate(id: &str, kind: ContextKind, content: &str) -> ContextCandidate {
    ContextCandidate::new(id, kind, content, vec![EventId::new()], 1).expect("candidate")
}

fn confirmed_memory_candidate() -> ContextCandidate {
    let item = MemoryItem {
        id: "memory".to_owned(),
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        kind: MemoryKind::Fact,
        content: "memory".to_owned(),
        source_event_ids: vec![EventId::new()],
        scope: MemoryScope::Mission,
        freshness: MemoryFreshness::Fresh,
        version: 1,
        status: MemoryStatus::Confirmed,
        author: MemoryAuthor::Supervisor,
    };
    ContextCandidate::from_memory(&item).expect("confirmed memory")
}

#[test]
fn context_pack_is_deterministic_and_records_budget_exclusions() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let candidates = vec![
        confirmed_memory_candidate(),
        candidate("evidence", ContextKind::Evidence, "evidence"),
        candidate("risk", ContextKind::Risk, "risk"),
        candidate("route", ContextKind::RouteCheckpoint, "route"),
        candidate("contract", ContextKind::Contract, "contract"),
    ];

    let first = build_context_pack(mission, route, 5, &candidates).expect("pack");
    let second = build_context_pack(
        mission,
        route,
        5,
        &candidates.into_iter().rev().collect::<Vec<_>>(),
    )
    .expect("pack");
    assert_eq!(first, second);
    assert_eq!(first.hash.len(), 64);
    assert_eq!(
        first
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["contract", "route", "risk"]
    );
    assert!(
        first
            .excluded
            .iter()
            .any(|item| { item.id == "evidence" && item.reason == ExclusionReason::Budget })
    );
    assert!(
        first
            .excluded
            .iter()
            .any(|item| { item.id == "memory" && item.reason == ExclusionReason::Budget })
    );
}

#[test]
fn context_pack_rejects_zero_budget() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let memory = confirmed_memory_candidate();
    assert_eq!(
        build_context_pack(mission, route, 0, &[memory]),
        Err(ContextPackError::BudgetInvalid)
    );
}

#[test]
fn confirmed_memory_candidates_must_come_from_a_confirmed_memory_item() {
    assert_eq!(
        ContextCandidate::new(
            "memory",
            ContextKind::ConfirmedMemory,
            "forged",
            vec![EventId::new()],
            1,
        ),
        Err(ContextPackError::UnconfirmedMemory)
    );

    let mut item = MemoryItem {
        id: "memory".to_owned(),
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        kind: MemoryKind::Fact,
        content: "confirmed".to_owned(),
        source_event_ids: vec![EventId::new()],
        scope: MemoryScope::Mission,
        freshness: MemoryFreshness::Fresh,
        version: 1,
        status: MemoryStatus::Candidate,
        author: MemoryAuthor::Supervisor,
    };
    assert_eq!(
        ContextCandidate::from_memory(&item),
        Err(ContextPackError::UnconfirmedMemory)
    );
    item.status = MemoryStatus::Confirmed;
    assert!(ContextCandidate::from_memory(&item).is_ok());
}
