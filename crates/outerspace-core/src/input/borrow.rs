//! "Borrow" mode: real input through SendInput, for apps that ignore posted messages. The cursor
//! (and, for the keyboard, the foreground) is borrowed for a few dozen milliseconds and handed back.
//! It waits for the user to pause first, so their own keystrokes do not land in the agent's window.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSE_EVENT_FLAGS, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, GetForegroundWindow, GetMessageW, HC_ACTION, IsWindow, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SetCursorPos, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_QUIT,
};

use super::keys::{Combo, Modifier, is_extended};
use super::{Borrowing, Button, InputError};
use crate::apps::give_foreground_to;

/// The user counts as paused after this long without touching mouse or keyboard...
const USER_PAUSE: Duration = Duration::from_millis(500);
/// ...and if they never pause within this, the agent is told to try again rather than barging in.
const MAX_WAIT: Duration = Duration::from_secs(3);
/// Time for the target to take the input from its queue before the cursor/focus go back.
const SETTLE: Duration = Duration::from_millis(40);
const WHEEL_DELTA: i32 = 120;

/// One borrow at a time: there is only one cursor and one foreground to lend.
static TURN: Mutex<()> = Mutex::new(());
/// Tick (GetTickCount64) of the last physical key or mouse event the watcher saw.
static LAST_PHYSICAL: AtomicU64 = AtomicU64::new(0);

pub fn click(at: POINT, button: Button, double: bool, modifiers: &[Modifier]) -> Result<(), InputError> {
    let _turn = wait_for_user_pause()?;
    let _borrowing = Borrowing::start();
    let (cursor, foreground) = (cursor(), unsafe { GetForegroundWindow() });
    let (down, up) = match button {
        Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    };
    let mut inputs: Vec<INPUT> = modifiers.iter().map(|m| key(m.vk(), false)).collect();
    for _ in 0..if double { 2 } else { 1 } {
        inputs.extend([mouse(down, 0), mouse(up, 0)]);
    }
    inputs.extend(modifiers.iter().rev().map(|m| key(m.vk(), true)));

    unsafe { SetCursorPos(at.x, at.y) }.map_err(|e| InputError::Failed(e.message()))?;
    let sent = send(&inputs);
    std::thread::sleep(SETTLE);
    give_back(cursor, foreground);
    sent
}

pub fn type_text(window: HWND, text: &str) -> Result<(), InputError> {
    let mut inputs = Vec::new();
    for unit in text.replace("\r\n", "\n").encode_utf16() {
        if unit == u16::from(b'\n') {
            inputs.extend([key(VK_RETURN, false), key(VK_RETURN, true)]);
        } else {
            inputs.extend([unicode(unit, false), unicode(unit, true)]);
        }
    }
    with_focus(window, &inputs)
}

pub fn press(window: HWND, combo: &Combo) -> Result<(), InputError> {
    let mut inputs: Vec<INPUT> = combo.modifiers.iter().map(|m| key(m.vk(), false)).collect();
    inputs.extend([key(combo.key, false), key(combo.key, true)]);
    inputs.extend(combo.modifiers.iter().rev().map(|m| key(m.vk(), true)));
    with_focus(window, &inputs)
}

/// The wheel goes to the window under the cursor (Windows' "scroll inactive windows"), so only the
/// cursor is borrowed, never the focus.
pub fn scroll(at: POINT, dy: i32, dx: i32) -> Result<(), InputError> {
    let _turn = wait_for_user_pause()?;
    let _borrowing = Borrowing::start();
    let (cursor, foreground) = (cursor(), unsafe { GetForegroundWindow() });
    let mut inputs = Vec::new();
    if dy != 0 {
        inputs.push(mouse(MOUSEEVENTF_WHEEL, dy * WHEEL_DELTA));
    }
    if dx != 0 {
        inputs.push(mouse(MOUSEEVENTF_HWHEEL, dx * WHEEL_DELTA));
    }
    unsafe { SetCursorPos(at.x, at.y) }.map_err(|e| InputError::Failed(e.message()))?;
    let sent = send(&inputs);
    std::thread::sleep(SETTLE);
    give_back(cursor, foreground);
    sent
}

/// Brings `window` to the foreground, sends the keys, and returns the foreground to the user.
fn with_focus(window: HWND, inputs: &[INPUT]) -> Result<(), InputError> {
    let _turn = wait_for_user_pause()?;
    let _borrowing = Borrowing::start();
    let (cursor, foreground) = (cursor(), unsafe { GetForegroundWindow() });
    if foreground != window && !give_foreground_to(window) {
        return Err(InputError::Failed(
            "Windows did not let OuterSpace focus that window. Click inside it first with input_mode \"borrow\", \
             then type again."
                .into(),
        ));
    }
    let sent = send(inputs);
    std::thread::sleep(SETTLE);
    give_back(cursor, foreground);
    sent
}

/// Takes the borrow turn and waits until the user's own keyboard and mouse have been still for
/// [`USER_PAUSE`], measured from now. Watches them directly: the system-wide idle counter
/// (GetLastInputInfo) is also reset by game controllers and other devices, so with a gamepad
/// connected it may never look idle even while the user's hands are off the keyboard and mouse.
fn wait_for_user_pause() -> Result<MutexGuard<'static, ()>, InputError> {
    let turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _watch = Watch::start();
    let started = Instant::now();
    loop {
        let quiet = unsafe { GetTickCount64() }.saturating_sub(LAST_PHYSICAL.load(Ordering::Relaxed));
        if quiet >= USER_PAUSE.as_millis() as u64 {
            return Ok(turn);
        }
        if started.elapsed() >= MAX_WAIT {
            return Err(InputError::UserBusy);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Low-level keyboard and mouse hooks on their own thread, for as long as this value lives. Only the
/// time of the last *physical* event is kept — never which key — and injected input is ignored.
struct Watch {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    fn start() -> Watch {
        LAST_PHYSICAL.store(unsafe { GetTickCount64() }, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let module = GetModuleHandleW(None).ok().map(Into::into);
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), module, 0);
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_keyboard), module, 0);
            let _ = sender.send(GetCurrentThreadId());
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
            for hook in [mouse, keyboard].into_iter().flatten() {
                let _ = UnhookWindowsHookEx(hook);
            }
        });
        Watch { thread_id: receiver.recv().unwrap_or(0), thread: Some(thread) }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

unsafe extern "system" fn on_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).flags } & LLMHF_INJECTED == 0 {
        LAST_PHYSICAL.store(unsafe { GetTickCount64() }, Ordering::Relaxed);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn on_keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && unsafe { (*(lparam.0 as *const KBDLLHOOKSTRUCT)).flags }.0 & LLKHF_INJECTED.0 == 0 {
        LAST_PHYSICAL.store(unsafe { GetTickCount64() }, Ordering::Relaxed);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn give_back(cursor: Option<POINT>, foreground: HWND) {
    if let Some(cursor) = cursor {
        let _ = unsafe { SetCursorPos(cursor.x, cursor.y) };
    }
    let now = unsafe { GetForegroundWindow() };
    if now != foreground && unsafe { IsWindow(Some(foreground)) }.as_bool() {
        give_foreground_to(foreground);
    }
}

fn cursor() -> Option<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok().map(|()| point)
}

fn send(inputs: &[INPUT]) -> Result<(), InputError> {
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        // Blocked by UIPI: the target runs elevated and OuterSpace does not.
        Err(InputError::Failed("Windows blocked the input (is that window running as administrator?)".into()))
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { mouseData: data as u32, dwFlags: flags, ..Default::default() } },
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if is_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, ..Default::default() } },
    }
}

fn unicode(unit: u16, up: bool) -> INPUT {
    let flags = if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wScan: unit, dwFlags: flags, ..Default::default() } },
    }
}
