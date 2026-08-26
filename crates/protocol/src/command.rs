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
    ReviewMemory,
    BuildContextPack,
    BuildRecoveryPackage,
    HandoffProvider,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContinuityErrorCode {
    ProviderUnavailable,
    ProviderNotSelected,
    LoadoutMismatch,
    MemorySourceRequired,
    ContextPackBudgetExceeded,
    RecoveryTampered,
    RecoverySequenceInvalid,
    PendingApprovalMismatch,
    PermissionExpansion,
    UnsupportedCommand,
}

impl ContinuityErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::ProviderNotSelected => "PROVIDER_NOT_SELECTED",
            Self::LoadoutMismatch => "LOADOUT_MISMATCH",
            Self::MemorySourceRequired => "MEMORY_SOURCE_REQUIRED",
            Self::ContextPackBudgetExceeded => "CONTEXTPACK_BUDGET_EXCEEDED",
            Self::RecoveryTampered => "RECOVERY_TAMPERED",
            Self::RecoverySequenceInvalid => "RECOVERY_SEQUENCE_INVALID",
            Self::PendingApprovalMismatch => "PENDING_APPROVAL_MISMATCH",
            Self::PermissionExpansion => "PERMISSION_EXPANSION",
            Self::UnsupportedCommand => "UNSUPPORTED_COMMAND",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewMemoryRequest {
    pub mission_id: String,
    pub candidate_ids: Vec<String>,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPackRequest {
    pub mission_id: String,
    pub route_id: String,
    pub max_tokens: u32,
    pub expected_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildRecoveryPackageRequest {
    pub mission_id: String,
    pub route_id: String,
    pub contract_version: u64,
    pub checkpoint_id: String,
    pub ledger_sequence: u64,
    pub loadout_fingerprint: String,
    pub context_pack_hash: String,
    pub pending_approval_hash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHandoffRequest {
    pub mission_id: String,
    pub route_id: String,
    pub target_provider: String,
    pub context_pack_hash: String,
    pub pending_approval_hash: Option<String>,
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
            Self::ReviewMemory,
            Self::BuildContextPack,
            Self::BuildRecoveryPackage,
            Self::HandoffProvider,
        ]
    }
}
