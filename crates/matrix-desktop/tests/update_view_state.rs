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
fn lock_is_asked_again_after_the_download_held_through_the_install_and_unknown_means_busy() {
    use std::{cell::RefCell, convert::Infallible, rc::Rc};
    let postponed = "Update postponed: message not sent";
    // `Some(true)` = the lock is granted; the lock is a token that records when it is dropped.
    let run = |answers: Vec<bool>| {
        let (asked, staged, installed) = (Cell::new(0), Cell::new(0), Cell::new(0));
        let (held, held_at_install) = (Rc::new(Cell::new(0)), Cell::new(None));
        let answers = RefCell::new(answers.into_iter());
        struct Token(Rc<Cell<i32>>);
        impl Drop for Token { fn drop(&mut self) { self.0.set(self.0.get() - 1) } }
        let line = update::stage_then_install(
            || {
                asked.set(asked.get() + 1);
                answers.borrow_mut().next().unwrap().then(|| { held.set(held.get() + 1); Token(held.clone()) })
            },
            || { staged.set(staged.get() + 1); Ok(()) },
            |()| -> Result<Infallible, String> { installed.set(installed.get() + 1); held_at_install.set(Some(held.get())); Err("Update failed: error".into()) },
        );
        (line, asked.get(), staged.get(), installed.get(), held.get(), held_at_install.get())
    };
    // Idle at the click and after the download: installs with the lock held; the failed install releases it.
    assert_eq!(run(vec![true, true]), ("Update failed: error".into(), 2, 1, 1, 0, Some(1)));
    // A message starts during the download: staged, never installed.
    assert_eq!(run(vec![true, false]).0, postponed);
    assert_eq!(run(vec![true, false]).3, 0);
    // Busy at the click: nothing is even downloaded. Unknown state is the same answer (`None`).
    assert_eq!(run(vec![false]), (postponed.into(), 1, 0, 0, 0, None));
}
