//! Keeps the agent's windows on the virtual display and out of the user's foreground. Some apps
//! (Firefox, anything that restores its saved position) move their window back where it was last
//! time — the user's screen — and grab the foreground, a moment after they first appear.
//!
//! Tracks individual windows, never whole processes: the user may have their own windows of the
//! same app open, and those must stay where they are.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOTOWNER, GetAncestor, GetForegroundWindow, IsIconic, IsWindow, MSG, PM_NOREMOVE, PeekMessageW, SW_MINIMIZE,
    SW_SHOWNOACTIVATE, ShowWindow,
};

use super::{Hwnd, frame_rect, give_foreground_to, handle, is_app_window, set_frame, work_area};
use crate::geom::Rect;
use crate::system::dpi_aware;

const TICK: Duration = Duration::from_millis(100);
/// Same rule as everywhere else: a window is "on the display" when most of it is there.
const ON_DISPLAY: f64 = 0.5;
/// How long after it appears a window is left out of the minimize trick: apps still setting up
/// their rendering (Blender's Vulkan backend) can crash when minimized then.
const STARTUP: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct Tracked {
    since: Instant,
    /// The last frame it had on the virtual display.
    last_good: Option<Rect>,
}

pub struct Keeper {
    windows: Arc<Mutex<HashMap<Hwnd, Tracked>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Keeper {
    /// `display` reports where the virtual display is, or `None` while it is off.
    pub fn start(display: impl Fn() -> Option<Rect> + Send + 'static) -> Self {
        let windows: Arc<Mutex<HashMap<Hwnd, Tracked>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("window-keeper".into())
            .spawn({
                let (windows, stop) = (windows.clone(), stop.clone());
                move || {
                    dpi_aware();
                    // AttachThreadInput (how the focus is handed back) needs this thread to own a
                    // message queue; peeking creates one.
                    let mut msg = MSG::default();
                    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
                    let mut focus = FocusGuard::default();
                    while !stop.load(Ordering::Relaxed) {
                        if let Some(display) = display() {
                            let mut windows = lock(&windows);
                            keep(&mut windows, display);
                            focus.check(&windows, display);
                        }
                        std::thread::sleep(TICK);
                    }
                }
            })
            .expect("spawn window keeper");
        Keeper { windows, stop, thread: Some(thread) }
    }

    /// Starts (or refreshes) watching a window the agent placed on the virtual display.
    pub fn track(&self, hwnd: Hwnd, frame: Rect) {
        let mut windows = lock(&self.windows);
        let since = windows.get(&hwnd).map_or_else(Instant::now, |tracked| tracked.since);
        windows.insert(hwnd, Tracked { since, last_good: Some(frame) });
    }

    pub fn forget(&self, hwnd: Hwnd) {
        lock(&self.windows).remove(&hwnd);
    }
}

impl Drop for Keeper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn keep(windows: &mut HashMap<Hwnd, Tracked>, display: Rect) {
    windows.retain(|&hwnd, _| handle(hwnd).is_ok_and(|h| unsafe { IsWindow(Some(h)) }.as_bool()));
    for (&hwnd, Tracked { last_good, .. }) in windows.iter_mut() {
        let Ok(window) = handle(hwnd) else { continue };
        if unsafe { IsIconic(window) }.as_bool() {
            continue;
        }
        let frame = frame_rect(window);
        if frame.fraction_inside(&display) > ON_DISPLAY {
            *last_good = Some(frame);
            continue;
        }
        let target =
            last_good.filter(|r| r.fraction_inside(&display) > ON_DISPLAY).unwrap_or_else(|| work_area(display));
        tracing::info!(hwnd, ?frame, ?target, "window left the virtual display; bringing it back");
        if let Err(err) = set_frame(hwnd, target) {
            tracing::warn!(hwnd, %err, "could not bring the window back");
        }
    }
}

/// The agent's windows live on a display the user does not work on, so they have no business in
/// the foreground: when one (or a dialog it owns) takes it — apps may, once the user has been idle
/// for a while — the foreground goes back to the user's last window. One try per steal: retrying
/// every tick once had Blender minimized and restored ten times a second until it crashed.
#[derive(Default)]
struct FocusGuard {
    /// The user's most recent foreground app window: never an agent window, never on the virtual
    /// display, never the taskbar or other shell UI (they can't be handed the foreground).
    user_window: Option<Hwnd>,
    /// The foreground window at the last tick, whatever it was.
    previous: Option<Hwnd>,
    /// The agent window in the foreground that was already dealt with (handed back, or left there):
    /// nothing more happens until the foreground moves.
    handled: Option<Hwnd>,
}

impl FocusGuard {
    fn check(&mut self, windows: &HashMap<Hwnd, Tracked>, display: Rect) {
        let foreground = unsafe { GetForegroundWindow() };
        // Borrow-mode input holds the foreground on purpose for a moment and hands it back itself.
        if foreground.is_invalid() || crate::input::borrowing() {
            return;
        }
        let previous = self.previous.replace(id(foreground));
        let root = unsafe { GetAncestor(foreground, GA_ROOTOWNER) };
        let is_agent = |hwnd: Hwnd| windows.contains_key(&hwnd);
        if !is_agent(id(foreground)) && !is_agent(id(root)) {
            if is_app_window(foreground, true) && frame_rect(foreground).fraction_inside(&display) <= ON_DISPLAY {
                self.user_window = Some(id(foreground));
            }
            self.handled = None;
            return;
        }
        if self.handled == Some(id(foreground)) {
            return;
        }
        self.handled = Some(id(foreground));
        // Arriving from the taskbar, Alt+Tab or Start: the user picked this window themselves.
        if previous
            .filter(|&p| p != id(foreground))
            .and_then(|p| handle(p).ok())
            .is_some_and(|p| !is_app_window(p, true))
        {
            tracing::info!(agent = id(foreground), "the user switched to an agent window; leaving it in front");
            return;
        }
        let target = self
            .user_window
            .filter(|&hwnd| !is_agent(hwnd))
            .and_then(|hwnd| handle(hwnd).ok())
            .or_else(|| top_user_window(windows, display));
        let Some(target) = target else { return };
        let window = if is_agent(id(root)) { root } else { foreground };
        let started = windows.get(&id(window)).is_some_and(|tracked| tracked.since.elapsed() >= STARTUP);
        let mut restored = give_foreground_to(target);
        if !restored && started {
            // Windows refused SetForegroundWindow. Minimizing the active window makes it pass the
            // foreground on (to the next window: the user's); restore it right away, inactive. It
            // all happens on the virtual display, out of the user's sight.
            unsafe {
                let _ = ShowWindow(window, SW_MINIMIZE);
                let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
            }
            restored = unsafe { GetForegroundWindow() } != foreground;
        }
        tracing::info!(agent = id(foreground), user = id(target), restored, "agent window took the foreground");
    }
}

/// Front-most window on the user's own monitors, for when we never saw their foreground window.
fn top_user_window(windows: &HashMap<Hwnd, Tracked>, display: Rect) -> Option<HWND> {
    super::list()
        .into_iter()
        .find(|w| !windows.contains_key(&w.hwnd) && !w.minimized && w.rect.fraction_inside(&display) <= ON_DISPLAY)
        .and_then(|w| handle(w.hwnd).ok())
}

fn id(hwnd: HWND) -> Hwnd {
    hwnd.0 as isize as Hwnd
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
