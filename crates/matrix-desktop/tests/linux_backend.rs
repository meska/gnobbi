//! Probe the pinned toolkit's backend selection without opening a display.
#![cfg(all(target_os = "linux", feature = "desktop"))]

#[test]
fn native_wayland_is_preferred_with_explicit_x11_fallback() {
    for (wayland, x11, expected) in [
        (Some("wayland-test"), Some(":99"), "Wayland"),
        (Some("wayland-test"), None, "Wayland"),
        (None, Some(":99"), "X11"),
        (Some(""), Some(":99"), "X11"),
        (None, None, "Headless"),
    ] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "backend_selection_child", "--nocapture"])
            .env_remove("ZED_HEADLESS")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env("DIME_EXPECT_BACKEND", expected);
        if let Some(display) = wayland {
            child.env("WAYLAND_DISPLAY", display);
        }
        if let Some(display) = x11 {
            child.env("DISPLAY", display);
        }
        let result = child.output().unwrap();
        assert!(
            result.status.success(),
            "WAYLAND_DISPLAY={wayland:?}, DISPLAY={x11:?}: {}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
        );
    }
}

#[test]
fn backend_selection_child() {
    if let Ok(expected) = std::env::var("DIME_EXPECT_BACKEND") {
        assert_eq!(gpui::guess_compositor(), expected);
    }
}
