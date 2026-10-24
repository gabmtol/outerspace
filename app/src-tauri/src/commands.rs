use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use outerspace_core::setup::{self, Client, ClientInfo};
use outerspace_core::{DisplayMode, MCP_PORT, PowerController, PowerState, apps, config, paths, system};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::tray;

pub struct AppState {
    pub power: Arc<PowerController>,
    pub installing: AtomicBool,
    /// Stops the MCP server on exit.
    pub mcp_stop: outerspace_mcp::CancellationToken,
    /// Keeps the user's mouse out of the virtual display for as long as the app runs.
    pub _fence: outerspace_core::input::fence::Fence,
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
    /// Windows mostly on the virtual display, while it is on.
    windows_on_display: Option<usize>,
    /// The chosen resolution, and the ones to choose from.
    mode: DisplayMode,
    modes: Vec<DisplayMode>,
}

pub fn snapshot(app: &AppHandle) -> Status {
    let state = app.state::<AppState>();
    let power = state.power.state();
    let windows_on_display = match &power {
        PowerState::On { display } => {
            let rect = display.rect();
            Some(apps::list().iter().filter(|w| w.rect.fraction_inside(&rect) > 0.5).count())
        }
        _ => None,
    };
    Status {
        power,
        installing: state.installing.load(Ordering::Relaxed),
        port: MCP_PORT,
        version: app.package_info().version.to_string(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        transparency: system::transparency_enabled(),
        windows_on_display,
        mode: state.power.mode(),
        modes: state.power.modes().to_vec(),
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

/// Chooses the virtual display's resolution, remembered across restarts. A display that is on
/// switches right away.
#[tauri::command]
pub async fn set_resolution(app: AppHandle, width: u32, height: u32) -> Result<Status, String> {
    let power = app.state::<AppState>().power.clone();
    let mode = power
        .modes()
        .iter()
        .copied()
        .find(|m| (m.width, m.height) == (width, height))
        .ok_or_else(|| format!("{width}×{height} is not an available resolution"))?;
    tauri::async_runtime::spawn_blocking(move || power.set_mode(mode))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mut settings = config::load_or_create();
    settings.mode = mode;
    config::save(&settings);
    tracing::info!(width, height, "resolution chosen");
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
pub async fn client_snippets() -> Result<Vec<ClientInfo>, String> {
    // Reads three config files; keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(clients).await.map_err(|e| e.to_string())
}

/// Adds OuterSpace to one client's config. Returns what was done and the refreshed list.
#[tauri::command]
pub async fn add_client(id: String) -> Result<(String, Vec<ClientInfo>), String> {
    let client = Client::parse(&id).ok_or_else(|| format!("unknown client {id}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let done = setup::add(client, &setup::shim_path()).map_err(|e| e.to_string());
        tracing::info!(client = id, ok = done.is_ok(), "add client");
        done.map(|message| (message, clients()))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn clients() -> Vec<ClientInfo> {
    let shim = setup::shim_path();
    Client::ALL.into_iter().map(|client| setup::describe(client, &shim)).collect()
}

/// Async on purpose: on Windows, building a WebView2 window from a synchronous command deadlocks
/// (the command runs on the main thread, which the new webview needs).
#[tauri::command]
pub async fn open_peek(app: AppHandle) -> Result<(), String> {
    crate::peek::open(&app);
    Ok(())
}

#[tauri::command]
pub fn open_logs() -> Result<(), String> {
    std::process::Command::new("explorer.exe").arg(paths::logs_dir()).spawn().map(drop).map_err(|e| e.to_string())
}
