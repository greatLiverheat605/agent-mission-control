use std::time::Duration;

use mission_policy::{
    ActionClass, ApprovalActor, BudgetChange, BudgetDimension, BudgetLimits, BudgetSignal,
    BudgetTracker, EnvelopeDecision, FlightEnvelope, FlightIdentity, UnknownUsagePolicy,
    UsageRecord, UsageSample,
};

fn limits() -> BudgetLimits {
    BudgetLimits {
        tokens: 100,
        money_micros: 100,
        wall_clock: Duration::from_millis(100),
        changed_lines: 100,
        changed_files: 100,
        model_calls: 100,
    }
}

#[test]
fn budget_warns_once_at_eighty_percent_and_pauses_at_safe_boundary() {
    let mut tracker = BudgetTracker::new(7, limits(), UnknownUsagePolicy::Pause);
    tracker.record(UsageRecord::Sample(UsageSample::tokens(80)));
    assert_eq!(
        tracker.evaluate_safe_boundary(),
        vec![BudgetSignal::Warning(BudgetDimension::Tokens)]
    );
    assert!(tracker.evaluate_safe_boundary().is_empty());

    tracker.record(UsageRecord::Sample(UsageSample::tokens(20)));
    assert_eq!(
        tracker.evaluate_safe_boundary(),
        vec![BudgetSignal::PauseAtSafeBoundary(BudgetDimension::Tokens)]
    );
}

#[test]
fn budget_enforces_every_integer_and_monotonic_duration_dimension() {
    let samples = [
        (BudgetDimension::Tokens, UsageSample::tokens(100)),
        (BudgetDimension::MoneyMicros, UsageSample::money_micros(100)),
        (
            BudgetDimension::WallClock,
            UsageSample::wall_clock(Duration::from_millis(100)),
        ),
        (
            BudgetDimension::ChangedLines,
            UsageSample::changed_lines(100),
        ),
        (
            BudgetDimension::ChangedFiles,
            UsageSample::changed_files(100),
        ),
        (BudgetDimension::ModelCalls, UsageSample::model_calls(100)),
    ];

    for (dimension, sample) in samples {
        let mut tracker = BudgetTracker::new(7, limits(), UnknownUsagePolicy::Pause);
        tracker.record(UsageRecord::Sample(sample));
        assert_eq!(
            tracker.evaluate_safe_boundary(),
            vec![BudgetSignal::PauseAtSafeBoundary(dimension)]
        );
    }
}

#[test]
fn budget_unknown_usage_and_corrections_fail_closed_without_erasing_history() {
    let mut tracker = BudgetTracker::new(7, limits(), UnknownUsagePolicy::RequireApproval);
    tracker.record(UsageRecord::Unknown(BudgetDimension::Tokens));
    assert_eq!(
        tracker.evaluate_safe_boundary(),
        vec![BudgetSignal::RequireApproval(BudgetDimension::Tokens)]
    );

    tracker.record(UsageRecord::Correction {
        dimension: BudgetDimension::Tokens,
        corrected_total: 40,
    });
    assert!(tracker.evaluate_safe_boundary().is_empty());
    assert_eq!(tracker.records().len(), 2);
    assert!(matches!(tracker.records()[0], UsageRecord::Unknown(_)));
}

#[test]
fn budget_increase_requires_a_user_contract_version_change() {
    let mut tracker = BudgetTracker::new(7, limits(), UnknownUsagePolicy::Pause);
    let mut larger = limits();
    larger.tokens = 200;

    assert!(
        tracker
            .replace_limits(BudgetChange {
                actor: ApprovalActor::Agent,
                contract_version: 8,
                limits: larger.clone(),
            })
            .is_err()
    );
    assert!(
        tracker
            .replace_limits(BudgetChange {
                actor: ApprovalActor::User,
                contract_version: 7,
                limits: larger.clone(),
            })
            .is_err()
    );
    tracker
        .replace_limits(BudgetChange {
            actor: ApprovalActor::User,
            contract_version: 8,
            limits: larger,
        })
        .expect("new user contract version may expand budget");
    assert_eq!(tracker.contract_version(), 8);
}

#[test]
fn envelope_is_an_immutable_launch_snapshot_and_detects_identity_drift() {
    let identity = FlightIdentity {
        provider: "openai".to_owned(),
        model: "gpt-5".to_owned(),
        loadout_fingerprint: "loadout-1".to_owned(),
    };
    let envelope = FlightEnvelope::new(
        vec![ActionClass::Read, ActionClass::Write],
        vec!["C:/managed/route".to_owned()],
        vec!["api.openai.com".to_owned()],
        vec!["crates.io".to_owned()],
        identity.clone(),
        limits(),
    )
    .expect("valid envelope");

    assert_eq!(
        envelope.check_before_model_request(&identity),
        EnvelopeDecision::Allow
    );
    for changed in [
        FlightIdentity {
            provider: "other".to_owned(),
            ..identity.clone()
        },
        FlightIdentity {
            model: "gpt-6".to_owned(),
            ..identity.clone()
        },
        FlightIdentity {
            loadout_fingerprint: "loadout-2".to_owned(),
            ..identity.clone()
        },
    ] {
        assert_eq!(
            envelope.check_before_model_request(&changed),
            EnvelopeDecision::PauseIdentityChanged
        );
    }
    assert!(envelope.allows_action(ActionClass::Write));
    assert!(!envelope.allows_action(ActionClass::GitPush));
}
