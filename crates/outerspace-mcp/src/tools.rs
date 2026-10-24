//! The MCP tools (docs/PLANO.md §5). Coordinates agents see are always *display coordinates*:
//! (0, 0) is the top-left corner of the virtual display, in its physical pixels (it runs at 100%).

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use outerspace_core::apps::{self, Keeper, LaunchRequest, WindowInfo};
use outerspace_core::capture::{self, Format};
use outerspace_core::{DisplayInfo, PowerController, PowerState, Rect};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::log::ActionLog;

const OFF: &str = "OuterSpace is off, so the virtual display does not exist right now. Ask the user to \
                   turn it on (the switch in the OuterSpace window or its tray menu).";
/// A window counts as "on the virtual display" when most of it is there.
const ON_DISPLAY: f64 = 0.5;

type ToolResult = Result<CallToolResult, ErrorData>;

#[derive(Clone)]
pub struct OuterSpace {
    power: Arc<PowerController>,
    log: Arc<ActionLog>,
    /// Shared by all sessions: holds the agent's windows on the virtual display.
    keeper: Arc<Keeper>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LaunchParams {
    /// What to start: an executable name or path ("notepad", "C:\\Tools\\app.exe"), an App Paths name
    /// ("msedge", "chrome", "firefox", "Code.exe" — not "code", which resolves to a .cmd script), a document
    /// path, or a `shell:AppsFolder\\...` id. For web pages, launch the browser with `--new-window <url>`
    /// (Firefox: `-new-window <url>`) so the page does not open in the user's own browser window.
    program: String,
    /// Command-line arguments, one per item (quoting is handled for you).
    #[serde(default)]
    args: Vec<String>,
    /// Working directory.
    cwd: Option<String>,
    /// How long to wait for the first window, in milliseconds (default 10000).
    wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Also list the user's windows on their own monitors (default false). Their `rect` is still in
    /// display coordinates, so it falls outside the virtual display.
    #[serde(default)]
    all: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
pub struct Region {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    /// Lossless; best for reading text.
    #[default]
    Png,
    /// Smaller; fine for photos and video frames.
    Jpeg,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotParams {
    /// Capture just this window (clipped to the virtual display).
    hwnd: Option<i64>,
    /// Capture just this part of the display, in display coordinates.
    region: Option<Region>,
    /// Scale factor from 0.1 to 1 (default 0.667, i.e. 1280×720 for the full display). Coordinates you
    /// pass to other tools are always display coordinates: display = region origin + image / scale.
    scale: Option<f32>,
    #[serde(default)]
    format: ImageFormat,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    /// The whole display above its taskbar.
    Fill,
    Left,
    Right,
    /// Keep the size, center it.
    Center,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MoveParams {
    /// Window to move; get it from `list_windows` (use `all: true` to bring one of the user's windows over
    /// — only when the user asked for it).
    hwnd: i64,
    /// A ready-made placement; overrides x/y/w/h.
    preset: Option<Preset>,
    /// New left edge in display coordinates; omitted values keep the current ones.
    x: Option<i32>,
    y: Option<i32>,
    w: Option<i32>,
    h: Option<i32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CloseParams {
    /// Window to close (must be on the virtual display).
    hwnd: i64,
    /// Kill the process instead of asking the window to close. Unsaved work is lost.
    #[serde(default)]
    force: bool,
}

/// A window as agents see it.
#[derive(Debug, Serialize)]
struct AgentWindow {
    hwnd: i64,
    pid: u32,
    process: String,
    title: String,
    class: String,
    /// Visible frame in display coordinates.
    rect: Rect,
    on_virtual_display: bool,
    minimized: bool,
    focused: bool,
}

impl AgentWindow {
    fn new(window: WindowInfo, display: Rect) -> Self {
        AgentWindow {
            hwnd: window.hwnd,
            pid: window.pid,
            process: window.process,
            title: window.title,
            class: window.class,
            rect: to_display(window.rect, display),
            on_virtual_display: window.rect.fraction_inside(&display) > ON_DISPLAY,
            minimized: window.minimized,
            focused: window.focused,
        }
    }
}

#[tool_router]
impl OuterSpace {
    pub fn new(power: Arc<PowerController>, log: Arc<ActionLog>, keeper: Arc<Keeper>) -> Self {
        OuterSpace { power, log, keeper }
    }

    #[tool(description = "Whether OuterSpace is on and where its virtual display is. Call this first. Works even \
                          when OuterSpace is off.")]
    async fn status(&self) -> ToolResult {
        let state = self.power.state();
        let (display, work_area, windows) = match &state {
            PowerState::On { display } => {
                let rect = display.rect();
                let (work, count) = blocking(move || {
                    let count = apps::list().iter().filter(|w| w.rect.fraction_inside(&rect) > ON_DISPLAY).count();
                    (apps::work_area(rect), count)
                })
                .await?;
                (
                    Some(json!({ "width": display.width, "height": display.height, "hz": display.hz })),
                    Some(to_display(work, rect)),
                    Some(count),
                )
            }
            _ => (None, None, None),
        };
        let state_name = match &state {
            PowerState::Off => "off",
            PowerState::Starting => "starting",
            PowerState::Stopping => "stopping",
            PowerState::On { .. } => "on",
            PowerState::Error { .. } => "error",
        };
        json_result(json!({
            "enabled": matches!(state, PowerState::On { .. }),
            "state": state_name,
            "display": display,
            "work_area": work_area,
            "windows_on_display": windows,
            "version": env!("CARGO_PKG_VERSION"),
        }))
    }

    #[tool(description = "Start a program on the virtual display. Its first window fills the display (above the \
                          taskbar) without taking focus from the user. Returns the window, or `pending: true` if \
                          none appeared in time (check `list_windows` again).")]
    async fn launch(&self, Parameters(params): Parameters<LaunchParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display,
            Err(off) => return Ok(off),
        };
        let args = json!({ "program": params.program, "args": params.args });
        let request = LaunchRequest {
            program: params.program,
            args: params.args,
            cwd: params.cwd,
            wait: Duration::from_millis(params.wait_ms.unwrap_or(10_000).min(60_000)),
        };
        let rect = display.rect();
        match blocking(move || apps::launch(&request, rect)).await? {
            Ok(launched) => {
                self.log.record("launch", args, true, if launched.pending { "pending" } else { "window" });
                let window = match launched.window {
                    Some(window) => {
                        self.keeper.track(window.hwnd, apps::work_area(rect));
                        Some(blocking(move || settled_on(window, rect)).await?)
                    }
                    None => None,
                };
                json_result(json!({
                    "pid": launched.pid,
                    "window": window.map(|w| AgentWindow::new(w, rect)),
                    "pending": launched.pending,
                    "focus_repaired": launched.focus_repaired,
                }))
            }
            Err(err) => self.fail("launch", args, err),
        }
    }

    #[tool(description = "List windows on the virtual display, front to back, with their hwnd, process, title and \
                          rect in display coordinates.")]
    async fn list_windows(&self, Parameters(params): Parameters<ListParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        let windows: Vec<AgentWindow> = blocking(apps::list)
            .await?
            .into_iter()
            .map(|w| AgentWindow::new(w, display))
            .filter(|w| params.all || w.on_virtual_display)
            .collect();
        json_result(windows)
    }

    #[tool(description = "Take a screenshot of the virtual display (or one window or region of it). Look before \
                          you click: returns the image plus its size, scale and the display region it covers.")]
    async fn screenshot(&self, Parameters(params): Parameters<ScreenshotParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        let area = match (params.hwnd, params.region) {
            (Some(hwnd), _) => match blocking(move || apps::describe_window(hwnd)).await? {
                Ok(window) => window.rect,
                Err(err) => return tool_error(err.to_string()),
            },
            (None, Some(r)) => display.offset(Rect::new(r.x, r.y, r.w, r.h)),
            (None, None) => display,
        };
        let Some(area) = area.intersection(&display) else {
            return tool_error("that window or region is not on the virtual display");
        };
        let scale = params.scale.unwrap_or(2.0 / 3.0);
        let format = match params.format {
            ImageFormat::Png => Format::Png,
            ImageFormat::Jpeg => Format::Jpeg,
        };
        let shot = blocking(move || capture::grab(area).and_then(|image| capture::encode(&image, scale, format)))
            .await?
            .map_err(|err| ErrorData::internal_error(err.to_string(), None))?;
        let data = base64::engine::general_purpose::STANDARD.encode(&shot.bytes);
        let info = json!({
            "width": shot.width,
            "height": shot.height,
            "scale": shot.width as f64 / area.w as f64,
            "region": to_display(area, display),
        });
        Ok(CallToolResult::success(vec![ContentBlock::image(data, shot.mime), ContentBlock::text(info.to_string())]))
    }

    #[tool(description = "Move or resize a window, without activating it. Use a preset (fill, left, right, center) \
                          or x/y/w/h in display coordinates. The result must stay on the virtual display.")]
    async fn move_window(&self, Parameters(params): Parameters<MoveParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        let args = json!({ "hwnd": params.hwnd, "preset": format!("{:?}", params.preset) });
        let hwnd = params.hwnd;
        let current = match blocking(move || apps::describe_window(hwnd)).await? {
            Ok(window) => window.rect,
            Err(err) => return self.fail("move_window", args, err),
        };
        let work = blocking(move || apps::work_area(display)).await?;
        let frame = match params.preset {
            Some(Preset::Fill) => work,
            Some(Preset::Left) => Rect::new(work.x, work.y, work.w / 2, work.h),
            Some(Preset::Right) => Rect::new(work.x + work.w / 2, work.y, work.w - work.w / 2, work.h),
            Some(Preset::Center) => {
                let (w, h) = (current.w.min(work.w), current.h.min(work.h));
                Rect::new(work.x + (work.w - w) / 2, work.y + (work.h - h) / 2, w, h)
            }
            None => {
                let relative = to_display(current, display);
                display.offset(Rect::new(
                    params.x.unwrap_or(relative.x),
                    params.y.unwrap_or(relative.y),
                    params.w.unwrap_or(current.w).max(1),
                    params.h.unwrap_or(current.h).max(1),
                ))
            }
        };
        if frame.fraction_inside(&display) <= ON_DISPLAY {
            return tool_error("that placement would put the window mostly outside the virtual display");
        }
        match blocking(move || apps::set_frame(hwnd, frame)).await? {
            Ok(rect) => {
                self.log.record("move_window", args, true, "");
                self.keeper.track(hwnd, rect);
                json_result(json!({ "rect": to_display(rect, display) }))
            }
            Err(err) => self.fail("move_window", args, err),
        }
    }

    #[tool(description = "Close a window on the virtual display, like clicking its X. Returns `closed: false` if it \
                          is still open (e.g. a save prompt appeared — take a screenshot). `force` kills the process.")]
    async fn close_window(&self, Parameters(params): Parameters<CloseParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        let args = json!({ "hwnd": params.hwnd, "force": params.force });
        let hwnd = params.hwnd;
        match blocking(move || apps::describe_window(hwnd)).await? {
            Ok(window) if window.rect.fraction_inside(&display) > ON_DISPLAY => {}
            Ok(_) => return tool_error("OuterSpace only closes windows on the virtual display"),
            Err(err) => return self.fail("close_window", args, err),
        }
        let force = params.force;
        match blocking(move || apps::close(hwnd, force)).await? {
            Ok(closed) => {
                self.log.record("close_window", args, true, if closed { "closed" } else { "still open" });
                if closed {
                    self.keeper.forget(hwnd);
                }
                json_result(json!({ "closed": closed }))
            }
            Err(err) => self.fail("close_window", args, err),
        }
    }
}

#[tool_handler(
    name = "outerspace",
    instructions = "OuterSpace gives you your own virtual monitor on the user's Windows PC, so you can run and \
                    drive apps without taking over the user's screen, mouse or keyboard. Call `status` first; if \
                    OuterSpace is off, ask the user to turn it on. Start apps with `launch` (they open on the \
                    virtual display), take a `screenshot` before acting, and use display coordinates: (0, 0) is the \
                    top-left of the virtual display whatever the screenshot scale. Never move or close the user's \
                    own windows unless they ask you to."
)]
impl ServerHandler for OuterSpace {}

impl OuterSpace {
    fn display(&self) -> Result<DisplayInfo, CallToolResult> {
        match self.power.state() {
            PowerState::On { display } => Ok(display),
            _ => Err(CallToolResult::error(vec![ContentBlock::text(OFF)])),
        }
    }

    fn fail(&self, tool: &str, args: serde_json::Value, err: impl std::fmt::Display) -> ToolResult {
        let message = err.to_string();
        self.log.record(tool, args, false, &message);
        tool_error(message)
    }
}

/// Apps that restore their saved position can slip off the display right after launch; give the
/// keeper a moment to bring them back so the agent gets the window's real place.
fn settled_on(window: WindowInfo, display: Rect) -> WindowInfo {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut current = window;
    while current.rect.fraction_inside(&display) <= ON_DISPLAY && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        match apps::describe_window(current.hwnd) {
            Ok(fresh) => current = fresh,
            Err(_) => break,
        }
    }
    current
}

fn to_display(rect: Rect, display: Rect) -> Rect {
    Rect::new(rect.x - display.x, rect.y - display.y, rect.w, rect.h)
}

fn json_result(value: impl Serialize) -> ToolResult {
    let text = serde_json::to_string_pretty(&value).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn tool_error(message: impl Into<String>) -> ToolResult {
    Ok(CallToolResult::error(vec![ContentBlock::text(message.into())]))
}

/// Win32 calls block (launch waits for windows, close waits for the window to go): keep them off
/// the async workers.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, ErrorData> {
    tokio::task::spawn_blocking(work).await.map_err(|e| ErrorData::internal_error(e.to_string(), None))
}
