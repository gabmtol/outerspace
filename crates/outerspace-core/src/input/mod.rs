//! Clicking, typing and scrolling on the virtual display without taking over the user's mouse and
//! keyboard (docs/PLANO.md §2.2). Two ways in:
//! - [`post`]: window messages straight to the target. Invisible to the user; some apps ignore them.
//! - [`borrow`]: real input via SendInput, borrowing the cursor/foreground for a few dozen ms.
//!
//! `Mode::Auto` posts unless the request cannot be expressed that way (Alt/Win held down, clicks on
//! a window's frame, key combos with modifiers).

mod borrow;
pub mod fence;
pub mod keys;
mod post;

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, HTCLIENT, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_NCHITTEST, WindowFromPoint,
};

use crate::apps::Hwnd;
use crate::system::dpi_aware;
use keys::{Combo, Modifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Auto,
    Post,
    Borrow,
}

/// How an action was actually delivered, reported back so the agent can switch if nothing happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Used {
    Post,
    Borrow,
}

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("there is no window at that point")]
    NoWindow,
    #[error("the user is typing or moving the mouse right now; try again in a moment")]
    UserBusy,
    #[error("{0}")]
    Failed(String),
}

/// Set while borrow mode deliberately holds the foreground, so the window keeper does not snatch it back.
static BORROWING: AtomicBool = AtomicBool::new(false);

pub(crate) fn borrowing() -> bool {
    BORROWING.load(Ordering::Relaxed)
}

struct Borrowing;

impl Borrowing {
    fn start() -> Self {
        BORROWING.store(true, Ordering::Relaxed);
        Borrowing
    }
}

impl Drop for Borrowing {
    fn drop(&mut self) {
        BORROWING.store(false, Ordering::Relaxed);
    }
}

/// Clicks at desktop coordinates. Returns the top-level window that was hit.
pub fn click(
    x: i32,
    y: i32,
    button: Button,
    double: bool,
    modifiers: &[Modifier],
    mode: Mode,
) -> Result<(Hwnd, Used), InputError> {
    dpi_aware();
    let at = POINT { x, y };
    let (top, control) = window_at(at).ok_or(InputError::NoWindow)?;
    let borrow = match mode {
        Mode::Post => false,
        Mode::Borrow => true,
        Mode::Auto => {
            modifiers.iter().any(|m| matches!(m, Modifier::Alt | Modifier::Win)) || !in_client_area(control, at)
        }
    };
    if borrow {
        borrow::click(at, button, double, modifiers)?;
    } else {
        post::click(control, at, button, double, modifiers)?;
    }
    Ok((id(top), if borrow { Used::Borrow } else { Used::Post }))
}

/// Types text into the focused control of top-level window `window`.
pub fn type_text(window: Hwnd, text: &str, mode: Mode) -> Result<Used, InputError> {
    dpi_aware();
    let window = handle(window);
    if mode == Mode::Borrow {
        borrow::type_text(window, text)?;
        Ok(Used::Borrow)
    } else {
        post::type_text(window, text)?;
        Ok(Used::Post)
    }
}

pub fn press(window: Hwnd, combo: &Combo, mode: Mode) -> Result<Used, InputError> {
    dpi_aware();
    let window = handle(window);
    let borrow = match mode {
        Mode::Post => false,
        Mode::Borrow => true,
        Mode::Auto => !combo.modifiers.is_empty(),
    };
    if borrow {
        borrow::press(window, combo)?;
        Ok(Used::Borrow)
    } else {
        let combo = Combo { modifiers: Vec::new(), key: combo.key };
        post::press(window, &combo)?;
        Ok(Used::Post)
    }
}

/// Scrolls at desktop coordinates: `dy` notches up (negative: down), `dx` notches right.
pub fn scroll(x: i32, y: i32, dy: i32, dx: i32, mode: Mode) -> Result<(Hwnd, Used), InputError> {
    dpi_aware();
    let at = POINT { x, y };
    let (top, control) = window_at(at).ok_or(InputError::NoWindow)?;
    if mode == Mode::Borrow {
        borrow::scroll(at, dy, dx)?;
        Ok((id(top), Used::Borrow))
    } else {
        post::scroll(control, at, dy, dx)?;
        Ok((id(top), Used::Post))
    }
}

/// The top-level window and the innermost control under a desktop point.
pub fn window_at(at: POINT) -> Option<(HWND, HWND)> {
    let control = unsafe { WindowFromPoint(at) };
    if control.is_invalid() {
        return None;
    }
    let top = unsafe { GetAncestor(control, GA_ROOT) };
    Some((if top.is_invalid() { control } else { top }, control))
}

/// Title bars, borders and caption buttons only react to real input.
fn in_client_area(control: HWND, at: POINT) -> bool {
    let lparam = LPARAM(((at.y as u16 as u32) << 16 | at.x as u16 as u32) as i32 as isize);
    let mut result = 0usize;
    let answered = unsafe {
        SendMessageTimeoutW(control, WM_NCHITTEST, WPARAM(0), lparam, SMTO_ABORTIFHUNG, 200, Some(&mut result))
    };
    answered.0 == 0 || result as u32 == HTCLIENT
}

fn handle(hwnd: Hwnd) -> HWND {
    HWND(hwnd as isize as *mut core::ffi::c_void)
}

fn id(hwnd: HWND) -> Hwnd {
    hwnd.0 as isize as Hwnd
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, ES_MULTILINE, GetCursorPos, GetForegroundWindow, GetMessageW,
        GetWindowTextW, MSG, PostMessageW, SW_SHOWNOACTIVATE, ShowWindow, TranslateMessage, WINDOW_STYLE, WM_CLOSE,
        WS_OVERLAPPEDWINDOW, WS_VSCROLL,
    };
    use windows::core::w;

    use super::*;
    use crate::display::{DisplayMode, VirtualDisplayBackend, parsec::ParsecVdd};

    /// A classic Win32 text box on its own thread — a target whose content the test can read back,
    /// unlike real apps that may hold the user's documents.
    struct TestEdit {
        hwnd: HWND,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TestEdit {
        fn open(frame: crate::Rect) -> TestEdit {
            let (sender, receiver) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || unsafe {
                dpi_aware();
                let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WINDOW_STYLE(ES_MULTILINE as u32);
                let hwnd = CreateWindowExW(
                    Default::default(),
                    w!("EDIT"),
                    w!(""), // for an EDIT window the "title" is its initial text
                    style,
                    frame.x,
                    frame.y,
                    frame.w,
                    frame.h,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("create test window");
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                sender.send(hwnd.0 as isize).unwrap();
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            });
            let hwnd = HWND(receiver.recv().unwrap() as *mut core::ffi::c_void);
            crate::apps::set_frame(id(hwnd), frame).expect("place test window");
            TestEdit { hwnd, thread: Some(thread) }
        }

        fn text(&self) -> String {
            let mut buffer = [0u16; 256];
            let len = unsafe { GetWindowTextW(self.hwnd, &mut buffer) }.max(0) as usize;
            String::from_utf16_lossy(&buffer[..len])
        }

        /// Polls until the window shows `expected` (input is processed asynchronously).
        fn wait_for(&self, expected: &str) -> String {
            let deadline = Instant::now() + Duration::from_secs(2);
            while self.text() != expected && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            self.text()
        }
    }

    impl Drop for TestEdit {
        fn drop(&mut self) {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                // Closing a top-level EDIT destroys it; end its thread's loop.
                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
            }
            let _ = self.thread.take();
        }
    }

    fn cursor() -> POINT {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.unwrap();
        point
    }

    /// Plugs a display for the test unless one is on already (e.g. by the running app).
    fn display(vdd: &mut ParsecVdd) -> crate::Rect {
        dpi_aware();
        crate::display::parsec::plugged_display()
            .unwrap_or_else(|| vdd.connect(DisplayMode::DEFAULT).expect("connect").rect())
    }

    /// Acceptance check for phase F3, post mode. Safe while the user works: it never touches their
    /// cursor or foreground. (The user may still switch windows themselves, so the check is that the
    /// test window never becomes the foreground, not that the foreground stays the same.)
    /// `cargo test -p outerspace-core post_input -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Parsec driver; types into a test window on the virtual display"]
    fn post_input_reaches_the_window_without_touching_the_users_cursor_or_focus() {
        let mut vdd = ParsecVdd::new();
        let display = display(&mut vdd);
        let edit = TestEdit::open(crate::apps::work_area(display));
        let (cx, cy) = (display.x + 400, display.y + 300);
        let user_cursor = cursor();

        let (hit, used) = click(cx, cy, Button::Left, false, &[], Mode::Auto).expect("click");
        assert_eq!((hit, used), (id(edit.hwnd), Used::Post));
        assert_eq!(type_text(id(edit.hwnd), "hello", Mode::Auto).unwrap(), Used::Post);
        assert_eq!(edit.wait_for("hello"), "hello");
        assert_eq!(press(id(edit.hwnd), &keys::parse("backspace").unwrap(), Mode::Auto).unwrap(), Used::Post);
        assert_eq!(edit.wait_for("hell"), "hell");
        assert_eq!(scroll(cx, cy, -3, 0, Mode::Auto).unwrap().1, Used::Post);
        assert_eq!(cursor(), user_cursor, "post mode moved the user's cursor");
        assert_ne!(unsafe { GetForegroundWindow() }, edit.hwnd, "post mode activated the test window");
        println!("post mode: ok, the user's cursor and foreground were never touched");
    }

    /// Acceptance check for phase F3, borrow mode. Needs the user to keep hands off mouse and
    /// keyboard for a few seconds — borrow mode waits for a pause and gives up after 3 s.
    /// `cargo test -p outerspace-core borrow_input -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Parsec driver and an idle user; briefly borrows the cursor and foreground"]
    fn borrow_input_gives_the_cursor_and_focus_back() {
        let mut vdd = ParsecVdd::new();
        let display = display(&mut vdd);
        let edit = TestEdit::open(crate::apps::work_area(display));
        let (cx, cy) = (display.x + 400, display.y + 300);
        let (user_cursor, user_foreground) = (cursor(), unsafe { GetForegroundWindow() });

        let started = Instant::now();
        assert_eq!(type_text(id(edit.hwnd), "hello world", Mode::Borrow).unwrap(), Used::Borrow);
        let typed = started.elapsed();
        assert_eq!(edit.wait_for("hello world"), "hello world");
        let started = Instant::now();
        assert_eq!(click(cx, cy, Button::Left, false, &[], Mode::Borrow).unwrap().1, Used::Borrow);
        let clicked = started.elapsed();
        // The click left the caret at the end of the text. (A plain EDIT control has no Ctrl+A.)
        assert_eq!(press(id(edit.hwnd), &keys::parse("shift+home").unwrap(), Mode::Auto).unwrap(), Used::Borrow);
        assert_eq!(type_text(id(edit.hwnd), "x", Mode::Borrow).unwrap(), Used::Borrow);
        assert_eq!(edit.wait_for("x"), "x", "shift+home then typing should replace the line");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(cursor(), user_cursor, "borrow mode did not give the cursor back");
        assert_eq!(unsafe { GetForegroundWindow() }, user_foreground, "borrow mode did not give the foreground back");
        println!("borrow mode: ok (type {typed:.0?}, click {clicked:.0?}; both include waiting for the user to pause)");
    }
}
