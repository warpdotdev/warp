use std::io::Cursor;

use image::{ImageBuffer, ImageFormat, Luma};
use qrcode::{Color, QrCode};

pub const QUIET_ZONE_MODULES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrMatrix {
    width: usize,
    modules: Vec<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum QrCodeError {
    #[error("Unable to encode QR code")]
    Encode,
    #[error("Unable to encode QR code image")]
    Image(#[from] image::ImageError),
}

impl QrMatrix {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        self.modules
            .get(y.saturating_mul(self.width).saturating_add(x))
            .copied()
            .unwrap_or(false)
    }
}

pub fn qr_matrix_for_url(url: &str) -> Result<QrMatrix, QrCodeError> {
    let code = QrCode::new(url.as_bytes()).map_err(|_| QrCodeError::Encode)?;
    let width = code.width();
    let modules = code
        .to_colors()
        .into_iter()
        .map(|color| matches!(color, Color::Dark))
        .collect();
    Ok(QrMatrix { width, modules })
}

pub fn qr_png_for_url(url: &str, pixel_size: u32) -> Result<Vec<u8>, QrCodeError> {
    let matrix = qr_matrix_for_url(url)?;
    let modules_with_quiet_zone = matrix.width().saturating_add(QUIET_ZONE_MODULES * 2);
    let module_size = (pixel_size as usize / modules_with_quiet_zone).max(1);
    let image_size = modules_with_quiet_zone.saturating_mul(module_size) as u32;
    let mut image = ImageBuffer::from_pixel(image_size, image_size, Luma([255u8]));

    for y in 0..matrix.width() {
        for x in 0..matrix.width() {
            if matrix.is_dark(x, y) {
                let start_x = (x + QUIET_ZONE_MODULES) * module_size;
                let start_y = (y + QUIET_ZONE_MODULES) * module_size;
                for pixel_y in start_y..start_y + module_size {
                    for pixel_x in start_x..start_x + module_size {
                        image.put_pixel(pixel_x as u32, pixel_y as u32, Luma([0u8]));
                    }
                }
            }
        }
    }

    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}

#[cfg(test)]
#[path = "qr_code_tests.rs"]
mod tests;
