#![cfg(any(target_os = "linux", target_os = "windows"))]
//! View/state of the update offer (AUTO_UPDATE.md §3): dot + row from one state, «Più tardi», guard, channels.
//! One process-global state (`shared_state`, `dismiss_indicator`): the order lives in a single test.
use matrix_desktop::update::{self, channel::Channel, stage::Ready};
use std::cell::Cell;

#[test]
fn ready_shows_dot_row_and_label_until_later() {
    assert_eq!(update::announced(), None, "nothing staged: no dot, no row");
    update::shared_state().lock().unwrap().ready =
        Some(Ready { version: "1.2.3".into(), build: 9, path: "x".into(), size: 1, sha256: String::new() });
    // Dot (menu button) and row (drawer) both read announced().
    let v = update::announced().expect("Ready -> dot and row");
    assert_eq!(update::indicator_row(&v), "Update available · 1.2.3");
    assert_eq!(update::indicator_label(&v), "Update available, version 1.2.3");
    update::dismiss_indicator();
    assert_eq!(update::announced(), None, "«Più tardi» hides dot and row");
    assert!(update::shared_state().lock().unwrap().ready.is_some(), "the update stays ready");
}

#[test]
fn unsent_text_defers_and_never_installs() {
    let calls = Cell::new(0);
    let spy = || { calls.set(calls.get() + 1); Err("Update failed: folder not writable".into()) };
    assert_eq!(update::install_unless_unsent(true, spy), "Update postponed: message not sent");
    assert_eq!(calls.get(), 0, "install_staged must not run");
    assert_eq!(update::install_unless_unsent(false, spy), "Update failed: folder not writable");
    assert_eq!(calls.get(), 1);
}

#[test]
fn check_switch_only_on_self_updating_channels() {
    let p = || "x".into();
    for (c, shown) in [
        (Channel::AppImage { target: p() }, true),
        (Channel::WindowsPortable { exe: p() }, true),
        (Channel::MacStore, false),
        (Channel::Flatpak, false),
        (Channel::SystemPackage, false),
        (Channel::Unknown, false),
    ] {
        assert_eq!(c.self_updates(), shown, "{c:?}");
    }
}

#[test]
fn unsent_is_asked_again_after_the_download_and_unknown_means_unsent() {
    use std::{cell::RefCell, convert::Infallible};
    let postponed = "Update postponed: message not sent";
    let run = |answers: Vec<Option<bool>>| {
        let (asked, staged, installed) = (Cell::new(0), Cell::new(0), Cell::new(0));
        let answers = RefCell::new(answers.into_iter());
        let line = update::stage_then_install(
            || { asked.set(asked.get() + 1); answers.borrow_mut().next().unwrap() },
            || { staged.set(staged.get() + 1); Ok(()) },
            |()| -> Result<Infallible, String> { installed.set(installed.get() + 1); Err("Update failed: error".into()) },
        );
        (line, asked.get(), staged.get(), installed.get())
    };
    // Nothing unsent at the click and after the download: installs.
    assert_eq!(run(vec![Some(false), Some(false)]), ("Update failed: error".into(), 2, 1, 1));
    // A message starts during the download: staged, but never installed.
    assert_eq!(run(vec![Some(false), Some(true)]), (postponed.into(), 2, 1, 0));
    // Unsent at the click: nothing is even downloaded.
    assert_eq!(run(vec![Some(true)]), (postponed.into(), 1, 0, 0));
    // Unknown state is fail-safe, at the click and after the download.
    assert_eq!(run(vec![None]), (postponed.into(), 1, 0, 0));
    assert_eq!(run(vec![Some(false), None]), (postponed.into(), 2, 1, 0));
}
