//! The on/off switch. One [`PowerController`] per process; the UI, the tray and the MCP server all go
//! through it, so "off" really means off for everyone.

use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

use crate::display::{DisplayError, DisplayInfo, DisplayMode, DriverStatus, VirtualDisplayBackend};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PowerState {
    Off,
    Starting,
    Stopping,
    On {
        display: DisplayInfo,
    },
    Error {
        code: ErrorCode,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    DriverMissing,
    DriverDisabled,
    RestartRequired,
    ConnectFailed,
    InstallFailed,
}

type Listener = Box<dyn Fn(&PowerState) + Send + Sync>;

pub struct PowerController {
    /// Also serializes power operations: only one connect/disconnect/install runs at a time.
    backend: Mutex<Box<dyn VirtualDisplayBackend>>,
    state: Mutex<PowerState>,
    /// The resolution chosen by the user, kept while the display is off.
    mode: Mutex<DisplayMode>,
    /// The backend's offered modes, read once: asking the backend would wait out a connect.
    modes: Vec<DisplayMode>,
    listener: Listener,
}

impl PowerController {
    /// `listener` runs after every state change, on the thread that made it.
    pub fn new(
        backend: Box<dyn VirtualDisplayBackend>,
        listener: impl Fn(&PowerState) + Send + Sync + 'static,
    ) -> Self {
        let state = driver_problem(backend.driver_status()).unwrap_or(PowerState::Off);
        PowerController {
            modes: backend.modes(),
            backend: Mutex::new(backend),
            state: Mutex::new(state),
            mode: Mutex::new(DisplayMode::DEFAULT),
            listener: Box::new(listener),
        }
    }

    pub fn state(&self) -> PowerState {
        lock(&self.state).clone()
    }

    pub fn is_on(&self) -> bool {
        matches!(self.state(), PowerState::On { .. })
    }

    /// The resolution the display has, or gets when it turns on.
    pub fn mode(&self) -> DisplayMode {
        *lock(&self.mode)
    }

    /// The resolutions [`set_mode`](Self::set_mode) accepts, smallest first.
    pub fn modes(&self) -> &[DisplayMode] {
        &self.modes
    }

    /// Chooses the resolution. A display that is on switches right away; if Windows refuses the
    /// mode, the display and the choice stay as they were.
    pub fn set_mode(&self, mode: DisplayMode) -> Result<PowerState, DisplayError> {
        if !self.modes.contains(&mode) {
            return Err(DisplayError::Failed(format!("{}×{} is not an available resolution", mode.width, mode.height)));
        }
        let mut backend = lock(&self.backend);
        if let PowerState::On { display } = self.state()
            && (display.width, display.height) != (mode.width, mode.height)
        {
            let display = backend.set_mode(mode)?;
            *lock(&self.mode) = mode;
            return Ok(self.set(PowerState::On { display }));
        }
        *lock(&self.mode) = mode;
        Ok(self.state())
    }

    /// Blocks while the display is plugged in and arranged (a few seconds at most).
    pub fn turn_on(&self) -> PowerState {
        let mut backend = lock(&self.backend);
        if self.is_on() {
            return self.state();
        }
        self.connect(&mut backend)
    }

    pub fn turn_off(&self) -> PowerState {
        let mut backend = lock(&self.backend);
        match self.state() {
            PowerState::On { .. } => {
                self.set(PowerState::Stopping);
                backend.disconnect();
                self.set(PowerState::Off)
            }
            // Clearing an error goes back to plain "off", unless the driver itself is the problem.
            PowerState::Error { .. } => self.set(driver_problem(backend.driver_status()).unwrap_or(PowerState::Off)),
            state => state,
        }
    }

    /// Runs the driver installer (UAC prompt) and, if the driver comes up, turns the display on —
    /// installing is only ever asked for on the way to turning on.
    pub fn install_driver(&self) -> PowerState {
        let mut backend = lock(&self.backend);
        if let Err(err) = backend.install_driver() {
            tracing::warn!(%err, "driver install failed");
            return self.set(PowerState::Error { code: ErrorCode::InstallFailed, message: Some(err.to_string()) });
        }
        match driver_problem(backend.driver_status()) {
            Some(problem) => self.set(problem),
            None => self.connect(&mut backend),
        }
    }

    fn connect(&self, backend: &mut MutexGuard<'_, Box<dyn VirtualDisplayBackend>>) -> PowerState {
        self.set(PowerState::Starting);
        match backend.connect(self.mode()) {
            Ok(display) => self.set(PowerState::On { display }),
            Err(DisplayError::Driver(status)) => self.set(
                driver_problem(status).unwrap_or(PowerState::Error { code: ErrorCode::ConnectFailed, message: None }),
            ),
            Err(err) => {
                tracing::warn!(%err, "connect failed");
                self.set(PowerState::Error { code: ErrorCode::ConnectFailed, message: Some(err.to_string()) })
            }
        }
    }

    fn set(&self, state: PowerState) -> PowerState {
        *lock(&self.state) = state.clone();
        (self.listener)(&state);
        state
    }
}

fn driver_problem(status: DriverStatus) -> Option<PowerState> {
    let code = match status {
        DriverStatus::Ready => return None,
        DriverStatus::NotInstalled => ErrorCode::DriverMissing,
        DriverStatus::Disabled => ErrorCode::DriverDisabled,
        DriverStatus::RestartRequired => ErrorCode::RestartRequired,
        DriverStatus::Broken => {
            return Some(PowerState::Error {
                code: ErrorCode::ConnectFailed,
                message: Some("The virtual display driver reported a problem. Reinstalling it usually helps.".into()),
            });
        }
    };
    Some(PowerState::Error { code, message: None })
}

/// A panic while holding a lock must not brick the switch; the protected data stays consistent.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct Fake {
        status: DriverStatus,
        status_after_install: DriverStatus,
        fail_connect: bool,
    }

    impl Fake {
        fn ready() -> Self {
            Fake { status: DriverStatus::Ready, status_after_install: DriverStatus::Ready, fail_connect: false }
        }
    }

    impl VirtualDisplayBackend for Fake {
        fn driver_status(&self) -> DriverStatus {
            self.status
        }

        fn connect(&mut self, mode: DisplayMode) -> Result<DisplayInfo, DisplayError> {
            if self.status != DriverStatus::Ready {
                return Err(DisplayError::Driver(self.status));
            }
            if self.fail_connect {
                return Err(DisplayError::Failed("boom".into()));
            }
            Ok(placed(mode))
        }

        fn modes(&self) -> Vec<DisplayMode> {
            vec![DisplayMode::DEFAULT, QHD]
        }

        fn set_mode(&mut self, mode: DisplayMode) -> Result<DisplayInfo, DisplayError> {
            if self.fail_connect {
                return Err(DisplayError::Failed("Windows rejected the display mode".into()));
            }
            Ok(placed(mode))
        }

        fn disconnect(&mut self) {}

        fn install_driver(&mut self) -> Result<(), DisplayError> {
            self.status = self.status_after_install;
            Ok(())
        }
    }

    const QHD: DisplayMode = DisplayMode { width: 2560, height: 1440, hz: 60 };

    /// Where a display in `mode` lands next to a 1920×1080 screen.
    fn placed(mode: DisplayMode) -> DisplayInfo {
        DisplayInfo {
            width: mode.width,
            height: mode.height,
            hz: mode.hz,
            x: 1920,
            y: 1 - mode.height as i32,
            device_name: "fake".into(),
        }
    }

    fn controller(backend: Fake) -> (PowerController, Arc<Mutex<Vec<PowerState>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        (PowerController::new(Box::new(backend), move |s| log.lock().unwrap().push(s.clone())), seen)
    }

    fn kinds(states: &[PowerState]) -> Vec<&'static str> {
        states
            .iter()
            .map(|s| match s {
                PowerState::Off => "off",
                PowerState::Starting => "starting",
                PowerState::Stopping => "stopping",
                PowerState::On { .. } => "on",
                PowerState::Error { .. } => "error",
            })
            .collect()
    }

    #[test]
    fn turns_on_and_off_announcing_each_step() {
        let (power, seen) = controller(Fake::ready());
        assert_eq!(power.state(), PowerState::Off);
        assert!(power.turn_on().is_on_state());
        assert!(power.is_on());
        assert_eq!(power.turn_off(), PowerState::Off);
        assert_eq!(kinds(&seen.lock().unwrap()), ["starting", "on", "stopping", "off"]);
    }

    #[test]
    fn turning_on_twice_connects_once() {
        let (power, seen) = controller(Fake::ready());
        power.turn_on();
        power.turn_on();
        assert_eq!(kinds(&seen.lock().unwrap()), ["starting", "on"]);
    }

    #[test]
    fn starts_in_error_when_the_driver_is_missing() {
        let (power, _) = controller(Fake { status: DriverStatus::NotInstalled, ..Fake::ready() });
        assert_eq!(power.state(), PowerState::Error { code: ErrorCode::DriverMissing, message: None });
        // Turning off does not hide a missing driver.
        assert_eq!(power.turn_off(), PowerState::Error { code: ErrorCode::DriverMissing, message: None });
    }

    #[test]
    fn a_failed_connect_is_an_error_that_turning_off_clears() {
        let (power, _) = controller(Fake { fail_connect: true, ..Fake::ready() });
        assert!(matches!(power.turn_on(), PowerState::Error { code: ErrorCode::ConnectFailed, .. }));
        assert_eq!(power.turn_off(), PowerState::Off);
    }

    #[test]
    fn installing_the_driver_goes_straight_to_on() {
        let (power, _) = controller(Fake { status: DriverStatus::NotInstalled, ..Fake::ready() });
        assert!(power.install_driver().is_on_state());
    }

    #[test]
    fn an_install_that_needs_a_restart_says_so() {
        let (power, _) = controller(Fake {
            status: DriverStatus::NotInstalled,
            status_after_install: DriverStatus::RestartRequired,
            ..Fake::ready()
        });
        assert_eq!(power.install_driver(), PowerState::Error { code: ErrorCode::RestartRequired, message: None });
    }

    #[test]
    fn a_mode_chosen_while_off_is_used_when_turning_on() {
        let (power, seen) = controller(Fake::ready());
        assert_eq!(power.set_mode(QHD).unwrap(), PowerState::Off);
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(power.turn_on(), PowerState::On { display: placed(QHD) });
        assert_eq!(power.mode(), QHD);
    }

    #[test]
    fn changing_the_mode_while_on_switches_the_live_display() {
        let (power, seen) = controller(Fake::ready());
        power.turn_on();
        assert_eq!(power.set_mode(QHD).unwrap(), PowerState::On { display: placed(QHD) });
        assert_eq!(power.state(), PowerState::On { display: placed(QHD) });
        assert_eq!(kinds(&seen.lock().unwrap()), ["starting", "on", "on"]);
    }

    #[test]
    fn a_refused_mode_leaves_the_display_and_the_choice_alone() {
        let (power, _) = controller(Fake::ready());
        power.turn_on();
        // Same switch as a failed connect, flipped after turning on.
        let mut backend = lock(&power.backend);
        *backend = Box::new(Fake { fail_connect: true, ..Fake::ready() });
        drop(backend);
        assert!(power.set_mode(QHD).is_err());
        assert_eq!(power.mode(), DisplayMode::DEFAULT);
        assert_eq!(power.state(), PowerState::On { display: placed(DisplayMode::DEFAULT) });
    }

    #[test]
    fn modes_the_backend_does_not_offer_are_rejected() {
        let (power, _) = controller(Fake::ready());
        assert!(power.set_mode(DisplayMode { width: 1000, height: 1000, hz: 60 }).is_err());
        assert_eq!(power.mode(), DisplayMode::DEFAULT);
    }

    impl PowerState {
        fn is_on_state(&self) -> bool {
            matches!(self, PowerState::On { .. })
        }
    }
}
