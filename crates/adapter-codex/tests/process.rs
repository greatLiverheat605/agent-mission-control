use adapter_codex::CodexAdapter;
use adapter_core::{AgentAdapter, StartAgentRequest};
use mission_domain::{MissionId, RouteId};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn app_server_run_consumes_events_and_supports_pause_and_terminate() {
    let executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("agents")
        .join("bin")
        .join("fake-codex-app-server.cmd");
    let workspace = tempfile::tempdir().expect("workspace");
    let adapter = CodexAdapter::new(executable);
    let request = StartAgentRequest {
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        project_root: workspace.path().display().to_string(),
        route_workspace: workspace.path().display().to_string(),
        read_only: true,
        approved_environment: Vec::new(),
        model: None,
        loadout_fingerprint: "fixture".to_owned(),
        resume_token: None,
    };
    let (sink, mut sink_rx) = mpsc::unbounded_channel();
    let handle = adapter.start(request, sink).await.expect("start");
    let first = timeout(Duration::from_secs(5), handle.next_event())
        .await
        .expect("start event timeout")
        .expect("start event");
    assert!(matches!(
        first.event_kind,
        mission_domain::EventKind::AgentRunStarted
    ));
    let run_id = handle.run_id().to_owned();
    assert!(
        timeout(Duration::from_secs(5), handle.next_event())
            .await
            .is_ok()
    );
    assert!(
        timeout(Duration::from_secs(5), handle.next_event())
            .await
            .is_ok()
    );
    assert!(
        timeout(Duration::from_secs(5), handle.next_event())
            .await
            .is_ok()
    );

    adapter.request_safe_pause(&run_id).await.expect("pause");
    let mut saw_pause = false;
    for _ in 0..4 {
        if let Ok(Some(event)) = timeout(Duration::from_secs(2), handle.next_event()).await
            && matches!(event.event_kind, mission_domain::EventKind::PauseRequested)
        {
            saw_pause = true;
            break;
        }
    }
    assert!(saw_pause);
    adapter
        .terminate_owned_tree(&run_id)
        .await
        .expect("terminate");
    let _ = sink_rx.try_recv();
}
