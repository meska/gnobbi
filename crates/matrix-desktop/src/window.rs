//! The one place that builds `WindowOptions`, so every window carries the same app id.
use gpui::{TitlebarOptions, WindowOptions};

/// Wayland app_id / X11 WM_CLASS; matches the installed desktop entry.
pub const APP_ID: &str = "net.meskatech.gnobbi";
pub const TITLE: &str = "Gnobbi";

pub fn window_options() -> WindowOptions {
    WindowOptions {
        app_id: Some(APP_ID.into()),
        titlebar: Some(TitlebarOptions { title: Some(TITLE.into()), ..Default::default() }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_carry_app_id_and_title() {
        let o = window_options();
        assert_eq!(o.app_id.as_deref(), Some("net.meskatech.gnobbi"));
        assert_eq!(o.titlebar.and_then(|t| t.title).map(|t| t.to_string()).as_deref(), Some("Gnobbi"));
    }
}
