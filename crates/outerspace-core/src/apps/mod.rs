//! Top-level windows: listing them, moving them onto the virtual display and closing them, without
//! ever activating them. Launching lives in [`launch`].

mod keeper;
mod launch;

use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
    PostMessageW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow,
    SetWindowPos, ShowWindow, WM_CLOSE, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

pub use keeper::Keeper;
pub use launch::{LaunchRequest, Launched, launch};

use crate::geom::Rect;
use crate::system::dpi_aware;

/// Window handle as exposed to agents: a plain integer.
pub type Hwnd = i64;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("there is no window {0}")]
    NoSuchWindow(Hwnd),
    #[error("could not start {program}: {reason}")]
    Launch { program: String, reason: String },
    #[error("{0}")]
    Failed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub hwnd: Hwnd,
    pub pid: u32,
    pub process: String,
    pub title: String,
    pub class: String,
    /// Visible frame in desktop coordinates (no invisible resize borders).
    pub rect: Rect,
    pub minimized: bool,
    pub focused: bool,
}

/// Visible, user-facing top-level windows (what Alt+Tab would show), front to back.
pub fn list() -> Vec<WindowInfo> {
    dpi_aware();
    let foreground = unsafe { GetForegroundWindow() };
    top_level_windows().into_iter().filter(|&hwnd| is_app_window(hwnd, true)).map(|h| describe(h, foreground)).collect()
}

pub fn describe_window(hwnd: Hwnd) -> Result<WindowInfo, AppError> {
    dpi_aware();
    let handle = handle(hwnd)?;
    Ok(describe(handle, unsafe { GetForegroundWindow() }))
}

/// The part of the display not covered by the taskbar (Windows shows one on every monitor by default).
pub fn work_area(display: Rect) -> Rect {
    dpi_aware();
    let (x, y) = display.center();
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    if monitor.is_invalid() || !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return display;
    }
    to_rect(info.rcWork)
}

/// Moves and resizes the window so its *visible* frame is exactly `frame`, without activating it.
/// Restores minimized or maximized windows first.
pub fn set_frame(hwnd: Hwnd, frame: Rect) -> Result<Rect, AppError> {
    dpi_aware();
    let handle = handle(hwnd)?;
    unsafe {
        if IsIconic(handle).as_bool() || IsZoomed(handle).as_bool() {
            let _ = ShowWindow(handle, SW_SHOWNOACTIVATE);
        }
        let flags = SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER;
        // Move first: crossing into a monitor with another DPI makes the app rescale itself, and the
        // invisible borders change size with it — so measure them again before the final placement.
        SetWindowPos(handle, None, frame.x, frame.y, 0, 0, flags | SWP_NOSIZE).map_err(win32)?;
        let (outer, visible) = (window_rect(handle), frame_rect(handle));
        let (left, top) = (visible.x - outer.x, visible.y - outer.y);
        let (right, bottom) = (outer.right() - visible.right(), outer.bottom() - visible.bottom());
        SetWindowPos(
            handle,
            None,
            frame.x - left,
            frame.y - top,
            frame.w + left + right,
            frame.h + top + bottom,
            flags,
        )
        .map_err(win32)?;
    }
    Ok(frame_rect(handle))
}

/// Asks the window to close (like clicking ✕), or kills its process when `force` is set. Returns
/// whether it is gone after a short wait — a "save changes?" prompt keeps it open.
pub fn close(hwnd: Hwnd, force: bool) -> Result<bool, AppError> {
    let handle = handle(hwnd)?;
    if force {
        let pid = pid_of(handle);
        let process = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }.map_err(win32)?;
        let killed = unsafe { TerminateProcess(process, 1) };
        unsafe {
            let _ = CloseHandle(process);
        }
        killed.map_err(win32)?;
    } else {
        unsafe { PostMessageW(Some(handle), WM_CLOSE, Default::default(), Default::default()) }.map_err(win32)?;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !unsafe { IsWindow(Some(handle)) }.as_bool() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(false)
}

/// Puts `target` back in the foreground from a background process: borrowing the input queue of the
/// window that holds the foreground now is what lets SetForegroundWindow through.
pub(crate) fn give_foreground_to(target: HWND) -> bool {
    unsafe {
        let current = GetForegroundWindow();
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(current, None);
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let restored = SetForegroundWindow(target).as_bool();
        if attached {
            let _ = AttachThreadInput(ours, theirs, false);
        }
        restored
    }
}

fn handle(hwnd: Hwnd) -> Result<HWND, AppError> {
    let handle = HWND(hwnd as isize as *mut c_void);
    if unsafe { IsWindow(Some(handle)) }.as_bool() { Ok(handle) } else { Err(AppError::NoSuchWindow(hwnd)) }
}

fn top_level_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
        unsafe { (*(list.0 as *mut Vec<HWND>)).push(hwnd) };
        true.into()
    }
    let mut list: Vec<HWND> = Vec::new();
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&raw mut list as isize)) };
    list
}

/// A window a person would think of as "an app": visible, not cloaked (e.g. a suspended UWP
/// frame), not owned by another window, not a tool window. `need_title` drops untitled helpers.
fn is_app_window(hwnd: HWND, need_title: bool) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return false;
        }
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&raw mut cloaked).cast(), size_of::<u32>() as u32);
        cloaked == 0 && (!need_title || !title(hwnd).is_empty())
    }
}

fn describe(hwnd: HWND, foreground: HWND) -> WindowInfo {
    let pid = pid_of(hwnd);
    WindowInfo {
        hwnd: hwnd.0 as isize as Hwnd,
        pid,
        process: process_name(pid).unwrap_or_default(),
        title: title(hwnd),
        class: class(hwnd),
        rect: frame_rect(hwnd),
        minimized: unsafe { IsIconic(hwnd) }.as_bool(),
        focused: hwnd == foreground,
    }
}

fn title(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..len])
}

fn class(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..len])
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// File name of the process image, e.g. `Notepad.exe`.
fn process_name(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buffer.as_mut_ptr()), &mut len)
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    result.ok()?;
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    Path::new(&path).file_name().map(|name| name.to_string_lossy().into_owned())
}

fn window_rect(hwnd: HWND) -> Rect {
    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    to_rect(rect)
}

/// The frame the user sees; `GetWindowRect` also counts the invisible resize borders.
fn frame_rect(hwnd: HWND) -> Rect {
    let mut rect = RECT::default();
    let found = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut rect).cast(), size_of::<RECT>() as u32)
    };
    if found.is_ok() { to_rect(rect) } else { window_rect(hwnd) }
}

fn to_rect(rect: RECT) -> Rect {
    Rect::new(rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top)
}

fn win32(err: windows::core::Error) -> AppError {
    AppError::Failed(err.message())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::capture;

    /// Acceptance check for phase F2. Turn the virtual display on in OuterSpace first, then:
    /// `cargo test -p outerspace-core notepad -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the virtual display switched on; opens Notepad on it"]
    fn notepad_opens_on_the_virtual_display_without_taking_focus() {
        let display = crate::display::parsec::plugged_display().expect("switch the virtual display on first");
        let user_foreground = unsafe { GetForegroundWindow() };

        let started = Instant::now();
        let request =
            LaunchRequest { program: "notepad.exe".into(), args: vec![], cwd: None, wait: Duration::from_secs(10) };
        let launched = launch(&request, display).expect("launch");
        println!("launched in {:.2?}: {launched:#?}", started.elapsed());
        let window = launched.window.expect("a Notepad window");
        assert_eq!(window.rect, work_area(display), "Notepad should fill the virtual display above its taskbar");
        assert_eq!(unsafe { GetForegroundWindow() }, user_foreground, "the user's foreground window changed");
        assert!(list().iter().any(|w| w.hwnd == window.hwnd));

        let started = Instant::now();
        let image = capture::grab(display).expect("grab");
        let grabbed = started.elapsed();
        let shot = capture::encode(&image, 2.0 / 3.0, capture::Format::Png).expect("encode");
        println!(
            "screenshot {}x{} {} KB: grab {grabbed:.2?}, total {:.2?}",
            shot.width,
            shot.height,
            shot.bytes.len() / 1024,
            started.elapsed()
        );
        // Stays inside the build directory: Win11 Notepad restores the user's open tabs.
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/notepad-screenshot.png");
        std::fs::write(out, &shot.bytes).unwrap();

        assert!(close(window.hwnd, false).expect("close"), "Notepad did not close");
    }
}
