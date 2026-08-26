use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Codex,
    Claude,
    OpenCode,
    ZCode,
}

impl ProviderId {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::OpenCode, Self::ZCode];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::ZCode => "zcode",
        }
    }
}

impl Default for ProviderId {
    fn default() -> Self {
        Self::Codex
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "opencode" => Ok(Self::OpenCode),
            "zcode" => Ok(Self::ZCode),
            _ => Err("unknown provider"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    Installed,
    Missing,
    DetectedNotRunnable,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Capability {
    pub structured_events: bool,
    pub resume: bool,
    pub approval: bool,
    pub safe_pause: bool,
    pub terminal_fallback: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentCapabilityReport {
    #[serde(default)]
    pub provider: ProviderId,
    pub agent: String,
    pub version: Option<String>,
    pub install_state: InstallState,
    pub capability: Capability,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    pub executable_hash: Option<String>,
    pub configuration_source: Option<String>,
}

impl AgentCapabilityReport {
    pub fn is_available(&self) -> bool {
        self.install_state == InstallState::Installed
            && self.unavailable_reason.is_none()
            && self.capability.structured_events
    }
}

pub type ProviderCapability = Capability;
