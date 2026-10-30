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
        Some((section, error)) => format!(
            "{TITLE} — {section}{}",
            if error { " · errore" } else { "" }
        ),
    }
}

/// Signed-in title: the active account as «Name (@id)», the bare id without a display
/// name, «Gnobbi» until the SDK profile arrives. Never a sign-in section.
pub fn account_title(profile: Option<&(String, Option<String>)>) -> String {
    match profile {
        None => TITLE.into(),
        Some((id, Some(name))) if !name.trim().is_empty() && name != id => format!("{name} ({id})"),
        Some((id, _)) => id.clone(),
    }
}

/// Sign-in step name; "Accedi" only exists here, i.e. never with an active session.
pub fn signin_section(recovery: bool, unlock: bool) -> &'static str {
    if recovery {
        "Cronologia cifrata"
    } else if unlock {
        "Sblocca"
    } else {
        "Accedi"
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
        assert_eq!(
            title_for(Some(("Accedi", true))),
            "Gnobbi — Accedi · errore"
        );
        // after login and after a restored session the sign-in section must be gone
        assert_eq!(title_for(None), "Gnobbi");
    }

    #[test]
    fn every_state_has_its_title() {
        let t = |r, u, e| title_for(Some((signin_section(r, u), e)));
        assert_eq!(t(false, false, false), "Gnobbi — Accedi");
        assert_eq!(t(false, true, false), "Gnobbi — Sblocca");
        assert_eq!(t(true, false, false), "Gnobbi — Cronologia cifrata");
        assert_eq!(t(true, true, true), "Gnobbi — Cronologia cifrata · errore");
        assert_eq!(t(false, true, true), "Gnobbi — Sblocca · errore");
        assert!(!title_for(None).contains("Accedi"));
    }

    #[test]
    fn signed_in_title_is_the_active_account() {
        let p = |id: &str, name: Option<&str>| (id.to_string(), name.map(str::to_string));
        assert_eq!(account_title(None), "Gnobbi");
        let alice = p("@alice:example.org", Some("Alice"));
        assert_eq!(account_title(Some(&alice)), "Alice (@alice:example.org)");
        // switching account: the title follows the profile of the shown account
        let bob = p("@bob:example.org", Some("Bob"));
        assert_eq!(account_title(Some(&bob)), "Bob (@bob:example.org)");
        // no display name (or empty, as Tuwunel may return): bare Matrix ID
        assert_eq!(account_title(Some(&p("@c:x", None))), "@c:x");
        assert_eq!(account_title(Some(&p("@c:x", Some("")))), "@c:x");
        assert_eq!(account_title(Some(&p("@c:x", Some("@c:x")))), "@c:x");
        let long = "N".repeat(300);
        let t = account_title(Some(&p("@l:x", Some(&long))));
        assert_eq!(t, format!("{long} (@l:x)"));
        for t in [account_title(None), account_title(Some(&alice))] {
            assert!(!t.contains("Accedi") && !t.contains("Sblocca") && !t.contains("Cronologia"));
        }
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
