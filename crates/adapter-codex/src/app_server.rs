use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use std::{io, mem};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

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
    #[error("app-server protocol limit exceeded: {0}")]
    Protocol(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Value,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

pub struct AppServerClient {
    child: Child,
    owned_job: OwnedJob,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    queued_lines: VecDeque<String>,
    queued_responses: VecDeque<JsonRpcResponse>,
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
    let owned_job = OwnedJob::from_child(&child).map_err(JsonRpcError::Start)?;
    let stdin = child.stdin.take().ok_or(JsonRpcError::Closed)?;
    let stdout = child.stdout.take().ok_or(JsonRpcError::Closed)?;
    let stderr = child.stderr.take().ok_or(JsonRpcError::Closed)?;
    tokio::spawn(drain_stderr(stderr));
    Ok(AppServerClient {
        child,
        owned_job,
        stdin,
        stdout: BufReader::new(stdout),
        queued_lines: VecDeque::new(),
        queued_responses: VecDeque::new(),
        next_id: 1,
        max_line_bytes: 4 * 1024 * 1024,
    })
}

impl AppServerClient {
    /// Adjust the framing limit for deterministic protocol tests and policy probes.
    pub fn max_line_bytes_for_test(&mut self, max: usize) {
        self.max_line_bytes = max;
    }

    pub fn child_id(&self) -> Option<u32> {
        self.child.id()
    }

    pub(crate) fn requeue_line(&mut self, line: String) {
        self.queued_lines.push_front(line);
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
        if let Some(index) = self
            .queued_responses
            .iter()
            .position(|response| response.id == id)
        {
            return Ok(self
                .queued_responses
                .remove(index)
                .expect("response index exists"));
        }
        loop {
            let line = tokio::time::timeout(
                timeout,
                read_bounded_line(&mut self.stdout, self.max_line_bytes),
            )
            .await
            .map_err(|_| JsonRpcError::Timeout)??;
            let Some(line) = line else {
                return Err(JsonRpcError::Closed);
            };
            let value = parse_json_line(&line, self.max_line_bytes)?;
            if value.get("id").is_none() || value.get("method").is_some() {
                if self.queued_lines.len() >= 1024 {
                    return Err(JsonRpcError::Protocol("notification queue full".to_owned()));
                }
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
            if self.queued_responses.len() >= 1024 {
                return Err(JsonRpcError::Protocol("response queue full".to_owned()));
            }
            self.queued_responses.push_back(response);
        }
    }

    pub async fn respond(&mut self, id: Value, result: Value) -> Result<(), JsonRpcError> {
        let response = serde_json::json!({"jsonrpc":"2.0","id":id,"result":result});
        let mut encoded = serde_json::to_vec(&response)?;
        encoded.push(b'\n');
        self.stdin.write_all(&encoded).await?;
        self.stdin.flush().await?;
        Ok(())
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
        loop {
            let Some(line) = read_bounded_line(&mut self.stdout, self.max_line_bytes).await? else {
                return Ok(None);
            };
            let value = parse_json_line(&line, self.max_line_bytes)?;
            if value.get("id").is_some() && value.get("method").is_none() {
                let response: JsonRpcResponse = serde_json::from_value(value)?;
                if self.queued_responses.len() >= 1024 {
                    return Err(JsonRpcError::Protocol("response queue full".to_owned()));
                }
                self.queued_responses.push_back(response);
                continue;
            }
            return Ok(Some(line.trim_end_matches(['\r', '\n']).to_owned()));
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), JsonRpcError> {
        if self
            .child
            .try_wait()
            .map_err(JsonRpcError::Start)?
            .is_none()
        {
            self.owned_job.terminate().map_err(JsonRpcError::Start)?;
            if self
                .child
                .try_wait()
                .map_err(JsonRpcError::Start)?
                .is_none()
            {
                self.child.kill().await.map_err(JsonRpcError::Start)?;
            }
        }
        let _ = self.child.wait().await;
        Ok(())
    }
}

struct OwnedJob {
    #[cfg(windows)]
    handle: Option<HANDLE>,
}

// Job handles are process-scoped kernel objects and are owned by the app-server task.
unsafe impl Send for OwnedJob {}
unsafe impl Sync for OwnedJob {}

impl OwnedJob {
    fn from_child(child: &Child) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            } != 0;
            let raw_handle = child
                .raw_handle()
                .ok_or_else(|| io::Error::other("Codex process handle unavailable"))?;
            let assigned =
                configured && unsafe { AssignProcessToJobObject(handle, raw_handle.cast()) } != 0;
            if !assigned {
                unsafe { CloseHandle(handle) };
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                handle: Some(handle),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(Self {})
        }
    }

    fn terminate(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        if let Some(handle) = self.handle
            && unsafe { TerminateJobObject(handle, 1) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for OwnedJob {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe { CloseHandle(handle) };
        }
    }
}

fn parse_json_line(line: &str, max_bytes: usize) -> Result<Value, JsonRpcError> {
    if line.len() > max_bytes {
        return Err(JsonRpcError::Protocol("response line too large".to_owned()));
    }
    let value: Value = serde_json::from_str(line.trim_end())?;
    if json_depth(&value) > 64 {
        return Err(JsonRpcError::Protocol("JSON nesting too deep".to_owned()));
    }
    Ok(value)
}

/// Read one newline-delimited frame without allowing an unterminated frame to
/// grow beyond the protocol limit. `read_line` cannot enforce this because it
/// buffers until newline or EOF first.
async fn read_bounded_line(
    reader: &mut BufReader<ChildStdout>,
    max_bytes: usize,
) -> Result<Option<String>, JsonRpcError> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            return String::from_utf8(bytes).map(Some).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "response is not UTF-8").into()
            });
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let bytes_to_consume = newline.map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(bytes_to_consume) > max_bytes {
            let remaining = max_bytes
                .saturating_add(1)
                .saturating_sub(bytes.len())
                .min(available.len());
            reader.consume(remaining.max(1));
            return Err(JsonRpcError::Protocol("response line too large".to_owned()));
        }

        bytes.extend_from_slice(&available[..bytes_to_consume]);
        reader.consume(bytes_to_consume);
        if newline.is_some() {
            return String::from_utf8(bytes).map(Some).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "response is not UTF-8").into()
            });
        }
    }
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.iter().map(json_depth).max().unwrap_or(0) + 1,
        Value::Object(values) => values.values().map(json_depth).max().unwrap_or(0) + 1,
        _ => 0,
    }
}

async fn drain_stderr(stderr: ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while lines.next_line().await.ok().flatten().is_some() {}
}
