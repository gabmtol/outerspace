use std::os::windows::io::IntoRawHandle;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
use windows::core::w;

/// Windows "Transparency effects" setting. When it is off the UI switches to its solid ("tinted") look.
pub fn transparency_enabled() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    result.is_err() || value != 0
}

/// Makes `file` this process's stderr for good. A windowed app has none, so otherwise the last words
/// of a crash (a panic's message, or "memory allocation of N bytes failed" before an abort) are lost.
pub fn stderr_to(file: std::fs::File) {
    let _ = unsafe { SetStdHandle(STD_ERROR_HANDLE, HANDLE(file.into_raw_handle())) };
}

/// Window and screen coordinates from Win32 must be physical pixels whatever thread asks
/// (tokio workers, tests), so every entry point that touches them calls this first.
pub(crate) fn dpi_aware() {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}
