#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod supervisor_bridge;
mod update_guard;

use std::sync::Arc;

use supervisor_bridge::{
    ALLOWED_COMMANDS, LocalSupervisorTransport, MISSION_ALLOWED_COMMANDS, MissionCommandRequest,
    MissionCommandResult, PublicSupervisorStatus, SupervisorBridge,
};
use tauri::{Manager, State};

type NativeSupervisorBridge = Arc<SupervisorBridge<LocalSupervisorTransport>>;

macro_rules! tauri_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

#[tauri::command]
async fn supervisor_status(
    state: State<'_, NativeSupervisorBridge>,
) -> Result<PublicSupervisorStatus, ()> {
    Ok(Arc::clone(state.inner()).supervisor_status_async().await)
}

#[tauri::command]
async fn ping_supervisor(
    state: State<'_, NativeSupervisorBridge>,
) -> Result<PublicSupervisorStatus, ()> {
    Ok(Arc::clone(state.inner()).ping_supervisor_async().await)
}

async fn dispatch_mission(
    state: State<'_, NativeSupervisorBridge>,
    command: &'static str,
    request: MissionCommandRequest,
) -> Result<MissionCommandResult, String> {
    let request =
        serde_json::to_value(request).map_err(|_| "MISSION_REQUEST_INVALID".to_owned())?;
    let result = Arc::clone(state.inner())
        .dispatch_mission_async(command, request)
        .await?;
    serde_json::from_value(result).map_err(|_| "MISSION_RESULT_INVALID".to_owned())
}

macro_rules! mission_command {
    ($name:ident) => {
        #[tauri::command]
        async fn $name(
            state: State<'_, NativeSupervisorBridge>,
            request: MissionCommandRequest,
        ) -> Result<MissionCommandResult, String> {
            dispatch_mission(state, stringify!($name), request).await
        }
    };
}

mission_command!(create_mission);
mission_command!(update_mission_contract);
mission_command!(launch_route);
mission_command!(subscribe_mission);
mission_command!(request_safe_pause);
mission_command!(request_force_termination);
mission_command!(force_terminate);
mission_command!(resolve_approval);
mission_command!(build_recovery_package);
mission_command!(verify_recovery);
mission_command!(resolve_recovery);
mission_command!(review_memory);
mission_command!(handoff_provider);
mission_command!(provider_capabilities);
mission_command!(storage_preview);
mission_command!(export_preview);
mission_command!(diagnostic_preview);
mission_command!(archive_mission);
mission_command!(delete_mission);
mission_command!(materialize_export);

fn main() {
    debug_assert_eq!(ALLOWED_COMMANDS, ["supervisor_status", "ping_supervisor"]);
    debug_assert_eq!(
        MISSION_ALLOWED_COMMANDS,
        [
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
            "materialize_export"
        ]
    );
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(debug_assertions)]
            let data_dir = std::env::var_os("MISSION_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            #[cfg(not(debug_assertions))]
            let data_dir = app.path().app_data_dir()?;
            let transport = LocalSupervisorTransport::production(data_dir)?;
            app.manage(Arc::new(SupervisorBridge::new(transport)));
            Ok(())
        })
        .invoke_handler(tauri_handler!(
            supervisor_status,
            ping_supervisor,
            create_mission,
            update_mission_contract,
            launch_route,
            subscribe_mission,
            request_safe_pause,
            request_force_termination,
            force_terminate,
            resolve_approval,
            build_recovery_package,
            verify_recovery,
            resolve_recovery,
            review_memory,
            handoff_provider,
            provider_capabilities,
            storage_preview,
            export_preview,
            diagnostic_preview,
            archive_mission,
            delete_mission,
            materialize_export
        ))
        .run(tauri::generate_context!())
        .expect("run Agent Mission Control desktop");
}
