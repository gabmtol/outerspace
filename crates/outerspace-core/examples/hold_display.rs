//! Plugs the virtual display and holds it until the process is killed. Used to check that a crashed
//! OuterSpace never leaves a display behind: once the pings stop, the driver unplugs it on its own.
//!
//! cargo run -p outerspace-core --example hold_display

use outerspace_core::display::parsec::ParsecVdd;
use outerspace_core::display::{DisplayMode, VirtualDisplayBackend};

fn main() {
    let mut vdd = ParsecVdd::new();
    let info = vdd.connect(DisplayMode::DEFAULT).expect("connect");
    println!("connected {}x{} at ({}, {})", info.width, info.height, info.x, info.y);
    std::thread::sleep(std::time::Duration::from_secs(120));
}
