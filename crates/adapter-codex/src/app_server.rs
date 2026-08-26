use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

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
    queued_lines: VecDeque<String>,
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
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or(JsonRpcError::Closed)?;
    let stdout = child.stdout.take().ok_or(JsonRpcError::Closed)?;
    let stderr = child.stderr.take().ok_or(JsonRpcError::Closed)?;
    tokio::spawn(drain_stderr(stderr));
    Ok(AppServerClient {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        queued_lines: VecDeque::new(),
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
            let value: Value = serde_json::from_str(line.trim_end())?;
            if value.get("id").is_none() {
                self.queued_lines
                    .push_back(line.trim_end_matches(['\r', '\n']).to_owned());
                continue;
            }
            let response: JsonRpcResponse = serde_json::from_value(value)?;
            if response.id == id {
                if let Some(error) = &response.error {
                    return Err(JsonRpcError::Remote(error.to_string()));
                }
                return Ok(response);
            }
        }
    }

    pub async fn notify(&mut self, method: &str, params: Value) -> Result<(), JsonRpcError> {
        let request = serde_json::json!({"jsonrpc":"2.0","method":method,"params":params});
        let mut encoded = serde_json::to_vec(&request)?;
        encoded.push(b'\n');
        self.stdin.write_all(&encoded).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    pub async fn next_line(&mut self) -> Result<Option<String>, JsonRpcError> {
        if let Some(line) = self.queued_lines.pop_front() {
            return Ok(Some(line));
        }
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).await?;
        if read == 0 {
            return Ok(None);
        }
        if line.len() > self.max_line_bytes {
            return Err(JsonRpcError::Remote("response line too large".to_owned()));
        }
        Ok(Some(line.trim_end_matches(['\r', '\n']).to_owned()))
    }

    pub async fn shutdown(&mut self) -> Result<(), JsonRpcError> {
        if self
            .child
            .try_wait()
            .map_err(JsonRpcError::Start)?
            .is_none()
        {
            self.child.kill().await.map_err(JsonRpcError::Start)?;
        }
        let _ = self.child.wait().await;
        Ok(())
    }
}

async fn drain_stderr(stderr: ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while lines.next_line().await.ok().flatten().is_some() {}
}
