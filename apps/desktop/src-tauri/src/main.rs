#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod supervisor_bridge;

use std::sync::Arc;

use supervisor_bridge::{
    ALLOWED_COMMANDS, LocalSupervisorTransport, MISSION_ALLOWED_COMMANDS, MissionCommandRequest,
    MissionCommandResult, PublicSupervisorStatus, SupervisorBridge, validate_mission_request,
};
use tauri::{Manager, State};

macro_rules! tauri_handler {
    ($($command:ident),+ $(,)?) => {
        tauri::generate_handler![$($command),+]
    };
}

type NativeSupervisorBridge = Arc<SupervisorBridge<LocalSupervisorTransport>>;

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

fn unsupported_mission_command(
    request: MissionCommandRequest,
) -> Result<MissionCommandResult, String> {
    validate_mission_request(&request).map_err(str::to_owned)?;
    Ok(MissionCommandResult {
        accepted: false,
        mission_id: request.mission_id,
        sequence: None,
        error_code: Some("MISSION_COMMAND_UNAVAILABLE"),
    })
}

#[tauri::command]
async fn create_mission(request: MissionCommandRequest) -> Result<MissionCommandResult, String> {
    unsupported_mission_command(request)
}
#[tauri::command]
async fn update_mission_contract(
    request: MissionCommandRequest,
) -> Result<MissionCommandResult, String> {
    unsupported_mission_command(request)
}
#[tauri::command]
async fn launch_route(request: MissionCommandRequest) -> Result<MissionCommandResult, String> {
    unsupported_mission_command(request)
}
#[tauri::command]
async fn subscribe_mission(request: MissionCommandRequest) -> Result<MissionCommandResult, String> {
    unsupported_mission_command(request)
}
#[tauri::command]
async fn request_safe_pause(
    request: MissionCommandRequest,
) -> Result<MissionCommandResult, String> {
    unsupported_mission_command(request)
}
#[tauri::command]
async fn force_terminate(request: MissionCommandRequest) -> Result<MissionCommandResult, String> {
    unsupported_mission_command(request)
}

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
            "force_terminate"
        ]
    );
    tauri::Builder::default()
        .setup(|app| {
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
            force_terminate
        ))
        .run(tauri::generate_context!())
        .expect("run Agent Mission Control desktop");
}
