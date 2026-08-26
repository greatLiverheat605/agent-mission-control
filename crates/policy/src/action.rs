use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    Read,
    Write,
    Test,
    Build,
    DependencyInstall,
    NetworkAccess,
    CredentialAccess,
    ContractChange,
    ProviderChange,
    GitPush,
    GitMerge,
    Deploy,
    PermanentDelete,
    Unknown,
}

impl ActionClass {
    pub(crate) const fn is_assisted_low_risk(self) -> bool {
        matches!(self, Self::Read | Self::Write | Self::Test | Self::Build)
    }

    pub(crate) const fn always_requires_user(self) -> bool {
        matches!(
            self,
            Self::CredentialAccess
                | Self::ContractChange
                | Self::ProviderChange
                | Self::GitPush
                | Self::GitMerge
                | Self::Deploy
                | Self::PermanentDelete
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentOrigin {
    Supervisor,
    AdapterStructured,
    TerminalText,
    RepositoryContent,
    WebContent,
    McpContent,
    Memory,
    ToolOutput,
}

impl IntentOrigin {
    pub(crate) const fn is_trusted(self) -> bool {
        matches!(self, Self::Supervisor | Self::AdapterStructured)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Deviation {
    #[default]
    None,
    Suspected,
    Confirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionEvidence {
    pub event_id: String,
    pub action_digest: String,
}

impl ActionEvidence {
    pub(crate) fn is_complete(&self) -> bool {
        !self.event_id.trim().is_empty() && !self.action_digest.trim().is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionIntent {
    pub class: ActionClass,
    pub origin: IntentOrigin,
    pub planned: bool,
    pub within_allowed_paths: bool,
    pub deviation: Deviation,
    pub evidence: ActionEvidence,
}
