#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clients;
mod commands;
mod i18n;
mod tray;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use outerspace_core::display::parsec::ParsecVdd;
use outerspace_core::{PowerController, paths};
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
            commands::open_logs,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let power = PowerController::new(Box::new(ParsecVdd::new()), move |_| commands::publish(&handle));
            app.manage(commands::AppState { power: Arc::new(power), installing: AtomicBool::new(false) });
            tray::create(app.handle())?;
            if !std::env::args().any(|arg| arg == HIDDEN_ARG) {
                tray::show_window(app.handle());
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
            // Unplug cleanly; the driver would also drop the display ~1 s after the pings stop.
            app.state::<commands::AppState>().power.turn_off();
            tracing::info!("OuterSpace exiting");
        }
    });
}

fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let file = tracing_appender::rolling::never(paths::logs_dir(), "outerspace.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    guard
}
