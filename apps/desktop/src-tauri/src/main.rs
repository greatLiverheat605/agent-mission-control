#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod supervisor_bridge;

use std::sync::Arc;

use supervisor_bridge::{
    ALLOWED_COMMANDS, LocalSupervisorTransport, PublicSupervisorStatus, SupervisorBridge,
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

fn main() {
    debug_assert_eq!(ALLOWED_COMMANDS, ["supervisor_status", "ping_supervisor"]);
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let transport = LocalSupervisorTransport::production(data_dir)?;
            app.manage(Arc::new(SupervisorBridge::new(transport)));
            Ok(())
        })
        .invoke_handler(supervisor_bridge::supervisor_commands!(tauri_handler))
        .run(tauri::generate_context!())
        .expect("run Agent Mission Control desktop");
}
