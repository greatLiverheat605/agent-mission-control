use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adapter_claude::{ClaudeAdapter, ClaudeAdapterOptions};
use adapter_codex::CodexAdapter;
use adapter_core::{
    AgentAdapter, AgentCapabilityReport, AgentHandle, LoadoutSnapshot, ProviderId,
    StartAgentRequest,
};
use mission_domain::{EventKind, MissionId, RouteId};
use mission_ledger::{EncryptedLedger, WindowsCredentialKeyStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;

pub const MISSION_COMMANDS: [&str; 6] = [
    "create_mission",
    "update_mission_contract",
    "launch_route",
    "subscribe_mission",
    "request_safe_pause",
    "force_terminate",
];

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionCommandRequest {
    #[serde(default)]
    pub provider: Option<ProviderId>,
    #[serde(default)]
    pub loadout: Option<LoadoutSnapshot>,
    pub mission_id: Option<String>,
    pub route_id: Option<String>,
    pub expected_version: Option<u64>,
    pub project_root: Option<String>,
    pub goal: Option<String>,
    pub reason: Option<String>,
    pub confirmation_token: Option<String>,
    #[serde(default)]
    pub loadout_fingerprint: Option<String>,
    #[serde(default)]
    pub resume_token: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionCommandResult {
    pub accepted: bool,
    pub mission_id: Option<String>,
    pub route_id: Option<String>,
    pub sequence: Option<u64>,
    pub error_code: Option<String>,
    #[serde(default)]
    pub provider: Option<ProviderId>,
    #[serde(default)]
    pub capability: Option<AgentCapabilityReport>,
    #[serde(default)]
    pub events: Vec<Value>,
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
    runs: Arc<Mutex<HashMap<String, String>>>,
    run_providers: Arc<Mutex<HashMap<String, ProviderId>>>,
    adapters: Arc<HashMap<ProviderId, Arc<dyn AgentAdapter>>>,
    loadouts: Arc<Mutex<crate::loadout_monitor::LoadoutMonitor>>,
    ledger_path: PathBuf,
    last_ui_seen: Mutex<Instant>,
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
            runs: Arc::new(Mutex::new(HashMap::new())),
            run_providers: Arc::new(Mutex::new(HashMap::new())),
            adapters: Arc::new(adapters),
            loadouts: Arc::new(Mutex::new(crate::loadout_monitor::LoadoutMonitor::default())),
            ledger_path: data_dir.join("mission-ledger.db"),
            last_ui_seen: Mutex::new(Instant::now()),
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
            missions.entry(mission_id.to_string()).or_insert_with(|| {
                crate::mission_actor::MissionActor::new(mission_id, route_id, ledger)
            });
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
                if expired {
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
        let result = self.runtime.block_on(self.dispatch(command, request))?;
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
            "force_terminate" => self.force_terminate(request).await,
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
            provider: None,
            capability: None,
            events,
        })
    }

    fn create_mission(
        &self,
        request: MissionCommandRequest,
    ) -> Result<MissionCommandResult, String> {
        let mission_id = request
            .mission_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "MISSION_ID_INVALID".to_owned())?
            .unwrap_or_default();
        let route_id = request
            .route_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "ROUTE_ID_INVALID".to_owned())?
            .unwrap_or_default();
        let mut actor =
            crate::mission_actor::MissionActor::new(mission_id, route_id, self.open_ledger()?);
        actor
            .record_event(
                EventKind::MissionCreated,
                serde_json::json!({"project_root": request.project_root, "goal": request.goal}),
            )
            .map_err(|error| format!("MISSION_CREATE_FAILED:{error:?}"))?;
        actor
            .record_event(
                EventKind::RouteCreated,
                serde_json::json!({"route_id": route_id}),
            )
            .map_err(|error| format!("ROUTE_CREATE_FAILED:{error:?}"))?;
        let mission_key = mission_id.to_string();
        let mut missions = self
            .missions
            .lock()
            .map_err(|_| "MISSION_STATE_POISONED".to_owned())?;
        if missions.contains_key(&mission_key) {
            return Err("MISSION_ALREADY_EXISTS".to_owned());
        }
        missions.insert(mission_key.clone(), actor);
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
            provider: request.provider,
            capability: None,
            events,
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
        let provider = request.provider.unwrap_or_default();
        if request
            .loadout
            .as_ref()
            .is_some_and(|loadout| loadout.provider != provider)
        {
            return Err("LOADOUT_PROVIDER_MISMATCH".to_owned());
        }
        let adapter = self.adapter(provider)?;
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
            loadout_fingerprint,
            resume_token: request.resume_token.clone(),
            loadout: request.loadout.clone(),
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
        let run_id = handle.run_id().to_owned();
        self.runtime.spawn(async move {
            while let Some(event) = handle.next_event().await {
                let requires_pause = event.requires_safe_pause;
                let mut should_remove = false;
                let mut adapter_pause = false;
                if let Ok(mut missions) = missions.lock() {
                    if let Some(actor) = missions.get_mut(&mission_id.to_string()) {
                        if actor.record_agent_event(event).is_err() {
                            should_remove = true;
                        } else if requires_pause {
                            let _ = actor.request_safe_pause("adapter requires safe pause");
                            adapter_pause = true;
                        }
                    } else {
                        should_remove = true;
                    }
                } else {
                    should_remove = true;
                }
                if should_remove {
                    break;
                }
                if adapter_pause {
                    let _ = adapter.request_safe_pause(&run_id).await;
                }
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
