use adapter_detect::{AgentKind, InstallState, ProbeOptions, detect, detect_all};
use std::fs;
use std::path::Path;

fn shim(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = if cfg!(windows) {
        dir.join(format!("{name}.cmd"))
    } else {
        dir.join(name)
    };
    fs::write(&path, body).expect("write shim");
    path
}

#[test]
fn probes_only_version_and_classifies_non_runnable_agents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = if cfg!(windows) {
        "@echo off\nif \"%1\"==\"--version\" echo codex 1.2.3\n"
    } else {
        "#!/bin/sh\n[ \"$1\" = \"--version\" ] && echo codex 1.2.3\n"
    };
    let _codex = shim(dir.path(), "codex", body);
    let _open = shim(dir.path(), "opencode", body);
    let detections = detect_all(Some(dir.path()), &ProbeOptions::default());
    assert_eq!(
        detections
            .iter()
            .find(|d| d.agent == AgentKind::Codex)
            .unwrap()
            .report
            .install_state,
        InstallState::Installed
    );
    assert_eq!(
        detections
            .iter()
            .find(|d| d.agent == AgentKind::OpenCode)
            .unwrap()
            .report
            .install_state,
        InstallState::DetectedNotRunnable
    );
    assert_eq!(
        detect(
            AgentKind::Claude,
            Some(dir.path()),
            &ProbeOptions::default()
        )
        .report
        .install_state,
        InstallState::Missing
    );
}
