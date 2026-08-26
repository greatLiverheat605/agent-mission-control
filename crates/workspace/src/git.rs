use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

#[derive(Clone, Debug)]
pub struct GitRunner {
    binary: PathBuf,
    cwd: PathBuf,
    timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl GitRunner {
    pub fn new(binary: &OsStr, cwd: impl AsRef<Path>, timeout: Duration) -> Result<Self, GitError> {
        if binary.is_empty() || timeout.is_zero() {
            return Err(GitError::InvalidConfiguration);
        }
        let cwd = cwd
            .as_ref()
            .canonicalize()
            .map_err(|source| GitError::Io { source })?;
        if !cwd.is_dir() {
            return Err(GitError::InvalidConfiguration);
        }
        Ok(Self {
            binary: PathBuf::from(binary),
            cwd,
            timeout,
        })
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn run_text(&self, args: &[&str]) -> Result<GitOutput, GitError> {
        let args: Vec<_> = args.iter().map(OsString::from).collect();
        self.run(&args)
    }

    pub fn run(&self, args: &[OsString]) -> Result<GitOutput, GitError> {
        let output = self.run_unchecked(args)?;
        if output.status == 0 {
            Ok(output)
        } else {
            Err(GitError::CommandFailed(output))
        }
    }

    pub fn run_unchecked(&self, args: &[OsString]) -> Result<GitOutput, GitError> {
        let mut child = Command::new(&self.binary)
            .args(args)
            .current_dir(&self.cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| GitError::Io { source })?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let stdout_reader = thread::spawn(move || read_all(stdout));
        let stderr_reader = thread::spawn(move || read_all(stderr));
        let started = Instant::now();

        let status = loop {
            if let Some(status) = child.try_wait().map_err(|source| GitError::Io { source })? {
                break status;
            }
            if started.elapsed() >= self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(GitError::Timeout(self.timeout));
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| GitError::OutputReaderPanicked)??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| GitError::OutputReaderPanicked)??;
        Ok(GitOutput {
            status: status.code().unwrap_or(-1),
            stdout: redact(&String::from_utf8_lossy(&stdout)),
            stderr: redact(&String::from_utf8_lossy(&stderr)),
        })
    }
}

fn read_all(mut reader: impl Read) -> Result<Vec<u8>, GitError> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| GitError::Io { source })?;
    Ok(bytes)
}

fn redact(value: &str) -> String {
    value
        .split_inclusive(char::is_whitespace)
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            if lower.contains("token=")
                || lower.contains("password=")
                || lower.contains("authorization:")
            {
                if part.ends_with(char::is_whitespace) {
                    "[REDACTED]\n"
                } else {
                    "[REDACTED]"
                }
            } else {
                part
            }
        })
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BaselineState {
    Clean { head: String },
    DetachedHead { head: String },
    SelectionRequired { code: &'static str },
}

pub fn inspect_baseline(runner: &GitRunner) -> Result<BaselineState, GitError> {
    let status = runner.run_text(&["status", "--porcelain=v1", "--untracked-files=all"])?;
    if !status.stdout.is_empty() {
        return Ok(BaselineState::SelectionRequired {
            code: "BASELINE_SELECTION_REQUIRED",
        });
    }
    let head = runner
        .run_text(&["rev-parse", "--verify", "HEAD"])?
        .stdout
        .trim()
        .to_owned();
    let symbolic = runner.run_unchecked(
        &["symbolic-ref", "-q", "HEAD"]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>(),
    )?;
    if symbolic.status == 0 {
        Ok(BaselineState::Clean { head })
    } else {
        Ok(BaselineState::DetachedHead { head })
    }
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("invalid Git runner configuration")]
    InvalidConfiguration,
    #[error("Git I/O failed: {source}")]
    Io { source: std::io::Error },
    #[error("Git command timed out after {0:?}")]
    Timeout(Duration),
    #[error("Git command failed with status {0:?}")]
    CommandFailed(GitOutput),
    #[error("Git output reader panicked")]
    OutputReaderPanicked,
}
