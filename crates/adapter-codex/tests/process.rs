use adapter_codex::CodexAdapter;
use adapter_core::{AgentAdapter, StartAgentRequest};
use mission_domain::{MissionId, RouteId};
use std::fs;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

#[cfg(windows)]
fn write_probe_fixture(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join("fake-codex-probe.cmd");
    fs::write(&path, body).expect("write probe fixture");
    path
}

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
        provider: adapter_core::ProviderId::Codex,
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        project_root: workspace.path().display().to_string(),
        route_workspace: workspace.path().display().to_string(),
        read_only: true,
        approved_environment: Vec::new(),
        model: None,
        goal: Some("fixture mission goal".to_owned()),
        loadout_fingerprint: "fixture".to_owned(),
        contract_version: 7,
        resume_thread_id: None,
        loadout: None,
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
    for _ in 0..16 {
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

#[tokio::test]
async fn approval_payload_derives_action_class_and_contract_version() {
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
        provider: adapter_core::ProviderId::Codex,
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        project_root: workspace.path().display().to_string(),
        route_workspace: workspace.path().display().to_string(),
        read_only: true,
        approved_environment: Vec::new(),
        model: None,
        goal: Some("approval derivation goal".to_owned()),
        loadout_fingerprint: "fixture".to_owned(),
        contract_version: 7,
        resume_thread_id: None,
        loadout: None,
    };
    let (sink, _sink_rx) = mpsc::unbounded_channel();
    let handle = adapter.start(request, sink).await.expect("start");
    let mut approval = None;
    for _ in 0..16 {
        let event = timeout(Duration::from_secs(5), handle.next_event())
            .await
            .expect("approval event timeout")
            .expect("approval event");
        if matches!(
            event.event_kind,
            mission_domain::EventKind::ApprovalRequested
        ) {
            approval = Some(event);
            break;
        }
    }
    let approval = approval.expect("fake app-server must request approval");
    assert_eq!(approval.payload["action_class"], "exec");
    assert_eq!(approval.payload["contract_version"], 7);
    adapter
        .terminate_owned_tree(handle.run_id())
        .await
        .expect("terminate");
}

#[cfg(windows)]
#[tokio::test]
async fn probe_reports_real_version_hash_and_structured_capability() {
    let dir = tempfile::tempdir().expect("tempdir");
    let executable = write_probe_fixture(
        dir.path(),
        "@echo off\nif \"%1\"==\"--version\" (echo codex 1.2.3 & exit /b 0)\nif \"%1\"==\"exec\" (echo {\"type\":\"thread.started\"} & exit /b 0)\nexit /b 1\n",
    );
    let report = CodexAdapter::new(executable).probe().await.expect("probe");

    assert_eq!(report.install_state, adapter_core::InstallState::Installed);
    assert_eq!(report.version.as_deref(), Some("codex 1.2.3"));
    assert!(
        report
            .executable_hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    assert!(report.is_available());
    assert_eq!(report.configuration_source.as_deref(), Some("codex-cli"));
}

#[cfg(windows)]
#[tokio::test]
async fn probe_never_reports_installed_for_missing_or_non_runnable_executable() {
    let missing = CodexAdapter::new("C:\\does-not-exist\\codex.exe")
        .probe()
        .await
        .expect("missing probe report");
    assert_eq!(missing.install_state, adapter_core::InstallState::Missing);
    assert!(!missing.is_available());

    let dir = tempfile::tempdir().expect("tempdir");
    let non_runnable = write_probe_fixture(
        dir.path(),
        "@echo off\nif \"%1\"==\"--version\" (echo codex 1.2.3 & exit /b 0)\nexit /b 1\n",
    );
    let report = CodexAdapter::new(non_runnable)
        .probe()
        .await
        .expect("non-runnable probe report");
    assert_eq!(
        report.install_state,
        adapter_core::InstallState::DetectedNotRunnable
    );
    assert!(!report.is_available());
}
