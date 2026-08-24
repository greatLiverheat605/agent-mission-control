use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrivingMode {
    Manual,
    Assisted,
    Autopilot,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    pub require_plan_approval: bool,
    pub require_write_approval: bool,
    pub require_final_acceptance: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub max_tokens: Option<u64>,
    pub max_duration_seconds: Option<u64>,
    pub max_cost_micros: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Loadout {
    pub provider: String,
    pub model: String,
    pub configuration_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionContract {
    pub version: u64,
    pub goal: String,
    pub non_goals: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub allowed_paths: Vec<String>,
    pub forbidden_scopes: Vec<String>,
    pub driving_mode: DrivingMode,
    pub approval_policy: ApprovalPolicy,
    pub budget: Budget,
    pub loadout: Loadout,
    pub confirmed_assumptions: Vec<String>,
    pub risks: Vec<String>,
}

impl Default for MissionContract {
    fn default() -> Self {
        Self {
            version: 1,
            goal: String::new(),
            non_goals: Vec::new(),
            acceptance_criteria: Vec::new(),
            allowed_paths: Vec::new(),
            forbidden_scopes: Vec::new(),
            driving_mode: DrivingMode::Manual,
            approval_policy: ApprovalPolicy::default(),
            budget: Budget::default(),
            loadout: Loadout::default(),
            confirmed_assumptions: Vec::new(),
            risks: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContractPatch {
    pub goal: Option<String>,
    pub non_goals: Option<Vec<String>>,
    pub acceptance_criteria: Option<Vec<String>>,
    pub allowed_paths: Option<Vec<String>>,
    pub forbidden_scopes: Option<Vec<String>>,
    pub driving_mode: Option<DrivingMode>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub budget: Option<Budget>,
    pub loadout: Option<Loadout>,
    pub confirmed_assumptions: Option<Vec<String>>,
    pub risks: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchActor {
    User,
    System,
    Agent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContractFieldDiff {
    pub field: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContractDiff {
    pub from_version: u64,
    pub to_version: u64,
    pub fields: Vec<ContractFieldDiff>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum VersionConflict {
    #[error("expected contract version {expected}, found {actual}")]
    ExpectedVersion { expected: u64, actual: u64 },
    #[error("agent actors cannot modify mission contracts")]
    AgentMutationForbidden,
}

impl MissionContract {
    pub fn apply_patch(
        &self,
        expected_version: u64,
        patch: ContractPatch,
        actor: PatchActor,
    ) -> Result<(Self, ContractDiff), VersionConflict> {
        if actor == PatchActor::Agent {
            return Err(VersionConflict::AgentMutationForbidden);
        }
        if expected_version != self.version {
            return Err(VersionConflict::ExpectedVersion {
                expected: expected_version,
                actual: self.version,
            });
        }
        let mut next = self.clone();
        let mut fields = Vec::new();
        macro_rules! update {
            ($field:ident) => {
                if let Some(value) = patch.$field {
                    let before =
                        serde_json::to_value(&next.$field).expect("domain fields serialize");
                    let after = serde_json::to_value(&value).expect("domain fields serialize");
                    if before != after {
                        next.$field = value;
                        fields.push(ContractFieldDiff {
                            field: stringify!($field).to_owned(),
                            before,
                            after,
                        });
                    }
                }
            };
        }
        update!(goal);
        update!(non_goals);
        update!(acceptance_criteria);
        update!(allowed_paths);
        update!(forbidden_scopes);
        update!(driving_mode);
        update!(approval_policy);
        update!(budget);
        update!(loadout);
        update!(confirmed_assumptions);
        update!(risks);
        next.version += 1;
        Ok((
            next,
            ContractDiff {
                from_version: self.version,
                to_version: self.version + 1,
                fields,
            },
        ))
    }
}
