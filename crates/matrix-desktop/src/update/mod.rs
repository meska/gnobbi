//! In-app update (Linux AppImage, Windows portable). One file per card; no GUI here.
pub mod channel;
pub mod label;
#[cfg(feature = "live")]
pub mod check;
pub mod scheduler;
pub mod verify;
pub mod startup;
pub mod stage;
#[cfg(all(test, unix))]
mod stage_tests;
#[cfg(test)]
mod startup_tests;


use crate::Preferences;

pub const DEFAULT_UPDATE_BASE: &str = "https://gnobbi.meskatech.net/download/";

/// Download base with exactly one trailing slash: `update_base` from settings, else the default.
pub fn effective_base(prefs: &Preferences) -> String {
    let base = prefs.update_base.as_deref().unwrap_or(DEFAULT_UPDATE_BASE);
    format!("{}/", base.trim().trim_end_matches('/'))
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
    fn build_number_parses_digits_only() {
        assert_eq!(build_number("1234"), Some(1234));
        assert_eq!(build_number("sconosciuto"), None);
        assert_eq!(build_number(""), None);
        assert!(build_number(crate::build_info::BUILD_INFO.build).is_some() || crate::build_info::BUILD_INFO.build == "sconosciuto");
    }
}
