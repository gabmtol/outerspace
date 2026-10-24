//! Screenshots of the virtual display (docs/PLANO.md §2.4).
//!
//! Plain GDI: since Windows 8 the screen DC is DWM's composed output, so DirectX/GPU-rendered
//! windows come out right, and a virtual display has no hardware overlay planes that could hide
//! content from it. That keeps this a few dozen lines instead of a Direct3D + WinRT capture pipeline.

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, RgbImage};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};

use crate::geom::Rect;
use crate::system::dpi_aware;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

pub struct Screenshot {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error)]
#[error("screenshot failed: {0}")]
pub struct CaptureError(String);

/// Copies `area` (desktop coordinates) off the screen.
pub fn grab(area: Rect) -> Result<RgbImage, CaptureError> {
    if area.w <= 0 || area.h <= 0 {
        return Err(CaptureError(format!("empty area {area:?}")));
    }
    dpi_aware();
    let mut bgra = vec![0u8; area.w as usize * area.h as usize * 4];
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, area.w, area.h);
        let previous = SelectObject(memory, bitmap.into());
        let copied = BitBlt(memory, 0, 0, area.w, area.h, Some(screen), area.x, area.y, SRCCOPY);
        // GetDIBits wants the bitmap out of any DC.
        SelectObject(memory, previous);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: area.w,
                biHeight: -area.h, // negative: rows top to bottom
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let lines =
            GetDIBits(memory, bitmap, 0, area.h as u32, Some(bgra.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        copied.map_err(|e| CaptureError(e.message()))?;
        if lines != area.h {
            return Err(CaptureError("could not read the captured pixels".into()));
        }
    }
    let rgb: Vec<u8> = bgra.chunks_exact(4).flat_map(|px| [px[2], px[1], px[0]]).collect();
    RgbImage::from_raw(area.w as u32, area.h as u32, rgb).ok_or_else(|| CaptureError("bad pixel buffer".into()))
}

/// Scales by `scale` (0.1–1) and encodes. PNG for crisp text, JPEG (quality 80) when size matters.
pub fn encode(image: &RgbImage, scale: f32, format: Format) -> Result<Screenshot, CaptureError> {
    let scale = scale.clamp(0.1, 1.0);
    let width = ((image.width() as f32 * scale).round() as u32).max(1);
    let height = ((image.height() as f32 * scale).round() as u32).max(1);
    let resized;
    let image = if (width, height) == image.dimensions() {
        image
    } else {
        resized = image::imageops::resize(image, width, height, FilterType::Triangle);
        &resized
    };

    let mut bytes = Vec::new();
    let (written, mime) = match format {
        Format::Png => (
            PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, PngFilter::Adaptive).write_image(
                image.as_raw(),
                width,
                height,
                ExtendedColorType::Rgb8,
            ),
            "image/png",
        ),
        Format::Jpeg => (
            JpegEncoder::new_with_quality(&mut bytes, 80).write_image(
                image.as_raw(),
                width,
                height,
                ExtendedColorType::Rgb8,
            ),
            "image/jpeg",
        ),
    };
    written.map_err(|e| CaptureError(e.to_string()))?;
    Ok(Screenshot { bytes, mime, width, height })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_scaled_png_and_jpeg() {
        let image = RgbImage::from_pixel(1920, 1080, image::Rgb([232, 121, 43]));
        let png = encode(&image, 2.0 / 3.0, Format::Png).unwrap();
        assert_eq!((png.width, png.height, png.mime), (1280, 720, "image/png"));
        assert!(png.bytes.starts_with(b"\x89PNG"));
        let jpeg = encode(&image, 1.0, Format::Jpeg).unwrap();
        assert_eq!((jpeg.width, jpeg.height), (1920, 1080));
        assert!(jpeg.bytes.starts_with(&[0xFF, 0xD8]));
    }
}
