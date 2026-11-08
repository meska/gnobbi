use matrix_desktop::install_offer::{Choice, State};

#[test]
fn asks_once_and_only_when_ready_and_not_typing() {
    let mut s = State::default();
    s.poll(false, false, || panic!("not signed in/synced"));
    s.poll(true, true, || panic!("user is typing"));
    assert!(!s.open);
    s.poll(true, false, || true);
    assert!(s.open);
    s.open = false;
    s.poll(true, false, || panic!("already checked this launch"));
}

#[test]
fn not_due_stays_closed() {
    let mut s = State::default();
    s.poll(true, false, || false);
    assert!(!s.open);
}

#[test]
fn choices() {
    let open = || {
        let mut s = State::default();
        s.open = true;
        s
    };
    let mut dont = false;
    let mut s = open();
    assert!(!s.choose(Choice::Later, &mut dont, || panic!()));
    assert!(!s.open && !dont);
    let mut s = open();
    assert!(s.choose(Choice::Never, &mut dont, || panic!()));
    assert!(!s.open && dont);
    // Settings «Aggiungi al menu» works after «Non chiedere più» and resets it.
    let mut s = State::default();
    assert!(s.choose(Choice::Add, &mut dont, || Ok(())));
    assert!(!dont);
    // Failure: dialog stays, error kept, nothing saved.
    let mut s = open();
    dont = true;
    assert!(!s.choose(Choice::Add, &mut dont, || Err("no".into())));
    assert!(s.open && dont && s.error.as_deref() == Some("no"));
}

#[test]
fn hidden_while_compact_chat_covers_the_window_but_stays_open() {
    let mut s = State::default();
    s.open = true;
    assert!(s.visible(false) && !s.visible(true));
    s.open = false;
    assert!(!s.visible(false));
}
