use mission_domain::{EventId, MissionId, RouteId};
use mission_memory::{
    MemoryAuthor, MemoryFreshness, MemoryItem, MemoryKind, MemoryScope, MemoryStatus,
    recall_confirmed,
};

fn item(
    id: &str,
    mission_id: MissionId,
    route_id: RouteId,
    content: &str,
    status: MemoryStatus,
) -> MemoryItem {
    MemoryItem {
        id: id.to_owned(),
        mission_id,
        route_id,
        kind: MemoryKind::Fact,
        content: content.to_owned(),
        source_event_ids: vec![EventId::new()],
        scope: MemoryScope::Route,
        freshness: MemoryFreshness::Fresh,
        version: 1,
        status,
        author: MemoryAuthor::Supervisor,
    }
}

#[test]
fn recall_is_confirmed_only_scoped_and_deterministic() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let mut candidates = vec![
        item(
            "z",
            mission,
            route,
            "read-only workspace",
            MemoryStatus::Confirmed,
        ),
        item(
            "a",
            mission,
            route,
            "workspace constraint",
            MemoryStatus::Confirmed,
        ),
        item(
            "pending",
            mission,
            route,
            "workspace pending",
            MemoryStatus::Candidate,
        ),
        item(
            "other-route",
            mission,
            RouteId::new(),
            "workspace other",
            MemoryStatus::Confirmed,
        ),
        item(
            "other-mission",
            MissionId::new(),
            route,
            "workspace other",
            MemoryStatus::Confirmed,
        ),
    ];

    let first = recall_confirmed(&candidates, mission, route, "workspace", 10);
    candidates.reverse();
    let second = recall_confirmed(&candidates, mission, route, "workspace", 10);
    assert_eq!(first, second);
    assert_eq!(
        first
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "z"]
    );
    assert!(first.iter().all(|item| !item.id.contains("pending")));
}

#[test]
fn recall_respects_limit_and_empty_queries_remain_safe() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let items = vec![
        item("b", mission, route, "second", MemoryStatus::Confirmed),
        item("a", mission, route, "first", MemoryStatus::Confirmed),
    ];

    assert_eq!(recall_confirmed(&items, mission, route, "", 1).len(), 1);
    assert!(recall_confirmed(&items, mission, route, "", 0).is_empty());
}
