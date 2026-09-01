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

#[tokio::test]
async fn app_server_rejects_oversized_and_deep_json_responses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let command = if cfg!(windows) {
        let path = dir.path().join("bad-codex.cmd");
        fs::write(&path, "@echo off\nset /p line=\necho {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":\"%line%\"}}\n").expect("fixture");
        path
    } else {
        let path = dir.path().join("bad-codex.sh");
        fs::write(
            &path,
            "#!/bin/sh\nread line\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'\n",
        )
        .expect("fixture");
        path
    };
    let mut client = spawn_app_server(&command, dir.path()).await.expect("spawn");
    client.max_line_bytes_for_test(8);
    let error = client
        .request("initialize", serde_json::json!({}), Duration::from_secs(1))
        .await
        .expect_err("oversized response must fail closed");
    assert!(matches!(error, adapter_codex::JsonRpcError::Protocol(_)));
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn request_rejects_no_newline_oversized_response_without_waiting_for_eof() {
    let dir = tempfile::tempdir().expect("tempdir");
    let command = if cfg!(windows) {
        let path = dir.path().join("oversized-no-newline.cmd");
        fs::write(
            &path,
            "@echo off\nset /p line=\n<nul set /p =123456789\nping -n 6 127.0.0.1 >nul\n",
        )
        .expect("fixture");
        path
    } else {
        let path = dir.path().join("oversized-no-newline.sh");
        fs::write(&path, "#!/bin/sh\nread line\nprintf 123456789\nsleep 5\n").expect("fixture");
        path
    };
    let mut client = spawn_app_server(&command, dir.path()).await.expect("spawn");
    client.max_line_bytes_for_test(8);
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        client.request("initialize", serde_json::json!({}), Duration::from_secs(30)),
    )
    .await
    .expect("oversized response must be rejected promptly")
    .expect_err("oversized response must fail closed");
    assert!(matches!(
        error,
        adapter_codex::JsonRpcError::Protocol(message) if message == "response line too large"
    ));
    client.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn next_line_rejects_no_newline_oversized_response_without_waiting_for_eof() {
    let dir = tempfile::tempdir().expect("tempdir");
    let command = if cfg!(windows) {
        let path = dir.path().join("oversized-no-newline-next-line.cmd");
        fs::write(
            &path,
            "@echo off\n<nul set /p =123456789\nping -n 6 127.0.0.1 >nul\n",
        )
        .expect("fixture");
        path
    } else {
        let path = dir.path().join("oversized-no-newline-next-line.sh");
        fs::write(&path, "#!/bin/sh\nprintf 123456789\nsleep 5\n").expect("fixture");
        path
    };
    let mut client = spawn_app_server(&command, dir.path()).await.expect("spawn");
    client.max_line_bytes_for_test(8);
    let error = tokio::time::timeout(Duration::from_secs(1), client.next_line())
        .await
        .expect("oversized line must be rejected promptly")
        .expect_err("oversized response must fail closed");
    assert!(matches!(
        error,
        adapter_codex::JsonRpcError::Protocol(message) if message == "response line too large"
    ));
    client.shutdown().await.expect("shutdown");
}
