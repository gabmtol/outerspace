//! "Peek": a live, view-only window onto the virtual display, for the user. The page pulls each
//! frame from the `peek://` protocol, so the display is only captured while the window is open, and
//! only as fast as the window can show it.

use outerspace_core::PowerState;
use outerspace_core::capture::{self, Format};
use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::commands::AppState;

pub const LABEL: &str = "peek";
pub const SCHEME: &str = "peek";

/// Shows the peek window, creating it on the async runtime if needed: on Windows, building a
/// WebView2 window from an event handler or a synchronous command deadlocks the app.
pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        return crate::tray::reveal(&window);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let built = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("peek.html".into()))
            .title("OuterSpace — Virtual display")
            .inner_size(800.0, 490.0)
            .min_inner_size(400.0, 265.0)
            .decorations(false)
            .shadow(true)
            .center()
            .build();
        match built {
            Ok(window) => crate::tray::reveal(&window),
            Err(err) => tracing::warn!(%err, "could not open the peek window"),
        }
    });
}

/// `peek://localhost/frame?w=<pixels>`: the virtual display as a JPEG scaled to that width, or
/// 204 No Content while it is off.
pub fn serve(ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let width = request
        .uri()
        .query()
        .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("w=")))
        .and_then(|w| w.parse::<u32>().ok())
        .unwrap_or(960);
    tauri::async_runtime::spawn_blocking(move || responder.respond(frame(&app, width)));
}

fn frame(app: &AppHandle, width: u32) -> Response<Vec<u8>> {
    let PowerState::On { display } = app.state::<AppState>().power.state() else {
        return empty(StatusCode::NO_CONTENT);
    };
    // Scaled while copying, not after: a live view at ~8 fps must stay cheap.
    match capture::grab_scaled(display.rect(), width as i32)
        .and_then(|image| capture::encode(&image, 1.0, Format::Jpeg))
    {
        Ok(shot) => Response::builder()
            .header("Content-Type", shot.mime)
            .header("Cache-Control", "no-store")
            // The page lives on another origin (tauri.localhost, or the dev server) than peek.localhost.
            // Only the app's own webviews can reach this protocol, so any origin is fine.
            .header("Access-Control-Allow-Origin", "*")
            .body(shot.bytes)
            .unwrap_or_else(|_| empty(StatusCode::INTERNAL_SERVER_ERROR)),
        Err(err) => {
            tracing::warn!(%err, "peek frame failed");
            empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn empty(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(status).header("Access-Control-Allow-Origin", "*").body(Vec::new()).unwrap_or_default()
}
