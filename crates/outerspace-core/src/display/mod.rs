//! Virtual display backends. The MVP ships [`parsec::ParsecVdd`]; anything else (e.g. the MTT
//! Virtual-Display-Driver) plugs in by implementing [`VirtualDisplayBackend`] — nothing outside this
//! module may depend on Parsec specifics (docs/PLANO.md §2.1).

pub mod layout;
pub mod parsec;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub hz: u32,
}

impl DisplayMode {
    pub const DEFAULT: DisplayMode = DisplayMode { width: 1920, height: 1080, hz: 60 };
}

/// Where the virtual display ended up on the Windows desktop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub hz: u32,
    /// Top-left corner in virtual-desktop coordinates.
    pub x: i32,
    pub y: i32,
    /// GDI device name, e.g. `\\.\DISPLAY3`.
    #[serde(skip)]
    pub device_name: String,
}

impl DisplayInfo {
    pub fn rect(&self) -> crate::Rect {
        crate::Rect::new(self.x, self.y, self.width as i32, self.height as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverStatus {
    Ready,
    NotInstalled,
    Disabled,
    RestartRequired,
    /// Present but not usable (driver error, disabled service, unknown problem).
    Broken,
}

#[derive(Debug, thiserror::Error)]
pub enum DisplayError {
    #[error("virtual display driver is not ready ({0:?})")]
    Driver(DriverStatus),
    #[error("{0}")]
    Failed(String),
}

impl DisplayError {
    pub fn failed(context: &str, err: impl std::fmt::Display) -> Self {
        DisplayError::Failed(format!("{context}: {err}"))
    }
}

pub trait VirtualDisplayBackend: Send {
    fn driver_status(&self) -> DriverStatus;

    /// Plugs in one virtual display and arranges it next to the user's monitors. Blocks until
    /// Windows reports it attached to the desktop.
    fn connect(&mut self, mode: DisplayMode) -> Result<DisplayInfo, DisplayError>;

    /// Unplugs the display. Idempotent.
    fn disconnect(&mut self);

    /// Installs the driver (asks for elevation). Blocks until the installer exits.
    fn install_driver(&mut self) -> Result<(), DisplayError>;
}
