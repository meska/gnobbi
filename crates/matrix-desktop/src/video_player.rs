//! Opens a downloaded video in the system's own player; nothing is decoded here.
use std::path::Path;

/// `GNOBBI_VIDEO_OPENER` replaces the platform opener (used by the screenshot proof, where
/// the virtual display has no player).
fn opener() -> String {
    std::env::var("GNOBBI_VIDEO_OPENER").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        }
        .into()
    })
}

/// Start `program <path>` without waiting; the child is reaped on its own thread.
pub(crate) fn open_with(program: &str, path: &Path) -> std::io::Result<()> {
    let mut child = std::process::Command::new(program).arg(path).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

pub(crate) fn open(path: &Path) -> std::io::Result<()> {
    open_with(&opener(), path)
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_opener_gets_the_file_path_as_its_only_argument() {
        let dir = std::env::temp_dir().join(format!("gnobbi-opener-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (script, seen) = (dir.join("opener.sh"), dir.join("seen"));
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s|%s' \"$#\" \"$1\" > '{}'\n",
                seen.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let video = dir.join("clip.mp4");
        super::open_with(script.to_str().unwrap(), &video).unwrap();
        for _ in 0..100 {
            if seen.exists() && std::fs::metadata(&seen).unwrap().len() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(&seen).unwrap(),
            format!("1|{}", video.display())
        );
        assert!(super::open_with("/nonexistent/opener", &video).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
