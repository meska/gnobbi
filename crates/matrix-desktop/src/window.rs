//! The one place that builds `WindowOptions`, so every window carries the same app id.
use gpui::{TitlebarOptions, WindowOptions};

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

pub fn window_options() -> WindowOptions {
    WindowOptions {
        app_id: Some(APP_ID.into()),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            ..Default::default()
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
