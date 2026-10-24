#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod i18n;
mod tray;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use outerspace_core::display::parsec::ParsecVdd;
use outerspace_core::input::fence::Fence;
use outerspace_core::{MCP_PORT, PowerController, PowerState, config, paths};
use outerspace_mcp::CancellationToken;
use tauri::{Manager, RunEvent, WindowEvent};
use tracing_subscriber::EnvFilter;

/// Passed by the "Start with Windows" entry: start in the tray without showing the window.
const HIDDEN_ARG: &str = "--hidden";

fn main() {
    let _log_guard = init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "OuterSpace starting");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_window(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::set_power,
            commands::install_driver,
            commands::set_autostart,
            commands::client_snippets,
            commands::add_client,
            commands::open_logs,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = config::load_or_create();
            Fence::set_enabled(settings.fence);
            let fence = Fence::start();
            let power = Arc::new(PowerController::new(Box::new(ParsecVdd::new()), move |state| {
                Fence::set_area(match state {
                    PowerState::On { display } => Some(display.rect()),
                    _ => None,
                });
                commands::publish(&handle)
            }));
            let mcp_stop = CancellationToken::new();
            tauri::async_runtime::spawn({
                let (power, stop) = (power.clone(), mcp_stop.clone());
                let token = settings.token;
                async move {
                    if let Err(err) = outerspace_mcp::run(power, MCP_PORT, token, stop).await {
                        tracing::error!(%err, port = MCP_PORT, "MCP server could not run");
                    }
                }
            });
            app.manage(commands::AppState { power, installing: AtomicBool::new(false), mcp_stop, _fence: fence });
            tray::create(app.handle())?;
            if !std::env::args().any(|arg| arg == HIDDEN_ARG) {
                tray::show_window(app.handle());
            }
            // Dev builds only: lets the hardware tests restart the app without a human at the switch.
            if cfg!(debug_assertions) && std::env::var_os("OUTERSPACE_DEV_AUTO_ON").is_some() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move { commands::switch_power(&handle, true).await });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window only hides it; the app keeps living in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the OuterSpace app");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            let state = app.state::<commands::AppState>();
            state.mcp_stop.cancel();
            // Unplug cleanly; the driver would also drop the display ~1 s after the pings stop.
            state.power.turn_off();
            tracing::info!("OuterSpace exiting");
        }
    });
}

/// `logs/outerspace.YYYY-MM-DD.log`, one file a day, the last week kept.
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("outerspace")
        .filename_suffix("log")
        .max_log_files(7)
        .build(paths::logs_dir())
        .expect("the logs folder is writable");
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    guard
}
