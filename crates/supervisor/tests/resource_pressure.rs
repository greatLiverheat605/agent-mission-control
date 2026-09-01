use mission_supervisor::resource_budget::{ResourceBudget, ResourceDecision, ResourcePressure};

#[test]
fn disk_pressure_throttles_then_pauses_without_silent_kill() {
    let budget = ResourceBudget::new(1_000, 90)
        .expect("budget")
        .with_disk_limit(10_000)
        .expect("disk limit");
    assert!(matches!(
        budget.evaluate_pressure(ResourcePressure { memory_bytes: 100, cpu_percent: 10, disk_bytes: 8_100 }),
        ResourceDecision::Throttle { reason } if reason.contains("disk")
    ));
    assert!(matches!(
        budget.evaluate_pressure(ResourcePressure { memory_bytes: 100, cpu_percent: 10, disk_bytes: 10_000 }),
        ResourceDecision::PauseAtSafeBoundary { reason } if reason.contains("disk")
    ));
}
