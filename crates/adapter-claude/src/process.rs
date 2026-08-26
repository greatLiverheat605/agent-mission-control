use adapter_core::{
    AdapterError, AgentAdapter, AgentCapabilityReport, AgentControl, AgentEvent, AgentHandle,
    EventSink, InstallState, ProviderId, StartAgentRequest,
};
use async_trait::async_trait;
use mission_domain::{EventId, EventKind};
use serde_json::json;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::{io, mem};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, timeout};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

#[derive(Clone, Debug)]
pub struct ClaudeAdapterOptions {
    pub executable: PathBuf,
    pub launcher_args: Vec<OsString>,
}

impl ClaudeAdapterOptions {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            launcher_args: Vec::new(),
        }
    }

    pub fn powershell(script: impl Into<PathBuf>) -> Self {
        Self {
            executable: PathBuf::from("powershell.exe"),
            launcher_args: vec![
                OsString::from("-NoProfile"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-File"),
                script.into().into_os_string(),
            ],
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.args(&self.launcher_args);
        command
    }
}

#[derive(Clone)]
pub struct ClaudeAdapter {
    options: ClaudeAdapterOptions,
    runs: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<AgentControl>>>>,
}

impl ClaudeAdapter {
    pub fn new(options: ClaudeAdapterOptions) -> Self {
        Self {
            options,
            runs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn probe_version(&self) -> Result<String, AdapterError> {
        let mut command = self.options.command();
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = timeout(Duration::from_secs(10), command.output())
            .await
            .map_err(|_| AdapterError::Timeout)?
            .map_err(|error| AdapterError::Unavailable(error.to_string()))?;
        if !output.status.success() {
            return Err(AdapterError::Unavailable(format!(
                "Claude version probe exited with {}",
                output.status
            )));
        }
        let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if version.is_empty() {
            return Err(AdapterError::Unavailable(
                "Claude version probe returned no version".to_owned(),
            ));
        }
        Ok(version.lines().next().unwrap_or_default().to_owned())
    }

    fn unavailable_report(reason: String) -> AgentCapabilityReport {
        AgentCapabilityReport {
            provider: ProviderId::Claude,
            agent: "claude".to_owned(),
            version: None,
            install_state: InstallState::Missing,
            capability: adapter_core::Capability {
                structured_events: false,
                resume: false,
                approval: false,
                safe_pause: false,
                terminal_fallback: false,
            },
            unavailable_reason: Some(reason),
            executable_hash: None,
            configuration_source: None,
        }
    }
}

struct OwnedJob {
    #[cfg(windows)]
    handle: Option<HANDLE>,
}

// Job handles are process-scoped kernel objects and are only accessed by the
// owning adapter task; moving the wrapper between Tokio tasks is safe.
unsafe impl Send for OwnedJob {}
unsafe impl Sync for OwnedJob {}

impl OwnedJob {
    fn from_child(child: &tokio::process::Child) -> io::Result<Self> {
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
                .ok_or_else(|| io::Error::other("Claude process handle unavailable"))?;
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
        if let Some(handle) = self.handle {
            if unsafe { TerminateJobObject(handle, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
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

#[async_trait]
impl AgentAdapter for ClaudeAdapter {
    async fn probe(&self) -> Result<AgentCapabilityReport, AdapterError> {
        match self.probe_version().await {
            Ok(version) => Ok(AgentCapabilityReport {
                provider: ProviderId::Claude,
                agent: "claude".to_owned(),
                version: Some(version),
                install_state: InstallState::Installed,
                capability: adapter_core::Capability {
                    structured_events: true,
                    resume: false,
                    approval: true,
                    safe_pause: true,
                    terminal_fallback: true,
                },
                unavailable_reason: None,
                executable_hash: None,
                configuration_source: Some("claude-cli".to_owned()),
            }),
            Err(error) => Ok(Self::unavailable_report(error.to_string())),
        }
    }

    async fn start(
        &self,
        request: StartAgentRequest,
        sink: EventSink,
    ) -> Result<AgentHandle, AdapterError> {
        crate::loadout::validate_start_request(&request)?;
        let run_id = uuid::Uuid::now_v7().to_string();
        let mut command = self.options.command();
        command
            .args(["--output-format", "stream-json", "--verbose"])
            .current_dir(&request.route_workspace)
            .env_clear()
            .env("MISSION_PROVIDER", "claude")
            .env("MISSION_AGENT_RUN_ID", &run_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in ["SystemRoot", "ComSpec", "PATH", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in &request.approved_environment {
            command.env(name, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| AdapterError::Unavailable(error.to_string()))?;
        let mut owned_job = match OwnedJob::from_child(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill().await;
                return Err(AdapterError::Unavailable(format!(
                    "owned process job unavailable: {error}"
                )));
            }
        };
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AdapterError::Protocol("Claude stdout was not piped".to_owned()))?;
        let stderr = child.stderr.take();
        let mut stdin = child.stdin.take();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (control_tx, mut control_rx) = mpsc::unbounded_channel();
        self.runs
            .lock()
            .await
            .insert(run_id.clone(), control_tx.clone());
        let runs = Arc::clone(&self.runs);
        let event_run_id = run_id.clone();
        tokio::spawn(async move {
            if let Some(stderr) = stderr {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while lines.next_line().await.ok().flatten().is_some() {}
                });
            }
            let mut lines = BufReader::new(stdout).lines();
            let normalizer = crate::normalize::ClaudeNormalizer::default();
            loop {
                tokio::select! {
                    control = control_rx.recv() => match control {
                        Some(AgentControl::SafePause { .. }) => {
                            if let Some(input) = stdin.as_mut() {
                                let _ = input.write_all(&[3]).await;
                                let _ = input.flush().await;
                            }
                            emit(&sink, &events_tx, AgentEvent {
                                event_id: EventId::new(),
                                agent_run_id: Some(event_run_id.clone()),
                                event_kind: EventKind::PauseRequested,
                                payload: json!({"provider":"claude","reason":"safe pause requested"}),
                                requires_safe_pause: false,
                                raw_evidence: None,
                            });
                        }
                        Some(AgentControl::Terminate) | None => {
                            let _ = owned_job.terminate();
                            let _ = child.kill().await;
                            break;
                        }
                    },
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => {
                            let normalized = normalizer.normalize_line_lossless(&line);
                            let terminal = normalized.terminal;
                            let mut event = normalized.event;
                            event.agent_run_id = Some(event_run_id.clone());
                            emit(&sink, &events_tx, event);
                            if terminal {
                                break;
                            }
                        }
                        Ok(None) => {
                            emit(&sink, &events_tx, AgentEvent {
                                event_id: EventId::new(),
                                agent_run_id: Some(event_run_id.clone()),
                                event_kind: EventKind::Unknown("adapter.stdout_eof".to_owned()),
                                payload: json!({"provider":"claude","reason":"stdout_eof"}),
                                requires_safe_pause: true,
                                raw_evidence: None,
                            });
                            break;
                        }
                        Err(error) => {
                            emit(&sink, &events_tx, AgentEvent {
                                event_id: EventId::new(),
                                agent_run_id: Some(event_run_id.clone()),
                                event_kind: EventKind::Unknown("adapter.protocol_error".to_owned()),
                                payload: json!({"provider":"claude","error":error.to_string()}),
                                requires_safe_pause: true,
                                raw_evidence: None,
                            });
                            break;
                        }
                    }
                }
            }
            let _ = owned_job.terminate();
            let _ = child.kill().await;
            let _ = child.wait().await;
            runs.lock().await.remove(&event_run_id);
        });
        Ok(AgentHandle::with_control(run_id, events_rx, control_tx))
    }

    async fn request_safe_pause(&self, run_id: &str) -> Result<(), AdapterError> {
        let sender = self
            .runs
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| AdapterError::Unavailable("unknown run".to_owned()))?;
        sender
            .send(AgentControl::SafePause {
                reason: "adapter request".to_owned(),
            })
            .map_err(|_| AdapterError::Unavailable("agent run ended".to_owned()))
    }

    async fn terminate_owned_tree(&self, run_id: &str) -> Result<(), AdapterError> {
        let sender = self
            .runs
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| AdapterError::Unavailable("unknown run".to_owned()))?;
        sender
            .send(AgentControl::Terminate)
            .map_err(|_| AdapterError::Unavailable("agent run ended".to_owned()))
    }
}

fn emit(sink: &EventSink, events: &mpsc::UnboundedSender<AgentEvent>, event: AgentEvent) {
    let _ = sink.send(event.clone());
    let _ = events.send(event);
}
