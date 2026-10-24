//! OuterSpace core: everything that is not UI. The Tauri app and the MCP server both drive the
//! virtual display through [`power::PowerController`].

pub mod apps;
pub mod capture;
pub mod config;
pub mod display;
pub mod geom;
pub mod input;
pub mod paths;
pub mod power;
pub mod system;

pub use display::{DisplayInfo, DisplayMode, DriverStatus};
pub use geom::Rect;
pub use power::{ErrorCode, PowerController, PowerState};

/// Fixed for the MVP (docs/PLANO.md §2.5); becomes configurable later.
pub const MCP_PORT: u16 = 7342;
