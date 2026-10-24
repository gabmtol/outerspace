//! "Post" mode: input as window messages straight to the target window. The real cursor, the
//! keyboard state and the foreground window are never touched, so the user notices nothing — but
//! apps that read the global input state (GetKeyState/GetCursorPos, raw input, most games) ignore it.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW, VIRTUAL_KEY};
use windows::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, PostMessageW, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

use super::keys::{Combo, Modifier, is_extended};
use super::{Button, InputError};

const WHEEL_DELTA: i32 = 120;
// wParam key-state flags of mouse messages (MK_*).
const MK_LBUTTON: usize = 0x01;
const MK_RBUTTON: usize = 0x02;
const MK_SHIFT: usize = 0x04;
const MK_CONTROL: usize = 0x08;
const MK_MBUTTON: usize = 0x10;

pub fn click(target: HWND, at: POINT, button: Button, double: bool, modifiers: &[Modifier]) -> Result<(), InputError> {
    let mut client = at;
    let _ = unsafe { ScreenToClient(target, &mut client) };
    let position = point_lparam(client.x, client.y);
    let keys = modifier_flags(modifiers);
    let (down, up, dblclk, held) = match button {
        Button::Left => (WM_LBUTTONDOWN, WM_LBUTTONUP, WM_LBUTTONDBLCLK, MK_LBUTTON),
        Button::Right => (WM_RBUTTONDOWN, WM_RBUTTONUP, WM_RBUTTONDBLCLK, MK_RBUTTON),
        Button::Middle => (WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MBUTTONDBLCLK, MK_MBUTTON),
    };
    post(target, WM_MOUSEMOVE, keys, position)?;
    post(target, down, keys | held, position)?;
    post(target, up, keys, position)?;
    if double {
        post(target, dblclk, keys | held, position)?;
        post(target, up, keys, position)?;
    }
    Ok(())
}

/// Characters go to whichever control has the keyboard focus inside `window`'s thread — the text box
/// the agent clicked last — even though the window itself is not in the foreground.
pub fn type_text(window: HWND, text: &str) -> Result<(), InputError> {
    let target = focus_within(window);
    for unit in text.replace("\r\n", "\n").encode_utf16() {
        let unit = if unit == u16::from(b'\n') { u16::from(b'\r') } else { unit };
        post(target, WM_CHAR, unit as usize, LPARAM(1))?;
    }
    Ok(())
}

/// Only for combos without modifiers: posted messages cannot make Ctrl/Alt/Shift look held down to
/// an app that checks GetKeyState, which is what nearly all shortcut handling does.
pub fn press(window: HWND, combo: &Combo) -> Result<(), InputError> {
    debug_assert!(combo.modifiers.is_empty());
    let target = focus_within(window);
    let vk = combo.key;
    post(target, WM_KEYDOWN, vk.0 as usize, key_lparam(vk, false))?;
    post(target, WM_KEYUP, vk.0 as usize, key_lparam(vk, true))
}

/// `dy` notches up (negative: down), `dx` notches right (negative: left).
pub fn scroll(target: HWND, at: POINT, dy: i32, dx: i32) -> Result<(), InputError> {
    let position = point_lparam(at.x, at.y); // wheel messages carry screen coordinates
    if dy != 0 {
        post(target, WM_MOUSEWHEEL, wheel_wparam(dy * WHEEL_DELTA), position)?;
    }
    if dx != 0 {
        post(target, WM_MOUSEHWHEEL, wheel_wparam(dx * WHEEL_DELTA), position)?;
    }
    Ok(())
}

fn focus_within(window: HWND) -> HWND {
    let thread = unsafe { GetWindowThreadProcessId(window, None) };
    let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    match unsafe { GetGUIThreadInfo(thread, &mut info) } {
        Ok(()) if !info.hwndFocus.is_invalid() => info.hwndFocus,
        _ => window,
    }
}

fn post(target: HWND, message: u32, wparam: usize, lparam: LPARAM) -> Result<(), InputError> {
    unsafe { PostMessageW(Some(target), message, WPARAM(wparam), lparam) }
        .map_err(|e| InputError::Failed(format!("could not post input to the window: {}", e.message())))
}

fn modifier_flags(modifiers: &[Modifier]) -> usize {
    modifiers.iter().fold(0, |flags, m| match m {
        Modifier::Ctrl => flags | MK_CONTROL,
        Modifier::Shift => flags | MK_SHIFT,
        _ => flags,
    })
}

fn point_lparam(x: i32, y: i32) -> LPARAM {
    LPARAM(((y as u16 as u32) << 16 | x as u16 as u32) as i32 as isize)
}

fn wheel_wparam(delta: i32) -> usize {
    ((delta as i16 as u16 as u32) << 16) as usize
}

/// lParam of WM_KEYDOWN/UP: repeat count 1, scan code, extended flag, and the "was down" and
/// "being released" bits for key-up.
fn key_lparam(vk: VIRTUAL_KEY, up: bool) -> LPARAM {
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } & 0xff;
    let mut value = 1 | (scan << 16);
    if is_extended(vk) {
        value |= 1 << 24;
    }
    if up {
        value |= (1 << 30) | (1 << 31);
    }
    LPARAM(value as i32 as isize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_coordinates_and_wheel_deltas() {
        assert_eq!(point_lparam(10, 20).0, (20 << 16) | 10);
        // Negative screen coordinates (a display above the primary one) keep their sign per word.
        assert_eq!(point_lparam(1930, -1059).0 as u32, ((-1059i16 as u16 as u32) << 16) | 1930);
        assert_eq!(wheel_wparam(-360) >> 16, (-360i16 as u16) as usize);
    }
}
