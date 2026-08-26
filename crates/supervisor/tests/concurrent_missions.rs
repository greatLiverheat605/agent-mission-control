use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use mission_domain::MissionId;
use mission_supervisor::resource_budget::{ObservedResources, ResourceBudget, ResourceDecision};
use mission_supervisor::scheduler::{
    ActionAuthorization, MissionScheduleState, MissionScheduler, ScheduledAction, SchedulerConfig,
    SchedulerError,
};

fn action(mission_id: MissionId, number: u64) -> ScheduledAction {
    ScheduledAction {
        mission_id,
        action_id: format!("action-{number}"),
        cwd: PathBuf::from(format!("C:/managed/{mission_id}")),
        env: BTreeMap::from([("MISSION_ID".to_owned(), mission_id.to_string())]),
        stdout_channel: format!("stdout-{mission_id}"),
    }
}

#[test]
fn concurrent_missions_respect_default_limit_release_slots_and_make_fair_progress() {
    let mut scheduler = MissionScheduler::new(SchedulerConfig::default()).expect("scheduler");
    let missions: Vec<_> = (0..5).map(|_| MissionId::new()).collect();
    for (index, &mission_id) in missions.iter().enumerate() {
        scheduler
            .register(mission_id, index == 0)
            .expect("register");
        for number in 0..12 {
            scheduler
                .enqueue(
                    action(mission_id, number),
                    ActionAuthorization::PolicyAllowed,
                )
                .expect("enqueue");
        }
    }
    scheduler.activate_ready();
    assert_eq!(scheduler.active_count(), 3);
    assert!(
        missions[..3]
            .iter()
            .all(|id| scheduler.state(*id) == Some(MissionScheduleState::Active))
    );
    assert!(
        missions[3..]
            .iter()
            .all(|id| scheduler.state(*id) == Some(MissionScheduleState::Queued))
    );

    let mut first_round = Vec::new();
    for _ in 0..12 {
        first_round.push(scheduler.next().expect("scheduled action").mission_id);
    }
    let foreground = first_round.iter().filter(|id| **id == missions[0]).count();
    let background = first_round.iter().filter(|id| **id == missions[1]).count();
    assert!(foreground > background);
    assert!(missions[..3].iter().all(|id| first_round.contains(id)));

    scheduler
        .pause(missions[0], "user approval required")
        .expect("pause");
    assert_eq!(scheduler.active_count(), 3);
    assert_eq!(
        scheduler.state(missions[3]),
        Some(MissionScheduleState::Active)
    );
    scheduler.complete(missions[1]).expect("complete");
    assert_eq!(scheduler.active_count(), 3);
    assert_eq!(
        scheduler.state(missions[4]),
        Some(MissionScheduleState::Active)
    );

    let mut progressed: BTreeSet<_> = first_round.into_iter().collect();
    for _ in 0..30 {
        if let Some(next) = scheduler.next() {
            assert_eq!(next.env["MISSION_ID"], next.mission_id.to_string());
            assert_eq!(next.stdout_channel, format!("stdout-{}", next.mission_id));
            progressed.insert(next.mission_id);
        }
    }
    assert!(missions.iter().all(|id| progressed.contains(id)));
    assert!(scheduler.active_count() <= 3);
}

#[test]
fn concurrent_missions_reject_invalid_limits_and_unapproved_actions() {
    for invalid in [0, 6] {
        assert!(matches!(
            SchedulerConfig::new(invalid),
            Err(SchedulerError::InvalidConcurrency)
        ));
    }
    for valid in 1..=5 {
        assert_eq!(
            SchedulerConfig::new(valid).expect("valid").max_active(),
            valid
        );
    }

    let mission_id = MissionId::new();
    let mut scheduler = MissionScheduler::default();
    scheduler.register(mission_id, false).expect("register");
    assert!(matches!(
        scheduler.enqueue(action(mission_id, 1), ActionAuthorization::Denied),
        Err(SchedulerError::ActionNotApproved)
    ));
}

#[test]
fn concurrent_missions_resource_pressure_is_explainable_and_preserves_queue() {
    let mission_id = MissionId::new();
    let mut scheduler = MissionScheduler::default();
    scheduler.register(mission_id, false).expect("register");
    scheduler
        .enqueue(action(mission_id, 1), ActionAuthorization::ApprovalConsumed)
        .expect("enqueue");
    scheduler.activate_ready();
    let budget = ResourceBudget::new(1_000, 80).expect("budget");

    let constrained = budget.evaluate(ObservedResources {
        memory_bytes: 850,
        cpu_percent: 70,
    });
    assert!(matches!(constrained, ResourceDecision::Throttle { .. }));
    scheduler
        .apply_resource_decision(mission_id, constrained)
        .expect("throttle");
    assert_eq!(scheduler.queued_actions(mission_id), 1);
    assert!(scheduler.resource_reason(mission_id).is_some());

    let critical = budget.evaluate(ObservedResources {
        memory_bytes: 1_000,
        cpu_percent: 80,
    });
    assert!(matches!(
        critical,
        ResourceDecision::PauseAtSafeBoundary { .. }
    ));
    scheduler
        .apply_resource_decision(mission_id, critical)
        .expect("pause");
    assert_eq!(
        scheduler.state(mission_id),
        Some(MissionScheduleState::Paused)
    );
    assert_eq!(scheduler.queued_actions(mission_id), 1);
}
