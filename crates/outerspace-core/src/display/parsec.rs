//! Parsec Virtual Display Driver backend.
//!
//! Rust port of `core/parsec-vdd.h` from https://github.com/nomi-san/parsec-vdd:
//!
//! > Copyright (c) 2023, Nguyen Duy <wuuyi123@gmail.com> All rights reserved.
//! >
//! > Redistribution and use in source and binary forms, with or without modification, are
//! > permitted provided that the following conditions are met:
//! > * Redistributions of source code must retain the above copyright notice, this list of
//! >   conditions and the following disclaimer.
//! > * Redistributions in binary form must reproduce the above copyright notice, this list of
//! >   conditions and the following disclaimer in the documentation and/or other materials
//! >   provided with the distribution.
//! >
//! > THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
//! > IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
//! > AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR
//! > CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
//! > CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//! > SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
//! > THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
//! > OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
//! > POSSIBILITY OF SUCH DAMAGE.
//!
//! The driver itself is Parsec's signed binary; OuterSpace never bundles it — [`ParsecVdd::install_driver`]
//! downloads the official installer and checks its SHA-256 before running it.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use sha2::{Digest, Sha256};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVNODE_STATUS_FLAGS, CM_Get_DevNode_Status, CM_PROB, CM_PROB_DISABLED, CM_PROB_HARDWARE_DISABLED,
    CM_PROB_NEED_RESTART, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, DN_DRIVER_LOADED, DN_HAS_PROBLEM,
    DN_STARTED, HDEVINFO, SETUP_DI_GET_CLASS_DEVS_FLAGS, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SP_DEVINFO_DATA, SPDRP_HARDWAREID, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
    SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    SetupDiGetDeviceRegistryPropertyW,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED, FILE_FLAG_WRITE_THROUGH,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{GUID, HSTRING, PCWSTR, w};

use super::{DisplayError, DisplayInfo, DisplayMode, DriverStatus, VirtualDisplayBackend, layout};
use crate::paths;

/// Interface GUID used to open the adapter.
const ADAPTER_GUID: GUID = GUID::from_u128(0x00b41627_04c4_429e_a26e_0265cf50c8fa);
/// Display adapter device class, used to find the device node and its status.
const DISPLAY_CLASS_GUID: GUID = GUID::from_u128(0x4d36e968_e325_11ce_bfc1_08002be10318);
const HARDWARE_ID: &str = r"Root\Parsec\VDA";
/// EDID product code of the monitors this driver plugs in (`MONITOR\PSCCDD0\...`).
const MONITOR_ID: &str = "PSCCDD0";

const IOCTL_ADD: u32 = 0x0022_e004;
const IOCTL_REMOVE: u32 = 0x0022_a008;
const IOCTL_UPDATE: u32 = 0x0022_a00c;
const IOCTL_VERSION: u32 = 0x0022_e010;

/// The driver unplugs every display after ~1 s without an update; ping well inside that.
const PING_INTERVAL: Duration = Duration::from_millis(50);
const IOCTL_TIMEOUT_MS: u32 = 5_000;
const ATTACH_TIMEOUT: Duration = Duration::from_secs(10);

/// Official installer (Authenticode-signed by Parsec Cloud, Inc.). Fallback if 0.45 misbehaves on a
/// machine: parsec-vdd-0.41.0.0.exe, sha256 317977c5abd58f1c3f7b27ef7f96947efd07e6d35b27e415cea5daa6007629b1.
const INSTALLER_URL: &str = "https://builds.parsec.app/vdd/parsec-vdd-0.45.0.0.exe";
const INSTALLER_FILE: &str = "parsec-vdd-0.45.0.0.exe";
const INSTALLER_SHA256: &str = "e23332448fdaf5aa017cb308db5ef6855fac526a7ded05d80c039404126d5362";

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Offered resolutions: the driver's built-in modes in the common aspect ratios (16:9, 16:10,
/// 21:9). The driver has every one of them at 60 Hz (docs/PARSEC_VDD_SPECS.md in parsec-vdd).
const MODES: [(u32, u32); 13] = [
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1440, 900),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2560, 1080),
    (2560, 1440),
    (2560, 1600),
    (3440, 1440),
    (3840, 2160),
];

#[derive(Default)]
pub struct ParsecVdd {
    session: Option<Session>,
    /// GDI name of the output driving the display, while connected.
    output: Option<String>,
}

impl ParsecVdd {
    pub fn new() -> Self {
        Self::default()
    }
}

impl VirtualDisplayBackend for ParsecVdd {
    fn driver_status(&self) -> DriverStatus {
        query_status()
    }

    fn connect(&mut self, mode: DisplayMode) -> Result<DisplayInfo, DisplayError> {
        self.disconnect();
        match query_status() {
            DriverStatus::Ready => {}
            status => return Err(DisplayError::Driver(status)),
        }

        let device = Arc::new(Device::open()?);
        tracing::info!(minor_version = device.ioctl(IOCTL_VERSION, &[])?, "parsec vdd opened");

        let started = std::time::Instant::now();
        let index = device.ioctl(IOCTL_ADD, &[])?;
        device.ioctl(IOCTL_UPDATE, &[])?;
        tracing::debug!(elapsed = ?started.elapsed(), index, "display added");
        // From here on, dropping `session` unplugs the display again — including on the error paths below.
        let session = Session::start(device, index);

        let name = layout::wait_for_extended(MONITOR_ID, ATTACH_TIMEOUT)?;
        tracing::debug!(elapsed = ?started.elapsed(), %name, "display on the desktop");
        let info = layout::place_beside_desktop(&name, mode)?;
        tracing::debug!(elapsed = ?started.elapsed(), "display placed");
        tracing::info!(?info, index, "virtual display connected");
        self.session = Some(session);
        self.output = Some(name);
        Ok(info)
    }

    fn modes(&self) -> Vec<DisplayMode> {
        MODES.iter().map(|&(width, height)| DisplayMode { width, height, hz: 60 }).collect()
    }

    fn set_mode(&mut self, mode: DisplayMode) -> Result<DisplayInfo, DisplayError> {
        let output = self.output.as_deref().ok_or_else(|| DisplayError::Failed("the virtual display is off".into()))?;
        // Re-placed, not just resized: the one-pixel contact with the desktop depends on the height.
        let info = layout::place_beside_desktop(output, mode)?;
        tracing::info!(?info, "virtual display mode changed");
        Ok(info)
    }

    fn disconnect(&mut self) {
        self.output = None;
        if let Some(session) = self.session.take() {
            drop(session);
            tracing::info!("virtual display disconnected");
        }
    }

    fn install_driver(&mut self) -> Result<(), DisplayError> {
        let installer = paths::driver_dir().join(INSTALLER_FILE);
        if !has_expected_hash(&installer) {
            download(INSTALLER_URL, &installer)?;
            if !has_expected_hash(&installer) {
                let _ = std::fs::remove_file(&installer);
                return Err(DisplayError::Failed("the downloaded driver installer failed its integrity check".into()));
            }
        }
        let code = run_elevated(&installer, "/S")?;
        tracing::info!(code, "driver installer finished");
        if code != 0 {
            return Err(DisplayError::Failed(format!("the driver installer exited with code {code}")));
        }
        Ok(())
    }
}

/// One plugged-in display, kept alive by a ping thread. Dropping it unplugs the display.
struct Session {
    device: Arc<Device>,
    index: u32,
    stop: Arc<AtomicBool>,
    pinger: Option<JoinHandle<()>>,
}

impl Session {
    fn start(device: Arc<Device>, index: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let pinger = std::thread::Builder::new()
            .name("vdd-ping".into())
            .spawn({
                let device = device.clone();
                let stop = stop.clone();
                move || {
                    while !stop.load(Ordering::Relaxed) {
                        if let Err(err) = device.ioctl(IOCTL_UPDATE, &[]) {
                            tracing::warn!(%err, "vdd ping failed");
                        }
                        std::thread::sleep(PING_INTERVAL);
                    }
                }
            })
            .expect("spawn vdd ping thread");
        Session { device, index, stop, pinger: Some(pinger) }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // The driver expects the display index as a 16-bit big-endian value.
        let index = (self.index as u16).to_be_bytes();
        if let Err(err) = self.device.ioctl(IOCTL_REMOVE, &index) {
            tracing::warn!(%err, "vdd remove failed; the display will drop once pings stop");
        }
        let _ = self.device.ioctl(IOCTL_UPDATE, &[]);
        self.stop.store(true, Ordering::Relaxed);
        if let Some(pinger) = self.pinger.take() {
            let _ = pinger.join();
        }
    }
}

/// Overlapped handle to the VDD adapter. DeviceIoControl is safe to call from several threads as
/// long as each call brings its own OVERLAPPED, which [`Device::ioctl`] does.
struct Device(HANDLE);

unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    fn open() -> Result<Device, DisplayError> {
        let set = DevInfoSet::new(&ADAPTER_GUID, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)?;
        let mut interface =
            SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
        for i in 0.. {
            if unsafe { SetupDiEnumDeviceInterfaces(set.0, None, &ADAPTER_GUID, i, &mut interface) }.is_err() {
                break;
            }
            let mut size = 0u32;
            let _ = unsafe { SetupDiGetDeviceInterfaceDetailW(set.0, &interface, None, 0, Some(&mut size), None) };
            // SP_DEVICE_INTERFACE_DETAIL_DATA_W is a cbSize header followed by a variable-length path.
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let detail = buffer.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
            if unsafe { SetupDiGetDeviceInterfaceDetailW(set.0, &interface, Some(detail), size, None, None) }.is_err() {
                continue;
            }
            let path = PCWSTR(unsafe { (&raw const (*detail).DevicePath).cast() });
            let handle = unsafe {
                CreateFileW(
                    path,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING | FILE_FLAG_OVERLAPPED | FILE_FLAG_WRITE_THROUGH,
                    None,
                )
            };
            if let Ok(handle) = handle {
                return Ok(Device(handle));
            }
        }
        Err(DisplayError::Failed("could not open the virtual display adapter".into()))
    }

    /// Sends one control code with up to 32 bytes of input and returns the driver's 32-bit answer.
    fn ioctl(&self, code: u32, input: &[u8]) -> Result<u32, DisplayError> {
        let mut in_buffer = [0u8; 32];
        in_buffer[..input.len()].copy_from_slice(input);
        let mut out_buffer = 0u32;
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|e| DisplayError::failed("CreateEvent", e))?;
        let mut overlapped = OVERLAPPED { hEvent: event, ..Default::default() };
        let mut transferred = 0u32;

        // Overlapped calls normally report ERROR_IO_PENDING here; the real outcome comes from the wait.
        let _ = unsafe {
            DeviceIoControl(
                self.0,
                code,
                Some(in_buffer.as_ptr().cast()),
                in_buffer.len() as u32,
                Some((&raw mut out_buffer).cast()),
                size_of::<u32>() as u32,
                None,
                Some(&mut overlapped),
            )
        };
        let result = unsafe { GetOverlappedResultEx(self.0, &overlapped, &mut transferred, IOCTL_TIMEOUT_MS, false) };
        if result.is_err() {
            // The driver may still write into our buffers; cancel and wait for it to let go of them.
            unsafe {
                let _ = CancelIoEx(self.0, Some(&overlapped));
                let _ = GetOverlappedResult(self.0, &overlapped, &mut transferred, true);
            }
        }
        unsafe {
            let _ = CloseHandle(event);
        }
        result.map(|()| out_buffer).map_err(|e| DisplayError::failed("virtual display driver did not respond", e))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct DevInfoSet(HDEVINFO);

impl DevInfoSet {
    fn new(guid: &GUID, flags: SETUP_DI_GET_CLASS_DEVS_FLAGS) -> Result<Self, DisplayError> {
        unsafe { SetupDiGetClassDevsW(Some(guid), PCWSTR::null(), None, flags) }
            .map(DevInfoSet)
            .map_err(|e| DisplayError::failed("SetupDiGetClassDevs", e))
    }
}

impl Drop for DevInfoSet {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// Finds the `Root\Parsec\VDA` device node among display adapters and reports whether it is usable.
fn query_status() -> DriverStatus {
    let Ok(set) = DevInfoSet::new(&DISPLAY_CLASS_GUID, DIGCF_PRESENT) else {
        return DriverStatus::NotInstalled;
    };
    let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
    for i in 0.. {
        if unsafe { SetupDiEnumDeviceInfo(set.0, i, &mut info) }.is_err() {
            break;
        }
        if !hardware_ids(&set, &info).iter().any(|id| id.eq_ignore_ascii_case(HARDWARE_ID)) {
            continue;
        }
        let mut status = CM_DEVNODE_STATUS_FLAGS(0);
        let mut problem = CM_PROB(0);
        if unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, info.DevInst, 0) } != CR_SUCCESS {
            return DriverStatus::Broken;
        }
        return if status.0 & (DN_DRIVER_LOADED.0 | DN_STARTED.0) != 0 {
            DriverStatus::Ready
        } else if status.0 & DN_HAS_PROBLEM.0 != 0 {
            match problem {
                CM_PROB_NEED_RESTART => DriverStatus::RestartRequired,
                CM_PROB_DISABLED | CM_PROB_HARDWARE_DISABLED => DriverStatus::Disabled,
                _ => DriverStatus::Broken,
            }
        } else {
            DriverStatus::Broken
        };
    }
    DriverStatus::NotInstalled
}

fn hardware_ids(set: &DevInfoSet, info: &SP_DEVINFO_DATA) -> Vec<String> {
    let mut size = 0u32;
    let _ = unsafe { SetupDiGetDeviceRegistryPropertyW(set.0, info, SPDRP_HARDWAREID, None, None, Some(&mut size)) };
    if size == 0 {
        return Vec::new();
    }
    let mut buffer = vec![0u8; size as usize];
    if unsafe { SetupDiGetDeviceRegistryPropertyW(set.0, info, SPDRP_HARDWAREID, None, Some(&mut buffer), None) }
        .is_err()
    {
        return Vec::new();
    }
    // REG_MULTI_SZ: UTF-16 strings separated by NULs, ending with an empty string.
    let wide: Vec<u16> = buffer.as_chunks::<2>().0.iter().map(|&unit| u16::from_le_bytes(unit)).collect();
    wide.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

fn has_expected_hash(file: &Path) -> bool {
    let Ok(bytes) = std::fs::read(file) else { return false };
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex == INSTALLER_SHA256
}

fn download(url: &str, to: &Path) -> Result<(), DisplayError> {
    tracing::info!(url, "downloading driver installer");
    // curl.exe ships with Windows 10 1803+ and handles proxies/TLS the same way the system does.
    let status = Command::new("curl.exe")
        .args(["--fail", "--silent", "--show-error", "--location", "--output"])
        .arg(to)
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| DisplayError::failed("could not start the download", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(DisplayError::Failed(format!("downloading the driver failed ({status})")))
    }
}

/// Where a Parsec display that is already plugged in (e.g. by the running app) sits on the desktop.
#[cfg(test)]
pub(crate) fn plugged_display() -> Option<crate::Rect> {
    layout::rect_of_monitor(MONITOR_ID)
}

/// Runs `exe` through the UAC prompt and waits for it. Returns the process exit code.
fn run_elevated(exe: &Path, args: &str) -> Result<u32, DisplayError> {
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(err) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(if err.code() == ERROR_CANCELLED.to_hresult() {
            DisplayError::Failed("the administrator prompt was dismissed".into())
        } else {
            DisplayError::failed("could not start the driver installer", err)
        });
    }
    let process = info.hProcess;
    let mut code = 0u32;
    unsafe {
        let waited = WaitForSingleObject(process, INFINITE);
        let read = GetExitCodeProcess(process, &mut code);
        let _ = CloseHandle(process);
        if waited != WAIT_OBJECT_0 || read.is_err() {
            return Err(DisplayError::Failed("lost track of the driver installer".into()));
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test -p outerspace-core probe -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Parsec driver; plugs a real virtual display for a few seconds"]
    fn probe_outputs_while_a_display_is_plugged() {
        println!("driver status: {:?}", query_status());
        let device = Arc::new(Device::open().expect("open adapter"));
        println!("minor version: {}", device.ioctl(IOCTL_VERSION, &[]).unwrap());
        println!("--- before\n{}", layout::dump());
        let index = device.ioctl(IOCTL_ADD, &[]).expect("add");
        device.ioctl(IOCTL_UPDATE, &[]).unwrap();
        println!("added index {index}");
        let session = Session::start(device, index);
        for second in [1, 3, 6] {
            std::thread::sleep(Duration::from_secs(if second == 1 { 1 } else { 3 }));
            println!("--- after {second}s\n{}", layout::dump());
        }
        drop(session);
        std::thread::sleep(Duration::from_secs(2));
        println!("--- after removal\n{}", layout::dump());
    }

    /// Times the removal path: does IOCTL_REMOVE unplug the display while pings continue?
    /// `cargo test -p outerspace-core probe_remove -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Parsec driver; plugs a real virtual display for a few seconds"]
    fn probe_remove_timing() {
        let device = Arc::new(Device::open().expect("open adapter"));
        let index = device.ioctl(IOCTL_ADD, &[]).expect("add");
        device.ioctl(IOCTL_UPDATE, &[]).unwrap();
        let session = Session::start(device.clone(), index);
        let started = std::time::Instant::now();
        while !layout::monitor_active(MONITOR_ID) {
            std::thread::sleep(Duration::from_millis(20));
        }
        println!("index {index}: active after {:.2?}", started.elapsed());
        std::thread::sleep(Duration::from_secs(1));

        let started = std::time::Instant::now();
        let reply = device.ioctl(IOCTL_REMOVE, &(index as u16).to_be_bytes());
        println!("remove replied {reply:?} after {:.2?}", started.elapsed());
        device.ioctl(IOCTL_UPDATE, &[]).unwrap();
        while layout::monitor_active(MONITOR_ID) && started.elapsed() < Duration::from_secs(6) {
            std::thread::sleep(Duration::from_millis(20));
        }
        println!(
            "still pinging; monitor active = {} after {:.2?}",
            layout::monitor_active(MONITOR_ID),
            started.elapsed()
        );
        drop(session);
    }

    /// `cargo test -p outerspace-core connect_ -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Parsec driver; extends the desktop for a few seconds"]
    fn connect_extends_the_desktop_and_disconnect_removes_the_display() {
        let mut vdd = ParsecVdd::new();
        let info = vdd.connect(DisplayMode::DEFAULT).expect("connect");
        println!("{info:?}\n{}", layout::dump());
        assert_eq!((info.width, info.height), (1920, 1080));
        std::thread::sleep(Duration::from_secs(3));
        vdd.disconnect();
        std::thread::sleep(Duration::from_secs(2));
        let after = layout::dump();
        println!("--- after disconnect\n{after}");
        assert!(!after.contains(MONITOR_ID), "the virtual monitor is still there");
    }

    /// Every offered mode applies to a live display, which stays touching the desktop's corner.
    /// `cargo test -p outerspace-core every_offered -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Parsec driver; resizes a real virtual display a dozen times"]
    fn every_offered_mode_applies_to_the_live_display() {
        let mut vdd = ParsecVdd::new();
        let first = vdd.connect(DisplayMode::DEFAULT).expect("connect");
        let corner = (first.x, first.y + first.height as i32);
        for mode in vdd.modes() {
            let started = std::time::Instant::now();
            let info = vdd.set_mode(mode).expect("set mode");
            println!(
                "{}x{}: at ({}, {}) {} Hz in {:.2?}",
                info.width,
                info.height,
                info.x,
                info.y,
                info.hz,
                started.elapsed()
            );
            assert_eq!((info.width, info.height), (mode.width, mode.height));
            assert_eq!((info.x, info.y + info.height as i32), corner, "the display left the corner");
        }
        vdd.set_mode(DisplayMode::DEFAULT).expect("back to the default");
        vdd.disconnect();
    }

    /// Acceptance check for phase F1: on in ≤ 3 s, off in ≤ 2 s, 20 cycles without disturbing the
    /// user's monitors. `cargo test -p outerspace-core twenty_ -- --ignored --nocapture`
    #[test]
    #[ignore = "needs the Parsec driver; plugs and unplugs the display 20 times"]
    fn twenty_cycles_are_fast_and_leave_the_desktop_as_it_was() {
        let _ = tracing_subscriber::fmt().with_test_writer().with_max_level(tracing::Level::DEBUG).try_init();
        let desktop = layout::dump();
        let mut vdd = ParsecVdd::new();
        for cycle in 1..=20 {
            let started = std::time::Instant::now();
            let info = vdd.connect(DisplayMode::DEFAULT).expect("connect");
            let on = started.elapsed();
            std::thread::sleep(Duration::from_secs(1));

            let started = std::time::Instant::now();
            vdd.disconnect();
            while layout::monitor_active(MONITOR_ID) {
                assert!(started.elapsed() < Duration::from_secs(5), "cycle {cycle}: the display did not go away");
                std::thread::sleep(Duration::from_millis(20));
            }
            let off = started.elapsed();
            // Windows keeps tidying up a little after the monitor is gone; the end state must match.
            while layout::dump() != desktop {
                assert!(
                    started.elapsed() < Duration::from_secs(8),
                    "cycle {cycle}: desktop did not return to how it was"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            let settled = started.elapsed();
            println!("cycle {cycle:2}: on {on:.2?} at ({}, {})  off {off:.2?}  settled {settled:.2?}", info.x, info.y);
            assert!(on <= Duration::from_secs(3), "cycle {cycle}: turning on took {on:?}");
            assert!(off <= Duration::from_secs(2), "cycle {cycle}: turning off took {off:?}");
            // The driver needs a moment after a removal before the next add is quick again.
            std::thread::sleep(Duration::from_secs(3));
        }
    }
}
