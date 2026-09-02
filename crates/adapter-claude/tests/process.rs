use std::path::PathBuf;
use std::time::Duration;

use adapter_claude::{ClaudeAdapter, ClaudeAdapterOptions};
use adapter_core::{AgentAdapter, ProviderId, StartAgentRequest};
use mission_domain::{EventKind, MissionId, RouteId};
use tokio::sync::mpsc;

fn fake_claude_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/agents/bin/fake-claude.ps1")
}

fn request() -> StartAgentRequest {
    StartAgentRequest {
        provider: ProviderId::Claude,
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        project_root: env!("CARGO_MANIFEST_DIR").to_owned(),
        route_workspace: env!("CARGO_MANIFEST_DIR").to_owned(),
        read_only: true,
        approved_environment: vec![("CLAUDE_TEST_ALLOWED".to_owned(), "yes".to_owned())],
        model: None,
        goal: Some("fixture mission goal".to_owned()),
        loadout_fingerprint: "fixture-loadout".to_owned(),
        contract_version: 1,
        resume_thread_id: None,
        loadout: None,
    }
}

#[tokio::test]
async fn fake_claude_uses_non_bare_stream_json_and_allowlisted_environment() {
    let adapter = ClaudeAdapter::new(ClaudeAdapterOptions::powershell(fake_claude_script()));
    let report = adapter.probe().await.expect("probe fake Claude");
    assert_eq!(report.provider, ProviderId::Claude);
    assert!(report.is_available());

    let (sink, _sink_rx) = mpsc::unbounded_channel();
    let handle = adapter
        .start(request(), sink)
        .await
        .expect("start fake Claude");
    let first = tokio::time::timeout(Duration::from_secs(30), handle.next_event())
        .await
        .expect("first event timeout")
        .expect("first event");
    assert!(matches!(first.event_kind, EventKind::AgentRunStarted));
    assert_eq!(
        first.payload["data"]["argv"]["output_format"],
        "stream-json"
    );
    assert_eq!(first.payload["data"]["argv"]["verbose"], true);
    assert_eq!(first.payload["data"]["allowed_env"], "yes");
    assert!(first.payload["data"].get("secret_env").is_none());
    let child_pid = first.payload["data"]["child_pid"]
        .as_u64()
        .expect("fake Claude reports owned child pid");

    adapter
        .request_safe_pause(handle.run_id())
        .await
        .expect("safe pause");
    adapter
        .terminate_owned_tree(handle.run_id())
        .await
        .expect("terminate owned tree");
    let exited = (0..100).any(|_| {
        let status = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "if (Get-Process -Id {child_pid} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}"
                ),
            ])
            .status()
            .expect("check child process");
        if status.success() {
            true
        } else {
            std::thread::sleep(Duration::from_millis(100));
            false
        }
    });
    assert!(exited, "owned Claude child process survived termination");
    assert!(
        !PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("%SystemDrive%")
            .exists(),
        "cleared Windows environment created a literal SystemDrive path"
    );
}

#[test]
fn claude_loadout_contract_rejects_resume_and_provider_mismatch() {
    let mut request = request();
    request.resume_thread_id = Some("resume-token".to_owned());
    let error = adapter_claude::validate_start_request(&request).expect_err("resume rejected");
    assert!(matches!(error, adapter_core::AdapterError::Unsupported));

    request.resume_thread_id = None;
    request.provider = ProviderId::Codex;
    let error = adapter_claude::validate_start_request(&request).expect_err("provider rejected");
    assert!(matches!(error, adapter_core::AdapterError::Protocol(_)));
}
