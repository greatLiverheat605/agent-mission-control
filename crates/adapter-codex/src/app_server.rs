use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[derive(Debug, Error)]
pub enum JsonRpcError {
    #[error("failed to start app-server: {0}")]
    Start(#[from] std::io::Error),
    #[error("app-server request timed out")]
    Timeout,
    #[error("app-server returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("app-server closed stdout")]
    Closed,
    #[error("app-server returned JSON-RPC error: {0}")]
    Remote(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Value,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

pub struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    max_line_bytes: usize,
}

pub async fn spawn_app_server(
    executable: impl AsRef<Path>,
    cwd: impl AsRef<Path>,
) -> Result<AppServerClient, JsonRpcError> {
    let mut command = Command::new(executable.as_ref());
    command
        .arg("app-server")
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or(JsonRpcError::Closed)?;
    let stdout = child.stdout.take().ok_or(JsonRpcError::Closed)?;
    Ok(AppServerClient {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
        max_line_bytes: 4 * 1024 * 1024,
    })
}

impl AppServerClient {
    pub fn child_id(&self) -> Option<u32> {
        self.child.id()
    }

    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<JsonRpcResponse, JsonRpcError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let mut encoded = serde_json::to_vec(&request)?;
        encoded.push(b'\n');
        self.stdin.write_all(&encoded).await?;
        self.stdin.flush().await?;
        let mut line = String::new();
        loop {
            line.clear();
            let read = tokio::time::timeout(timeout, self.stdout.read_line(&mut line))
                .await
                .map_err(|_| JsonRpcError::Timeout)??;
            if read == 0 {
                return Err(JsonRpcError::Closed);
            }
            if line.len() > self.max_line_bytes {
                return Err(JsonRpcError::Remote("response line too large".to_owned()));
            }
            let response: JsonRpcResponse = serde_json::from_str(line.trim_end())?;
            if response.id == id {
                if let Some(error) = &response.error {
                    return Err(JsonRpcError::Remote(error.to_string()));
                }
                return Ok(response);
            }
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), JsonRpcError> {
        self.child.kill().await.map_err(JsonRpcError::Start)
    }
}
