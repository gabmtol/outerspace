//! "Borrow" mode: real input through SendInput, for apps that ignore posted messages. The cursor
//! (and, for the keyboard, the foreground) is borrowed for a few dozen milliseconds and handed back.
//! It waits for the user to pause first, so their own keystrokes do not land in the agent's window.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, LASTINPUTINFO, MAPVK_VK_TO_VSC, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY,
    VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, IsWindow, SetCursorPos};

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

pub fn click(at: POINT, button: Button, double: bool, modifiers: &[Modifier]) -> Result<(), InputError> {
    wait_for_user_pause()?;
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
    wait_for_user_pause()?;
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
    wait_for_user_pause()?;
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

fn wait_for_user_pause() -> Result<(), InputError> {
    let deadline = Instant::now() + MAX_WAIT;
    loop {
        let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        let idle = if unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            Duration::from_millis(u64::from(unsafe { GetTickCount() }.wrapping_sub(info.dwTime)))
        } else {
            USER_PAUSE
        };
        if idle >= USER_PAUSE {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(InputError::UserBusy);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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
