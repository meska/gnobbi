#![cfg(any(target_os = "linux", target_os = "windows"))]
//! `update::status()` / `start_update` seen from the UI. One process-global state: one test, in order.
//! No network: the offer points to a closed port and the test process is not a self-updating channel,
//! so the download is refused before any connection and nothing is written next to the test binary.
use matrix_desktop::{AppDirs, update::{self, Status, check::Offer}};
use std::time::{Duration, Instant};

fn wait_not_working() -> Status {
    let t = Instant::now();
    loop {
        let s = update::status();
        if s != Status::Working || t.elapsed() > Duration::from_secs(20) { return s }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn status_machine_and_failures_never_touch_the_running_app() {
    let root = std::env::temp_dir().join(format!("gnobbi-status-{}", std::process::id()));
    let dirs = AppDirs { config: root.join("c"), data: root.join("d"), cache: root.join("k") };
    dirs.ensure().unwrap();
    let cwd_before: Vec<_> = std::fs::read_dir(".").unwrap().flatten().map(|e| e.file_name()).collect();

    // Nothing offered: Idle, and start_update is a no-op (no thread, no state change).
    assert_eq!(update::status(), Status::Idle);
    update::start_update(dirs.clone(), false);
    assert_eq!(update::status(), Status::Idle);

    let offer = Offer { version: "0.1.0-nightly.9.999".into(), build: 999, url: "http://127.0.0.1:1/x.AppImage".into(), sha256: "0".repeat(64), size: 10 };
    update::shared_state().lock().unwrap().available = Some(offer.clone());
    assert_eq!(update::status(), Status::Available { version: offer.version.clone() });

    // A message is still being sent: nothing starts, the offer stays, the row says why.
    update::start_update(dirs.clone(), true);
    assert_eq!(wait_not_working(), Status::Error("Update postponed: message not sent".into()));
    assert!(update::shared_state().lock().unwrap().available.is_some(), "offer kept for a later click");

    // Real attempt that fails: back to a single error line, not «working», process alive, offer kept.
    update::start_update(dirs.clone(), false);
    match wait_not_working() {
        Status::Error(line) => assert!(!line.is_empty() && !line.contains('\n'), "{line:?}"),
        other => panic!("expected an error line, got {other:?}"),
    }
    assert!(update::shared_state().lock().unwrap().available.is_some());
    // Retry is allowed after an error (working flag released).
    update::start_update(dirs.clone(), false);
    assert!(matches!(wait_not_working(), Status::Error(_)));

    let cwd_after: Vec<_> = std::fs::read_dir(".").unwrap().flatten().map(|e| e.file_name()).collect();
    let mut a = cwd_before; a.sort();
    let mut b = cwd_after; b.sort();
    assert_eq!(a, b, "no staging file left in the working directory");
    let _ = std::fs::remove_dir_all(root);
}
