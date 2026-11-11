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
    assert_eq!(update::indicator_row(&v), "Aggiornamento disponibile · 1.2.3");
    assert_eq!(update::indicator_label(&v), "Aggiornamento disponibile, versione 1.2.3");
    update::dismiss_indicator();
    assert_eq!(update::announced(), None, "«Più tardi» hides dot and row");
    assert!(update::shared_state().lock().unwrap().ready.is_some(), "the update stays ready");
}

#[test]
fn unsent_text_defers_and_never_installs() {
    let calls = Cell::new(0);
    let spy = || { calls.set(calls.get() + 1); Err("Aggiornamento non riuscito: cartella non scrivibile".into()) };
    assert_eq!(update::install_unless_unsent(true, spy), "Aggiornamento rinviato: messaggio non inviato");
    assert_eq!(calls.get(), 0, "install_staged must not run");
    assert_eq!(update::install_unless_unsent(false, spy), "Aggiornamento non riuscito: cartella non scrivibile");
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
