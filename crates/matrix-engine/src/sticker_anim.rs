//! New animated sticker: decides what to upload for a file the user picked or pasted.
//! Static files return [`StickerPrep::Static`] and keep the editor/PNG path untouched.

use crate::anim::{self, NoAnim};

/// Animated files within these limits are uploaded as they are. 512 px is the sticker side the editor
/// already uses; 1 MiB keeps the upload and the decoding on phones cheap (most stickers are far smaller).
pub const KEEP_MAX_SIDE: u32 = anim::MAX_SIDE;
pub const KEEP_MAX_BYTES: usize = 1024 * 1024;
/// Crop and «fill the square» are NOT applied to animations (only fit-in-square): the UI hides them
/// when the picked file is animated.
pub const ANIMATED_CROP_AVAILABLE: bool = false;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimatedSticker {
    pub bytes: Vec<u8>,
    /// `image/gif`, `image/webp` or `image/png` (APNG; older clients still show its first frame).
    pub mimetype: &'static str,
    pub width: u32,
    pub height: u32,
    /// `true` when the frames were scaled down and re-encoded as GIF; `false` for the original bytes.
    pub reencoded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StickerPrep {
    /// Not animated (or animation unusable): the UI keeps the editor → PNG path.
    /// `warning` is a one-line English reason (a catalog key for the UI) when an animation was lost.
    Static { warning: Option<&'static str> },
    /// Upload this payload as is; skip the editor.
    Animated(AnimatedSticker),
}

pub fn prepare(data: &[u8]) -> StickerPrep {
    // Decodes up to 4x the sticker side, so a big GIF is shrunk instead of losing its animation.
    let frames = match anim::decode_up_to(data, anim::MAX_SIDE * 4) {
        Ok(f) => f,
        Err(NoAnim::NotAnimated | NoAnim::Broken) => return StickerPrep::Static { warning: None },
        Err(NoAnim::OverLimit) => return still("Animation too large: saved as a still image"),
    };
    let (width, height) = (frames.width, frames.height);
    if width.max(height) <= KEEP_MAX_SIDE && data.len() <= KEEP_MAX_BYTES {
        let mimetype = match image::guess_format(data) {
            Ok(image::ImageFormat::Gif) => "image/gif",
            Ok(image::ImageFormat::WebP) => "image/webp",
            _ => "image/png",
        };
        return StickerPrep::Animated(AnimatedSticker { bytes: data.to_vec(), mimetype, width, height, reencoded: false });
    }
    match reencode(&frames) {
        Some(s) => StickerPrep::Animated(s),
        None => still("Animation cannot be resized: saved as a still image"),
    }
}

fn still(msg: &'static str) -> StickerPrep {
    StickerPrep::Static { warning: Some(msg) }
}

/// Every frame scaled to fit the square (aspect kept), re-encoded as an endless GIF with the same delays.
fn reencode(f: &anim::Frames) -> Option<AnimatedSticker> {
    use image::{Delay, Frame, RgbaImage, codecs::gif, imageops};
    let scale = f64::from(KEEP_MAX_SIDE) / f64::from(f.width.max(f.height)).max(1.0);
    let (w, h) = (((f64::from(f.width) * scale).round() as u32).clamp(1, KEEP_MAX_SIDE), ((f64::from(f.height) * scale).round() as u32).clamp(1, KEEP_MAX_SIDE));
    let mut bytes = Vec::new();
    {
        // Speed 10 = fastest palette search: fine for stickers, keeps a 120-frame file quick.
        let mut enc = gif::GifEncoder::new_with_speed(&mut bytes, 10);
        enc.set_repeat(gif::Repeat::Infinite).ok()?;
        for (px, ms) in f.rgba.iter().zip(&f.delays_ms) {
            let src = RgbaImage::from_raw(f.width, f.height, px.clone())?;
            let small = imageops::resize(&src, w, h, imageops::FilterType::Lanczos3);
            enc.encode_frame(Frame::from_parts(small, 0, 0, Delay::from_numer_denom_ms(*ms, 1))).ok()?;
        }
    }
    Some(AnimatedSticker { bytes, mimetype: "image/gif", width: w, height: h, reencoded: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Delay, Frame, Rgba, RgbaImage, codecs::gif::GifEncoder};

    fn gif(w: u32, h: u32, delays: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = GifEncoder::new(&mut out);
            for (i, ms) in delays.iter().enumerate() {
                let c = Rgba([(i * 80) as u8, 255 - (i * 80) as u8, 0, 255]);
                e.encode_frame(Frame::from_parts(RgbaImage::from_pixel(w, h, c), 0, 0, Delay::from_numer_denom_ms(*ms, 1))).unwrap();
            }
        }
        out
    }

    #[test]
    fn small_gif_is_kept_byte_for_byte() {
        let g = gif(40, 30, &[30, 50]);
        let StickerPrep::Animated(a) = prepare(&g) else { panic!() };
        assert_eq!((a.bytes, a.mimetype, a.width, a.height, a.reencoded), (g, "image/gif", 40, 30, false));
    }

    #[test]
    fn big_gif_is_shrunk_keeping_frames_delays_and_aspect() {
        let StickerPrep::Animated(a) = prepare(&gif(1000, 500, &[30, 50, 100])) else { panic!() };
        assert_eq!((a.mimetype, a.width, a.height, a.reencoded), ("image/gif", 512, 256, true));
        let back = anim::decode(&a.bytes).unwrap();
        assert_eq!((back.width, back.height), (512, 256));
        assert_eq!(back.delays_ms, [30, 50, 100]);
    }

    #[test]
    fn single_frame_and_png_are_static_without_warning() {
        assert_eq!(prepare(&gif(8, 8, &[40])), StickerPrep::Static { warning: None });
        let mut png = std::io::Cursor::new(Vec::new());
        RgbaImage::new(4, 4).write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert_eq!(prepare(png.get_ref()), StickerPrep::Static { warning: None });
    }

    #[test]
    fn too_many_frames_falls_back_to_still_with_a_warning() {
        let StickerPrep::Static { warning } = prepare(&gif(4, 4, &vec![40; anim::MAX_FRAMES + 1])) else { panic!() };
        assert!(warning.is_some());
    }
}
