use std::time::Duration;

use mission_domain::{MissionId, RouteId};
use mission_policy::{
    ActionClass, BudgetDimension, BudgetLimits, BudgetSignal, EnvelopeDecision, FlightEnvelope,
    FlightIdentity,
};
use mission_supervisor::mission_actor::MissionActor;

fn limits() -> BudgetLimits {
    BudgetLimits {
        tokens: 100,
        money_micros: 100,
        wall_clock: Duration::from_secs(1),
        changed_lines: 100,
        changed_files: 100,
        model_calls: 100,
    }
}

#[test]
fn budget_pause_is_applied_once_at_a_safe_boundary() {
    let mut actor = MissionActor::new(MissionId::new(), RouteId::new(), Vec::new());
    let paused = actor
        .apply_budget_signals(&[
            BudgetSignal::Warning(BudgetDimension::Tokens),
            BudgetSignal::PauseAtSafeBoundary(BudgetDimension::MoneyMicros),
        ])
        .expect("apply signals");

    assert!(paused);
    assert_eq!(actor.ledger()[0].kind.as_str(), "budget_warning");
    assert_eq!(actor.ledger()[1].kind.as_str(), "budget_exceeded");
    assert_eq!(actor.ledger()[2].kind.as_str(), "pause_requested");
}

#[test]
fn envelope_identity_drift_pauses_before_the_next_model_request() {
    let identity = FlightIdentity {
        provider: "openai".to_owned(),
        model: "gpt-5".to_owned(),
        loadout_fingerprint: "loadout-1".to_owned(),
    };
    let envelope = FlightEnvelope::new(
        vec![ActionClass::Read],
        vec!["C:/managed/route".to_owned()],
        Vec::new(),
        Vec::new(),
        identity.clone(),
        limits(),
    )
    .expect("envelope");
    let mut actor = MissionActor::new(MissionId::new(), RouteId::new(), Vec::new());
    let changed = FlightIdentity {
        model: "gpt-6".to_owned(),
        ..identity
    };

    assert_eq!(
        actor
            .check_flight_identity(&envelope, &changed)
            .expect("check identity"),
        EnvelopeDecision::PauseIdentityChanged
    );
    assert_eq!(actor.ledger()[0].kind.as_str(), "flight_envelope_changed");
    assert_eq!(actor.ledger()[1].kind.as_str(), "pause_requested");
}
