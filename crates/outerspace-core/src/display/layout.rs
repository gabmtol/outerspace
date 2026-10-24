//! Finding a virtual display among the GDI outputs and placing it on the desktop, through the
//! classic EnumDisplayDevices / ChangeDisplaySettingsEx API (works the same for any IddCx driver).

use std::time::{Duration, Instant};

use windows::Win32::Devices::Display::{SDC_APPLY, SDC_TOPOLOGY_EXTEND, SetDisplayConfig};
use windows::Win32::Graphics::Gdi::{
    CDS_NORESET, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW, DEVMODE_FIELD_FLAGS, DEVMODEW,
    DISP_CHANGE_SUCCESSFUL, DISPLAY_DEVICE_ACTIVE, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICEW,
    DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS, EnumDisplayDevicesW,
    EnumDisplaySettingsW,
};
use windows::core::{HSTRING, PCWSTR};

use super::{DisplayError, DisplayInfo, DisplayMode};

const POLL: Duration = Duration::from_millis(100);
/// How long Windows gets to settle a new monitor on its own before we ask for "Extend".
const SETTLE: Duration = Duration::from_millis(1500);

/// A GDI source (`\\.\DISPLAYn`) and the monitors it drives.
struct Output {
    name: String,
    attached: bool,
    /// Device IDs of the active monitors on this output; more than one means they mirror each other.
    monitors: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    hz: u32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x + self.width as i32
    }
}

fn outputs() -> Vec<Output> {
    let mut list = Vec::new();
    for i in 0.. {
        let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut device, 0) }.as_bool() {
            break;
        }
        let name = from_wide(&device.DeviceName);
        let mut monitors = Vec::new();
        for j in 0.. {
            let mut monitor = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
            if !unsafe { EnumDisplayDevicesW(&HSTRING::from(&name), j, &mut monitor, 0) }.as_bool() {
                break;
            }
            if monitor.StateFlags.0 & DISPLAY_DEVICE_ACTIVE.0 != 0 {
                monitors.push(from_wide(&monitor.DeviceID));
            }
        }
        list.push(Output { name, attached: device.StateFlags.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP.0 != 0, monitors });
    }
    list
}

fn current(name: &str) -> Option<Rect> {
    let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    if !unsafe { EnumDisplaySettingsW(&HSTRING::from(name), ENUM_CURRENT_SETTINGS, &mut mode) }.as_bool() {
        return None;
    }
    let position = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
    Some(Rect {
        x: position.x,
        y: position.y,
        width: mode.dmPelsWidth,
        height: mode.dmPelsHeight,
        hz: mode.dmDisplayFrequency,
    })
}

/// Waits for the monitor whose device ID contains `monitor_id` to get its own part of the desktop and
/// returns the GDI name of the output driving it.
///
/// Windows may bring a new monitor up mirrored onto an existing screen (the "Duplicate" projection
/// mode, the default on some laptops) — useless here, since the user's screen would just be copied.
/// If it is still mirrored or detached after [`SETTLE`], this switches Windows to "Extend", the same
/// as Win+P → Extend.
pub fn wait_for_extended(monitor_id: &str, timeout: Duration) -> Result<String, DisplayError> {
    let deadline = Instant::now() + timeout;
    let mut first_seen: Option<Instant> = None;
    let mut asked_to_extend = false;
    loop {
        let found = outputs().into_iter().find(|o| o.monitors.iter().any(|m| m.contains(monitor_id)));
        if let Some(output) = found {
            if output.attached && output.monitors.len() == 1 {
                return Ok(output.name);
            }
            let seen = *first_seen.get_or_insert_with(Instant::now);
            if !asked_to_extend && seen.elapsed() >= SETTLE {
                tracing::info!(
                    output = %output.name,
                    attached = output.attached,
                    mirrored = output.monitors.len() > 1,
                    waited = ?seen.elapsed(),
                    "switching Windows to Extend"
                );
                extend_desktop()?;
                asked_to_extend = true;
            }
        }
        if Instant::now() >= deadline {
            return Err(DisplayError::Failed("Windows did not set up the virtual display in time".into()));
        }
        std::thread::sleep(POLL);
    }
}

fn extend_desktop() -> Result<(), DisplayError> {
    let result = unsafe { SetDisplayConfig(None, None, SDC_APPLY | SDC_TOPOLOGY_EXTEND) };
    if result == 0 {
        Ok(())
    } else {
        Err(DisplayError::Failed(format!("Windows refused to extend the desktop (error {result})")))
    }
}

/// Sets `mode` on output `name` and puts it just past the top-right corner of the desktop, sharing a
/// single pixel of edge with the right-most monitor so the user's cursor practically never wanders in.
pub fn place_beside_desktop(name: &str, mode: DisplayMode) -> Result<DisplayInfo, DisplayError> {
    let others: Vec<Rect> =
        outputs().iter().filter(|o| o.attached && o.name != name).filter_map(|o| current(&o.name)).collect();
    let (x, y) = position_beside(&others, mode);

    let fields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_POSITION;
    if apply(name, mode, x, y, fields | DM_DISPLAYFREQUENCY).is_err() {
        // Not every refresh rate exists in every resolution; let Windows pick one.
        apply(name, mode, x, y, fields)?;
    }

    let rect = current(name).ok_or_else(|| DisplayError::Failed("the virtual display has no active mode".into()))?;
    Ok(DisplayInfo {
        width: rect.width,
        height: rect.height,
        hz: rect.hz,
        x: rect.x,
        y: rect.y,
        device_name: name.to_owned(),
    })
}

/// Top-left corner for the new display: right of the right-most monitor (the higher one on ties),
/// raised so that only its bottom pixel row overlaps that monitor's top row.
fn position_beside(others: &[Rect], mode: DisplayMode) -> (i32, i32) {
    others
        .iter()
        .max_by_key(|r| (r.right(), -r.y))
        .map(|anchor| (anchor.right(), anchor.y - mode.height as i32 + 1))
        .unwrap_or((0, 0))
}

fn apply(name: &str, mode: DisplayMode, x: i32, y: i32, fields: DEVMODE_FIELD_FLAGS) -> Result<(), DisplayError> {
    let mut devmode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        dmFields: fields,
        dmPelsWidth: mode.width,
        dmPelsHeight: mode.height,
        dmDisplayFrequency: mode.hz,
        ..Default::default()
    };
    devmode.Anonymous1.Anonymous2.dmPosition.x = x;
    devmode.Anonymous1.Anonymous2.dmPosition.y = y;

    let staged = unsafe {
        ChangeDisplaySettingsExW(&HSTRING::from(name), Some(&devmode), None, CDS_UPDATEREGISTRY | CDS_NORESET, None)
    };
    if staged != DISP_CHANGE_SUCCESSFUL {
        return Err(DisplayError::Failed(format!("Windows rejected the display mode ({})", staged.0)));
    }
    // A null device applies everything staged with CDS_NORESET in one go.
    let applied = unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };
    if applied != DISP_CHANGE_SUCCESSFUL {
        return Err(DisplayError::Failed(format!("Windows could not apply the display layout ({})", applied.0)));
    }
    Ok(())
}

fn from_wide(chars: &[u16]) -> String {
    let len = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..len])
}

/// Desktop rectangle of the output driving the monitor whose device ID contains `monitor_id`.
#[cfg(test)]
pub(crate) fn rect_of_monitor(monitor_id: &str) -> Option<crate::Rect> {
    let output = outputs().into_iter().find(|o| o.attached && o.monitors.iter().any(|m| m.contains(monitor_id)))?;
    current(&output.name).map(|r| crate::Rect::new(r.x, r.y, r.width as i32, r.height as i32))
}

/// Whether a monitor whose device ID contains `monitor_id` is active on any output.
#[cfg(test)]
pub(crate) fn monitor_active(monitor_id: &str) -> bool {
    outputs().iter().any(|o| o.monitors.iter().any(|m| m.contains(monitor_id)))
}

/// Every GDI output with its flags, monitors and current mode — for the hardware probe tests.
#[cfg(test)]
pub(crate) fn dump() -> String {
    let mut out = String::new();
    for i in 0.. {
        let mut device = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut device, 0) }.as_bool() {
            break;
        }
        let name = from_wide(&device.DeviceName);
        out += &format!(
            "{name}  '{}'  flags={:#x}  mode={:?}\n",
            from_wide(&device.DeviceString),
            device.StateFlags.0,
            current(&name)
        );
        for j in 0.. {
            let mut monitor = DISPLAY_DEVICEW { cb: size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
            if !unsafe { EnumDisplayDevicesW(&HSTRING::from(&name), j, &mut monitor, 0) }.as_bool() {
                break;
            }
            out += &format!(
                "    monitor {}  '{}'  flags={:#x}  id={}\n",
                from_wide(&monitor.DeviceName),
                from_wide(&monitor.DeviceString),
                monitor.StateFlags.0,
                from_wide(&monitor.DeviceID)
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height, hz: 60 }
    }

    #[test]
    fn sits_past_the_top_right_corner_of_a_single_monitor() {
        assert_eq!(position_beside(&[rect(0, 0, 2560, 1440)], DisplayMode::DEFAULT), (2560, -1079));
    }

    #[test]
    fn anchors_on_the_right_most_monitor() {
        let monitors = [rect(0, 0, 1920, 1080), rect(-1920, 200, 1920, 1080), rect(1920, -300, 1280, 1024)];
        assert_eq!(position_beside(&monitors, DisplayMode::DEFAULT), (3200, -1379));
    }

    #[test]
    fn prefers_the_higher_monitor_when_right_edges_tie() {
        let monitors = [rect(0, 400, 1920, 1080), rect(0, -1080, 1920, 1080)];
        assert_eq!(position_beside(&monitors, DisplayMode::DEFAULT), (1920, -2159));
    }

    #[test]
    fn falls_back_to_the_origin_without_monitors() {
        assert_eq!(position_beside(&[], DisplayMode::DEFAULT), (0, 0));
    }
}
