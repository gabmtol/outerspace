//! OuterSpace core: everything that is not UI. The Tauri app and (later) the MCP server both drive
//! the virtual display through [`power::PowerController`].

pub mod display;
pub mod paths;
pub mod power;
pub mod system;

pub use display::{DisplayInfo, DisplayMode, DriverStatus};
pub use power::{ErrorCode, PowerController, PowerState};

/// Fixed for the MVP (docs/PLANO.md §2.5); becomes configurable later.
pub const MCP_PORT: u16 = 7342;
