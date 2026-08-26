use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum IpcCommand {
    Handshake,
    CreateMission,
    UpdateMissionContract,
    LaunchRoute,
    RequestSafePause,
    ForceTerminate,
    ResolveApproval,
    SubscribeMission,
    BuildRecoveryPackage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    User,
    Renderer,
    Supervisor,
    Agent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Deny,
    Revoke,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolveApproval {
    pub approval_id: String,
    pub expected_revision: u64,
    pub decision: ApprovalDecision,
    pub mission_id: String,
    pub route_id: String,
    pub contract_version: u64,
    pub loadout_fingerprint: String,
    pub action_digest: String,
    pub now_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub request_id: String,
    pub expected_revision: Option<u64>,
    pub actor: Actor,
    pub command: IpcCommand,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OkResponse<T> {
    pub request_id: String,
    pub revision: u64,
    pub value: T,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub request_id: String,
    pub code: String,
    pub message_key: String,
    pub retryable: bool,
    pub details: Option<serde_json::Value>,
}

impl IpcCommand {
    pub const fn allowlisted() -> &'static [Self] {
        &[
            Self::Handshake,
            Self::CreateMission,
            Self::UpdateMissionContract,
            Self::LaunchRoute,
            Self::RequestSafePause,
            Self::ForceTerminate,
            Self::ResolveApproval,
            Self::SubscribeMission,
            Self::BuildRecoveryPackage,
        ]
    }
}
