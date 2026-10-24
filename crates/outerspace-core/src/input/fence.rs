//! Keeps the user's own mouse out of the virtual display (docs/PLANO.md §2.2). The display only
//! touches the desktop by one pixel already; this low-level mouse hook makes sure physical movement
//! never crosses into it. Injected input — the agent's, and borrow mode's — passes untouched.

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, HC_ACTION, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
    PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEMOVE, WM_QUIT,
};

use crate::geom::Rect;
use crate::system::dpi_aware;

/// Where the virtual display is while it is on. Read by the hook on every mouse move, so it lives
/// in statics rather than behind the `Fence` value.
static AREA: RwLock<Option<Rect>> = RwLock::new(None);
static ENABLED: AtomicBool = AtomicBool::new(true);

pub struct Fence {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Fence {
    /// Installs the hook on its own thread (low-level hooks are called on the installing thread's
    /// message loop, which must never block).
    pub fn start() -> Fence {
        let (sender, receiver) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("mouse-fence".into())
            .spawn(move || {
                dpi_aware();
                let module = unsafe { GetModuleHandleW(None) }.ok();
                let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module.map(Into::into), 0) };
                let _ = sender.send(unsafe { GetCurrentThreadId() });
                let hook = match hook {
                    Ok(hook) => hook,
                    Err(err) => return tracing::warn!(%err, "mouse fence unavailable"),
                };
                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            })
            .expect("spawn mouse fence");
        let thread_id = receiver.recv().unwrap_or(0);
        Fence { thread_id, thread: Some(thread) }
    }

    /// The virtual display's desktop rectangle, or `None` while it is off.
    pub fn set_area(area: Option<Rect>) {
        *AREA.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = area;
    }

    pub fn set_enabled(enabled: bool) {
        ENABLED.store(enabled, Ordering::Relaxed);
    }

    pub fn enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }
}

impl Drop for Fence {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A physical move from outside the display to inside it.
pub fn crosses_in(from: POINT, to: POINT, area: Rect) -> bool {
    !area.contains(from.x, from.y) && area.contains(to.x, to.y)
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEMOVE && ENABLED.load(Ordering::Relaxed) {
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let area = *AREA.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(area) = area
            && event.flags & LLMHF_INJECTED == 0
        {
            let mut cursor = POINT::default();
            if unsafe { GetCursorPos(&mut cursor) }.is_ok() && crosses_in(cursor, event.pt, area) {
                return LRESULT(1); // swallow the move: the cursor stays where it was
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_moves_into_the_display_are_fenced() {
        let display = Rect::new(1920, -1079, 1920, 1080);
        let p = |x, y| POINT { x, y };
        assert!(crosses_in(p(1919, 0), p(1920, 0), display));
        assert!(!crosses_in(p(1900, 500), p(1919, 0), display)); // staying on the user's screen
        assert!(!crosses_in(p(2500, -500), p(2501, -500), display)); // already inside (borrow mode)
        assert!(!crosses_in(p(2500, -500), p(1000, 500), display)); // leaving is always fine
    }
}
