use adapter_codex::spawn_app_server;
use std::fs;
use std::time::Duration;

#[tokio::test]
async fn app_server_matches_json_rpc_request_ids() {
    let dir = tempfile::tempdir().expect("tempdir");
    let command = if cfg!(windows) {
        let path = dir.path().join("fake-codex.cmd");
        fs::write(&path, "@echo off\nset /p line=\nfor /f \"tokens=2 delims=:,\" %%a in (\"%line%\") do echo {\"jsonrpc\":\"2.0\",\"id\":%%a,\"result\":{\"ok\":true}}\n").expect("fixture");
        path
    } else {
        let path = dir.path().join("fake-codex.sh");
        fs::write(&path, "#!/bin/sh\nread line\necho '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}'\n").expect("fixture");
        path
    };
    let mut client = spawn_app_server(&command, dir.path()).await.expect("spawn");
    let response = client
        .request("initialize", serde_json::json!({}), Duration::from_secs(1))
        .await
        .expect("response");
    assert_eq!(response.result.unwrap()["ok"], true);
    client.shutdown().await.expect("shutdown");
}
