use matrix_desktop::{DemoHistory, TextSelection};
use matrix_domain::{AccountId, RoomId, RoomScope};

#[test]
fn queued_mouse_entry_cannot_replace_a_new_selection() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(66),
            room_id: RoomId::new("!mouse:test"),
        },
        2,
    );
    let old_id = &history.events()[0].id;
    let current_id = &history.events()[1].id;
    let text = "caffè 🙂\nriga";
    let mut selection = TextSelection::default();
    let painted = selection.generation();
    assert!(selection.begin_current(painted, old_id.clone(), text, 0));
    selection.begin(current_id.clone(), text, 0);
    selection.extend(current_id, text, text.len());
    let current = selection.generation();
    assert!(!selection.begin_current(painted, old_id.clone(), text, 3));
    assert_eq!(selection.generation(), current);
    assert_eq!(selection.copy(current_id, text), Some(text));
    assert!(!selection.is_active(old_id, text));
    selection.clear();
    assert!(!selection.begin_current(current, current_id.clone(), text, 0));
    let fresh = selection.generation();
    assert!(selection.begin_current(fresh, current_id.clone(), text, 3));
    assert!(selection.is_active(current_id, text));
    assert_eq!(selection.copy(current_id, text), None);
}

#[test]
fn reopened_selection_rejects_queued_intents_and_extensions() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(66),
            room_id: RoomId::new("!lifecycle:test"),
        },
        2,
    );
    let id = &history.events()[0].id;
    let text = "caffè 🙂\nriga";
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), text, 0);
    let old = selection.generation();
    selection.extend(id, text, text.len());
    assert_eq!(
        selection.generation(),
        old,
        "range movement retains current handles"
    );
    selection.clear();
    selection.begin(id.clone(), text, 0);
    assert_ne!(
        selection.generation(),
        old,
        "same ID/body is a new selection instance"
    );
    let before = selection.generation();
    selection.extend_current(old, id, text, text.len());
    assert_eq!(
        selection.copy(id, text),
        None,
        "queued old drag must not extend new selection"
    );
    assert_eq!(selection.generation(), before);
    assert_ne!(
        TextSelection::default().generation(),
        before,
        "another timeline cannot accept this instance"
    );
}

#[test]
fn body_replacement_permanently_retires_the_selection() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(66),
            room_id: RoomId::new("!lifecycle:test"),
        },
        1,
    );
    let id = &history.events()[0].id;
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), "caffè 🙂", 0);
    selection.extend(id, "caffè 🙂", usize::MAX);
    let old = selection.generation();
    selection.retain_current([(id, "caffè 🙂")]);
    assert_eq!(selection.generation(), old);
    selection.retain_current([(id, "replacement")]);
    selection.retain_current([(id, "caffè 🙂")]);
    assert_eq!(selection.copy(id, "caffè 🙂"), None);
    assert_ne!(selection.generation(), old);
    selection.begin(id.clone(), "caffè 🙂", 0);
    selection.retain_current([]);
    assert!(!selection.is_active(id, "caffè 🙂"));
}

#[test]
fn collapsing_then_extending_retires_queued_copy_and_cancel() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(66),
            room_id: RoomId::new("!lifecycle:test"),
        },
        1,
    );
    let id = &history.events()[0].id;
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), "caffè 🙂", 0);
    selection.extend(id, "caffè 🙂", usize::MAX);
    let selected = selection.generation();
    selection.key(id, "caffè 🙂", "home", false);
    assert_ne!(
        selection.generation(),
        selected,
        "removed Copy/Cancel must retire queued commands too"
    );
    let collapsed = selection.generation();
    selection.extend(id, "caffè 🙂", usize::MAX);
    assert_eq!(
        selection.generation(),
        collapsed,
        "new controls use the post-retirement generation"
    );
    let current = selection.generation();
    selection.extend(id, "caffè 🙂", 6);
    assert_eq!(
        selection.generation(),
        current,
        "nonempty subranges retain live handles"
    );
}

#[test]
fn collapsed_caret_is_active_only_for_the_current_scoped_body() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(65),
            room_id: RoomId::new("!selection:test"),
        },
        2,
    );
    let id = &history.events()[0].id;
    let mut selection = TextSelection::default();
    assert!(!selection.is_active(id, "è🙂\n"));
    selection.begin(id.clone(), "è🙂\n", 0);
    assert!(selection.is_active(id, "è🙂\n"));
    assert_eq!(selection.copy(id, "è🙂\n"), None);
    assert!(!selection.is_active(id, "replacement"));
    assert!(!selection.is_active(&history.events()[1].id, "è🙂\n"));
    selection.clear();
    assert!(!selection.is_active(id, "è🙂\n"));
}

#[test]
fn selection_clamps_to_utf8_boundaries_and_ignores_other_messages() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(0),
            room_id: RoomId::new("!s:fixture.invalid"),
        },
        2,
    );
    let id = &history.events()[0].id;
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), "è☕!", 1);
    selection.extend(id, "è☕!", 4);
    assert_eq!(selection.range(id, "è☕!"), Some(0..2));
    assert_eq!(selection.copy(id, "è☕!"), Some("è"));
    selection.extend(&history.events()[1].id, "è☕!", 6);
    assert_eq!(selection.range(id, "è☕!"), Some(0..2));
    selection.extend(id, "è☕!", usize::MAX);
    assert_eq!(selection.copy(id, "è☕!"), Some("è☕!"));
    selection.begin(id.clone(), "", usize::MAX);
    assert_eq!(selection.range(id, ""), None);
}

#[test]
fn unshifted_arrows_collapse_selection_to_the_directional_edge() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(7),
            room_id: RoomId::new("!selection:fixture.invalid"),
        },
        1,
    );
    let id = &history.events()[0].id;
    let text = "è☕\n文";
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), text, 0);
    selection.extend(id, text, text.len());
    assert!(selection.key(id, text, "left", false));
    assert!(selection.key(id, text, "right", true));
    assert_eq!(selection.copy(id, text), Some("è"));
    selection.begin(id.clone(), text, text.len());
    selection.extend(id, text, 0);
    assert!(selection.key(id, text, "right", false));
    assert!(selection.key(id, text, "left", true));
    assert_eq!(selection.copy(id, text), Some("文"));
}

#[test]
fn select_all_uses_only_the_active_message_body() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(7),
            room_id: RoomId::new("!selection:fixture.invalid"),
        },
        2,
    );
    let id = &history.events()[0].id;
    let text = "caffè ☕\nseconda riga";
    let mut selection = TextSelection::default();
    assert!(!selection.key(id, text, "select-all", false));
    selection.begin(id.clone(), text, 4);
    assert!(!selection.key(&history.events()[1].id, text, "select-all", false));
    assert!(selection.key(id, text, "select-all", false));
    assert_eq!(selection.copy(id, text), Some(text));
    assert!(selection.key(id, text, "home", false));
    assert!(selection.key(id, text, "end", true));
    assert_eq!(selection.copy(id, text), Some(text));
}

#[test]
fn keyboard_selection_moves_by_unicode_scalar_and_preserves_newlines() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(7),
            room_id: RoomId::new("!selection:fixture.invalid"),
        },
        1,
    );
    let id = &history.events()[0].id;
    let text = "è☕\n文";
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), text, 0);
    assert!(selection.key(id, text, "right", true));
    assert_eq!(selection.copy(id, text), Some("è"));
    assert!(selection.key(id, text, "end", true));
    assert_eq!(selection.copy(id, text), Some(text));
    assert!(selection.key(id, text, "left", true));
    assert_eq!(selection.copy(id, text), Some("è☕\n"));
    assert!(selection.key(id, text, "home", false));
    assert_eq!(selection.copy(id, text), None);
    assert!(selection.key(id, text, "left", true));
    assert_eq!(selection.copy(id, text), None);
    assert!(!selection.key(id, "replacement", "end", true));
    assert!(!selection.key(&history.events()[0].id, text, "enter", false));
}

#[test]
fn replaced_text_has_no_highlight_range() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(7),
            room_id: RoomId::new("!selection:fixture.invalid"),
        },
        1,
    );
    let id = &history.events()[0].id;
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), "caffè", 0);
    selection.extend(id, "caffè", usize::MAX);
    assert_eq!(selection.range(id, "caffè"), Some(0..6));
    assert_eq!(selection.range(id, "testo"), None);
}

#[test]
fn replaced_message_text_invalidates_selection_and_extension() {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(7),
            room_id: RoomId::new("!selection:fixture.invalid"),
        },
        1,
    );
    let id = &history.events()[0].id;
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), "caffè ☕\noriginale", 0);
    selection.extend(id, "caffè ☕\noriginale", 6);
    assert_eq!(selection.copy(id, "caffè ☕\noriginale"), Some("caffè"));
    assert_eq!(selection.copy(id, "nuovo ☕\ncontenuto"), None);
    selection.extend(id, "nuovo ☕\ncontenuto", usize::MAX);
    assert_eq!(selection.copy(id, "nuovo ☕\ncontenuto"), None);
    assert_eq!(selection.copy(id, "caffè ☕\noriginale"), Some("caffè"));
}

#[test]
fn selection_preserves_unicode_newlines_and_scoped_identity() {
    let scope = RoomScope {
        account_id: AccountId::new(7),
        room_id: RoomId::new("!selection:fixture.invalid"),
    };
    let history = DemoHistory::new(scope, 3);
    let id = history.events()[2].id.clone();
    let text = "caffè ☕\nseconda riga";
    let mut selection = TextSelection::default();
    selection.begin(id.clone(), text, text.len());
    selection.extend(&id, text, 3);
    assert_eq!(selection.range(&id, text), Some(3..text.len()));
    assert_eq!(selection.copy(&id, text), Some("fè ☕\nseconda riga"));
    assert_eq!(selection.copy(&history.events()[1].id, text), None);
    let mut other = id.clone();
    other.scope.account_id = AccountId::new(8);
    assert_eq!(selection.copy(&other, text), None);
    selection.clear();
    assert_eq!(selection.copy(&id, text), None);
}
