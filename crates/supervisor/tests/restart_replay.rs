use std::fmt;
use std::sync::mpsc;

use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId, replay};
use mission_supervisor::event_pipeline::{
    AppendOnlyLedger, EventPipeline, PipelineError, PipelineStatus,
};
use serde_json::json;

#[derive(Clone, Default)]
struct MemoryLedger {
    events: Vec<EventEnvelope>,
    fail_append: bool,
}

impl AppendOnlyLedger for MemoryLedger {
    type Error = MemoryError;

    fn append(&mut self, event: &EventEnvelope) -> Result<(), Self::Error> {
        if self.fail_append {
            return Err(MemoryError("injected append failure"));
        }
        self.events.push(event.clone());
        Ok(())
    }

    fn replay(&self, mission_id: &MissionId) -> Result<Vec<EventEnvelope>, Self::Error> {
        Ok(self
            .events
            .iter()
            .filter(|event| &event.mission_id == mission_id)
            .cloned()
            .collect())
    }
}

#[derive(Debug)]
struct MemoryError(&'static str);

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

fn event(
    mission_id: MissionId,
    route_id: RouteId,
    sequence: u64,
    kind: EventKind,
    payload: serde_json::Value,
) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(),
        mission_id,
        route_id,
        sequence,
        kind,
        payload,
    )
}

#[test]
fn restart_replay_matches_single_pass_and_subscriber_never_sees_uncommitted_state() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let events = vec![
        event(mission, route, 1, EventKind::MissionCreated, json!({})),
        event(mission, route, 2, EventKind::RouteCreated, json!({})),
        event(
            mission,
            route,
            3,
            EventKind::RouteStateChanged,
            json!({"state": "ReadOnlyExploration", "version": 1}),
        ),
    ];
    let mut ledger = MemoryLedger::default();
    let mut pipeline = EventPipeline::with_empty(ledger.clone());
    let (sender, receiver) = mpsc::channel();
    pipeline.subscribe(sender);
    for event in &events[..2] {
        pipeline
            .append(event.clone())
            .expect("append committed event");
    }
    let _ = receiver.try_iter().collect::<Vec<_>>();
    ledger.events = pipeline.ledger().events.clone();
    let mut restarted = EventPipeline::recover(ledger.clone(), &mission).expect("recover model");
    restarted
        .append(events[2].clone())
        .expect("append after restart");
    let expected = replay(events).expect("single pass replay");
    assert_eq!(restarted.model(), &expected);
}

#[test]
fn append_failure_pauses_before_broadcasting_success() {
    let mission = MissionId::new();
    let route = RouteId::new();
    let event = event(mission, route, 1, EventKind::MissionCreated, json!({}));
    let mut pipeline = EventPipeline::with_empty(MemoryLedger {
        fail_append: true,
        ..MemoryLedger::default()
    });
    let (sender, receiver) = mpsc::channel();
    pipeline.subscribe(sender);
    let _ = receiver.try_iter().collect::<Vec<_>>();
    assert!(matches!(
        pipeline.append(event),
        Err(PipelineError::Ledger(_))
    ));
    assert!(matches!(pipeline.status(), PipelineStatus::Paused { .. }));
    assert!(receiver.try_recv().is_err());
}
