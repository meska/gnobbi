//! In-app update (Linux AppImage, Windows portable, Android `direct` APK). One file per card; no GUI here.
mod android;
pub mod channel;
pub mod label;
pub mod check;
pub mod github;
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


use crate::{AppDirs, Preferences};
use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, Ordering}};

pub const DEFAULT_UPDATE_BASE: &str = "https://gnobbi.meskatech.net/download/";

/// Download base with exactly one trailing slash: `update_base` from settings, else the default.
pub fn effective_base(prefs: &Preferences) -> String {
    let base = prefs.update_base.as_deref().unwrap_or(DEFAULT_UPDATE_BASE);
    format!("{}/", base.trim().trim_end_matches('/'))
}

/// True when this build updates itself (`Channel::self_updates`); decided once. The Android Play build is the case that says no.
pub fn self_updates() -> bool {
    static YES: OnceLock<bool> = OnceLock::new();
    *YES.get_or_init(|| channel::detect_channel(&channel::Env::from_process()).self_updates())
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
    let version = s.ready.as_ref().map(|r| &r.version).or(s.available.as_ref().map(|o| &o.version));
    version.filter(|_| !LATER.load(Ordering::Relaxed)).cloned()
}

/// What the UI shows: one of four simple states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Idle,
    Available { version: String },
    Working,
    /// One line, already localized.
    Error(String),
}

pub fn status() -> Status {
    status_of(&shared_state())
}

fn status_of(state: &Mutex<stage::UpdateState>) -> Status {
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    match (&s.error, s.working, &s.available) {
        (_, true, _) => Status::Working,
        (Some(line), _, _) => Status::Error(line.clone()),
        (_, _, Some(o)) => Status::Available { version: o.version.clone() },
        _ => Status::Idle,
    }
}

/// «Aggiorna»: downloads, verifies and installs in a background thread (the UI never blocks), then
/// restarts the app. Any failure leaves the running build untouched and becomes `Status::Error`.
/// `lock` is `matrix_engine::lock_for_update` (see `stage_then_install`): nothing installs without it.
pub fn start_update<L>(dirs: AppDirs, lock: impl Fn() -> Option<L> + Send + 'static) {
    let state = shared_state();
    let Some(offer) = ({
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let offer = s.available.clone().filter(|_| !s.working);
        if offer.is_some() {
            (s.working, s.error) = (true, None);
        }
        offer
    }) else { return };
    let channel = channel::detect_channel(&channel::Env::from_process());
    let finish = state.clone();
    let spawned = std::thread::Builder::new().name("update-apply".into()).spawn(move || {
        let line = stage_then_install(
            lock,
            || scheduler::stage(&dirs, &state, &channel, &offer),
            |ready| install::install_and_restart(&dirs, &channel, &ready),
        );
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        (s.working, s.error) = (false, Some(line));
    });
    if spawned.is_err() {
        let mut s = finish.lock().unwrap_or_else(|e| e.into_inner());
        (s.working, s.error) = (false, Some(install::failed("")));
    }
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

/// `lock` closes every send path if nothing is being sent (`None` = busy or unknown: no update). It is tried
/// at the click (nothing is downloaded otherwise) and again after the download; that second lock is held
/// through `install`, so nothing can start between the check and the exit, and it is released if `install` fails.
pub fn stage_then_install<R, L>(
    lock: impl Fn() -> Option<L>,
    stage: impl FnOnce() -> Result<R, String>,
    install: impl FnOnce(R) -> Result<std::convert::Infallible, String>,
) -> String {
    let busy = lock().is_none(); // the probe's lock is dropped here: the gate is not held during the download
    install_unless_unsent(busy, || {
        let staged = stage()?;
        let held = lock();
        let line = install_unless_unsent(held.is_none(), || install(staged));
        drop(held);
        Err(line)
    })
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

#[cfg(test)]
mod status_tests {
    use super::*;

    fn offer() -> check::Offer {
        check::Offer { version: "0.1.0-nightly.1.9".into(), build: 9, url: "u".into(), sha256: String::new(), size: 1 }
    }

    #[test]
    fn four_simple_states() {
        let st = Mutex::new(stage::UpdateState::default());
        assert_eq!(status_of(&st), Status::Idle);
        st.lock().unwrap().available = Some(offer());
        assert_eq!(status_of(&st), Status::Available { version: "0.1.0-nightly.1.9".into() });
        st.lock().unwrap().error = Some("Update failed: error".into());
        assert_eq!(status_of(&st), Status::Error("Update failed: error".into()));
        st.lock().unwrap().working = true;
        assert_eq!(status_of(&st), Status::Working);
    }
}
