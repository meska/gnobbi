//! The one place that builds `WindowOptions`, so every window carries the same app id.
use gpui::{Pixels, TitlebarOptions, WindowOptions, px};

/// Wayland app_id / X11 WM_CLASS; matches the installed desktop entry.
pub const APP_ID: &str = "net.meskatech.gnobbi";
pub const TITLE: &str = "Gnobbi";

/// The one place that derives the window title from the current view:
/// `None` is the signed-in shell, `Some((section, error))` a sign-in step.
pub fn title_for(signin: Option<(&str, bool)>) -> String {
    match signin {
        None => TITLE.into(),
        Some((section, error)) => format!("{TITLE} — {section}{}", if error { " · errore" } else { "" }),
    }
}

/// Height of the transparent macOS titlebar the content must clear (the theme colour shows through it).
/// Zero elsewhere and in fullscreen, where the system titlebar is gone.
pub fn titlebar_inset(fullscreen: bool) -> Pixels {
    if cfg!(target_os = "macos") && !fullscreen { px(28.) } else { px(0.) }
}

pub fn window_options() -> WindowOptions {
    WindowOptions {
        app_id: Some(APP_ID.into()),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            // macOS: the theme background extends under the titlebar; native drag and double-click zoom stay.
            appears_transparent: cfg!(target_os = "macos"),
            traffic_light_position: None, // native position: a custom one goes through gpui's unsafe move path and sat too low
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_follows_the_view() {
        assert_eq!(title_for(Some(("Accedi", false))), "Gnobbi — Accedi");
        assert_eq!(title_for(Some(("Accedi", true))), "Gnobbi — Accedi · errore");
        // after login and after a restored session the sign-in section must be gone
        assert_eq!(title_for(None), "Gnobbi");
    }

    #[test]
    fn inset_only_on_macos_windowed() {
        assert_eq!(titlebar_inset(true), px(0.));
        assert_eq!(titlebar_inset(false), if cfg!(target_os = "macos") { px(28.) } else { px(0.) });
    }

    #[test]
    fn options_carry_app_id_and_title() {
        let o = window_options();
        assert_eq!(o.app_id.as_deref(), Some("net.meskatech.gnobbi"));
        assert_eq!(
            o.titlebar
                .and_then(|t| t.title)
                .map(|t| t.to_string())
                .as_deref(),
            Some("Gnobbi")
        );
    }
}
