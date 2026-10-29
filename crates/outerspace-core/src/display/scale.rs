//! A monitor's scale (Settings → Display → Scale). Windows sets it per monitor through two
//! DisplayConfig requests that Settings itself uses but the SDK headers leave out.

use serde::Serialize;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, LUID};

use super::DisplayError;
use super::layout::from_wide;

/// The scales Windows offers, in percent. Its requests count steps away from the recommended one.
const STEPS: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];
const GET_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-3);
const SET_SCALE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(-4);

#[repr(C)]
struct GetScale {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    min: i32,
    current: i32,
    max: i32,
}

#[repr(C)]
struct SetScale {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    steps: i32,
}

/// A monitor's scale and the ones Windows allows at its current resolution, in percent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scales {
    pub current: u32,
    pub recommended: u32,
    /// Smallest first.
    pub available: Vec<u32>,
}

impl Scales {
    /// `wanted`, or the largest allowed scale below it.
    pub fn closest(&self, wanted: u32) -> u32 {
        self.available.iter().copied().filter(|&s| s <= wanted).max().unwrap_or(STEPS[0])
    }
}

/// Scale of the display named `device_name` (`\\.\DISPLAY7`), or `None` if it is not active.
pub fn get(device_name: &str) -> Option<Scales> {
    let (adapter, id) = source(device_name)?;
    let mut request =
        GetScale { header: header(GET_SCALE, size_of::<GetScale>(), adapter, id), min: 0, current: 0, max: 0 };
    if unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } != 0 {
        return None;
    }
    // The smallest scale is always 100%, so `min` says how many steps up the recommended one is.
    let recommended = request.min.unsigned_abs() as usize;
    let step = |relative: i32| STEPS.get(recommended.checked_add_signed(relative as isize)?).copied();
    Some(Scales {
        current: step(request.current)?,
        recommended: step(0)?,
        available: (request.min..=request.max).filter_map(step).collect(),
    })
}

pub fn set(device_name: &str, percent: u32) -> Result<(), DisplayError> {
    let inactive = || DisplayError::Failed(format!("{device_name} is not an active display"));
    let scales = get(device_name).ok_or_else(inactive)?;
    if !scales.available.contains(&percent) {
        return Err(DisplayError::Failed(format!("Windows does not allow {percent}% at this resolution")));
    }
    let index = |percent: u32| STEPS.iter().position(|&s| s == percent).unwrap_or(0) as i32;
    let (adapter, id) = source(device_name).ok_or_else(inactive)?;
    let request = SetScale {
        header: header(SET_SCALE, size_of::<SetScale>(), adapter, id),
        steps: index(percent) - index(scales.recommended),
    };
    match unsafe { DisplayConfigSetDeviceInfo(&request.header) } {
        0 => Ok(()),
        code => Err(DisplayError::Failed(format!("Windows refused the scale (error {code})"))),
    }
}

/// The DisplayConfig source behind a GDI device name.
fn source(device_name: &str) -> Option<(LUID, u32)> {
    let (mut path_count, mut mode_count) = (0u32, 0u32);
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) } != ERROR_SUCCESS
    {
        return None;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    let queried = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    };
    if queried != ERROR_SUCCESS {
        return None;
    }
    paths.truncate(path_count as usize);
    paths.iter().map(|path| (path.sourceInfo.adapterId, path.sourceInfo.id)).find(|&(adapter, id)| {
        let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: header(
                DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
                adapter,
                id,
            ),
            ..Default::default()
        };
        let found = unsafe { DisplayConfigGetDeviceInfo(&mut name.header) };
        found == 0 && from_wide(&name.viewGdiDeviceName) == device_name
    })
}

fn header(
    kind: DISPLAYCONFIG_DEVICE_INFO_TYPE,
    size: usize,
    adapter: LUID,
    id: u32,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: kind, size: size as u32, adapterId: adapter, id }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_allowed_scale() {
        let scales = Scales { current: 125, recommended: 125, available: vec![100, 125, 150] };
        assert_eq!(scales.closest(125), 125);
        assert_eq!(scales.closest(200), 150);
        assert_eq!(scales.closest(50), 100);
    }

    /// Prints every active display's scale; changes nothing.
    /// `cargo test --release -p outerspace-core scales_of_active_displays -- --ignored --nocapture`
    #[test]
    #[ignore = "reads this PC's displays"]
    fn scales_of_active_displays() {
        for i in 1..=32 {
            let name = format!(r"\\.\DISPLAY{i}");
            if let Some(scales) = get(&name) {
                println!("{name}: {scales:?}");
            }
        }
    }
}
