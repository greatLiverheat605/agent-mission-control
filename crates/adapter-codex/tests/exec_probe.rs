use adapter_codex::run_exec_probe;
use std::fs;
use std::time::Duration;

#[tokio::test]
async fn exec_probe_marks_source_as_json_and_read_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let command = if cfg!(windows) {
        let path = dir.path().join("fake-codex.cmd");
        fs::write(&path, "@echo off\necho {\"type\":\"thread.started\"}\n").expect("fixture");
        path
    } else {
        let path = dir.path().join("fake-codex.sh");
        fs::write(&path, "#!/bin/sh\necho '{\"type\":\"thread.started\"}'\n").expect("fixture");
        path
    };
    let result = run_exec_probe(&command, dir.path(), "inspect", Duration::from_secs(1))
        .await
        .expect("probe");
    assert_eq!(result.events.len(), 1);
    assert_eq!(
        result.events[0].event.raw_evidence.as_ref().unwrap()["type"],
        "thread.started"
    );
}
