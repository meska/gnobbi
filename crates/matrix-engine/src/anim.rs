//! Animated GIF, WebP and APNG: every frame as straight RGBA with its delay, inside hard limits.
//! Over a limit (or not animated) there is no animation and the caller keeps the first frame it already has.

/// Most frames kept; more and the picture stays still.
pub const MAX_FRAMES: usize = 120;
/// Longest side of a frame, in pixels.
pub const MAX_SIDE: u32 = 512;
/// All decoded frames together (they live in memory as RGBA).
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Shortest delay played: browsers do the same with the 0-10 ms files that are common in the wild.
pub const MIN_DELAY_MS: u32 = 20;

pub struct Frames {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<Vec<u8>>,
    pub delays_ms: Vec<u32>,
}

impl Frames {
    pub fn bytes(&self) -> usize {
        self.rgba.iter().map(Vec::len).sum()
    }
}

/// The frame shown `elapsed_ms` after the start of the loop, and the milliseconds left before the next one.
pub fn frame_at(delays_ms: &[u32], elapsed_ms: u64) -> (usize, u32) {
    let total: u64 = delays_ms.iter().map(|d| u64::from(*d)).sum();
    if total == 0 {
        return (0, u32::MAX);
    }
    let mut left = elapsed_ms % total;
    for (i, d) in delays_ms.iter().enumerate() {
        if left < u64::from(*d) {
            return (i, (u64::from(*d) - left) as u32);
        }
        left -= u64::from(*d);
    }
    (0, delays_ms[0])
}

/// Why a file gives no animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoAnim {
    NotAnimated,
    /// Over the frame, side or memory limit.
    OverLimit,
    Broken,
}

/// All frames of an animated GIF, WebP or APNG; `None` when it is not animated, is over a limit or is broken.
#[cfg(feature = "sdk")]
pub fn decode(data: &[u8]) -> Option<Frames> {
    decode_up_to(data, MAX_SIDE).ok()
}

/// Like [`decode`] but accepts frames up to `max_side` (frame count and memory limits stay), and tells why it gave up.
#[cfg(feature = "sdk")]
pub fn decode_up_to(data: &[u8], max_side: u32) -> Result<Frames, NoAnim> {
    use image::{AnimationDecoder, ImageDecoder, ImageFormat, codecs};
    let cursor = std::io::Cursor::new(data);
    let broken = |_| NoAnim::Broken;
    let (dims, frames) = match image::guess_format(data).map_err(broken)? {
        ImageFormat::Gif => {
            let d = codecs::gif::GifDecoder::new(cursor).map_err(broken)?;
            (d.dimensions(), d.into_frames())
        }
        ImageFormat::WebP => {
            let d = codecs::webp::WebPDecoder::new(cursor).map_err(broken)?;
            if !d.has_animation() {
                return Err(NoAnim::NotAnimated);
            }
            (d.dimensions(), d.into_frames())
        }
        ImageFormat::Png => {
            let d = codecs::png::PngDecoder::new(cursor).map_err(broken)?;
            if !d.is_apng().map_err(broken)? {
                return Err(NoAnim::NotAnimated);
            }
            (d.dimensions(), d.apng().map_err(broken)?.into_frames())
        }
        _ => return Err(NoAnim::NotAnimated),
    };
    let (width, height) = dims;
    if width == 0 || height == 0 {
        return Err(NoAnim::Broken);
    }
    // One frame over the memory limit is refused before any frame is decoded.
    if width > max_side || height > max_side || width as usize * height as usize * 4 > MAX_BYTES {
        return Err(NoAnim::OverLimit);
    }
    let (mut rgba, mut delays_ms, mut bytes) = (Vec::new(), Vec::new(), 0usize);
    for frame in frames {
        let frame = frame.map_err(broken)?;
        let (n, d) = frame.delay().numer_denom_ms();
        let buffer = frame.into_buffer();
        bytes += buffer.as_raw().len();
        // Stops while decoding: a hostile file never gets to fill memory.
        if rgba.len() >= MAX_FRAMES || bytes > MAX_BYTES {
            return Err(NoAnim::OverLimit);
        }
        if buffer.dimensions() != (width, height) {
            return Err(NoAnim::Broken);
        }
        delays_ms.push((n / d.max(1)).max(MIN_DELAY_MS));
        rgba.push(buffer.into_raw());
    }
    if rgba.len() > 1 { Ok(Frames { width, height, rgba, delays_ms }) } else { Err(NoAnim::NotAnimated) }
}

#[cfg(all(test, feature = "sdk"))]
mod tests {
    use super::*;

    fn gif(delays: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            for (i, ms) in delays.iter().enumerate() {
                let colour = image::Rgba([(i * 80) as u8, 255 - (i * 80) as u8, 0, 255]);
                let frame = image::Frame::from_parts(image::RgbaImage::from_pixel(4, 3, colour), 0, 0, image::Delay::from_numer_denom_ms(*ms, 1));
                encoder.encode_frame(frame).unwrap();
            }
        }
        out
    }

    #[test]
    fn three_frames_keep_their_delays() {
        let f = decode(&gif(&[30, 50, 100])).unwrap();
        assert_eq!((f.width, f.height, f.rgba.len()), (4, 3, 3));
        assert_eq!(f.delays_ms, [30, 50, 100]);
        assert_ne!(f.rgba[0], f.rgba[1]);
    }

    #[test]
    fn too_many_frames_means_no_animation() {
        assert!(decode(&gif(&vec![40; MAX_FRAMES])).is_some());
        assert!(decode(&gif(&vec![40; MAX_FRAMES + 1])).is_none());
        assert!(decode(&gif(&[40])).is_none());
    }

    #[test]
    fn current_frame_follows_the_clock() {
        let d = [30, 50, 100];
        assert_eq!(frame_at(&d, 0), (0, 30));
        assert_eq!(frame_at(&d, 29), (0, 1));
        assert_eq!(frame_at(&d, 30), (1, 50));
        assert_eq!(frame_at(&d, 80), (2, 100));
        assert_eq!(frame_at(&d, 180 + 40), (1, 40));
        assert_eq!(frame_at(&[], 5).0, 0);
    }
}
