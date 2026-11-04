//! Build metadata compiled in by build.rs; nothing here is typed by hand.

pub const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD_PARTY_NOTICES.md");
pub const LICENSE_MIT: &str = include_str!("../../../LICENSE-MIT");
pub const LICENSE_APACHE: &str = include_str!("../../../LICENSE-APACHE");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: &'static str,
    pub build: &'static str,
    pub git_sha: &'static str,
    pub build_date: &'static str,
    pub matrix_sdk: &'static str,
}

pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: env!("CARGO_PKG_VERSION"),
    build: env!("GNOBBI_BUILD_NUMBER"),
    git_sha: env!("GNOBBI_GIT_SHA"),
    build_date: env!("GNOBBI_BUILD_DATE"),
    matrix_sdk: env!("GNOBBI_MATRIX_SDK_VERSION"),
};

/// OS name and version at runtime; falls back to the bare OS name.
pub fn system_version() -> String {
    let os = std::env::consts::OS;
    let ver = if cfg!(target_os = "macos") {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
    } else {
        std::fs::read_to_string("/etc/os-release").ok().and_then(|s| {
            s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string())
        })
    };
    match ver.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(v) if cfg!(target_os = "macos") => format!("macOS {v} ({})", std::env::consts::ARCH),
        Some(v) => format!("{v} ({})", std::env::consts::ARCH),
        None => format!("{os} ({})", std::env::consts::ARCH),
    }
}

impl BuildInfo {
    /// Plain text for «Copia informazioni». No account, user, path or host data.
    pub fn copy_text(&self, system: &str) -> String {
        format!(
            "Gnobbi {}\nBuild: {} ({}, {})\nSistema: {}\nMatrix SDK: {}\n",
            self.version, self.build, self.git_sha, self.build_date, system, self.matrix_sdk
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_compiled_env() {
        assert_eq!(BUILD_INFO.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(BUILD_INFO.build, env!("GNOBBI_BUILD_NUMBER"));
        assert_eq!(BUILD_INFO.git_sha, env!("GNOBBI_GIT_SHA"));
        assert_eq!(BUILD_INFO.build_date, env!("GNOBBI_BUILD_DATE"));
        assert_eq!(BUILD_INFO.matrix_sdk, env!("GNOBBI_MATRIX_SDK_VERSION"));
        assert!(BUILD_INFO.build_date.len() == 10 || BUILD_INFO.build_date == "sconosciuto");
    }

    #[test]
    fn sdk_version_comes_from_the_lockfile() {
        let lock = include_str!("../../../Cargo.lock");
        let want = format!("name = \"matrix-sdk\"\nversion = \"{}\"", BUILD_INFO.matrix_sdk);
        assert!(lock.contains(&want));
    }

    #[test]
    fn copy_text_has_the_build_fields_and_nothing_personal() {
        let t = BUILD_INFO.copy_text("Test OS 1.0 (x86_64)");
        for f in [BUILD_INFO.version, BUILD_INFO.build, BUILD_INFO.git_sha, BUILD_INFO.matrix_sdk, "Test OS 1.0"] {
            assert!(t.contains(f), "{f} missing in {t}");
        }
        let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        for p in ["@", "://", "/Users/", "/home/", user.as_str(), home.as_str()] {
            assert!(p.is_empty() || !t.contains(p), "personal data {p:?} in {t}");
        }
        assert!(!system_version().contains('\n'));
    }

    #[test]
    fn licenses_and_notices_are_bundled() {
        assert!(LICENSE_MIT.contains("MIT License"));
        assert!(LICENSE_APACHE.contains("Apache License"));
        assert!(THIRD_PARTY_NOTICES.contains("matrix-sdk 0.19.1"));
    }
}
