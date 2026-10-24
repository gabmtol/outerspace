//! Keeps the user's own mouse out of the virtual display. The display only
//! touches the desktop by one pixel; this low-level mouse hook makes sure physical movement never
//! gets into it — neither across that pixel nor through Windows 11's "Ease cursor movement between
//! displays", which carries a cursor pushed past the edge of a screen onto the closest one, and so
//! onto the virtual display from anywhere near the top of the user's right edge. Injected input —
//! the agent's, and borrow mode's — passes untouched.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, HC_ACTION, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
    PostThreadMessageW, SetCursorPos, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL,
    WM_MOUSEMOVE, WM_QUIT,
};

use crate::geom::Rect;
use crate::system::dpi_aware;

/// Where the virtual display is while it is on. Read by the hook on every mouse move, so it lives
/// in statics rather than behind the `Fence` value.
static AREA: RwLock<Option<Rect>> = RwLock::new(None);
static ENABLED: AtomicBool = AtomicBool::new(true);
/// Where the user's cursor last was outside the display: where it goes back to if it ever ends up
/// inside anyway. Only the hook thread touches it.
static LAST_OUTSIDE: Mutex<Option<POINT>> = Mutex::new(None);

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

/// What the hook does with one physical mouse move.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Allow,
    /// Swallowed: the cursor stays where it is.
    Block,
    /// Swallowed, and the cursor put here instead.
    Warp(i32, i32),
}

/// The monitor layout, as [`judge`] needs it.
trait Screens {
    /// Whether some monitor shows `point`.
    fn show(&self, point: POINT) -> bool;
    /// The desktop rectangle of the monitor closest to `point` (the one showing it, if any).
    fn nearest(&self, point: POINT) -> Option<Rect>;
}

/// Decides a physical move of the cursor from `from` to `to` while the display covers `area`.
fn judge(from: POINT, to: POINT, area: Rect, last_outside: Option<POINT>, screens: &impl Screens) -> Verdict {
    if area.contains(from.x, from.y) {
        // Inside without having crossed in: Windows carried it here some other way. Send it back.
        return match last_outside {
            Some(back) if screens.nearest(from) == Some(area) => Verdict::Warp(back.x, back.y),
            _ => Verdict::Allow,
        };
    }
    if area.contains(to.x, to.y) {
        return Verdict::Block;
    }
    if !screens.show(to) {
        // Pushed past the edge of the screen. Do what Windows does without "Ease cursor movement",
        // stop it at the edge, unless the closest screen is another of the user's own.
        let home = screens.nearest(from);
        let closest = screens.nearest(to);
        if let Some(home) = home
            && (closest == Some(area) || closest == Some(home))
        {
            let (x, y) = home.clamp(to.x, to.y);
            return Verdict::Warp(x, y);
        }
    }
    Verdict::Allow
}

/// The live layout. The hook thread is per-monitor DPI aware, so this is in physical pixels like
/// the hook's points.
struct Desktop;

impl Screens for Desktop {
    fn show(&self, point: POINT) -> bool {
        !unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) }.is_invalid()
    }

    fn nearest(&self, point: POINT) -> Option<Rect> {
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if monitor.is_invalid() || !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return None;
        }
        let r = info.rcMonitor;
        Some(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEMOVE && ENABLED.load(Ordering::Relaxed) {
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let area = *AREA.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut cursor = POINT::default();
        if let Some(area) = area
            && event.flags & LLMHF_INJECTED == 0
            && !super::borrowing()
            && unsafe { GetCursorPos(&mut cursor) }.is_ok()
        {
            let mut last_outside = LAST_OUTSIDE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let verdict = judge(cursor, event.pt, area, *last_outside, &Desktop);
            if !area.contains(cursor.x, cursor.y) {
                *last_outside = Some(cursor);
            }
            match verdict {
                Verdict::Allow => {}
                Verdict::Block => return LRESULT(1),
                Verdict::Warp(x, y) => {
                    if (x, y) != (cursor.x, cursor.y) {
                        let _ = unsafe { SetCursorPos(x, y) };
                    }
                    return LRESULT(1);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout this was written for: one screen, the display touching its top-right pixel.
    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };
    const DISPLAY: Rect = Rect { x: 1920, y: -1079, w: 1920, h: 1080 };

    struct Layout(Vec<Rect>);

    impl Screens for Layout {
        fn show(&self, point: POINT) -> bool {
            self.0.iter().any(|r| r.contains(point.x, point.y))
        }

        fn nearest(&self, point: POINT) -> Option<Rect> {
            self.0.iter().copied().min_by_key(|r| {
                let (x, y) = r.clamp(point.x, point.y);
                (x - point.x).pow(2) + (y - point.y).pow(2)
            })
        }
    }

    fn p(x: i32, y: i32) -> POINT {
        POINT { x, y }
    }

    #[test]
    fn crossing_the_shared_pixel_is_blocked() {
        let layout = Layout(vec![SCREEN, DISPLAY]);
        assert_eq!(judge(p(1919, 0), p(1920, 0), DISPLAY, None, &layout), Verdict::Block);
        assert_eq!(judge(p(1918, 1), p(1925, -3), DISPLAY, None, &layout), Verdict::Block);
    }

    #[test]
    fn pushing_past_the_edge_stops_at_the_edge() {
        let layout = Layout(vec![SCREEN, DISPLAY]);
        // Right edge, a few pixels down: the display is the closest screen, so "Ease cursor
        // movement" would carry this onto it.
        assert_eq!(judge(p(1917, 1), p(1926, 2), DISPLAY, None, &layout), Verdict::Warp(1919, 2));
        assert_eq!(judge(p(1915, 5), p(1925, 8), DISPLAY, None, &layout), Verdict::Warp(1919, 8));
        assert_eq!(judge(p(1919, 600), p(1930, 610), DISPLAY, None, &layout), Verdict::Warp(1919, 610));
        // Top and left edges.
        assert_eq!(judge(p(1910, 2), p(1912, -4), DISPLAY, None, &layout), Verdict::Warp(1912, 0));
        assert_eq!(judge(p(3, 500), p(-6, 498), DISPLAY, None, &layout), Verdict::Warp(0, 498));
    }

    #[test]
    fn moves_between_the_users_own_screens_are_left_alone() {
        let left = Rect::new(-1920, 200, 1920, 1080); // lower than the main screen
        let layout = Layout(vec![left, SCREEN, DISPLAY]);
        // Past the left edge, closest to the left screen: Windows may ease it over.
        assert_eq!(judge(p(2, 195), p(-30, 195), DISPLAY, None, &layout), Verdict::Allow);
        assert_eq!(judge(p(900, 500), p(910, 505), DISPLAY, None, &layout), Verdict::Allow);
    }

    #[test]
    fn a_cursor_found_inside_goes_back() {
        let layout = Layout(vec![SCREEN, DISPLAY]);
        let back = Some(p(1919, 6));
        assert_eq!(judge(p(1925, 0), p(1930, -2), DISPLAY, back, &layout), Verdict::Warp(1919, 6));
        // Nowhere known to go back to: leaving is always fine.
        assert_eq!(judge(p(2500, -500), p(2501, -500), DISPLAY, None, &layout), Verdict::Allow);
        // A stale area over the user's own screen never traps the cursor.
        let stale = Rect::new(0, 0, 800, 600);
        assert_eq!(judge(p(100, 100), p(110, 100), stale, back, &layout), Verdict::Allow);
    }
}
