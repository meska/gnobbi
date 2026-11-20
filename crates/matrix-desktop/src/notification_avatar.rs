//! The picture shown on an OS alert instead of the app icon: waited for briefly, shrunk, never logged.
use matrix_engine::avatar::AvatarPicture;
use std::{sync::mpsc::Receiver, time::Duration};

/// An alert never waits longer than this for a picture; without it, the alert goes out as before.
pub const WAIT: Duration = Duration::from_millis(1500);
const SIZE: u32 = 128;

/// Straight RGBA, at most `SIZE` on the longest side.
pub struct Avatar {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn shrink((width, height, rgba): (u32, u32, Vec<u8>)) -> Option<Avatar> {
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    if width == 0 || height == 0 {
        return None;
    }
    if width.max(height) <= SIZE {
        return Some(Avatar { width, height, rgba: image.into_raw() });
    }
    let (w, h) = ((width * SIZE / width.max(height)).max(1), (height * SIZE / width.max(height)).max(1));
    let small = image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle);
    Some(Avatar { width: w, height: h, rgba: small.into_raw() })
}

/// The answer of the avatar fetch, or `None` when it is missing, failed or late.
pub fn wait(rx: &Receiver<AvatarPicture>, timeout: Duration) -> Option<Avatar> {
    shrink(rx.recv_timeout(timeout).ok()?.rgba?)
}

#[cfg(any(target_os = "macos", target_os = "ios", test))]
mod file {
    use super::Avatar;
    use std::{fs, path::{Path, PathBuf}, time::Duration};

    pub fn png(avatar: &Avatar) -> Option<Vec<u8>> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(&mut out, &avatar.rgba, avatar.width, avatar.height, image::ExtendedColorType::Rgba8, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    }

    /// Private cache folder of the profile, never the global temp dir.
    pub fn dir() -> Option<PathBuf> {
        Some(crate::app_dirs(crate::Platform::CURRENT, |k| std::env::var(k).ok())?.cache.join("notification-avatars"))
    }

    /// Unpredictable name (random keys per process + clock), created new and owner-only.
    pub fn name() -> String {
        use std::hash::{BuildHasher, Hasher};
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(nanos);
        format!("{:016x}{:016x}.png", h.finish(), std::collections::hash_map::RandomState::new().build_hasher().finish())
    }

    pub fn write(dir: &Path, png: &[u8]) -> Option<PathBuf> {
        use std::io::Write;
        fs::create_dir_all(dir).ok()?;
        let path = dir.join(name());
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(&path).ok()?.write_all(png).ok()?;
        Some(path)
    }

    /// Removes leftovers (a crash between write and delivery) older than `max_age`.
    pub fn sweep(dir: &Path, max_age: Duration) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > max_age);
            if old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}
#[cfg(any(target_os = "macos", target_os = "ios", test))]
pub use file::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn picture(w: u32, h: u32) -> AvatarPicture {
        AvatarPicture { mxc: String::new(), rgba: Some((w, h, vec![200; (w * h * 4) as usize])), retry: false }
    }

    #[test]
    fn shrinks_the_long_side_to_128_and_keeps_small_pictures() {
        let big = shrink((512, 256, vec![9; 512 * 256 * 4])).unwrap();
        assert_eq!((big.width, big.height, big.rgba.len()), (128, 64, 128 * 64 * 4));
        let small = shrink((32, 32, vec![9; 32 * 32 * 4])).unwrap();
        assert_eq!((small.width, small.height), (32, 32));
        assert!(shrink((4, 4, vec![0; 3])).is_none(), "buffer shorter than the size is refused");
    }

    #[test]
    fn late_missing_or_failed_picture_means_no_avatar() {
        let (tx, rx) = mpsc::channel();
        assert!(wait(&rx, Duration::from_millis(20)).is_none(), "timeout");
        tx.send(AvatarPicture { rgba: None, retry: true, ..picture(1, 1) }).unwrap();
        assert!(wait(&rx, Duration::from_millis(20)).is_none(), "failed fetch");
        tx.send(picture(8, 8)).unwrap();
        assert_eq!(wait(&rx, Duration::from_millis(20)).unwrap().width, 8);
    }

    #[test]
    fn temp_file_is_private_unpredictable_png_and_old_ones_are_swept() {
        let dir = std::env::temp_dir().join(format!("gnobbi-avatar-test-{}", name()));
        let png = png(&shrink((8, 8, vec![255; 8 * 8 * 4])).unwrap()).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let (a, b) = (write(&dir, &png).unwrap(), write(&dir, &png).unwrap());
        assert_ne!(a, b);
        assert!(a.starts_with(&dir) && a.extension().unwrap() == "png");
        sweep(&dir, Duration::from_secs(3600));
        assert!(a.exists(), "fresh file stays");
        std::thread::sleep(Duration::from_millis(30));
        sweep(&dir, Duration::from_millis(10));
        assert!(!a.exists() && !b.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
