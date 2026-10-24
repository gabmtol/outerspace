use outerspace_core::PowerState;
use outerspace_core::input::fence::Fence;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, Rect, WebviewWindow, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::commands::{self, AppState};
use crate::i18n::t;

const TRAY_ID: &str = "outerspace";
const ICON_ON: &[u8] = include_bytes!("../icons/tray-on.png");
const ICON_OFF: &[u8] = include_bytes!("../icons/tray-off.png");
const ICON_ERROR: &[u8] = include_bytes!("../icons/tray-error.png");

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<AppState>().power.state();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(&state)?)
        .tooltip(header(&state))
        .menu(&menu(app, &state)?)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left, button_state: MouseButtonState::Up, rect, ..
            } = event
            {
                toggle_window(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    Ok(())
}

pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let state = app.state::<AppState>().power.state();
    if let Ok(image) = icon(&state) {
        let _ = tray.set_icon(Some(image));
    }
    let _ = tray.set_tooltip(Some(header(&state)));
    if let Ok(menu) = menu(app, &state) {
        let _ = tray.set_menu(Some(menu));
    }
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn icon(state: &PowerState) -> tauri::Result<Image<'static>> {
    Image::from_bytes(match state {
        PowerState::On { .. } | PowerState::Starting | PowerState::Stopping => ICON_ON,
        PowerState::Off => ICON_OFF,
        PowerState::Error { .. } => ICON_ERROR,
    })
}

fn header(state: &PowerState) -> String {
    t(match state {
        PowerState::Off => "tray.headerOff",
        PowerState::Starting => "tray.headerStarting",
        PowerState::Stopping => "tray.headerStopping",
        PowerState::On { .. } => "tray.headerOn",
        PowerState::Error { .. } => "tray.headerError",
    })
}

fn menu(app: &AppHandle, state: &PowerState) -> tauri::Result<Menu<Wry>> {
    let busy = matches!(state, PowerState::Starting | PowerState::Stopping);
    let on = matches!(state, PowerState::On { .. });
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "header", header(state), false, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "power", t(if on { "tray.turnOff" } else { "tray.turnOn" }), !busy, None::<&str>)?,
            &MenuItem::with_id(app, "open", t("tray.open"), true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &CheckMenuItem::with_id(app, "fence", t("tray.fence"), true, Fence::enabled(), None::<&str>)?,
            &CheckMenuItem::with_id(app, "autostart", t("tray.autostart"), true, autostart, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", t("tray.quit"), true, None::<&str>)?,
        ],
    )
}

fn on_menu(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "power" => {
            let app = app.clone();
            let on = !app.state::<AppState>().power.is_on();
            tauri::async_runtime::spawn(async move { commands::switch_power(&app, on).await });
        }
        "open" => show_window(app),
        "fence" => {
            Fence::set_enabled(!Fence::enabled());
            refresh(app);
        }
        "autostart" => {
            let launcher = app.autolaunch();
            let _ = if launcher.is_enabled().unwrap_or(false) { launcher.disable() } else { launcher.enable() };
            commands::publish(app);
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Left click on the tray icon: open the window right above the icon, or hide it if it is showing.
fn toggle_window(app: &AppHandle, tray: Rect) {
    let Some(window) = app.get_webview_window("main") else { return };
    if window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    if let Some(position) = position_near(&window, tray) {
        let _ = window.set_position(position);
    }
    show_window(app);
}

fn position_near(window: &WebviewWindow, tray: Rect) -> Option<PhysicalPosition<i32>> {
    let scale = window.scale_factor().ok()?;
    let anchor = tray.position.to_physical::<f64>(scale);
    let anchor_size = tray.size.to_physical::<f64>(scale);
    let size = window.outer_size().ok()?;
    let monitor = window.monitor_from_point(anchor.x, anchor.y).ok()??;
    let area = monitor.work_area();
    let (left, top) = (area.position.x, area.position.y);
    let (right, bottom) = (left + area.size.width as i32, top + area.size.height as i32);
    let gap = (12.0 * scale) as i32;

    let x = (anchor.x + anchor_size.width / 2.0) as i32 - size.width as i32 / 2;
    let x = x.clamp(left + gap, (right - size.width as i32 - gap).max(left + gap));
    // Taskbar at the bottom (the default) puts the icon below the work area: open upwards.
    let y = if anchor.y as i32 >= bottom { bottom - size.height as i32 - gap } else { top + gap };
    Some(PhysicalPosition::new(x, y))
}
