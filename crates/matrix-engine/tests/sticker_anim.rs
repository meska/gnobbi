//! Animated stickers: `prepare` on synthetic GIF / WebP / APNG fixtures (no binary files in the repo).

use image::{Delay, Frame, ImageEncoder, Rgba, RgbaImage, codecs::gif::GifEncoder};
use matrix_engine::anim::{self, MAX_FRAMES, MAX_SIDE, MIN_DELAY_MS};
use matrix_engine::sticker_anim::{KEEP_MAX_BYTES, StickerPrep, prepare};
use matrix_engine::stickers::MAX_BYTES;

fn colour(i: usize) -> Rgba<u8> {
    Rgba([(i * 80 % 256) as u8, 255 - (i * 80 % 256) as u8, (i * 37 % 256) as u8, 255])
}

fn gif(w: u32, h: u32, delays: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = GifEncoder::new(&mut out);
        for (i, ms) in delays.iter().enumerate() {
            e.encode_frame(Frame::from_parts(RgbaImage::from_pixel(w, h, colour(i)), 0, 0, Delay::from_numer_denom_ms(*ms, 1))).unwrap();
        }
    }
    out
}

fn png(w: u32, h: u32, c: Rgba<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(RgbaImage::from_pixel(w, h, c).as_raw(), w, h, image::ExtendedColorType::Rgba8).unwrap();
    out
}

fn kept(p: StickerPrep) -> matrix_engine::sticker_anim::AnimatedSticker {
    match p {
        StickerPrep::Animated(a) => a,
        other => panic!("expected Animated, got {other:?}"),
    }
}

// ---- 1. within limits: untouched ----
#[test]
fn small_three_frame_gif_is_identical_with_right_mime_and_size() {
    let g = gif(64, 48, &[30, 50, 100]);
    let a = kept(prepare(&g));
    assert_eq!(a.bytes, g);
    assert_eq!((a.mimetype, a.width, a.height, a.reencoded), ("image/gif", 64, 48, false));
}

#[test]
fn gif_exactly_at_the_side_limit_is_kept() {
    let g = gif(MAX_SIDE, 8, &[30, 30]);
    let a = kept(prepare(&g));
    assert!(!a.reencoded && a.bytes == g && a.width == MAX_SIDE);
}

// ---- 2. too big: shrunk, same frames and delays ----
#[test]
fn gif_wider_than_512_is_shrunk_keeping_frames_and_delays() {
    let delays = [30, 50, 100, 70];
    let a = kept(prepare(&gif(1000, 600, &delays)));
    assert_eq!((a.mimetype, a.reencoded), ("image/gif", true));
    assert!(a.width.max(a.height) <= MAX_SIDE);
    let back = anim::decode(&a.bytes).unwrap();
    assert_eq!(back.rgba.len(), delays.len());
    assert_eq!(back.delays_ms, delays);
    assert_eq!((back.width, back.height), (a.width, a.height));
}

#[test]
fn gif_over_the_byte_limit_but_within_side_is_reencoded() {
    // Noisy frames defeat LZW: 400x400 x 12 frames passes 1 MiB.
    let mut out = Vec::new();
    {
        let mut e = GifEncoder::new(&mut out);
        for i in 0..12u32 {
            let img = RgbaImage::from_fn(400, 400, |x, y| {
                let v = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503)).wrapping_add(i * 977);
                Rgba([(v >> 3) as u8, (v >> 11) as u8, (v >> 19) as u8, 255])
            });
            e.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(40, 1))).unwrap();
        }
    }
    assert!(out.len() > KEEP_MAX_BYTES, "fixture too small: {}", out.len());
    let a = kept(prepare(&out));
    assert!(a.reencoded && a.mimetype == "image/gif");
    // Never enlarged, never over the sticker byte limit.
    assert!(a.width <= 400 && a.height <= 400, "{}x{}", a.width, a.height);
    assert!(a.bytes.len() <= MAX_BYTES, "{} bytes", a.bytes.len());
    assert_eq!(anim::decode(&a.bytes).unwrap().delays_ms, [40; 12]);
}

fn noisy(w: u32, h: u32, n: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = GifEncoder::new(&mut out);
        for i in 0..n {
            let img = RgbaImage::from_fn(w, h, |x, y| {
                let v = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503)).wrapping_add(i * 977);
                Rgba([(v >> 3) as u8, (v >> 11) as u8, (v >> 19) as u8, 255])
            });
            e.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(40, 1))).unwrap();
        }
    }
    out
}

#[test]
fn small_heavy_gif_is_never_enlarged_and_fits_the_byte_limit_or_stays_still() {
    // 200x200 x 40 noisy frames is over 1 MiB: it used to come out 512x512 and 11 MB.
    let g = noisy(200, 200, 40);
    assert!(g.len() > KEEP_MAX_BYTES, "fixture too small: {}", g.len());
    match prepare(&g) {
        StickerPrep::Animated(a) => assert!(a.width <= 200 && a.height <= 200 && a.bytes.len() <= MAX_BYTES, "{}x{} {}", a.width, a.height, a.bytes.len()),
        StickerPrep::Static { warning } => assert!(warning.is_some()),
    }
}

#[test]
fn heavy_gif_retries_smaller_sides_before_giving_up() {
    // 400x400 x 14 noisy frames is over 1 MiB at full size and fits only on a smaller side.
    let g = noisy(400, 400, 14);
    assert!(g.len() > KEEP_MAX_BYTES);
    let StickerPrep::Animated(a) = prepare(&g) else { panic!("expected a smaller animation") };
    assert!(a.bytes.len() <= MAX_BYTES && a.width.max(a.height) < 400, "{}x{} {}", a.width, a.height, a.bytes.len());
}

// ---- B2: the memory limit counts the shrunk frames ----
#[test]
fn big_gif_with_many_frames_stays_animated_after_shrinking() {
    // 800x600 x 30 = 55 MiB at full size (over the old 16 MiB), 23 MiB once shrunk to 512x384.
    let a = kept(prepare(&gif(800, 600, &[40; 30])));
    assert_eq!((a.reencoded, a.width, a.height), (true, 512, 384));
    assert_eq!(anim::decode(&a.bytes).unwrap().rgba.len(), 30);
}

#[test]
fn memory_limit_still_stops_a_hostile_frame_count_after_shrinking() {
    // One 2048x2048 frame is 16 MiB: five of them are over 64 MiB at full size but 1.3 MiB each once shrunk, so they stay.
    assert!(matches!(prepare(&gif(2048, 2048, &[40; 5])), StickerPrep::Animated(_)));
}

// ---- I1: a big still GIF is not an animation, so no warning ----
#[test]
fn big_static_gif_has_no_warning() {
    assert_eq!(prepare(&gif(2100, 2100, &[40])), StickerPrep::Static { warning: None });
    assert_eq!(prepare(&gif(3000, 200, &[40])), StickerPrep::Static { warning: None });
}

#[test]
fn delays_are_rounded_to_the_gif_tick_not_truncated() {
    // WebP keeps the exact milliseconds (a GIF source is already in 10 ms ticks).
    let a = kept(prepare(&webp_anim(1000, 600, &[25, 35, 105])));
    assert_eq!(anim::decode(&a.bytes).unwrap().delays_ms, [30, 40, 110]);
}

#[test]
fn big_gif_with_odd_aspect_keeps_a_nonzero_side() {
    let a = kept(prepare(&gif(2000, 4, &[30, 30])));
    assert!(a.width <= MAX_SIDE && a.height >= 1);
}

// ---- 3. static PNG ----
#[test]
fn static_png_and_static_gif_take_the_old_path() {
    assert_eq!(prepare(&png(600, 600, colour(1))), StickerPrep::Static { warning: None });
    assert_eq!(prepare(&gif(10, 10, &[40])), StickerPrep::Static { warning: None });
}

// ---- 4. frame limit ----
#[test]
fn over_the_frame_limit_stops_with_a_warning() {
    assert!(matches!(prepare(&gif(4, 4, &vec![40; MAX_FRAMES])), StickerPrep::Animated(_)));
    match prepare(&gif(4, 4, &vec![40; MAX_FRAMES + 1])) {
        StickerPrep::Static { warning: Some(w) } => assert!(!w.is_empty()),
        other => panic!("expected Static with warning, got {other:?}"),
    }
}

#[test]
fn frame_over_the_memory_limit_is_refused_with_a_warning() {
    // 2100 px is over the 4x512 decode side (OverLimit before any frame is decoded).
    match prepare(&gif(2100, 2100, &[30, 30])) {
        StickerPrep::Static { warning: Some(_) } => {}
        other => panic!("expected Static with warning, got {other:?}"),
    }
}

// ---- 5. edge cases ----
#[test]
fn zero_and_tiny_delays_are_raised_to_the_minimum_when_shrunk() {
    let a = kept(prepare(&gif(1000, 1000, &[0, 1, 10, 20])));
    let back = anim::decode(&a.bytes).unwrap();
    assert_eq!(back.delays_ms.len(), 4);
    assert!(back.delays_ms.iter().all(|d| *d >= MIN_DELAY_MS), "{:?}", back.delays_ms);
}

#[test]
fn zero_delay_small_gif_is_still_kept_as_is() {
    let g = gif(16, 16, &[0, 0, 0]);
    assert_eq!(kept(prepare(&g)).bytes, g);
}

#[test]
fn corrupt_and_truncated_files_never_panic() {
    let g = gif(32, 32, &[30, 50, 100]);
    for cut in [0, 1, 5, 6, 12, g.len() / 3, g.len() / 2, g.len() - 1] {
        let _ = prepare(&g[..cut]);
    }
    let mut flipped = g.clone();
    for i in (13..flipped.len()).step_by(7) {
        flipped[i] ^= 0xA5;
    }
    let _ = prepare(&flipped);
    let big = gif(1000, 1000, &[30, 30]);
    let _ = prepare(&big[..big.len() / 2]);
    for junk in [&b""[..], b"GIF89a", b"\x89PNG\r\n\x1a\n", b"RIFF\0\0\0\0WEBP", b"not an image at all"] {
        assert!(matches!(prepare(junk), StickerPrep::Static { .. }));
    }
}

#[test]
fn truncated_gif_is_not_uploaded_as_an_animation() {
    let g = gif(32, 32, &[30, 50, 100]);
    assert!(matches!(prepare(&g[..g.len() / 2]), StickerPrep::Static { .. }));
}

// ---- animated WebP: no animated encoder in `image`, so the RIFF container is assembled by hand around lossless frames ----
fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = tag.to_vec();
    v.extend((body.len() as u32).to_le_bytes());
    v.extend(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn webp_anim(w: u32, h: u32, delays: &[u32]) -> Vec<u8> {
    let le24 = |n: u32| n.to_le_bytes()[..3].to_vec();
    let mut vp8x = vec![0b0001_0010]; // animation + alpha
    vp8x.extend([0, 0, 0]);
    vp8x.extend(le24(w - 1));
    vp8x.extend(le24(h - 1));
    let mut body = b"WEBP".to_vec();
    body.extend(chunk(b"VP8X", &vp8x));
    body.extend(chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]));
    for (i, ms) in delays.iter().enumerate() {
        let mut lossless = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut lossless).write_image(RgbaImage::from_pixel(w, h, colour(i)).as_raw(), w, h, image::ExtendedColorType::Rgba8).unwrap();
        // The encoder writes RIFF....WEBPVP8L<len><data>: keep the VP8L chunk only.
        let vp8l = &lossless[12..];
        let mut anmf = [le24(0), le24(0), le24(w - 1), le24(h - 1), le24(*ms)].concat();
        anmf.push(0b0000_0010); // no blend, keep
        anmf.extend(vp8l);
        body.extend(chunk(b"ANMF", &anmf));
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

#[test]
fn animated_webp_is_kept_as_is_with_webp_mime() {
    let w = webp_anim(24, 16, &[40, 60, 80]);
    let a = kept(prepare(&w));
    assert_eq!((a.mimetype, a.width, a.height, a.reencoded), ("image/webp", 24, 16, false));
    assert_eq!(a.bytes, w);
    assert_eq!(anim::decode(&w).unwrap().delays_ms, [40, 60, 80]);
}

#[test]
fn big_animated_webp_becomes_a_shrunk_gif() {
    let a = kept(prepare(&webp_anim(800, 600, &[40, 60])));
    assert_eq!((a.mimetype, a.reencoded), ("image/gif", true));
    assert!(a.width.max(a.height) <= MAX_SIDE);
    assert_eq!(anim::decode(&a.bytes).unwrap().delays_ms, [40, 60]);
}

#[test]
fn truncated_animated_webp_never_panics() {
    let w = webp_anim(24, 16, &[40, 60, 80]);
    for cut in [11, 20, 30, w.len() / 2, w.len() - 1] {
        let _ = prepare(&w[..cut]);
    }
}

// ---- APNG: acTL/fcTL/fdAT chunks injected by hand into PNGs made by the encoder ----
fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in data {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

fn png_chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u32).to_be_bytes().to_vec();
    let mut td = tag.to_vec();
    td.extend(body);
    v.extend(&td);
    v.extend(crc32(&td).to_be_bytes());
    v
}

/// IDAT payload and IHDR chunk of an encoder-made PNG.
fn idat_and_ihdr(png: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut i, mut idat, mut ihdr) = (8, Vec::new(), Vec::new());
    while i + 8 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        let tag = &png[i + 4..i + 8];
        let body = &png[i + 8..i + 8 + len];
        if tag == b"IDAT" {
            idat.extend(body);
        } else if tag == b"IHDR" {
            ihdr = body.to_vec();
        }
        i += 12 + len;
    }
    (idat, ihdr)
}

fn apng(w: u32, h: u32, delays_ms: &[u16]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut seq = 0u32;
    for (i, ms) in delays_ms.iter().enumerate() {
        let (idat, ihdr) = idat_and_ihdr(&png(w, h, colour(i)));
        if i == 0 {
            out.extend(png_chunk(b"IHDR", &ihdr));
            let mut actl = (delays_ms.len() as u32).to_be_bytes().to_vec();
            actl.extend(0u32.to_be_bytes());
            out.extend(png_chunk(b"acTL", &actl));
        }
        let mut fctl = seq.to_be_bytes().to_vec();
        seq += 1;
        for n in [w, h, 0, 0] {
            fctl.extend(n.to_be_bytes());
        }
        fctl.extend(ms.to_be_bytes());
        fctl.extend(1000u16.to_be_bytes());
        fctl.extend([0, 0]); // dispose none, blend source
        out.extend(png_chunk(b"fcTL", &fctl));
        if i == 0 {
            out.extend(png_chunk(b"IDAT", &idat));
        } else {
            let mut fdat = seq.to_be_bytes().to_vec();
            seq += 1;
            fdat.extend(idat);
            out.extend(png_chunk(b"fdAT", &fdat));
        }
    }
    out.extend(png_chunk(b"IEND", &[]));
    out
}

#[test]
fn apng_is_kept_as_is_with_png_mime() {
    let p = apng(20, 10, &[40, 60, 80]);
    let a = kept(prepare(&p));
    assert_eq!((a.mimetype, a.width, a.height, a.reencoded), ("image/png", 20, 10, false));
    assert_eq!(a.bytes, p);
    assert_eq!(anim::decode(&p).unwrap().delays_ms, [40, 60, 80]);
}

#[test]
fn big_apng_becomes_a_shrunk_gif() {
    let a = kept(prepare(&apng(900, 900, &[40, 60])));
    assert_eq!((a.mimetype, a.reencoded, a.width, a.height), ("image/gif", true, 512, 512));
    assert_eq!(anim::decode(&a.bytes).unwrap().delays_ms, [40, 60]);
}

#[test]
fn truncated_apng_never_panics() {
    let p = apng(20, 10, &[40, 60, 80]);
    for cut in [9, 20, 40, p.len() / 2, p.len() - 1] {
        let _ = prepare(&p[..cut]);
    }
}
