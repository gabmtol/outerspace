//! Window icons as small PNGs, for the list of windows in OuterSpace's own window.

use std::ffi::c_void;

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GdiFlush, SelectObject,
};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{
    DI_NORMAL, DestroyIcon, DrawIconEx, GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, HICON, ICON_BIG, ICON_SMALL2,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
};
use windows::core::HSTRING;

use super::{Hwnd, handle, pid_of, process_path};

/// Side of the square image, in pixels: the size Windows hands out as an app's "big" icon.
const SIZE: i32 = 32;

/// The icon the window shows in the taskbar, else its program's, as a PNG.
pub fn icon_png(hwnd: Hwnd) -> Option<Vec<u8>> {
    let window = handle(hwnd).ok()?;
    let image = match window_icon(window) {
        Some(icon) => render(icon),
        None => {
            // Extracted icons are ours to destroy; window and class icons belong to the app.
            let icon = program_icon(window)?;
            let image = render(icon);
            let _ = unsafe { DestroyIcon(icon) };
            image
        }
    }?;
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(image.as_raw(), image.width(), image.height(), ExtendedColorType::Rgba8)
        .ok()?;
    Some(png)
}

fn window_icon(window: HWND) -> Option<HICON> {
    for kind in [ICON_BIG, ICON_SMALL2] {
        let mut icon = 0usize;
        // A hung app must not hang the caller.
        let answered = unsafe {
            SendMessageTimeoutW(
                window,
                WM_GETICON,
                WPARAM(kind as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                100,
                Some(&mut icon),
            )
        };
        if answered.0 != 0 && icon != 0 {
            return Some(HICON(icon as *mut c_void));
        }
    }
    [GCLP_HICON, GCLP_HICONSM]
        .into_iter()
        .map(|index| unsafe { GetClassLongPtrW(window, index) })
        .find(|&icon| icon != 0)
        .map(|icon| HICON(icon as *mut c_void))
}

fn program_icon(window: HWND) -> Option<HICON> {
    let path = HSTRING::from(process_path(pid_of(window))?);
    let mut icon = HICON::default();
    let found = unsafe { ExtractIconExW(&path, 0, Some(&mut icon), None, 1) };
    (found > 0 && !icon.is_invalid()).then_some(icon)
}

/// Draws the icon over black and over white: the difference gives each pixel's opacity, whether
/// the icon has an alpha channel or only an old-style mask.
fn render(icon: HICON) -> Option<RgbaImage> {
    let (black, white) = (draw(icon, 0x00_00_00)?, draw(icon, 0xFF_FF_FF)?);
    let mut image = RgbaImage::new(SIZE as u32, SIZE as u32);
    for ((pixel, black), white) in image.pixels_mut().zip(black.as_chunks::<4>().0).zip(white.as_chunks::<4>().0) {
        // Over black a pixel shows color × alpha; over white, that plus 255 × (1 − alpha).
        let seen_through = (0..3).map(|c| white[c].saturating_sub(black[c])).max().unwrap_or(0);
        let alpha = 255 - u32::from(seen_through);
        let unblend = |c: u8| (u32::from(c) * 255).checked_div(alpha).map_or(0, |c| c.min(255) as u8);
        // GDI's order is BGRA.
        *pixel = Rgba([unblend(black[2]), unblend(black[1]), unblend(black[0]), alpha as u8]);
    }
    Some(image)
}

/// The icon drawn over a solid `background` (0xRRGGBB), as BGRA pixels.
fn draw(icon: HICON, background: u32) -> Option<Vec<u8>> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE,
            biHeight: -SIZE, // negative: rows top to bottom
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let pixels = (SIZE * SIZE) as usize;
    unsafe {
        let memory = CreateCompatibleDC(None);
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(memory);
            return None;
        };
        let previous = SelectObject(memory, bitmap.into());
        std::slice::from_raw_parts_mut(bits.cast::<u32>(), pixels).fill(background);
        let drawn = DrawIconEx(memory, 0, 0, icon, SIZE, SIZE, 0, None, DI_NORMAL).is_ok();
        let _ = GdiFlush();
        let copy = std::slice::from_raw_parts(bits.cast::<u8>(), pixels * 4).to_vec();
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        drawn.then_some(copy)
    }
}

#[cfg(test)]
mod tests {
    /// Writes the icon of every open window to `target/icons-test/` to look at; touches no window.
    /// `cargo test -p outerspace-core icons_of_open_windows -- --ignored`
    #[test]
    #[ignore = "reads the windows open on this PC"]
    fn icons_of_open_windows() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/icons-test");
        std::fs::create_dir_all(&dir).unwrap();
        let windows = super::super::list();
        let mut drawn = 0;
        for window in &windows {
            if let Some(png) = super::icon_png(window.hwnd) {
                std::fs::write(dir.join(format!("{}-{}.png", window.process, window.hwnd)), png).unwrap();
                drawn += 1;
            }
        }
        println!("{drawn} icons for {} windows in {}", windows.len(), dir.display());
        assert!(drawn > 0);
    }
}
