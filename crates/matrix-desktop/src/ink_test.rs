//! Pixel measurement of what a painted test really put on screen (grim screenshot of the private Wayland output).
//! The window is tiled full-output at (0,0), so a logical bound x maps to physical pixel x * scale.
//! Run: the gui-t0923 script (CT, sway headless). Evidence PNGs go to $SHOT_DIR (default /tmp/gui-out/shots).
use gpui::{Bounds, Pixels};
use image::RgbaImage;

/// Screenshot of the output, cropped to the window (origin 0,0) and kept as evidence.
/// `grim -s <gpui scale>`: GPUI rounds a fractional output scale (1.5) up to 2, so the capture is asked at the scale GPUI paints in.
pub(crate) fn grim(tag: &str, window: &gpui::Window) -> RgbaImage {
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp/gui-out/shots".into());
    std::fs::create_dir_all(&dir).unwrap();
    let path = format!("{dir}/{tag}.png");
    assert!(std::process::Command::new("grim").args(["-s", &window.scale_factor().to_string()]).arg(&path).status().expect("grim").success(), "grim failed");
    let full = image::open(&path).unwrap().into_rgba8();
    let (s, v) = (window.scale_factor(), window.viewport_size());
    let (w, h) = (((f32::from(v.width) * s).round() as u32).min(full.width()), ((f32::from(v.height) * s).round() as u32).min(full.height()));
    let img = image::imageops::crop_imm(&full, 0, 0, w, h).to_image();
    img.save(&path).unwrap();
    img
}

/// One measurement line, kept next to the screenshots (libtest hides stderr of passing tests).
pub(crate) fn note(line: &str) {
    use std::io::Write;
    let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp/gui-out/shots".into());
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(format!("{dir}/measurements.txt")).unwrap();
    writeln!(f, "{line}").unwrap();
    eprintln!("{line}");
}

pub(crate) fn rgb(c: u32) -> [f32; 3] {
    [(c >> 16 & 255) as f32, (c >> 8 & 255) as f32, (c & 255) as f32]
}

pub(crate) fn dist(img: &RgbaImage, x: u32, y: u32, to: [f32; 3]) -> f32 {
    let p = img.get_pixel(x.min(img.width() - 1), y.min(img.height() - 1)).0;
    (0..3).map(|i| (p[i] as f32 - to[i]).abs()).fold(0., f32::max)
}

/// Where the glyph's ink sits inside a hit area, in logical px of the window.
pub(crate) struct Ink {
    /// Centre of the box around the ink (what «centred» means for every glyph but Play).
    pub bbox: (f32, f32),
    /// Coverage-weighted centre of mass (the centroid, what Play is centred by).
    pub mass: (f32, f32),
}

/// Ink = pixels unlike the background sampled just inside the top edge (`round`: inside the disc of the hit area,
/// whose corners show the bubble behind it).
pub(crate) fn ink(img: &RgbaImage, b: Bounds<Pixels>, scale: f32, round: bool) -> Ink {
    let (l, t, r, bt) = (f32::from(b.left()) * scale, f32::from(b.top()) * scale, f32::from(b.right()) * scale, f32::from(b.bottom()) * scale);
    let (cx, cy) = ((l + r) / 2., (t + bt) / 2.);
    let radius = (r - l).min(bt - t) / 2. - 2. * scale;
    let bg = img.get_pixel(cx as u32, (t + 2. * scale) as u32).0;
    let bg = [bg[0] as f32, bg[1] as f32, bg[2] as f32];
    let mut cells = vec![];
    for y in (t.ceil() as u32)..(bt.floor() as u32) {
        for x in (l.ceil() as u32)..(r.floor() as u32) {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if round && dx.hypot(dy) > radius {
                continue;
            }
            cells.push((x as f32 + 0.5, y as f32 + 0.5, dist(img, x, y, bg)));
        }
    }
    let max = cells.iter().map(|c| c.2).fold(0., f32::max);
    assert!(max > 40., "no glyph found in {b:?} (max contrast {max}): wrong place or colour");
    let on: Vec<_> = cells.iter().filter(|c| c.2 > max * 0.35).collect();
    let (x0, x1) = (on.iter().map(|c| c.0 - 0.5).fold(f32::MAX, f32::min), on.iter().map(|c| c.0 + 0.5).fold(f32::MIN, f32::max));
    let (y0, y1) = (on.iter().map(|c| c.1 - 0.5).fold(f32::MAX, f32::min), on.iter().map(|c| c.1 + 0.5).fold(f32::MIN, f32::max));
    let w: f32 = cells.iter().map(|c| c.2).sum();
    Ink {
        bbox: ((x0 + x1) / 2. / scale, (y0 + y1) / 2. / scale),
        mass: (cells.iter().map(|c| c.0 * c.2).sum::<f32>() / w / scale, cells.iter().map(|c| c.1 * c.2).sum::<f32>() / w / scale),
    }
}

/// ±1 logical px (the task's tolerance).
pub(crate) fn assert_centred(what: &str, got: (f32, f32), b: Bounds<Pixels>, scale: f32) -> Result<(), String> {
    let want = (f32::from(b.left() + b.right()) / 2., f32::from(b.top() + b.bottom()) / 2.);
    let tol = 1.0 + 1e-3;
    note(&format!("CENTRED {what}: ink ({:.2}, {:.2}) hit area ({:.2}, {:.2}) off ({:+.2}, {:+.2}) px, scale {scale}", got.0, got.1, want.0, want.1, got.0 - want.0, got.1 - want.1));
    if (got.0 - want.0).abs() <= tol && (got.1 - want.1).abs() <= tol { Ok(()) } else { Err(format!("{what}: glyph centre {got:?} vs hit area centre {want:?} (tolerance {tol})")) }
}
