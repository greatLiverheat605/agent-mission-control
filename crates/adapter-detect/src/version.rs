use crate::path_probe::{ProbeOptions, VersionProbe, probe_executable, resolve_executable};
use adapter_core::{AgentCapabilityReport, Capability, InstallState, ProviderId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Codex,
    Claude,
    OpenCode,
    ZCode,
}

impl AgentKind {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::OpenCode, Self::ZCode];
    pub const fn executable(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::ZCode => "zcode",
        }
    }
    pub const fn runnable(self) -> bool {
        matches!(self, Self::Codex | Self::Claude)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub agent: AgentKind,
    pub report: AgentCapabilityReport,
    pub executable: Option<PathBuf>,
    pub reason: Option<String>,
}

pub fn detect(agent: AgentKind, search_path: Option<&Path>, options: &ProbeOptions) -> Detection {
    let executable = resolve_executable(agent.executable(), search_path);
    let Some(path) = executable.clone() else {
        return missing(agent, "executable not found");
    };
    let probe = match probe_executable(&path, options) {
        Ok(probe) => probe,
        Err(error) => return missing_with_path(agent, executable, error.to_string()),
    };
    from_probe(agent, probe)
}

pub fn detect_all(search_path: Option<&Path>, options: &ProbeOptions) -> Vec<Detection> {
    AgentKind::ALL
        .into_iter()
        .map(|agent| detect(agent, search_path, options))
        .collect()
}

fn from_probe(agent: AgentKind, probe: VersionProbe) -> Detection {
    let unknown_version = probe.version.is_none() || probe.timed_out;
    let install_state = if !agent.runnable() {
        InstallState::DetectedNotRunnable
    } else if unknown_version {
        InstallState::Unknown
    } else {
        InstallState::Installed
    };
    let reason = if probe.timed_out {
        Some("--version probe timed out".to_owned())
    } else if probe.version.is_none() {
        Some("version output is unknown".to_owned())
    } else {
        None
    };
    Detection {
        agent,
        report: AgentCapabilityReport {
            provider: provider_id(agent),
            agent: agent.executable().to_owned(),
            version: probe.version,
            install_state,
            capability: Capability {
                structured_events: agent.runnable() && !unknown_version,
                resume: agent == AgentKind::Codex && !unknown_version,
                approval: agent.runnable() && !unknown_version,
                safe_pause: agent.runnable() && !unknown_version,
                terminal_fallback: agent.runnable() && !unknown_version,
            },
            unavailable_reason: if !agent.runnable() {
                Some("provider detected but runtime adapter is unavailable".to_owned())
            } else if unknown_version {
                Some("provider version is unknown".to_owned())
            } else {
                None
            },
            executable_hash: Some(probe.executable_hash),
            configuration_source: Some("local_cli".to_owned()),
        },
        executable: Some(probe.executable),
        reason,
    }
}

fn missing(agent: AgentKind, reason: &str) -> Detection {
    missing_with_path(agent, None, reason.to_owned())
}
fn missing_with_path(agent: AgentKind, executable: Option<PathBuf>, reason: String) -> Detection {
    Detection {
        agent,
        report: AgentCapabilityReport {
            provider: provider_id(agent),
            agent: agent.executable().to_owned(),
            version: None,
            install_state: InstallState::Missing,
            capability: Capability {
                structured_events: false,
                resume: false,
                approval: false,
                safe_pause: false,
                terminal_fallback: false,
            },
            unavailable_reason: Some(reason.clone()),
            executable_hash: None,
            configuration_source: None,
        },
        executable,
        reason: Some(reason),
    }
}

const fn provider_id(agent: AgentKind) -> ProviderId {
    match agent {
        AgentKind::Codex => ProviderId::Codex,
        AgentKind::Claude => ProviderId::Claude,
        AgentKind::OpenCode => ProviderId::OpenCode,
        AgentKind::ZCode => ProviderId::ZCode,
    }
}
