//! Environment-driven test hooks (GNOBBI_EGUI_*, GNOBBI_DEBUG_*): live only in debug builds or with the `test-hooks` feature.
//! Release builds always see them as unset, so the environment cannot trigger auto-login, screenshots or forced views.

#[cfg(any(debug_assertions, feature = "test-hooks"))]
pub fn var(name: &str) -> Result<String, std::env::VarError> { std::env::var(name) }
#[cfg(not(any(debug_assertions, feature = "test-hooks")))]
pub fn var(_: &str) -> Result<String, std::env::VarError> { Err(std::env::VarError::NotPresent) }

#[cfg(any(debug_assertions, feature = "test-hooks"))]
pub fn var_os(name: &str) -> Option<std::ffi::OsString> { std::env::var_os(name) }
#[cfg(not(any(debug_assertions, feature = "test-hooks")))]
pub fn var_os(_: &str) -> Option<std::ffi::OsString> { None }
