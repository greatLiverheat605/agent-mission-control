use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProbeOptions {
    pub timeout: Duration,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VersionProbe {
    pub executable: PathBuf,
    pub version: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub executable_hash: String,
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("executable is missing")]
    Missing,
    #[error("failed to start executable: {0}")]
    Start(String),
    #[error("failed to read executable metadata: {0}")]
    Metadata(String),
}

pub fn resolve_executable(name: &str, search_path: Option<&Path>) -> Option<PathBuf> {
    let candidate = PathBuf::from(name);
    if candidate.components().count() > 1 || candidate.is_absolute() {
        return candidate.is_file().then_some(candidate);
    }
    let path = search_path
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("PATH").map(PathBuf::from))?;
    for directory in std::env::split_paths(&path) {
        for suffix in executable_suffixes() {
            let path = directory.join(format!("{name}{suffix}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn executable_suffixes() -> &'static [&'static str] {
    if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &["", ".sh"]
    }
}

pub fn probe_executable(
    path: impl AsRef<Path>,
    options: &ProbeOptions,
) -> Result<VersionProbe, ProbeError> {
    let executable = path.as_ref().to_path_buf();
    if !executable.is_file() {
        return Err(ProbeError::Missing);
    }
    let executable_hash = hash_file(&executable)?;
    let mut child = Command::new(&executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ProbeError::Start(error.to_string()))?;
    let deadline = Instant::now() + options.timeout;
    let mut timed_out = false;
    loop {
        match child
            .try_wait()
            .map_err(|error| ProbeError::Start(error.to_string()))?
        {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| ProbeError::Start(error.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let version =
        (!stdout.is_empty()).then(|| stdout.lines().next().unwrap_or_default().to_owned());
    Ok(VersionProbe {
        executable,
        version,
        stdout,
        stderr,
        timed_out,
        executable_hash,
    })
}

fn hash_file(path: &Path) -> Result<String, ProbeError> {
    let bytes = std::fs::read(path).map_err(|error| ProbeError::Metadata(error.to_string()))?;
    Ok(format!("sha256:{}", hex(&Sha256::digest(bytes))))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
