//! Last-panic breadcrumb: `crash-last.txt` in the data dir (the App Group on iOS) so a crash is
//! diagnosable without App Store Connect. Only version, SHA, thread, panic location and a capped
//! message/backtrace: no account or room data is added by us.
use std::path::{Path, PathBuf};

const LAST: &str = "crash-last.txt";
const PREV: &str = "crash-prev.txt";
const CAP: usize = 6000;

fn render(msg: &str, loc: &str, thread: &str, bt: &str) -> String {
    let b = &crate::build_info::BUILD_INFO;
    let mut s = format!("Gnobbi {} build {} sha {}\nthread: {thread}\nat: {loc}\nmessage: {msg}\n{bt}", b.version, b.build, b.git_sha);
    s.truncate(s.floor_char_boundary(CAP));
    s
}

/// Chains the existing hook (stderr/os_log keeps working); never panics itself.
pub fn install(dir: &Path) {
    let file = dir.join(LAST);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let _ = std::fs::write(&file, render(&msg, &loc, &thread, &bt));
        prev(info);
    }));
}

/// Startup: if the previous run panicked, log it once and keep the file as `crash-prev.txt`.
pub fn report_previous(dir: &Path) -> Option<PathBuf> {
    let last = dir.join(LAST);
    let text = std::fs::read_to_string(&last).ok()?;
    eprintln!("Gnobbi: previous run panicked:\n{}", text.lines().take(12).collect::<Vec<_>>().join("\n"));
    let prev = dir.join(PREV);
    std::fs::rename(&last, &prev).ok().map(|_| prev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_has_version_location_and_is_capped() {
        let r = render("boom", "a.rs:1:2", "main", &"x".repeat(10_000));
        assert!(r.contains("at: a.rs:1:2") && r.contains("message: boom") && r.contains("sha "));
        assert!(r.len() <= CAP);
    }

    #[test]
    fn previous_panic_is_reported_once() {
        let d = std::env::temp_dir().join(format!("gnobbi-crash-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(report_previous(&d).is_none());
        std::fs::write(d.join(LAST), "x").unwrap();
        assert!(report_previous(&d).is_some() && report_previous(&d).is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
