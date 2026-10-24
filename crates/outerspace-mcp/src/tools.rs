//! The MCP tools. Coordinates agents see are always *display coordinates*:
//! (0, 0) is the top-left corner of the virtual display, in its physical pixels (it runs at 100%).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use outerspace_core::apps::{self, Keeper, LaunchRequest, WindowInfo};
use outerspace_core::capture::{self, Format};
use outerspace_core::input::{self, Button, keys};
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
    /// Top-level window of the last click: where typing goes when no `hwnd` is given.
    last_clicked: Arc<Mutex<Option<i64>>>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    /// `post` whenever possible, `borrow` when the request needs it (Alt/Win held, clicking a title bar,
    /// shortcuts with modifiers).
    #[default]
    Auto,
    /// Window messages straight to the app: the user's mouse and keyboard are never touched. Some apps
    /// (games, some canvases) ignore them.
    Post,
    /// Real input: briefly borrows the user's cursor/keyboard focus (tens of ms) once they pause, then
    /// gives it back. Use when `post` had no visible effect.
    Borrow,
}

impl From<InputMode> for input::Mode {
    fn from(mode: InputMode) -> Self {
        match mode {
            InputMode::Auto => input::Mode::Auto,
            InputMode::Post => input::Mode::Post,
            InputMode::Borrow => input::Mode::Borrow,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ClickParams {
    /// Display coordinates (take them from a screenshot: region origin + image position / scale).
    x: i32,
    y: i32,
    #[serde(default)]
    button: MouseButton,
    #[serde(default)]
    double: bool,
    /// Keys held during the click: "ctrl", "shift", "alt", "win".
    #[serde(default)]
    modifiers: Vec<String>,
    #[serde(default)]
    input_mode: InputMode,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TypeParams {
    /// Text to type; "\n" presses Enter.
    text: String,
    /// Window to type into (default: the window you clicked last, else the front-most one on the display).
    /// The text goes to the control focused inside it — click the text box first.
    hwnd: Option<i64>,
    #[serde(default)]
    input_mode: InputMode,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct KeysParams {
    /// One key or combo: "enter", "tab", "esc", "ctrl+s", "ctrl+shift+t", "alt+f4", "f5", "pagedown", "a".
    keys: String,
    /// Window to send it to (default: as for `type_text`).
    hwnd: Option<i64>,
    #[serde(default)]
    input_mode: InputMode,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScrollParams {
    /// Display coordinates of the point to scroll at.
    x: i32,
    y: i32,
    /// Wheel notches: positive scrolls up, negative scrolls down (one notch is usually 3 lines).
    #[serde(default)]
    dy: i32,
    /// Horizontal notches: positive scrolls right, negative left.
    #[serde(default)]
    dx: i32,
    #[serde(default)]
    input_mode: InputMode,
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
    pub fn new(
        power: Arc<PowerController>,
        log: Arc<ActionLog>,
        keeper: Arc<Keeper>,
        last_clicked: Arc<Mutex<Option<i64>>>,
    ) -> Self {
        OuterSpace { power, log, keeper, last_clicked }
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
        self.log.record(
            "screenshot",
            json!({ "hwnd": params.hwnd, "scale": scale }),
            true,
            &format!("{}x{}", shot.width, shot.height),
        );
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

    #[tool(description = "Click at a point on the virtual display (display coordinates). By default the click is \
                          delivered without touching the user's mouse. Take a screenshot afterwards to see the \
                          effect; if nothing happened, retry with input_mode \"borrow\".")]
    async fn click(&self, Parameters(params): Parameters<ClickParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        let args = json!({ "x": params.x, "y": params.y, "button": format!("{:?}", params.button),
                           "double": params.double, "modifiers": params.modifiers, "mode": format!("{:?}", params.input_mode) });
        if !Rect::new(0, 0, display.w, display.h).contains(params.x, params.y) {
            return tool_error(format!(
                "({}, {}) is outside the virtual display ({}x{})",
                params.x, params.y, display.w, display.h
            ));
        }
        let modifiers =
            match params.modifiers.iter().map(|m| keys::Modifier::parse(m).ok_or(m)).collect::<Result<Vec<_>, _>>() {
                Ok(modifiers) => modifiers,
                Err(bad) => return tool_error(format!("unknown modifier \"{bad}\" (use ctrl, shift, alt, win)")),
            };
        let button = match params.button {
            MouseButton::Left => Button::Left,
            MouseButton::Right => Button::Right,
            MouseButton::Middle => Button::Middle,
        };
        let (x, y, double, mode) =
            (display.x + params.x, display.y + params.y, params.double, params.input_mode.into());
        match blocking(move || input::click(x, y, button, double, &modifiers, mode)).await? {
            Ok((hwnd, used)) => {
                *self.last_clicked.lock().unwrap_or_else(|p| p.into_inner()) = Some(hwnd);
                self.log.record("click", args, true, &format!("{used:?}"));
                json_result(json!({ "hwnd": hwnd, "input_mode_used": used }))
            }
            Err(err) => self.fail("click", args, err),
        }
    }

    #[tool(description = "Type text into a window on the virtual display — into the control focused inside it, so \
                          click the text box first. Works without touching the user's keyboard by default.")]
    async fn type_text(&self, Parameters(params): Parameters<TypeParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        // The text itself stays out of the log: agents type passwords too.
        let args = json!({ "chars": params.text.chars().count(), "hwnd": params.hwnd, "mode": format!("{:?}", params.input_mode) });
        let hwnd = match self.input_target(params.hwnd, display).await? {
            Ok(hwnd) => hwnd,
            Err(message) => return tool_error(message),
        };
        let (text, mode) = (params.text, params.input_mode.into());
        match blocking(move || input::type_text(hwnd, &text, mode)).await? {
            Ok(used) => {
                self.log.record("type_text", args, true, &format!("{used:?}"));
                json_result(json!({ "hwnd": hwnd, "input_mode_used": used }))
            }
            Err(err) => self.fail("type_text", args, err),
        }
    }

    #[tool(description = "Press a key or shortcut (\"enter\", \"tab\", \"ctrl+s\", \"alt+f4\") in a window on the \
                          virtual display. Plain keys go without touching the user's keyboard; shortcuts with \
                          modifiers need input_mode borrow, which auto picks for you.")]
    async fn press_keys(&self, Parameters(params): Parameters<KeysParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        let args = json!({ "keys": params.keys, "hwnd": params.hwnd, "mode": format!("{:?}", params.input_mode) });
        let combo = match keys::parse(&params.keys) {
            Ok(combo) => combo,
            Err(message) => return tool_error(message),
        };
        let hwnd = match self.input_target(params.hwnd, display).await? {
            Ok(hwnd) => hwnd,
            Err(message) => return tool_error(message),
        };
        let mode = params.input_mode.into();
        match blocking(move || input::press(hwnd, &combo, mode)).await? {
            Ok(used) => {
                self.log.record("press_keys", args, true, &format!("{used:?}"));
                json_result(json!({ "hwnd": hwnd, "input_mode_used": used }))
            }
            Err(err) => self.fail("press_keys", args, err),
        }
    }

    #[tool(description = "Scroll with the mouse wheel at a point on the virtual display: dy > 0 scrolls up, dy < 0 \
                          down; dx scrolls sideways.")]
    async fn scroll(&self, Parameters(params): Parameters<ScrollParams>) -> ToolResult {
        let display = match self.display() {
            Ok(display) => display.rect(),
            Err(off) => return Ok(off),
        };
        let args = json!({ "x": params.x, "y": params.y, "dy": params.dy, "dx": params.dx });
        if !Rect::new(0, 0, display.w, display.h).contains(params.x, params.y) {
            return tool_error(format!("({}, {}) is outside the virtual display", params.x, params.y));
        }
        let (x, y, dy, dx, mode) =
            (display.x + params.x, display.y + params.y, params.dy, params.dx, params.input_mode.into());
        match blocking(move || input::scroll(x, y, dy, dx, mode)).await? {
            Ok((hwnd, used)) => {
                self.log.record("scroll", args, true, &format!("{used:?}"));
                json_result(json!({ "hwnd": hwnd, "input_mode_used": used }))
            }
            Err(err) => self.fail("scroll", args, err),
        }
    }
}

#[tool_handler(
    name = "outerspace",
    instructions = "OuterSpace gives you your own virtual monitor on the user's Windows PC, so you can run and \
                    drive apps without taking over the user's screen, mouse or keyboard. Call `status` first; if \
                    OuterSpace is off, ask the user to turn it on. Start apps with `launch` (they open on the \
                    virtual display), take a `screenshot` before acting, and use display coordinates: (0, 0) is the \
                    top-left of the virtual display whatever the screenshot scale. Interact with `click`, \
                    `type_text`, `press_keys` and `scroll`: by default they reach the app without touching the \
                    user's mouse or keyboard; if a screenshot shows no effect, retry with input_mode \"borrow\", \
                    which briefly borrows the real cursor/keyboard once the user pauses. Never move or close the \
                    user's own windows unless they ask you to."
)]
impl ServerHandler for OuterSpace {}

impl OuterSpace {
    fn display(&self) -> Result<DisplayInfo, CallToolResult> {
        match self.power.state() {
            PowerState::On { display } => Ok(display),
            _ => Err(CallToolResult::error(vec![ContentBlock::text(OFF)])),
        }
    }

    /// Where keyboard input goes: the given window, else the last one clicked, else the front-most
    /// window on the display — and only ever a window on the virtual display.
    async fn input_target(&self, hwnd: Option<i64>, display: Rect) -> Result<Result<i64, String>, ErrorData> {
        let last_clicked = *self.last_clicked.lock().unwrap_or_else(|p| p.into_inner());
        blocking(move || {
            let on_display =
                |hwnd: i64| apps::describe_window(hwnd).is_ok_and(|w| w.rect.fraction_inside(&display) > ON_DISPLAY);
            if let Some(hwnd) = hwnd {
                return if on_display(hwnd) {
                    Ok(hwnd)
                } else {
                    Err(format!("window {hwnd} is not on the virtual display"))
                };
            }
            last_clicked
                .filter(|&hwnd| on_display(hwnd))
                .or_else(|| {
                    apps::list().into_iter().find(|w| w.rect.fraction_inside(&display) > ON_DISPLAY).map(|w| w.hwnd)
                })
                .ok_or_else(|| "there is no window on the virtual display to type into; launch one first".to_string())
        })
        .await
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
