//! Bounded, metadata-free PNG images for the message composer.
use eframe::egui;

pub const MAX_IMAGE_PIXELS: usize = 16_000_000;
pub const MAX_IMAGE_BYTES: usize = 25 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    Invalid,
    TooLarge,
}

/// Re-encode only pixels: no clipboard metadata or hidden transparent RGB is retained.
/// The server limit is applied while encoding, before buffering an oversized output.
pub fn prepare(picture: &egui::ColorImage, upload_limit: u64) -> Result<Vec<u8>, ImageError> {
    use image::ImageEncoder as _;
    let [width, height] = picture.size;
    let pixels = pixel_count(width, height)?;
    if picture.pixels.len() != pixels {
        return Err(ImageError::Invalid);
    }
    let limit = usize::try_from(upload_limit).unwrap_or(usize::MAX).min(MAX_IMAGE_BYTES);
    if limit == 0 {
        return Err(ImageError::TooLarge);
    }
    let mut rgba = Vec::with_capacity(pixels * 4);
    for pixel in &picture.pixels {
        let mut color = pixel.to_srgba_unmultiplied();
        if color[3] == 0 {
            color = [0; 4];
        }
        rgba.extend_from_slice(&color);
    }
    let mut output = BoundedPng { data: Vec::new(), limit, exceeded: false };
    let result = image::codecs::png::PngEncoder::new(&mut output).write_image(
        &rgba, width as u32, height as u32, image::ExtendedColorType::Rgba8,
    );
    if output.exceeded {
        return Err(ImageError::TooLarge);
    }
    match result {
        Ok(()) => Ok(output.data),
        Err(_) => Err(ImageError::Invalid),
    }
}

fn pixel_count(width: usize, height: usize) -> Result<usize, ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Invalid);
    }
    width.checked_mul(height).filter(|&n| n <= MAX_IMAGE_PIXELS).ok_or(ImageError::TooLarge)
}

struct BoundedPng {
    data: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl std::io::Write for BoundedPng {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.data.len()) {
            self.exceeded = true;
            return Err(std::io::Error::other("composer image exceeds upload limit"));
        }
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn decoded(data: &[u8]) -> Result<image::DynamicImage, ImageError> {
    if data.len() > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge);
    }
    let reader = || {
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_PIXELS as u32);
        limits.max_image_height = Some(MAX_IMAGE_PIXELS as u32);
        limits.max_alloc = Some(128 << 20);
        reader.limits(limits);
        reader
    };
    // Read dimensions before allowing any full image allocation.
    let (width, height) = reader().into_dimensions().map_err(|_| ImageError::Invalid)?;
    pixel_count(width as usize, height as usize)?;
    reader().decode().map_err(|_| ImageError::Invalid)
}

/// Decode the canonical PNG supplied by the platform image bridge with byte,
/// dimension and decoder allocation limits. No file or network access.
pub fn decode(data: &[u8]) -> Result<egui::ColorImage, ImageError> {
    let image = decoded(data)?.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize], image.as_raw(),
    ))
}

/// An in-memory thumbnail; malformed or unsupported formats have no preview.
pub fn thumbnail(data: &[u8]) -> Option<egui::ColorImage> {
    let image = decoded(data).ok()?;
    let image = if image.width() > 96 || image.height() > 96 { image.thumbnail(96, 96) } else { image };
    let image = image.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize], image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> egui::ColorImage {
        egui::ColorImage::new([2, 1], vec![egui::Color32::RED, egui::Color32::from_rgba_premultiplied(37, 88, 123, 0)])
    }

    #[test]
    fn sanitized_png_round_trip_and_thumbnail() {
        let png = prepare(&sample(), MAX_IMAGE_BYTES as u64).unwrap();
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgba8();
        assert_eq!(decoded.as_raw(), &[255, 0, 0, 255, 0, 0, 0, 0]);
        assert_eq!(thumbnail(&png).unwrap().size, [2, 1]);
        assert_eq!(decode(&png).unwrap().size, [2, 1]);
    }

    #[test]
    fn invalid_shape_and_pixel_budget_are_rejected_before_allocation() {
        let mut picture = sample();
        picture.size = [0, 1];
        assert_eq!(prepare(&picture, u64::MAX), Err(ImageError::Invalid));
        picture.size = [3, 1];
        assert_eq!(prepare(&picture, u64::MAX), Err(ImageError::Invalid));
        picture.size = [usize::MAX, 2];
        assert_eq!(prepare(&picture, u64::MAX), Err(ImageError::TooLarge));
        picture.size = [MAX_IMAGE_PIXELS + 1, 1];
        assert_eq!(prepare(&picture, u64::MAX), Err(ImageError::TooLarge));
    }

    #[test]
    fn encoded_upload_limit_is_enforced() {
        assert_eq!(prepare(&sample(), 1), Err(ImageError::TooLarge));
        assert_eq!(prepare(&sample(), 0), Err(ImageError::TooLarge));
        let png = prepare(&sample(), MAX_IMAGE_BYTES as u64).unwrap();
        assert_eq!(prepare(&sample(), png.len() as u64).unwrap(), png);
        assert_eq!(prepare(&sample(), png.len() as u64 - 1), Err(ImageError::TooLarge));
    }

    #[test]
    fn decoder_rejects_oversized_header_before_reading_pixels() {
        let mut png = prepare(&sample(), u64::MAX).unwrap();
        // Change a valid IHDR (including its CRC), without creating a huge image.
        png[16..20].copy_from_slice(&5000u32.to_be_bytes());
        png[20..24].copy_from_slice(&5000u32.to_be_bytes());
        let mut crc = !0u32;
        for &byte in &png[12..29] {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        png[29..33].copy_from_slice(&(!crc).to_be_bytes());
        assert!(matches!(decode(&png), Err(ImageError::TooLarge)));
        assert!(thumbnail(&png).is_none());
    }

    #[test]
    fn bounded_writer_keeps_previous_data_on_limit_failure() {
        use std::io::Write as _;
        let mut writer = BoundedPng { data: Vec::new(), limit: 3, exceeded: false };
        writer.write_all(&[1, 2]).unwrap();
        assert!(writer.write_all(&[3, 4]).is_err());
        assert_eq!(writer.data, [1, 2]);
        assert!(writer.exceeded);
    }

    #[test]
    fn thumbnail_scales_and_rejects_invalid_data() {
        let picture = egui::ColorImage::filled([192, 48], egui::Color32::BLUE);
        let png = prepare(&picture, u64::MAX).unwrap();
        assert_eq!(thumbnail(&png).unwrap().size, [96, 24]);
        assert!(thumbnail(b"not an image").is_none());
        assert!(thumbnail(&[]).is_none());
        assert!(thumbnail(&png[..png.len() / 2]).is_none());
    }
}
