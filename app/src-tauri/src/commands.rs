use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use outerspace_core::{MCP_PORT, PowerController, PowerState, paths, system};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::clients::{self, ClientSnippet};
use crate::tray;

pub struct AppState {
    pub power: Arc<PowerController>,
    pub installing: AtomicBool,
    /// Stops the MCP server on exit.
    pub mcp_stop: outerspace_mcp::CancellationToken,
}

/// Everything the UI renders. Mirrored by `Status` in app/ui/src/types.ts.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    power: PowerState,
    installing: bool,
    port: u16,
    version: String,
    autostart: bool,
    transparency: bool,
}

pub fn snapshot(app: &AppHandle) -> Status {
    let state = app.state::<AppState>();
    Status {
        power: state.power.state(),
        installing: state.installing.load(Ordering::Relaxed),
        port: MCP_PORT,
        version: app.package_info().version.to_string(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        transparency: system::transparency_enabled(),
    }
}

/// Pushes the current status to the window and the tray.
pub fn publish(app: &AppHandle) {
    let _ = app.emit("status", snapshot(app));
    tray::refresh(app);
}

/// Shared by the switch and the tray menu. Runs the blocking driver work off the async runtime.
pub async fn switch_power(app: &AppHandle, on: bool) {
    let power = app.state::<AppState>().power.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || if on { power.turn_on() } else { power.turn_off() }).await;
}

#[tauri::command]
pub fn status(app: AppHandle) -> Status {
    snapshot(&app)
}

#[tauri::command]
pub async fn set_power(app: AppHandle, on: bool) -> Result<Status, String> {
    switch_power(&app, on).await;
    Ok(snapshot(&app))
}

#[tauri::command]
pub async fn install_driver(app: AppHandle) -> Result<Status, String> {
    let state = app.state::<AppState>();
    if state.installing.swap(true, Ordering::Relaxed) {
        return Ok(snapshot(&app));
    }
    publish(&app);
    let power = state.power.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || power.install_driver()).await;
    state.installing.store(false, Ordering::Relaxed);
    publish(&app);
    Ok(snapshot(&app))
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<Status, String> {
    let launcher = app.autolaunch();
    let result = if enabled { launcher.enable() } else { launcher.disable() };
    result.map_err(|e| e.to_string())?;
    publish(&app);
    Ok(snapshot(&app))
}

#[tauri::command]
pub fn client_snippets() -> Vec<ClientSnippet> {
    clients::snippets()
}

#[tauri::command]
pub fn open_logs() -> Result<(), String> {
    std::process::Command::new("explorer.exe").arg(paths::logs_dir()).spawn().map(drop).map_err(|e| e.to_string())
}
