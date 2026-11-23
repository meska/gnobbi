//! «Send feedback»: the DM with the project's feedback user, and the diagnostic line put in its composer.
use crate::build_info::{BUILD_INFO, BuildInfo, system_version};

/// Where feedback goes: the one place this address is written. Only test builds may override it.
const FEEDBACK_MXID: &str = "@feedback:gnobbi.chat";

pub fn target() -> String {
    crate::hooks::var("GNOBBI_DEBUG_FEEDBACK_MXID").unwrap_or_else(|_| FEEDBACK_MXID.to_owned())
}

/// Model of this device, when the OS tells it; no serial, name or other identifier.
fn device_model() -> Option<String> {
    #[cfg(target_os = "macos")]
    let raw = std::process::Command::new("sysctl").args(["-n", "hw.model"]).output().ok().and_then(|o| String::from_utf8(o.stdout).ok());
    // On a real iPhone `uname` reports the model («iPhone16,1»); the simulator says «arm64», which is no model.
    #[cfg(target_os = "ios")]
    let raw = {
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        (unsafe { libc::uname(&mut u) } == 0).then(|| unsafe { std::ffi::CStr::from_ptr(u.machine.as_ptr()) }.to_string_lossy().into_owned()).filter(|m| m.contains(','))
    };
    #[cfg(target_os = "linux")]
    let raw = std::fs::read_to_string("/sys/class/dmi/id/product_name").ok();
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
    let raw: Option<String> = None;
    raw.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty())
}

impl BuildInfo {
    /// «Gnobbi 0.1.0 (1234) · abc1234 · macOS 15.1 (aarch64) · Mac14,2»: no account, user, path or host data.
    pub fn feedback_line(&self, system: &str, model: Option<&str>) -> String {
        let mut line = format!("Gnobbi {} ({}) · {} · {system}", self.version, self.build, self.git_sha);
        if let Some(m) = model {
            line.push_str(" · ");
            line.push_str(m);
        }
        line
    }
}

pub fn diagnostic() -> String {
    BUILD_INFO.feedback_line(&system_version(), device_model().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_has_version_build_commit_system_and_model() {
        let b = BuildInfo { version: "0.1.0", build: "1234", git_sha: "abc1234", build_date: "2026-11-23", matrix_sdk: "0.19.1" };
        assert_eq!(b.feedback_line("macOS 15.1 (aarch64)", Some("Mac14,2")), "Gnobbi 0.1.0 (1234) · abc1234 · macOS 15.1 (aarch64) · Mac14,2");
        assert_eq!(b.feedback_line("Debian (x86_64)", None), "Gnobbi 0.1.0 (1234) · abc1234 · Debian (x86_64)");
    }

    #[test]
    fn live_line_is_one_line_without_personal_data() {
        let line = diagnostic();
        assert!(line.starts_with("Gnobbi ") && !line.contains('\n') && !line.contains('@'), "{line}");
        let user = std::env::var("USER").unwrap_or_default();
        assert!(user.is_empty() || !line.contains(&user), "{line}");
    }

    #[test]
    fn target_is_the_feedback_user() {
        // The override is for debug and test-hooks builds only; a plain release build ignores it.
        assert_eq!(FEEDBACK_MXID, "@feedback:gnobbi.chat");
        if crate::hooks::var("GNOBBI_DEBUG_FEEDBACK_MXID").is_err() {
            assert_eq!(target(), FEEDBACK_MXID);
        }
    }
}
