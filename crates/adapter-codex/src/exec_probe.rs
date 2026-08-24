use crate::normalize::{CodexNormalizer, NormalizedEvent};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum ExecProbeError {
    #[error("failed to start codex exec: {0}")]
    Start(#[from] std::io::Error),
    #[error("codex exec probe timed out")]
    Timeout,
}

#[derive(Debug)]
pub struct ExecProbeResult {
    pub command: PathBuf,
    pub events: Vec<NormalizedEvent>,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

pub async fn run_exec_probe(
    executable: impl AsRef<Path>,
    project_root: impl AsRef<Path>,
    prompt: &str,
    timeout: Duration,
) -> Result<ExecProbeResult, ExecProbeError> {
    let mut child = Command::new(executable.as_ref())
        .arg("exec")
        .arg("--json")
        .arg("--sandbox")
        .arg("read-only")
        .arg("--cd")
        .arg(project_root.as_ref())
        .arg(prompt)
        .current_dir(project_root)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("missing stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("missing stderr"))?;
    let normalizer = CodexNormalizer::default();
    let read_stdout = async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut events = Vec::new();
        while let Some(line) = lines.next_line().await? {
            events.push(normalizer.normalize_line_lossless(&line));
        }
        Ok::<_, std::io::Error>(events)
    };
    let collect_stderr = async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut output = String::new();
        while let Some(line) = lines.next_line().await? {
            output.push_str(&line);
            output.push('\n');
        }
        Ok::<_, std::io::Error>(output)
    };
    let (events, stderr, status) = tokio::time::timeout(timeout, async {
        let (events, stderr) = tokio::try_join!(read_stdout, collect_stderr)?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((events, stderr, status))
    })
    .await
    .map_err(|_| ExecProbeError::Timeout)??;
    Ok(ExecProbeResult {
        command: executable.as_ref().to_path_buf(),
        events,
        stderr,
        exit_code: status.code(),
    })
}
