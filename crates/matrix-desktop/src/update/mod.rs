//! In-app update (Linux AppImage, Windows portable). One file per card; no GUI here.
pub mod channel;
pub mod label;
pub mod check;
pub mod scheduler;
pub mod verify;
pub mod startup;
pub mod stage;
pub mod install;
pub mod helper;
#[cfg(all(test, unix))]
mod stage_tests;
#[cfg(test)]
mod startup_tests;
#[cfg(test)]
mod install_tests;
#[cfg(test)]
mod helper_tests;


use crate::Preferences;
use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}};

pub const DEFAULT_UPDATE_BASE: &str = "https://gnobbi.meskatech.net/download/";

/// Download base with exactly one trailing slash: `update_base` from settings, else the default.
pub fn effective_base(prefs: &Preferences) -> String {
    let base = prefs.update_base.as_deref().unwrap_or(DEFAULT_UPDATE_BASE);
    format!("{}/", base.trim().trim_end_matches('/'))
}

/// The one state shared by the scheduler (writes `ready`) and the UI (reads it).
pub fn shared_state() -> Arc<Mutex<stage::UpdateState>> {
    static STATE: OnceLock<Arc<Mutex<stage::UpdateState>>> = OnceLock::new();
    STATE.get_or_init(Default::default).clone()
}

static LATER: AtomicBool = AtomicBool::new(false);

/// «Più tardi»: hides dot and row until the next start (never persisted).
pub fn dismiss_indicator() {
    LATER.store(true, Ordering::Relaxed);
}

/// Version to announce: only a staged build, and not after «Più tardi».
pub fn indicator(state: &Mutex<stage::UpdateState>) -> Option<String> {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.ready.as_ref().filter(|_| !LATER.load(Ordering::Relaxed)).map(|r| r.version.clone())
}

/// The announced version of the shared state, for the UI.
pub fn announced() -> Option<String> {
    indicator(&shared_state())
}

/// Row text and screen-reader label; the project's one-line rule keeps both short.
pub fn indicator_row(version: &str) -> String {
    crate::i18n::t("Update available · {version}").replace("{version}", version)
}
pub fn indicator_label(version: &str) -> String {
    crate::i18n::t("Update available, version {version}").replace("{version}", version)
}

/// Line for a stored outcome; `version` is the running one (the new build, or the restored one).
pub fn outcome_line(result: &str, version: &str) -> String {
    match result {
        startup::OUTCOME_UPDATED => crate::i18n::t("Updated to {version}").replace("{version}", version),
        startup::OUTCOME_RESTORED => crate::i18n::t("Update failed: restored {version}").replace("{version}", version),
        _ => crate::i18n::t("Update failed: error").into(),
    }
}

/// «Aggiorna ora»: `install` runs only with nothing unsent; the returned line is what the drawer shows.
pub fn install_unless_unsent(unsent: bool, install: impl FnOnce() -> Result<std::convert::Infallible, String>) -> String {
    if unsent {
        return crate::i18n::t("Update postponed: message not sent").into();
    }
    match install() {
        Err(line) => line,
        Ok(never) => match never {},
    }
}

/// `BUILD_INFO.build` as a number; `None` for «sconosciuto» or anything non-numeric.
pub fn build_number(build: &str) -> Option<u64> {
    build.trim().parse().ok()
}

#[cfg(test)]
mod prefs_tests {
    use super::*;

    #[test]
    fn effective_base_default_override_and_slash() {
        let mut p = Preferences::default();
        assert_eq!(effective_base(&p), DEFAULT_UPDATE_BASE);
        p.update_base = Some("http://192.0.2.17:8080/dl".into());
        assert_eq!(effective_base(&p), "http://192.0.2.17:8080/dl/");
        p.update_base = Some("http://lan/dl///".into());
        assert_eq!(effective_base(&p), "http://lan/dl/");
    }

    #[test]
    fn indicator_needs_ready_and_not_later() {
        let st = Mutex::new(stage::UpdateState::default());
        assert_eq!(indicator(&st), None);
        st.lock().unwrap().ready = Some(stage::Ready { version: "1.2.3".into(), build: 9, path: "x".into(), size: 1, sha256: String::new() });
        assert_eq!(indicator(&st).as_deref(), Some("1.2.3"));
        dismiss_indicator();
        assert_eq!(indicator(&st), None);
        assert_eq!(indicator_row("1.2.3"), "Update available · 1.2.3");
        assert_eq!(indicator_label("1.2.3"), "Update available, version 1.2.3");
    }

    #[test]
    fn outcome_lines_and_take_once() {
        assert_eq!(outcome_line("updated", "1.2.3"), "Updated to 1.2.3");
        assert_eq!(outcome_line("restored", "1.2.2"), "Update failed: restored 1.2.2");
    }

    #[test]
    fn build_number_parses_digits_only() {
        assert_eq!(build_number("1234"), Some(1234));
        assert_eq!(build_number("sconosciuto"), None);
        assert_eq!(build_number(""), None);
        assert!(build_number(crate::build_info::BUILD_INFO.build).is_some() || crate::build_info::BUILD_INFO.build == "sconosciuto");
    }
}
