use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adapter_claude::{ClaudeAdapter, ClaudeAdapterOptions};
use adapter_codex::CodexAdapter;
use adapter_core::{
    AgentAdapter, AgentCapabilityReport, AgentHandle, Capability, InstallState, LoadoutSnapshot,
    ProviderId, StartAgentRequest,
};
use cc_switch_bridge::CcSwitchBridge;
use mission_domain::{EventEnvelope, EventId, EventKind, MissionId, RouteId};
use mission_ledger::{
    ArchivePlan, BlobRef, DeleteImpactPlan, EncryptedBlobStore, EncryptedLedger, KeyStore,
    StorageBudget, WindowsCredentialKeyStore,
};
use mission_memory::{RecoveryConstraints, RecoveryInput, RecoveryPackage, build_recovery_package};
use mission_policy::{
    BudgetDimension, BudgetLimits, BudgetSignal, BudgetTracker, UnknownUsagePolicy, UsageRecord,
    UsageSample,
};
use mission_protocol::command::{Actor, ApprovalDecision, ApprovalGrantScope, ResolveApproval};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;

pub const MISSION_COMMANDS: [&str; 20] = [
    "create_mission",
    "update_mission_contract",
    "launch_route",
    "subscribe_mission",
    "request_safe_pause",
    "request_force_termination",
    "force_terminate",
    "resolve_approval",
    "build_recovery_package",
    "verify_recovery",
    "resolve_recovery",
    "review_memory",
    "handoff_provider",
    "provider_capabilities",
    "storage_preview",
    "export_preview",
    "diagnostic_preview",
    "archive_mission",
    "delete_mission",
    "materialize_export",
];

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionCommandRequest {
    #[serde(default)]
    pub provider: Option<ProviderId>,
    #[serde(default)]
    pub target_provider: Option<ProviderId>,
    #[serde(default)]
    pub loadout: Option<LoadoutSnapshot>,
    pub mission_id: Option<String>,
    pub route_id: Option<String>,
    pub expected_version: Option<u64>,
    pub project_root: Option<String>,
    pub goal: Option<String>,
    pub reason: Option<String>,
    pub confirmation_token: Option<String>,
    pub approval_id: Option<String>,
    pub approval_decision: Option<String>,
    pub approval_scope: Option<String>,
    pub action_digest: Option<String>,
    pub expected_revision: Option<u64>,
    pub now_ms: Option<u64>,
    #[serde(default)]
    pub loadout_fingerprint: Option<String>,
    #[serde(default)]
    pub resume_token: Option<String>,
    #[serde(default)]
    pub checkpoint_id: Option<String>,
    #[serde(default)]
    pub contract_version: Option<u64>,
    #[serde(default)]
    pub ledger_sequence: Option<u64>,
    #[serde(default)]
    pub context_pack_hash: Option<String>,
    #[serde(default)]
    pub pending_approval_hash: Option<String>,
    #[serde(default)]
    pub memory_id: Option<String>,
    #[serde(default)]
    pub memory_decision: Option<String>,
    #[serde(default)]
    pub project_limit_bytes: Option<u64>,
    #[serde(default)]
    pub global_limit_bytes: Option<u64>,
    #[serde(default)]
    pub recovery_package: Option<Value>,
    #[serde(default)]
    pub recovery_decision: Option<String>,
    #[serde(default)]
    pub budget: Option<MissionBudgetRequest>,
    #[serde(default)]
    pub impact_hash: Option<String>,
    #[serde(default)]
    pub archive_plan: Option<Value>,
    #[serde(default)]
    pub delete_plan: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionBudgetRequest {
    #[serde(default)]
    pub tokens: Option<u64>,
    #[serde(default)]
    pub money_micros: Option<u64>,
    #[serde(default)]
    pub wall_clock_ms: Option<u64>,
    #[serde(default)]
    pub changed_lines: Option<u64>,
    #[serde(default)]
    pub changed_files: Option<u64>,
    #[serde(default)]
    pub model_calls: Option<u64>,
}

const DEFAULT_BUDGET_LIMITS: BudgetLimits = BudgetLimits {
    tokens: 100_000,
    money_micros: 10_000_000,
    wall_clock: Duration::from_secs(30 * 60),
    changed_lines: 10_000,
    changed_files: 1_000,
    model_calls: 100,
};

impl MissionBudgetRequest {
    fn limits(&self) -> BudgetLimits {
        BudgetLimits {
            tokens: self.tokens.unwrap_or(DEFAULT_BUDGET_LIMITS.tokens),
            money_micros: self
                .money_micros
                .unwrap_or(DEFAULT_BUDGET_LIMITS.money_micros),
            wall_clock: Duration::from_millis(
                self.wall_clock_ms
                    .unwrap_or(DEFAULT_BUDGET_LIMITS.wall_clock.as_millis() as u64),
            ),
            changed_lines: self
                .changed_lines
                .unwrap_or(DEFAULT_BUDGET_LIMITS.changed_lines),
            changed_files: self
                .changed_files
                .unwrap_or(DEFAULT_BUDGET_LIMITS.changed_files),
            model_calls: self
                .model_calls
                .unwrap_or(DEFAULT_BUDGET_LIMITS.model_calls),
        }
    }
}

fn default_budget_tracker() -> BudgetTracker {
    BudgetTracker::new(1, DEFAULT_BUDGET_LIMITS, UnknownUsagePolicy::Pause)
}

fn budget_tracker_from_events(events: &[EventEnvelope]) -> BudgetTracker {
    let limits = events
        .iter()
        .rev()
        .find(|event| event.kind == EventKind::ContractUpdated)
        .and_then(|event| event.payload.get("budget"))
        .and_then(|value| serde_json::from_value::<MissionBudgetRequest>(value.clone()).ok())
        .map_or(DEFAULT_BUDGET_LIMITS, |request| request.limits());
    BudgetTracker::new(1, limits, UnknownUsagePolicy::Pause)
}

fn usage_record_from_event(
    event: &adapter_core::AgentEvent,
    previous_tokens: &mut u64,
) -> Option<UsageRecord> {
    if event.payload.get("native_type").and_then(Value::as_str) != Some("thread/tokenUsage/updated")
    {
        return None;
    }
    let usage = event
        .payload
        .get("tokenUsage")
        .or_else(|| {
            event
                .payload
                .get("data")
                .and_then(|value| value.get("tokenUsage"))
        })
        .and_then(Value::as_object);
    let total = usage
        .and_then(|value| value.get("total"))
        .and_then(Value::as_object)
        .and_then(|value| value.get("totalTokens"))
        .and_then(Value::as_u64);
    match total {
        Some(total) => {
            let delta = total.saturating_sub(*previous_tokens);
            *previous_tokens = (*previous_tokens).max(total);
            Some(UsageRecord::Sample(
                UsageSample::tokens(delta).with(BudgetDimension::ModelCalls, 1),
            ))
        }
        None => Some(UsageRecord::Unknown(BudgetDimension::Tokens)),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionCommandResult {
    pub accepted: bool,
    pub mission_id: Option<String>,
    pub route_id: Option<String>,
    pub sequence: Option<u64>,
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_token: Option<String>,
    #[serde(default)]
    pub provider: Option<ProviderId>,
    #[serde(default)]
    pub capability: Option<AgentCapabilityReport>,
    #[serde(default)]
    pub events: Vec<Value>,
    #[serde(default)]
    pub recovery_package: Option<Value>,
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityReport>,
    #[serde(default)]
    pub cc_switch: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default)]
    pub recovery_required: bool,
}

#[derive(Clone, Debug)]
struct PendingServerRequest {
    run_id: String,
    request_id: Value,
    provider: ProviderId,
}

pub fn validate_mission_request(request: &MissionCommandRequest) -> Result<(), &'static str> {
    if request
        .project_root
        .as_ref()
        .is_some_and(|value| value.len() > 4096)
    {
        return Err("PROJECT_ROOT_TOO_LONG");
    }
    if request
        .goal
        .as_ref()
        .is_some_and(|value| value.len() > 32_000)
    {
        return Err("GOAL_TOO_LONG");
    }
    if request
        .confirmation_token
        .as_ref()
        .is_some_and(|value| value.len() > 256)
    {
        return Err("CONFIRMATION_TOKEN_TOO_LONG");
    }
    if request
        .loadout_fingerprint
        .as_ref()
        .is_some_and(|value| value.len() > 512)
    {
        return Err("LOADOUT_FINGERPRINT_TOO_LONG");
    }
    if request
        .resume_token
        .as_ref()
        .is_some_and(|value| value.len() > 4096)
    {
        return Err("RESUME_TOKEN_TOO_LONG");
    }
    Ok(())
}

type MissionActorState = crate::mission_actor::MissionActor<EncryptedLedger>;

pub struct MissionService {
    missions: Arc<Mutex<HashMap<String, MissionActorState>>>,
    budgets: Arc<Mutex<HashMap<String, BudgetTracker>>>,
    runs: Arc<Mutex<HashMap<String, String>>>,
    run_providers: Arc<Mutex<HashMap<String, ProviderId>>>,
    pending_server_requests: Arc<Mutex<HashMap<String, PendingServerRequest>>>,
    adapters: Arc<HashMap<ProviderId, Arc<dyn AgentAdapter>>>,
    loadouts: Arc<Mutex<crate::loadout_monitor::LoadoutMonitor>>,
    ledger_path: PathBuf,
    last_ui_seen: Mutex<Instant>,
    in_flight_commands: AtomicUsize,
    runtime: Runtime,
}

impl MissionService {
    pub fn new(data_dir: PathBuf) -> Result<Arc<Self>, String> {
        let executable = std::env::var_os("MISSION_CODEX_EXECUTABLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("codex"));
        let claude_executable = std::env::var_os("MISSION_CLAUDE_EXECUTABLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("claude"));
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("MISSION_RUNTIME_FAILED:{error}"))?;
        let codex: Arc<dyn AgentAdapter> = Arc::new(CodexAdapter::new(executable));
        let claude: Arc<dyn AgentAdapter> = Arc::new(ClaudeAdapter::new(
            ClaudeAdapterOptions::new(claude_executable),
        ));
        let adapters = HashMap::from([(ProviderId::Codex, codex), (ProviderId::Claude, claude)]);
        let service = Arc::new(Self {
            missions: Arc::new(Mutex::new(HashMap::new())),
            budgets: Arc::new(Mutex::new(HashMap::new())),
            runs: Arc::new(Mutex::new(HashMap::new())),
            run_providers: Arc::new(Mutex::new(HashMap::new())),
            pending_server_requests: Arc::new(Mutex::new(HashMap::new())),
            adapters: Arc::new(adapters),
            loadouts: Arc::new(Mutex::new(crate::loadout_monitor::LoadoutMonitor::default())),
            ledger_path: data_dir.join("mission-ledger.db"),
            last_ui_seen: Mutex::new(Instant::now()),
            in_flight_commands: AtomicUsize::new(0),
            runtime,
        });
        service.restore_existing()?;
        Self::start_watchdog(Arc::clone(&service));
        Ok(service)
    }

    fn open_ledger(&self) -> Result<EncryptedLedger, String> {
        EncryptedLedger::open(
            &self.ledger_path,
            "mission-control-desktop-v1",
            WindowsCredentialKeyStore,
        )
        .map_err(|error| format!("LEDGER_OPEN_FAILED:{error}"))
    }

    fn restore_existing(&self) -> Result<(), String> {
        let ledger = self.open_ledger()?;
        ledger
            .integrity_report()
            .map_err(|error| format!("LEDGER_RECOVERY_REQUIRED:{error}"))?;
        let mission_ids = ledger
            .mission_ids()
            .map_err(|error| format!("MISSION_RESTORE_FAILED:{error}"))?;
        let mut missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        for mission_id in mission_ids {
            let ledger = self.open_ledger()?;
            let events = ledger
                .replay_events(&mission_id)
                .map_err(|error| format!("MISSION_RESTORE_FAILED:{error}"))?;
            let route_id = events
                .first()
                .map(|event| event.route_id)
                .ok_or_else(|| "MISSION_RESTORE_ROUTE_MISSING".to_owned())?;
            if let Ok(mut budgets) = self.budgets.lock() {
                budgets.insert(mission_id.to_string(), budget_tracker_from_events(&events));
            }
            if let std::collections::hash_map::Entry::Vacant(entry) =
                missions.entry(mission_id.to_string())
            {
                let actor =
                    crate::mission_actor::MissionActor::try_new(mission_id, route_id, ledger)
                        .map_err(|error| format!("MISSION_RESTORE_FAILED:{error}"))?;
                entry.insert(actor);
            }
        }
        Ok(())
    }

    fn start_watchdog(service: Arc<Self>) {
        let runtime = service.runtime.handle().clone();
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                let expired = service
                    .last_ui_seen
                    .lock()
                    .map(|last_seen| last_seen.elapsed() > Duration::from_secs(2))
                    .unwrap_or(true);
                let command_in_flight = service.in_flight_commands.load(Ordering::Acquire) > 0;
                if expired && !command_in_flight {
                    service.pause_for_ui_disconnect().await;
                }
            }
        });
    }

    pub fn touch_ui(&self) {
        if let Ok(mut last_seen) = self.last_ui_seen.lock() {
            *last_seen = Instant::now();
        }
        if let Ok(mut missions) = self.missions.lock() {
            for actor in missions.values_mut() {
                let _ = actor.set_ui_connected(true);
            }
        }
    }

    async fn pause_for_ui_disconnect(&self) {
        let mission_ids = {
            let Ok(mut missions) = self.missions.lock() else {
                return;
            };
            let mut mission_ids = Vec::new();
            for (mission_id, actor) in missions.iter_mut() {
                let was_connected = actor.ui_connected();
                let _ = actor.set_ui_connected(false);
                if was_connected {
                    mission_ids.push(mission_id.clone());
                }
            }
            mission_ids
        };
        for mission_id in mission_ids {
            if let Ok(Some(run_id)) = self.active_run_id_key(&mission_id) {
                let provider = self
                    .run_providers
                    .lock()
                    .ok()
                    .and_then(|providers| providers.get(&mission_id).copied())
                    .unwrap_or_default();
                if let Some(adapter) = self.adapters.get(&provider) {
                    let _ = adapter.request_safe_pause(&run_id).await;
                }
            }
        }
    }

    pub fn dispatch_json(&self, command: &str, request: Value) -> Result<Value, String> {
        if !MISSION_COMMANDS.contains(&command) {
            return Err("COMMAND_NOT_ALLOWED".to_owned());
        }
        let request: MissionCommandRequest =
            serde_json::from_value(request).map_err(|_| "MISSION_REQUEST_INVALID".to_owned())?;
        validate_mission_request(&request).map_err(str::to_owned)?;
        self.in_flight_commands.fetch_add(1, Ordering::AcqRel);
        let result = self.runtime.block_on(self.dispatch(command, request));
        self.in_flight_commands.fetch_sub(1, Ordering::AcqRel);
        let result = result?;
        serde_json::to_value(result).map_err(|_| "MISSION_RESULT_SERIALIZE_FAILED".to_owned())
    }

    async fn dispatch(
        &self,
        command: &str,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        match command {
            "create_mission" => self.create_mission(request),
            "update_mission_contract" => self.update_mission_contract(request),
            "launch_route" => self.launch_route(request).await,
            "subscribe_mission" => self.subscribe_mission(request),
            "request_safe_pause" => self.request_safe_pause(request).await,
            "request_force_termination" => self.request_force_termination(request).await,
            "force_terminate" => self.force_terminate(request).await,
            "resolve_approval" => self.resolve_approval(request).await,
            "build_recovery_package" => self.build_recovery_package(request),
            "verify_recovery" => self.verify_recovery(request),
            "resolve_recovery" => self.resolve_recovery(request).await,
            "review_memory" => self.review_memory(request),
            "handoff_provider" => self.handoff_provider(request).await,
            "provider_capabilities" => self.provider_capabilities().await,
            "storage_preview" => self.storage_preview(request),
            "export_preview" => self.export_preview(request),
            "diagnostic_preview" => self.diagnostic_preview(request),
            "archive_mission" => self.archive_mission(request),
            "delete_mission" => self.delete_mission(request),
            "materialize_export" => self.materialize_export(request),
            _ => Err("COMMAND_NOT_ALLOWED".to_owned()),
        }
    }

    fn mission_id(request: &MissionCommandRequest) -> Result<MissionId, String> {
        request
            .mission_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "MISSION_ID_INVALID".to_owned())?
            .ok_or_else(|| "MISSION_ID_REQUIRED".to_owned())
    }

    fn route_id(request: &MissionCommandRequest) -> Result<RouteId, String> {
        request
            .route_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "ROUTE_ID_INVALID".to_owned())?
            .ok_or_else(|| "ROUTE_ID_REQUIRED".to_owned())
    }

    fn result(
        mission_id: Option<String>,
        actor: Option<&MissionActorState>,
    ) -> Result<MissionCommandResult, String> {
        let (sequence, events) = match actor {
            Some(actor) => (
                Some(actor.sequence()),
                actor
                    .replay_after(0)
                    .into_iter()
                    .map(|event| {
                        serde_json::to_value(event).map_err(|_| "EVENT_SERIALIZE_FAILED".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            None => (None, Vec::new()),
        };
        Ok(MissionCommandResult {
            accepted: true,
            mission_id,
            route_id: actor.and_then(|actor| {
                actor
                    .replay_after(0)
                    .first()
                    .map(|event| event.route_id.to_string())
            }),
            sequence,
            error_code: None,
            confirmation_token: None,
            provider: None,
            capability: None,
            events,
            recovery_package: None,
            capabilities: Vec::new(),
            cc_switch: None,
            data: None,
            recovery_required: actor.is_some_and(|actor| actor.requires_recovery()),
        })
    }

    fn lifecycle_result(data: Value) -> Result<MissionCommandResult, String> {
        Ok(MissionCommandResult {
            accepted: true,
            mission_id: None,
            route_id: None,
            sequence: None,
            error_code: None,
            confirmation_token: None,
            provider: None,
            capability: None,
            events: Vec::new(),
            recovery_package: None,
            capabilities: Vec::new(),
            cc_switch: None,
            data: Some(data),
            recovery_required: false,
        })
    }

    fn storage_preview(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let ledger = self.open_ledger()?;
        let budget = StorageBudget::new(request.project_limit_bytes, request.global_limit_bytes);
        let plan = ledger
            .retention_plan_for_project(&budget, request.project_root.as_deref())
            .map_err(|error| format!("STORAGE_PREVIEW_FAILED:{error}"))?;
        Self::lifecycle_result(
            serde_json::to_value(plan)
                .map_err(|_| "STORAGE_PREVIEW_SERIALIZE_FAILED".to_owned())?,
        )
    }

    fn export_preview(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let ledger = self.open_ledger()?;
        let preview = ledger
            .export_preview(&mission_id)
            .map_err(|error| format!("EXPORT_PREVIEW_FAILED:{error}"))?;
        Self::lifecycle_result(
            serde_json::to_value(preview)
                .map_err(|_| "EXPORT_PREVIEW_SERIALIZE_FAILED".to_owned())?,
        )
    }

    fn diagnostic_preview(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let ledger = self.open_ledger()?;
        let export = ledger
            .export_preview(&mission_id)
            .map_err(|error| format!("DIAGNOSTIC_PREVIEW_FAILED:{error}"))?;
        let integrity = ledger
            .integrity_report()
            .map_err(|error| format!("DIAGNOSTIC_PREVIEW_FAILED:{error}"))?;
        Self::lifecycle_result(serde_json::json!({
            "mission_id": mission_id,
            "event_count": export.event_count,
            "export_hash": export.content_hash,
            "redaction_categories": export.categories,
            "ledger": integrity,
            "telemetry_enabled": false,
            "includes_source": false,
            "includes_provider_payload": false,
        }))
    }

    fn archive_mission(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let mut ledger = self.open_ledger()?;
        let plan = match request.archive_plan {
            Some(value) => serde_json::from_value::<ArchivePlan>(value)
                .map_err(|_| "ARCHIVE_PLAN_INVALID".to_owned())?,
            None => {
                let current = ledger
                    .archive_plan(mission_id)
                    .map_err(|error| format!("ARCHIVE_PLAN_FAILED:{error}"))?;
                if let Some(hash) = request.impact_hash.as_deref()
                    && hash != current.impact_hash
                {
                    return Err("ARCHIVE_PLAN_MISMATCH".to_owned());
                }
                // No impact hash means this is the preview phase.
                if request.impact_hash.is_none() {
                    let mut result = Self::lifecycle_result(
                        serde_json::to_value(&current)
                            .map_err(|_| "ARCHIVE_PLAN_SERIALIZE_FAILED".to_owned())?,
                    )?;
                    result.mission_id = Some(mission_id.to_string());
                    return Ok(result);
                }
                current
            }
        };
        if plan.mission_id != mission_id {
            return Err("ARCHIVE_PLAN_MISSION_MISMATCH".to_owned());
        }
        if let Some(hash) = request.impact_hash.as_deref()
            && hash != plan.impact_hash
        {
            return Err("ARCHIVE_PLAN_MISMATCH".to_owned());
        }
        let receipt = ledger
            .archive(&plan)
            .map_err(|error| format!("ARCHIVE_FAILED:{error}"))?;
        let mut result = Self::lifecycle_result(serde_json::json!({
            "plan": plan,
            "receipt": receipt,
            "impact_hash": request.impact_hash,
        }))?;
        result.mission_id = Some(mission_id.to_string());
        Ok(result)
    }

    fn delete_mission(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let execution_requested = request.delete_plan.is_some() || request.impact_hash.is_some();
        let mut ledger = self.open_ledger()?;
        let plan = match request.delete_plan {
            Some(value) => serde_json::from_value::<DeleteImpactPlan>(value)
                .map_err(|_| "DELETE_PLAN_INVALID".to_owned())?,
            None => {
                let current = ledger
                    .delete_impact(&mission_id)
                    .map_err(|error| format!("DELETE_PLAN_FAILED:{error}"))?;
                if let Some(hash) = request.impact_hash.as_deref()
                    && hash != current.impact_hash
                {
                    return Err("DELETE_PLAN_MISMATCH".to_owned());
                }
                if request.impact_hash.is_none() {
                    let mut result = Self::lifecycle_result(
                        serde_json::to_value(&current)
                            .map_err(|_| "DELETE_PLAN_SERIALIZE_FAILED".to_owned())?,
                    )?;
                    result.mission_id = Some(mission_id.to_string());
                    return Ok(result);
                }
                current
            }
        };
        if execution_requested && self.active_run_id(&mission_id)?.is_some() {
            return Err("MISSION_ACTIVE".to_owned());
        }
        if plan.mission_id != mission_id {
            return Err("DELETE_PLAN_MISSION_MISMATCH".to_owned());
        }
        if let Some(hash) = request.impact_hash.as_deref()
            && hash != plan.impact_hash
        {
            return Err("DELETE_PLAN_MISMATCH".to_owned());
        }
        let blob_refs: Vec<BlobRef> = plan
            .blob_refs
            .iter()
            .map(|blob| BlobRef {
                hash: blob.hash.clone(),
                size: blob.size,
                media_type: blob.media_type.clone(),
            })
            .collect();
        let receipt = ledger
            .delete_mission(&plan)
            .map_err(|error| format!("DELETE_FAILED:{error}"))?;
        drop(ledger);
        if !blob_refs.is_empty() {
            let store = self.open_recovery_store()?;
            for blob in &blob_refs {
                store
                    .delete_if_unreferenced(blob)
                    .map_err(|error| format!("DELETE_BLOB_CLEANUP_FAILED:{error}"))?;
            }
        }
        let mut missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        missions.remove(&mission_id.to_string());
        drop(missions);
        if let Ok(mut budgets) = self.budgets.lock() {
            budgets.remove(&mission_id.to_string());
        }
        let mut result = Self::lifecycle_result(serde_json::json!({
            "plan": plan,
            "receipt": receipt,
        }))?;
        result.mission_id = Some(mission_id.to_string());
        Ok(result)
    }

    fn materialize_export(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let ledger = self.open_ledger()?;
        let artifact = ledger
            .materialize_export(&mission_id)
            .map_err(|error| format!("EXPORT_FAILED:{error}"))?;
        let content =
            String::from_utf8(artifact.bytes).map_err(|_| "EXPORT_ENCODING_INVALID".to_owned())?;
        let mut result = Self::lifecycle_result(serde_json::json!({
            "mission_id": mission_id,
            "size_bytes": artifact.size_bytes,
            "content_hash": artifact.content_hash,
            "content": content,
        }))?;
        result.mission_id = Some(mission_id.to_string());
        Ok(result)
    }

    fn create_mission(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id: MissionId = request
            .mission_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "MISSION_ID_INVALID".to_owned())?
            .unwrap_or_default();
        let route_id: RouteId = request
            .route_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "ROUTE_ID_INVALID".to_owned())?
            .unwrap_or_default();
        let mission_key = mission_id.to_string();
        let budget_request = request.budget.clone().unwrap_or(MissionBudgetRequest {
            tokens: None,
            money_micros: None,
            wall_clock_ms: None,
            changed_lines: None,
            changed_files: None,
            model_calls: None,
        });
        let budget_limits = budget_request.limits();
        // Acquire shared state in the same order as the event consumer. This
        // also makes a poisoned budget map fail before any ledger append.
        let mut budgets = self
            .budgets
            .lock()
            .map_err(|_| "MISSION_BUDGET_STATE_POISONED".to_owned())?;
        let mut missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        if missions.contains_key(&mission_key) {
            return Err("MISSION_ALREADY_EXISTS".to_owned());
        }

        // Check the durable index before writing either creation event.  The
        // in-memory lock stays held through the append and insert so two
        // concurrent create requests cannot pass the checks together.
        let mut ledger = self.open_ledger()?;
        if ledger
            .mission_ids()
            .map_err(|error| format!("MISSION_CREATE_FAILED:{error}"))?
            .contains(&mission_id)
        {
            return Err("MISSION_ALREADY_EXISTS".to_owned());
        }

        let events = [
            EventEnvelope::new(
                EventId::new(),
                mission_id,
                route_id,
                1,
                EventKind::MissionCreated,
                serde_json::json!({"project_root": request.project_root, "goal": request.goal}),
            ),
            EventEnvelope::new(
                EventId::new(),
                mission_id,
                route_id,
                2,
                EventKind::RouteCreated,
                serde_json::json!({"route_id": route_id}),
            ),
            EventEnvelope::new(
                EventId::new(),
                mission_id,
                route_id,
                3,
                EventKind::ContractUpdated,
                serde_json::json!({
                    "goal": request.goal,
                    "contract_version": 1,
                    "budget": {
                        "tokens": budget_limits.tokens,
                        "moneyMicros": budget_limits.money_micros,
                        "wallClockMs": budget_limits.wall_clock.as_millis() as u64,
                        "changedLines": budget_limits.changed_lines,
                        "changedFiles": budget_limits.changed_files,
                        "modelCalls": budget_limits.model_calls,
                    }
                }),
            ),
        ];
        ledger
            .append_batch(&events)
            .map_err(|error| format!("MISSION_CREATE_FAILED:{error}"))?;
        let actor = crate::mission_actor::MissionActor::new(mission_id, route_id, ledger);
        missions.insert(mission_key.clone(), actor);
        budgets.insert(
            mission_key.clone(),
            BudgetTracker::new(1, budget_limits, UnknownUsagePolicy::Pause),
        );
        Self::result(Some(mission_key), missions.get(&mission_id.to_string()))
    }

    fn update_mission_contract(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let mut missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        let actor = missions
            .get_mut(&mission_id.to_string())
            .ok_or("MISSION_NOT_FOUND")?;
        actor
            .record_event(
                EventKind::ContractUpdated,
                serde_json::json!({"goal": request.goal, "expected_version": request.expected_version}),
            )
            .map_err(|error| format!("CONTRACT_UPDATE_FAILED:{error:?}"))?;
        Self::result(Some(mission_id.to_string()), Some(actor))
    }

    async fn launch_route(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let route_id = Self::route_id(&request)?;
        {
            let mut missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get_mut(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            actor
                .record_event(
                    EventKind::ExplorationStarted,
                    serde_json::json!({"route_id": route_id, "read_only": true}),
                )
                .map_err(|error| format!("ROUTE_LAUNCH_FAILED:{error:?}"))?;
        }
        if self.active_run_id(&mission_id)?.is_some() {
            return Err("ROUTE_ALREADY_RUNNING".to_owned());
        }
        let provider = request.provider.unwrap_or_default();
        let (_, selected_provider, capability) =
            match self.launch_agent(mission_id, route_id, &request).await {
                Ok(value) => value,
                Err(error) => {
                    let mut missions = self
                        .missions
                        .lock()
                        .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
                    if let Some(actor) = missions.get_mut(&mission_id.to_string()) {
                        let _ = actor.record_event(
                            EventKind::Unknown("adapter.start_failed".to_owned()),
                            serde_json::json!({"error": error}),
                        );
                    }
                    return Err(format!(
                        "{}_START_FAILED",
                        provider.as_str().to_ascii_uppercase()
                    ));
                }
            };
        let missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        let mut result = Self::result(
            Some(mission_id.to_string()),
            missions.get(&mission_id.to_string()),
        )?;
        result.provider = Some(selected_provider);
        result.capability = Some(capability);
        Ok(result)
    }

    fn subscribe_mission(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let after = request.expected_version.unwrap_or(0);
        let missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        let actor = missions
            .get(&mission_id.to_string())
            .ok_or("MISSION_NOT_FOUND")?;
        let events = actor
            .replay_after(after)
            .into_iter()
            .map(|event| {
                serde_json::to_value(event).map_err(|_| "EVENT_SERIALIZE_FAILED".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MissionCommandResult {
            accepted: true,
            mission_id: Some(mission_id.to_string()),
            route_id: actor
                .replay_after(0)
                .first()
                .map(|event| event.route_id.to_string())
                .or(request.route_id),
            sequence: Some(actor.sequence()),
            error_code: None,
            confirmation_token: None,
            provider: request.provider,
            capability: None,
            events,
            recovery_package: None,
            capabilities: Vec::new(),
            cc_switch: None,
            data: None,
            recovery_required: actor.requires_recovery(),
        })
    }

    async fn request_safe_pause(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let reason = request
            .reason
            .unwrap_or_else(|| "user requested".to_owned());
        let active_run = self.active_run_id(&mission_id)?;
        let result = {
            let mut missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get_mut(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            actor
                .request_safe_pause(reason)
                .map_err(|error| format!("SAFE_PAUSE_FAILED:{error:?}"))?;
            Self::result(Some(mission_id.to_string()), Some(actor))?
        };
        if let Some(run_id) = active_run {
            let provider = self.run_provider(&mission_id).unwrap_or_default();
            self.adapter(provider)?
                .request_safe_pause(&run_id)
                .await
                .map_err(|error| format!("SAFE_PAUSE_CONTROL_FAILED:{error}"))?;
        }
        Ok(result)
    }

    async fn resolve_approval(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let route_id = Self::route_id(&request)?;
        let approval_id = request
            .approval_id
            .filter(|value| !value.trim().is_empty())
            .ok_or("APPROVAL_ID_REQUIRED")?;
        let (decision, scope) = match request.approval_decision.as_deref() {
            Some("approve-once") => (ApprovalDecision::Approve, Some(ApprovalGrantScope::Once)),
            Some("approve-route") => (
                ApprovalDecision::Approve,
                Some(ApprovalGrantScope::RouteActionClass),
            ),
            Some("approve") => (
                ApprovalDecision::Approve,
                match request.approval_scope.as_deref() {
                    None | Some("once") | Some("approve-once") => Some(ApprovalGrantScope::Once),
                    Some("route") | Some("route_action_class") | Some("approve-route") => {
                        Some(ApprovalGrantScope::RouteActionClass)
                    }
                    Some(_) => return Err("APPROVAL_SCOPE_INVALID".to_owned()),
                },
            ),
            Some("deny") => (ApprovalDecision::Deny, None),
            Some("revoke") => (ApprovalDecision::Revoke, None),
            _ => return Err("APPROVAL_DECISION_INVALID".to_owned()),
        };
        let expected_revision = request
            .expected_revision
            .or(request.expected_version)
            .ok_or("APPROVAL_REVISION_REQUIRED")?;
        let action_digest = request
            .action_digest
            .filter(|value| !value.trim().is_empty())
            .ok_or("ACTION_DIGEST_REQUIRED")?;
        let contract_version = request
            .contract_version
            .ok_or("CONTRACT_VERSION_REQUIRED")?;
        let loadout_fingerprint = request
            .loadout_fingerprint
            .filter(|value| !value.trim().is_empty())
            .ok_or("LOADOUT_FINGERPRINT_REQUIRED")?;
        let now_ms = request.now_ms.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or_default()
        });
        let result = {
            let mut missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get_mut(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            actor
                .resolve_approval(
                    Actor::User,
                    ResolveApproval {
                        approval_id: approval_id.clone(),
                        expected_revision,
                        decision,
                        mission_id: mission_id.to_string(),
                        route_id: route_id.to_string(),
                        contract_version,
                        loadout_fingerprint,
                        action_digest,
                        now_ms,
                        scope,
                    },
                )
                .map_err(|error| format!("APPROVAL_RESOLUTION_FAILED:{error:?}"))?;
            Self::result(Some(mission_id.to_string()), Some(actor))?
        };
        let pending = self
            .pending_server_requests
            .lock()
            .map_err(|_| "MISSION_APPROVAL_STATE_POISONED".to_owned())?
            .remove(&approval_id);
        if let Some(pending) = pending {
            let decision = match request.approval_decision.as_deref() {
                Some("approve-route") => "acceptForSession",
                Some("approve-once") | Some("approve") => "accept",
                Some("deny") | Some("revoke") => "decline",
                _ => "decline",
            };
            self.adapter(pending.provider)?
                .respond_to_server_request(&pending.run_id, pending.request_id, decision)
                .await
                .map_err(|error| format!("APPROVAL_RESPONSE_FAILED:{error}"))?;
        }
        Ok(result)
    }

    async fn request_force_termination(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let active_run = self.active_run_id(&mission_id)?;
        let (token, mut result) = {
            let mut missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get_mut(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            let token = actor
                .request_force_termination()
                .map_err(|error| format!("FORCE_TERMINATE_REQUEST_FAILED:{error:?}"))?;
            let result = Self::result(Some(mission_id.to_string()), Some(actor))?;
            (token, result)
        };
        if let Some(run_id) = active_run {
            let provider = self.run_provider(&mission_id).unwrap_or_default();
            self.adapter(provider)?
                .request_safe_pause(&run_id)
                .await
                .map_err(|error| format!("SAFE_PAUSE_CONTROL_FAILED:{error}"))?;
        }
        result.confirmation_token = Some(token);
        Ok(result)
    }

    async fn force_terminate(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let token = request.confirmation_token.ok_or("FORCE_TOKEN_REQUIRED")?;
        let active_run = self.active_run_id(&mission_id)?;
        {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            if !actor.can_force_terminate(&token) {
                return Err("FORCE_TERMINATE_FAILED:invalid confirmation token".to_owned());
            }
        }
        if let Some(run_id) = active_run {
            let provider = self.run_provider(&mission_id).unwrap_or_default();
            self.adapter(provider)?
                .terminate_owned_tree(&run_id)
                .await
                .map_err(|error| format!("FORCE_TERMINATE_CONTROL_FAILED:{error}"))?;
        }
        let mut missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        let actor = missions
            .get_mut(&mission_id.to_string())
            .ok_or("MISSION_NOT_FOUND")?;
        actor
            .force_terminate(&token)
            .map_err(|error| format!("FORCE_TERMINATE_FAILED:{error:?}"))?;
        Self::result(Some(mission_id.to_string()), Some(actor))
    }

    fn open_recovery_store(&self) -> Result<EncryptedBlobStore, String> {
        let key = WindowsCredentialKeyStore
            .load_database_key("mission-control-desktop-v1")
            .map_err(|error| format!("RECOVERY_KEY_UNAVAILABLE:{error}"))?;
        let root = self
            .ledger_path
            .parent()
            .ok_or("RECOVERY_ROOT_UNAVAILABLE")?
            .join("recovery-blobs");
        EncryptedBlobStore::open_for_ledger(&root, &self.ledger_path, key)
            .map_err(|error| format!("RECOVERY_STORE_OPEN_FAILED:{error}"))
    }

    fn build_recovery_package(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let route_id = Self::route_id(&request)?;
        let checkpoint_id = request
            .checkpoint_id
            .filter(|value| !value.trim().is_empty())
            .ok_or("CHECKPOINT_ID_REQUIRED")?;
        let contract_version = request
            .contract_version
            .ok_or("CONTRACT_VERSION_REQUIRED")?;
        let ledger_sequence = request.ledger_sequence.ok_or("LEDGER_SEQUENCE_REQUIRED")?;
        let loadout_fingerprint = request
            .loadout_fingerprint
            .filter(|value| !value.trim().is_empty())
            .ok_or("LOADOUT_FINGERPRINT_REQUIRED")?;
        let context_pack_hash = request
            .context_pack_hash
            .filter(|value| !value.trim().is_empty())
            .ok_or("CONTEXT_PACK_HASH_REQUIRED")?;
        let (current_sequence, events) = {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            (
                actor.sequence(),
                actor
                    .replay_after(0)
                    .into_iter()
                    .map(|event| {
                        serde_json::to_value(event).map_err(|_| "EVENT_SERIALIZE_FAILED".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        };
        if ledger_sequence > current_sequence {
            return Err("RECOVERY_SEQUENCE_INVALID".to_owned());
        }
        let payload = serde_json::to_vec(&serde_json::json!({
            "mission_id": mission_id,
            "route_id": route_id,
            "checkpoint_id": checkpoint_id,
            "ledger_sequence": ledger_sequence,
            "events": events,
        }))
        .map_err(|_| "RECOVERY_PAYLOAD_SERIALIZE_FAILED".to_owned())?;
        let store = self.open_recovery_store()?;
        let package = build_recovery_package(
            &store,
            RecoveryInput {
                mission_id,
                route_id,
                contract_version,
                checkpoint_id,
                ledger_sequence,
                loadout_fingerprint,
                context_pack_hash,
                pending_approval_hash: request.pending_approval_hash,
                permissions: Default::default(),
                payload,
            },
        )
        .map_err(|error| format!("RECOVERY_BUILD_FAILED:{error}"))?;
        let mut result = {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            Self::result(
                Some(mission_id.to_string()),
                missions.get(&mission_id.to_string()),
            )?
        };
        result.recovery_package = Some(
            serde_json::to_value(package)
                .map_err(|_| "RECOVERY_RESULT_SERIALIZE_FAILED".to_owned())?,
        );
        Ok(result)
    }

    fn verify_recovery_request(
        &self,
        request: &MissionCommandRequest,
    ) -> Result<(MissionId, RouteId, RecoveryPackage), String> {
        let mission_id = Self::mission_id(request)?;
        let route_id = Self::route_id(request)?;
        let package_value = request
            .recovery_package
            .clone()
            .ok_or("RECOVERY_PACKAGE_REQUIRED")?;
        let package: RecoveryPackage =
            serde_json::from_value(package_value).map_err(|_| "RECOVERY_PACKAGE_INVALID")?;
        let (current_sequence, requires_recovery) = {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            (actor.sequence(), actor.requires_recovery())
        };
        if !requires_recovery {
            return Err("RECOVERY_NOT_REQUIRED".to_owned());
        }
        let contract_version = request
            .contract_version
            .ok_or("CONTRACT_VERSION_REQUIRED")?;
        let ledger_sequence = request.ledger_sequence.ok_or("LEDGER_SEQUENCE_REQUIRED")?;
        if package.manifest.ledger_sequence != ledger_sequence {
            return Err("RECOVERY_SEQUENCE_INVALID".to_owned());
        }
        if package.manifest.ledger_sequence != current_sequence {
            return Err("RECOVERY_SEQUENCE_INVALID".to_owned());
        }
        let loadout_fingerprint = request
            .loadout_fingerprint
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or("LOADOUT_FINGERPRINT_REQUIRED")?;
        let context_pack_hash = request
            .context_pack_hash
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or("CONTEXT_PACK_HASH_REQUIRED")?;
        let constraints = RecoveryConstraints {
            mission_id,
            route_id,
            contract_version,
            ledger_sequence: current_sequence,
            loadout_fingerprint: loadout_fingerprint.to_owned(),
            context_pack_hash: context_pack_hash.to_owned(),
            pending_approval_hash: request.pending_approval_hash.clone(),
            permissions: Default::default(),
        };
        let store = self.open_recovery_store()?;
        package
            .verify(&store, &constraints)
            .map_err(|error| format!("RECOVERY_VERIFY_FAILED:{error}"))?;
        Ok((mission_id, route_id, package))
    }

    fn verify_recovery(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let (mission_id, _route_id, package) = self.verify_recovery_request(&request)?;
        let mut result = {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            Self::result(
                Some(mission_id.to_string()),
                missions.get(&mission_id.to_string()),
            )?
        };
        result.data = Some(serde_json::json!({
            "verified": true,
            "entry_hash": package.manifest.entry_hash,
            "ledger_sequence": package.manifest.ledger_sequence,
        }));
        Ok(result)
    }

    async fn resolve_recovery(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let (mission_id, route_id, package) = self.verify_recovery_request(&request)?;
        let decision = request
            .recovery_decision
            .as_deref()
            .ok_or("RECOVERY_DECISION_REQUIRED")?;
        let manifest = serde_json::to_value(&package.manifest)
            .map_err(|_| "RECOVERY_MANIFEST_SERIALIZE_FAILED".to_owned())?;
        let (project_root, provider, thread_id) = {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            let events = actor.replay_after(0);
            let project_root = events
                .iter()
                .find(|event| event.kind == EventKind::MissionCreated)
                .and_then(|event| event.payload.get("project_root"))
                .and_then(Value::as_str)
                .ok_or("PROJECT_ROOT_REQUIRED")?
                .to_owned();
            let provider = events
                .iter()
                .rev()
                .find(|event| event.kind.as_str() == "loadout_snapshot")
                .and_then(|event| event.payload.get("provider"))
                .and_then(Value::as_str)
                .and_then(|value| value.parse().ok())
                .or(request.provider)
                .unwrap_or_default();
            let thread_id = events
                .iter()
                .rev()
                .find(|event| event.kind == EventKind::AgentRunStarted)
                .and_then(|event| event.payload.get("thread_id"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            (project_root, provider, thread_id)
        };
        let result = {
            let mut missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get_mut(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            actor
                .resolve_recovery(decision, manifest)
                .map_err(|error| format!("RECOVERY_RESOLUTION_FAILED:{error:?}"))?;
            Self::result(Some(mission_id.to_string()), Some(actor))?
        };
        if decision == "continue" {
            let thread_id = thread_id.ok_or("RECOVERY_THREAD_ID_MISSING")?;
            let mut launch_request = request;
            launch_request.provider = Some(provider);
            launch_request.project_root = Some(project_root);
            launch_request.mission_id = Some(mission_id.to_string());
            launch_request.route_id = Some(route_id.to_string());
            launch_request.loadout_fingerprint = Some(package.manifest.loadout_fingerprint);
            launch_request.resume_token = Some(thread_id);
            let _ = self
                .launch_agent(mission_id, route_id, &launch_request)
                .await?;
        }
        Ok(result)
    }

    fn review_memory(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let memory_id = request.memory_id.ok_or("MEMORY_ID_REQUIRED")?;
        let decision = request
            .memory_decision
            .as_deref()
            .ok_or("MEMORY_DECISION_REQUIRED")?;
        let status = match decision {
            "confirm" => "confirmed",
            "reject" => "rejected",
            "defer" => "deferred",
            _ => return Err("MEMORY_DECISION_INVALID".to_owned()),
        };
        let mut missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        let actor = missions
            .get_mut(&mission_id.to_string())
            .ok_or("MISSION_NOT_FOUND")?;
        if request
            .expected_version
            .is_some_and(|expected| expected != actor.sequence())
        {
            return Err("MISSION_VERSION_CONFLICT".to_owned());
        }
        let mut item = actor
            .replay_after(0)
            .into_iter()
            .rev()
            .find_map(|event| {
                (event.kind == EventKind::MemoryItemChanged)
                    .then(|| event.payload.get("item").cloned())
                    .flatten()
                    .filter(|candidate| {
                        candidate.get("id").and_then(Value::as_str) == Some(memory_id.as_str())
                    })
            })
            .ok_or("MEMORY_NOT_FOUND")?;
        if item.get("kind").and_then(Value::as_str) == Some("inference") && status == "confirmed" {
            return Err("MEMORY_INFERENCE_CANNOT_CONFIRM".to_owned());
        }
        let object = item.as_object_mut().ok_or("MEMORY_ITEM_INVALID")?;
        object.insert("status".to_owned(), Value::String(status.to_owned()));
        object.insert("author".to_owned(), Value::String("user".to_owned()));
        actor
            .record_event(
                EventKind::MemoryItemChanged,
                serde_json::json!({"action": decision, "item": item}),
            )
            .map_err(|error| format!("MEMORY_REVIEW_FAILED:{error:?}"))?;
        Self::result(Some(mission_id.to_string()), Some(actor))
    }

    async fn provider_capabilities(&self) -> Result<MissionCommandResult, String> {
        let mut capabilities = Vec::with_capacity(ProviderId::ALL.len());
        for provider in ProviderId::ALL {
            let capability = match self.adapters.get(&provider) {
                Some(adapter) => match adapter.probe().await {
                    Ok(report) => report,
                    Err(error) => unavailable_capability(provider, error.to_string()),
                },
                None => unavailable_capability(provider, "runtime adapter is not installed"),
            };
            capabilities.push(capability);
        }
        let cc_switch = if let Some(endpoint) = std::env::var_os("MISSION_CC_SWITCH_ENDPOINT") {
            let endpoint = endpoint.to_string_lossy().into_owned();
            match CcSwitchBridge::new(endpoint) {
                Ok(bridge) => match bridge.health().await {
                    Ok(health) => serde_json::json!({
                        "available": health.ok,
                        "version": health.version,
                        "endpoint": "loopback",
                    }),
                    Err(error) => serde_json::json!({
                        "available": false,
                        "unavailableReason": error.to_string(),
                        "endpoint": "loopback",
                    }),
                },
                Err(error) => serde_json::json!({
                    "available": false,
                    "unavailableReason": error.to_string(),
                    "endpoint": "rejected",
                }),
            }
        } else {
            serde_json::json!({
                "available": false,
                "unavailableReason": "CC Switch endpoint is not configured",
                "endpoint": "not-configured",
            })
        };
        Ok(MissionCommandResult {
            accepted: true,
            mission_id: None,
            route_id: None,
            sequence: None,
            error_code: None,
            confirmation_token: None,
            provider: None,
            capability: None,
            events: Vec::new(),
            recovery_package: None,
            capabilities,
            cc_switch: Some(cc_switch),
            data: None,
            recovery_required: false,
        })
    }

    async fn handoff_provider(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = Self::mission_id(&request)?;
        let route_id = Self::route_id(&request)?;
        let target = request
            .target_provider
            .or(request.provider)
            .ok_or("PROVIDER_NOT_SELECTED")?;
        let context_pack_hash = request
            .context_pack_hash
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or("CONTEXT_PACK_HASH_REQUIRED")?;
        let (project_root, current_provider, loadout_fingerprint) = {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            let events = actor.replay_after(0);
            if events
                .first()
                .is_none_or(|event| event.route_id != route_id)
            {
                return Err("ROUTE_MISMATCH".to_owned());
            }
            let project_root = events
                .iter()
                .find(|event| event.kind == EventKind::MissionCreated)
                .and_then(|event| event.payload.get("project_root"))
                .and_then(Value::as_str)
                .ok_or("PROJECT_ROOT_REQUIRED")?
                .to_owned();
            let current_provider = self
                .run_provider(&mission_id)
                .or_else(|| {
                    events
                        .iter()
                        .rev()
                        .find(|event| event.kind.as_str() == "loadout_snapshot")
                        .and_then(|event| event.payload.get("provider"))
                        .and_then(Value::as_str)
                        .and_then(|value| value.parse().ok())
                })
                .unwrap_or_default();
            let loadout_fingerprint = events
                .iter()
                .rev()
                .find(|event| event.kind.as_str() == "loadout_snapshot")
                .and_then(|event| event.payload.get("fingerprint"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("handoff-{context_pack_hash}"));
            (project_root, current_provider, loadout_fingerprint)
        };
        if current_provider == target {
            return Err("HANDOFF_PROVIDER_UNCHANGED".to_owned());
        }
        if self.active_run_id(&mission_id)?.is_some() {
            let _ = self
                .request_safe_pause(MissionCommandRequest {
                    provider: Some(current_provider),
                    target_provider: None,
                    loadout: None,
                    mission_id: Some(mission_id.to_string()),
                    route_id: Some(route_id.to_string()),
                    expected_version: None,
                    project_root: None,
                    goal: None,
                    reason: Some("provider handoff requested".to_owned()),
                    confirmation_token: None,
                    approval_id: None,
                    approval_decision: None,
                    approval_scope: None,
                    action_digest: None,
                    expected_revision: None,
                    now_ms: None,
                    loadout_fingerprint: None,
                    resume_token: None,
                    checkpoint_id: None,
                    contract_version: None,
                    ledger_sequence: None,
                    context_pack_hash: None,
                    pending_approval_hash: None,
                    memory_id: None,
                    memory_decision: None,
                    project_limit_bytes: None,
                    global_limit_bytes: None,
                    recovery_package: None,
                    recovery_decision: None,
                    impact_hash: None,
                    archive_plan: None,
                    delete_plan: None,
                    budget: None,
                })
                .await;
            return Err("HANDOFF_REQUIRES_PAUSED_RUN".to_owned());
        }
        {
            let mut missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get_mut(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            actor
                .record_event(
                    EventKind::Unknown("provider_handoff_requested".to_owned()),
                    serde_json::json!({"target_provider": target, "context_pack_hash": context_pack_hash}),
                )
                .map_err(|error| format!("HANDOFF_RECORD_FAILED:{error:?}"))?;
        }
        let launch_request = MissionCommandRequest {
            provider: Some(target),
            target_provider: None,
            loadout: None,
            mission_id: Some(mission_id.to_string()),
            route_id: Some(route_id.to_string()),
            expected_version: None,
            project_root: Some(project_root),
            goal: None,
            reason: None,
            confirmation_token: None,
            approval_id: None,
            approval_decision: None,
            approval_scope: None,
            action_digest: None,
            expected_revision: None,
            now_ms: None,
            loadout_fingerprint: Some(loadout_fingerprint),
            resume_token: None,
            checkpoint_id: None,
            contract_version: None,
            ledger_sequence: None,
            context_pack_hash: Some(context_pack_hash),
            pending_approval_hash: request.pending_approval_hash,
            memory_id: None,
            memory_decision: None,
            project_limit_bytes: None,
            global_limit_bytes: None,
            recovery_package: None,
            recovery_decision: None,
            impact_hash: None,
            archive_plan: None,
            delete_plan: None,
            budget: None,
        };
        let (_, selected_provider, capability) = self
            .launch_agent(mission_id, route_id, &launch_request)
            .await?;
        let missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        let mut result = Self::result(
            Some(mission_id.to_string()),
            missions.get(&mission_id.to_string()),
        )?;
        result.provider = Some(selected_provider);
        result.capability = Some(capability);
        Ok(result)
    }

    fn active_run_id(&self, mission_id: &MissionId) -> Result<Option<String>, String> {
        self.active_run_id_key(&mission_id.to_string())
    }

    fn active_run_id_key(&self, mission_key: &str) -> Result<Option<String>, String> {
        self.runs
            .lock()
            .map_err(|_| "MISSION_RUN_STATE_POISONED".to_owned())
            .map(|runs| runs.get(mission_key).cloned())
    }

    fn run_provider(&self, mission_id: &MissionId) -> Option<ProviderId> {
        self.run_providers
            .lock()
            .ok()
            .and_then(|providers| providers.get(&mission_id.to_string()).copied())
    }

    pub async fn check_loadout_before_model_request(
        &self,
        mission_id: MissionId,
        next_fingerprint: &str,
    ) -> Result<Option<crate::loadout_monitor::LoadoutChange>, String> {
        let change = self
            .loadouts
            .lock()
            .map_err(|_| "LOADOUT_STATE_POISONED".to_owned())?
            .check_fingerprint(&mission_id, next_fingerprint)
            .map_err(str::to_owned)?;
        let Some(change) = change else {
            return Ok(None);
        };
        {
            let mut missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            let actor = missions
                .get_mut(&mission_id.to_string())
                .ok_or("MISSION_NOT_FOUND")?;
            actor
                .check_loadout_change(&change.previous_fingerprint, &change.next_fingerprint)
                .map_err(|error| format!("LOADOUT_CHANGE_FAILED:{error:?}"))?;
        }
        if let Some(run_id) = self.active_run_id(&mission_id)? {
            let provider = self.run_provider(&mission_id).unwrap_or_default();
            self.adapter(provider)
                .map_err(|error| error.to_string())?
                .request_safe_pause(&run_id)
                .await
                .map_err(|error| format!("LOADOUT_PAUSE_FAILED:{error}"))?;
        }
        Ok(Some(change))
    }

    fn adapter(&self, provider: ProviderId) -> Result<Arc<dyn AgentAdapter>, String> {
        self.adapters
            .get(&provider)
            .cloned()
            .ok_or_else(|| format!("PROVIDER_UNAVAILABLE:{}", provider.as_str()))
    }

    async fn launch_agent(
        &self,
        mission_id: MissionId,
        route_id: RouteId,
        request: &MissionCommandRequest,
    ) -> Result<(String, ProviderId, AgentCapabilityReport), String> {
        let project_root = request
            .project_root
            .clone()
            .ok_or_else(|| "PROJECT_ROOT_REQUIRED".to_owned())?;
        let goal = if request.goal.is_some() {
            request.goal.clone()
        } else {
            self.missions
                .lock()
                .ok()
                .and_then(|missions| {
                    missions
                        .get(&mission_id.to_string())
                        .map(|actor| actor.replay_after(0))
                })
                .and_then(|events| {
                    events
                        .into_iter()
                        .find(|event| event.kind == EventKind::MissionCreated)
                        .and_then(|event| event.payload.get("goal").cloned())
                        .and_then(|value| value.as_str().map(str::to_owned))
                })
        };
        let provider = request.provider.unwrap_or_default();
        if request
            .loadout
            .as_ref()
            .is_some_and(|loadout| loadout.provider != provider)
        {
            return Err("LOADOUT_PROVIDER_MISMATCH".to_owned());
        }
        let adapter = self.adapter(provider)?;
        {
            let missions = self
                .missions
                .lock()
                .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
            if missions
                .get(&mission_id.to_string())
                .is_some_and(crate::mission_actor::MissionActor::requires_recovery)
            {
                return Err("RECOVERY_REQUIRED".to_owned());
            }
        }
        let capability = adapter
            .probe()
            .await
            .map_err(|error| format!("PROVIDER_PROBE_FAILED:{error}"))?;
        if !capability.is_available() {
            return Err(format!(
                "PROVIDER_UNAVAILABLE:{}",
                capability
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("capability unavailable")
            ));
        }
        if request.resume_token.is_some() && !capability.capability.resume {
            return Err("RESUME_UNSUPPORTED".to_owned());
        }
        let loadout_fingerprint = request
            .loadout_fingerprint
            .clone()
            .or_else(|| {
                request
                    .loadout
                    .as_ref()
                    .map(|loadout| loadout.fingerprint_material().join("|"))
            })
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "LOADOUT_FINGERPRINT_REQUIRED".to_owned())?;
        self.loadouts
            .lock()
            .map_err(|_| "LOADOUT_STATE_POISONED".to_owned())?
            .freeze_fingerprint(&mission_id, loadout_fingerprint.clone())
            .map_err(str::to_owned)?;
        let start_request = StartAgentRequest {
            provider,
            mission_id,
            route_id,
            project_root: project_root.clone(),
            route_workspace: project_root,
            read_only: true,
            approved_environment: Vec::new(),
            model: request
                .loadout
                .as_ref()
                .and_then(|loadout| loadout.model.clone()),
            goal,
            loadout_fingerprint,
            resume_thread_id: request.resume_token.clone(),
            loadout: request.loadout.clone(),
            contract_version: request.contract_version.unwrap_or_else(|| {
                self.missions
                    .lock()
                    .ok()
                    .and_then(|missions| {
                        missions.get(&mission_id.to_string()).map(|actor| {
                            actor
                                .replay_after(0)
                                .iter()
                                .rev()
                                .find(|event| event.kind == EventKind::ContractUpdated)
                                .and_then(|event| event.payload.get("contract_version"))
                                .and_then(Value::as_u64)
                                .unwrap_or(1)
                        })
                    })
                    .unwrap_or(1)
            }),
        };
        let (sink, _sink_rx) = mpsc::unbounded_channel();
        let handle = adapter.start(start_request, sink).await.map_err(|error| {
            format!(
                "{}_ADAPTER_START_FAILED:{error}",
                provider.as_str().to_ascii_uppercase()
            )
        })?;
        let run_id = handle.run_id().to_owned();
        self.runs
            .lock()
            .map_err(|_| "MISSION_RUN_STATE_POISONED".to_owned())?
            .insert(mission_id.to_string(), run_id.clone());
        self.run_providers
            .lock()
            .map_err(|_| "MISSION_RUN_PROVIDER_STATE_POISONED".to_owned())?
            .insert(mission_id.to_string(), provider);
        self.spawn_event_consumer(mission_id, handle, adapter);
        Ok((run_id, provider, capability))
    }

    fn spawn_event_consumer(
        &self,
        mission_id: MissionId,
        handle: AgentHandle,
        adapter: Arc<dyn AgentAdapter>,
    ) {
        let missions = Arc::clone(&self.missions);
        let runs = Arc::clone(&self.runs);
        let run_providers = Arc::clone(&self.run_providers);
        let budgets = Arc::clone(&self.budgets);
        let pending_server_requests = Arc::clone(&self.pending_server_requests);
        let run_id = handle.run_id().to_owned();
        self.runtime.spawn(async move {
            let mut previous_tokens = 0_u64;
            let run_started = Instant::now();
            while let Some(event) = handle.next_event().await {
                let requires_pause = event.requires_safe_pause;
                let usage_record = usage_record_from_event(&event, &mut previous_tokens);
                let mut budget_pause = false;
                if let Some(record) = usage_record {
                    let signals = budgets
                        .lock()
                        .ok()
                        .map(|mut trackers| {
                            let tracker = trackers
                                .entry(mission_id.to_string())
                                .or_insert_with(default_budget_tracker);
                            tracker.record(record);
                            tracker.evaluate_safe_boundary()
                        })
                        .unwrap_or_default();
                    if !signals.is_empty()
                        && let Ok(mut missions_guard) = missions.lock()
                        && let Some(actor) = missions_guard.get_mut(&mission_id.to_string())
                    {
                        budget_pause = signals
                            .iter()
                            .any(|signal| matches!(signal, BudgetSignal::PauseAtSafeBoundary(_)));
                        if actor.apply_budget_signals(&signals).is_err() {
                            budget_pause = true;
                        }
                    }
                }
                if event.event_kind == EventKind::ApprovalRequested
                    && let Some(payload) = event.payload.as_object()
                    && let (Some(approval_id), Some(request_id)) = (
                        payload.get("approval_id").and_then(Value::as_str),
                        payload.get("server_request_id").cloned(),
                    )
                {
                    let provider = run_providers
                        .lock()
                        .ok()
                        .and_then(|providers| providers.get(&mission_id.to_string()).copied())
                        .unwrap_or_default();
                    if let Ok(mut pending) = pending_server_requests.lock() {
                        pending.insert(
                            approval_id.to_owned(),
                            PendingServerRequest {
                                run_id: run_id.clone(),
                                request_id,
                                provider,
                            },
                        );
                    }
                }
                let mut should_remove = false;
                let mut adapter_pause = budget_pause;
                if let Ok(mut missions) = missions.lock() {
                    if let Some(actor) = missions.get_mut(&mission_id.to_string()) {
                        match actor.record_agent_event(event) {
                            Ok(()) if requires_pause => {
                                if actor
                                    .request_safe_pause("adapter requires safe pause")
                                    .is_err()
                                {
                                    let _ = actor.record_event(
                                        EventKind::Unknown("mission.degraded".to_owned()),
                                        serde_json::json!({
                                            "reason": "safe pause audit append failed",
                                            "safe_pause": true,
                                        }),
                                    );
                                    should_remove = true;
                                }
                                adapter_pause = true;
                            }
                            Ok(()) => {}
                            Err(_) => {
                                // A failed append is a degraded safety state. Preserve an audit
                                // marker when the ledger is still writable, then stop the run.
                                let _ = actor.record_event(
                                    EventKind::Unknown("mission.degraded".to_owned()),
                                    serde_json::json!({
                                        "reason": "agent event append failed",
                                        "safe_pause": true,
                                    }),
                                );
                                let _ = actor.request_safe_pause("agent event append failed");
                                should_remove = true;
                                adapter_pause = true;
                            }
                        }
                    } else {
                        should_remove = true;
                        adapter_pause = true;
                    }
                } else {
                    should_remove = true;
                    adapter_pause = true;
                }
                if adapter_pause && adapter.request_safe_pause(&run_id).await.is_err() {
                    should_remove = true;
                }
                if should_remove {
                    break;
                }
            }
            let wall_clock_record =
                UsageRecord::Sample(UsageSample::wall_clock(run_started.elapsed()));
            let wall_clock_signals = budgets
                .lock()
                .ok()
                .map(|mut trackers| {
                    let tracker = trackers
                        .entry(mission_id.to_string())
                        .or_insert_with(default_budget_tracker);
                    tracker.record(wall_clock_record);
                    tracker.evaluate_safe_boundary()
                })
                .unwrap_or_default();
            if !wall_clock_signals.is_empty()
                && let Ok(mut missions_guard) = missions.lock()
                && let Some(actor) = missions_guard.get_mut(&mission_id.to_string())
            {
                let _ = actor.apply_budget_signals(&wall_clock_signals);
            }
            if let Ok(mut runs) = runs.lock()
                && runs
                    .get(&mission_id.to_string())
                    .is_some_and(|active| active == &run_id)
            {
                runs.remove(&mission_id.to_string());
                if let Ok(mut providers) = run_providers.lock() {
                    providers.remove(&mission_id.to_string());
                }
            }
            if let Ok(mut pending) = pending_server_requests.lock() {
                pending.retain(|_, request| request.run_id != run_id);
            }
        });
    }
}

impl crate::ipc::IpcDispatcher for MissionService {
    fn dispatch(&self, command: &str, request: Value) -> Result<Value, String> {
        self.dispatch_json(command, request)
    }

    fn touch_ui(&self) {
        self.touch_ui();
    }
}

fn unavailable_capability(
    provider: ProviderId,
    reason: impl Into<String>,
) -> AgentCapabilityReport {
    AgentCapabilityReport {
        provider,
        agent: provider.as_str().to_owned(),
        version: None,
        install_state: InstallState::DetectedNotRunnable,
        capability: Capability {
            structured_events: false,
            resume: false,
            approval: false,
            safe_pause: false,
            terminal_fallback: false,
        },
        unavailable_reason: Some(reason.into()),
        executable_hash: None,
        configuration_source: None,
    }
}

#[cfg(test)]
mod continuity_tests {
    use super::{MISSION_COMMANDS, MissionCommandRequest, MissionService, usage_record_from_event};
    use adapter_core::AgentEvent;
    use mission_domain::{EventId, EventKind, MissionId, RouteId};
    use mission_policy::{BudgetLimits, BudgetTracker, UnknownUsagePolicy};
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn recovery_command_and_request_fields_are_exposed() {
        assert!(MISSION_COMMANDS.contains(&"build_recovery_package"));
        assert!(MISSION_COMMANDS.contains(&"verify_recovery"));
        assert!(MISSION_COMMANDS.contains(&"resolve_recovery"));
        let request: MissionCommandRequest = serde_json::from_value(serde_json::json!({
            "missionId": "00000000-0000-0000-0000-000000000001",
            "routeId": "00000000-0000-0000-0000-000000000002",
            "checkpointId": "checkpoint-1",
            "contractVersion": 1,
            "ledgerSequence": 3,
            "loadoutFingerprint": "loadout",
            "contextPackHash": "context",
            "pendingApprovalHash": "approval"
        }))
        .expect("request");
        assert_eq!(request.checkpoint_id.as_deref(), Some("checkpoint-1"));
        assert_eq!(request.context_pack_hash.as_deref(), Some("context"));
    }

    #[test]
    fn duplicate_create_is_rejected_before_any_ledger_append() {
        let temp = tempfile::tempdir().expect("temporary mission data directory");
        let service = MissionService::new(temp.path().to_path_buf()).expect("mission service");
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let request = json!({
            "missionId": mission_id.to_string(),
            "routeId": route_id.to_string(),
            "projectRoot": "C:/managed/duplicate-test",
            "goal": "verify duplicate create"
        });

        service
            .dispatch_json("create_mission", request.clone())
            .expect("first create");
        let ledger = service.open_ledger().expect("open ledger after create");
        let before = ledger
            .replay_events(&mission_id)
            .expect("replay created mission");
        assert_eq!(before.len(), 3);
        assert_eq!(before.last().expect("contract event").sequence, 3);
        drop(ledger);

        // Exercise the durable duplicate check independently of the in-memory
        // map, as would happen after a partial in-process state loss.
        service
            .missions
            .lock()
            .expect("mission map")
            .remove(&mission_id.to_string());
        assert_eq!(
            service.dispatch_json("create_mission", request),
            Err("MISSION_ALREADY_EXISTS".to_owned())
        );
        let ledger = service
            .open_ledger()
            .expect("reopen ledger after duplicate");
        let after = ledger
            .replay_events(&mission_id)
            .expect("replay duplicate mission");
        assert_eq!(after.len(), before.len());
        assert_eq!(
            after.last().expect("route event").sequence,
            before.last().expect("contract event").sequence
        );
    }

    #[test]
    fn token_usage_over_limit_records_budget_events_and_requests_safe_pause() {
        let mission_id = MissionId::new();
        let route_id = RouteId::new();
        let mut actor = crate::mission_actor::MissionActor::new(mission_id, route_id, Vec::new());
        let event = AgentEvent {
            event_id: EventId::new(),
            agent_run_id: Some("run-budget".to_owned()),
            event_kind: EventKind::AgentMessage,
            payload: json!({
                "native_type": "thread/tokenUsage/updated",
                "tokenUsage": {"total": {"totalTokens": 101}}
            }),
            requires_safe_pause: false,
            raw_evidence: None,
        };
        let mut previous_tokens = 0;
        let record = usage_record_from_event(&event, &mut previous_tokens).expect("usage record");
        let mut tracker = BudgetTracker::new(
            1,
            BudgetLimits {
                tokens: 100,
                money_micros: 1_000,
                wall_clock: Duration::from_secs(60),
                changed_lines: 1_000,
                changed_files: 100,
                model_calls: 100,
            },
            UnknownUsagePolicy::Pause,
        );
        tracker.record(record);
        let signals = tracker.evaluate_safe_boundary();
        assert!(signals.iter().any(|signal| {
            matches!(
                signal,
                mission_policy::BudgetSignal::PauseAtSafeBoundary(
                    mission_policy::BudgetDimension::Tokens
                )
            )
        }));
        assert!(
            actor
                .apply_budget_signals(&signals)
                .expect("budget signals")
        );
        assert!(
            actor
                .ledger()
                .iter()
                .any(|event| event.kind == EventKind::BudgetExceeded)
        );
        assert!(
            actor
                .ledger()
                .iter()
                .any(|event| event.kind == EventKind::PauseRequested)
        );
    }
}
