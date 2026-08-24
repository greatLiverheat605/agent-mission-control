use serde::{Deserialize, Serialize};

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
    pub agent: String,
    pub version: Option<String>,
    pub install_state: InstallState,
    pub capability: Capability,
    pub executable_hash: Option<String>,
    pub configuration_source: Option<String>,
}
