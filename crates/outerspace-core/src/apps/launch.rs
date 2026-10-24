//! Starting a program straight onto the virtual display without taking the foreground from the user.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, POINT};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::GetProcessId;
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_HMONITOR, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, LSFW_LOCK, LSFW_UNLOCK, LockSetForegroundWindow, SW_SHOWNOACTIVATE,
};
use windows::core::{HSTRING, PCWSTR};

use super::{
    AppError, WindowInfo, describe, frame_rect, give_foreground_to, is_app_window, pid_of, process_name, set_frame,
    top_level_windows, work_area,
};
use crate::geom::Rect;
use crate::system::dpi_aware;

const POLL: Duration = Duration::from_millis(15);
/// Some apps reposition their first window shortly after showing it; keep it in place this long.
const SETTLE: Duration = Duration::from_millis(800);

pub struct LaunchRequest {
    /// An executable name or path, an App Paths name ("chrome"), a document, a URL or a `shell:` path.
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// How long to wait for the program's first window.
    pub wait: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct Launched {
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowInfo>,
    /// No window showed up in time; the program may still be starting (`list_windows` will find it).
    pub pending: bool,
    /// The program grabbed the foreground anyway and OuterSpace handed it back to the user.
    pub focus_repaired: bool,
    /// The window belongs to a process that was running before the launch: the program passed the
    /// request on to it (Win11 Notepad, Windows Terminal, an open browser), and the window may be
    /// one the user already had, with their own tabs or documents in it.
    pub handed_off: bool,
}

/// Starts the program, waits for its first window, fills the virtual display (above its taskbar)
/// with it and makes sure the user's foreground window stays in front.
pub fn launch(request: &LaunchRequest, display: Rect) -> Result<Launched, AppError> {
    dpi_aware();
    let target = work_area(display);
    let existing: HashSet<isize> = top_level_windows().into_iter().map(|h| h.0 as isize).collect();
    let user_foreground = unsafe { GetForegroundWindow() };
    // Refuse SetForegroundWindow from other processes while the program starts. This only takes
    // effect when we hold foreground rights ourselves — exactly the case where the program could
    // inherit them and steal the focus. The user pressing Alt or clicking lifts it anyway.
    let _lock = ForegroundLock::acquire();
    let running: HashSet<u32> = process_parents().into_keys().collect();
    let pid = start(request, display)?;
    let exe = exe_stem(&request.program);

    let deadline = Instant::now() + request.wait;
    let (window, parents) = loop {
        let parents = process_parents();
        let found = top_level_windows()
            .into_iter()
            .filter(|h| !existing.contains(&(h.0 as isize)) && is_app_window(*h, false))
            .find(|&h| belongs_to(pid_of(h), pid, exe.as_deref(), &parents));
        if let Some(window) = found {
            break (window, parents);
        }
        if Instant::now() >= deadline {
            tracing::info!(program = %request.program, ?pid, "launch: no window yet");
            return Ok(Launched { pid, window: None, pending: true, focus_repaired: false, handed_off: false });
        }
        std::thread::sleep(POLL);
    };
    let owner = pid_of(window);
    let handed_off = running.contains(&owner) && !pid.is_some_and(|started| descends_from(owner, started, &parents));

    let hwnd = window.0 as isize as super::Hwnd;
    let settled = Instant::now() + SETTLE;
    while Instant::now() < settled {
        if frame_rect(window) != target {
            set_frame(hwnd, target)?;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let focus_repaired = hand_back_focus(user_foreground, window);
    let info = describe(window, unsafe { GetForegroundWindow() });
    tracing::info!(program = %request.program, ?pid, hwnd, owner, handed_off, focus_repaired, "launched");
    Ok(Launched { pid, window: Some(info), pending: false, focus_repaired, handed_off })
}

fn start(request: &LaunchRequest, display: Rect) -> Result<Option<u32>, AppError> {
    let (x, y) = display.center();
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) };
    let file = HSTRING::from(request.program.as_str());
    let params = HSTRING::from(request.args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let dir = request.cwd.as_deref().map(HSTRING::from);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // HMONITOR tells the new process which monitor its default-positioned windows belong on.
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | SEE_MASK_HMONITOR,
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNOACTIVATE.0,
        ..Default::default()
    };
    info.Anonymous.hMonitor = HANDLE(monitor.0);
    unsafe { ShellExecuteExW(&mut info) }
        .map_err(|e| AppError::Launch { program: request.program.clone(), reason: e.message() })?;
    if info.hProcess.is_invalid() {
        // Opened by an already-running process (a URL in the browser, a document in its editor).
        return Ok(None);
    }
    let pid = unsafe { GetProcessId(info.hProcess) };
    unsafe {
        let _ = CloseHandle(info.hProcess);
    }
    Ok(Some(pid))
}

/// A new window is the program's if its process is the one started or a descendant — or, for
/// programs that hand off to another process (Win11 Notepad, a browser that is already open), if
/// it runs the same executable.
fn belongs_to(window_pid: u32, started: Option<u32>, exe: Option<&str>, parents: &HashMap<u32, u32>) -> bool {
    started.is_some_and(|started| descends_from(window_pid, started, parents))
        || exe.is_some_and(|exe| {
            process_name(window_pid).and_then(|name| exe_stem(&name)).is_some_and(|name| name == exe)
        })
}

/// Whether `pid` is `ancestor` itself or one of its descendants.
fn descends_from(mut pid: u32, ancestor: u32, parents: &HashMap<u32, u32>) -> bool {
    for _ in 0..16 {
        if pid == ancestor {
            return true;
        }
        match parents.get(&pid) {
            Some(&parent) if parent != 0 && parent != pid => pid = parent,
            _ => return false,
        }
    }
    false
}

fn process_parents() -> HashMap<u32, u32> {
    let mut parents = HashMap::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return parents };
    let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    parents
}

struct ForegroundLock;

impl ForegroundLock {
    fn acquire() -> Self {
        let _ = unsafe { LockSetForegroundWindow(LSFW_LOCK) };
        ForegroundLock
    }
}

impl Drop for ForegroundLock {
    fn drop(&mut self) {
        let _ = unsafe { LockSetForegroundWindow(LSFW_UNLOCK) };
    }
}

/// If the launched program ended up in the foreground, give it back to the window the user had.
/// Leaves things alone when the user has since moved on to something else.
fn hand_back_focus(previous: HWND, launched: HWND) -> bool {
    let current = unsafe { GetForegroundWindow() };
    if previous.is_invalid() || current == previous || pid_of(current) != pid_of(launched) {
        return false;
    }
    give_foreground_to(previous)
}

/// Lower-case file stem ("notepad" for `C:\Windows\notepad.exe`); `None` for URLs, `shell:` paths
/// and other schemes (a colon anywhere but after a drive letter).
fn exe_stem(program: &str) -> Option<String> {
    if program.find(':').is_some_and(|i| i != 1) {
        return None;
    }
    std::path::Path::new(program).file_stem().map(|stem| stem.to_string_lossy().to_lowercase())
}

/// Quotes one argument the way `CommandLineToArgvW` (and the C runtime) will split it back.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_round_trips_through_the_windows_rules() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote(""), r#""""#);
        assert_eq!(quote(r"C:\Program Files\app"), r#""C:\Program Files\app""#);
        assert_eq!(quote(r"C:\dir with space\"), r#""C:\dir with space\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn exe_stems() {
        assert_eq!(exe_stem(r"C:\Windows\System32\notepad.exe").as_deref(), Some("notepad"));
        assert_eq!(exe_stem("Code.exe").as_deref(), Some("code"));
        assert_eq!(exe_stem("chrome").as_deref(), Some("chrome"));
        assert_eq!(exe_stem("https://example.com"), None);
        assert_eq!(exe_stem(r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"), None);
    }

    #[test]
    fn descendants_belong_to_the_started_process() {
        let parents = HashMap::from([(30, 20), (20, 10), (10, 4)]);
        assert!(belongs_to(30, Some(10), None, &parents));
        assert!(!belongs_to(30, Some(99), None, &parents));
        assert!(descends_from(30, 10, &parents) && !descends_from(10, 30, &parents));
    }
}
