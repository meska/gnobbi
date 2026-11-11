use matrix_desktop::{Composer, RoomDrafts};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn scope(account: u64, room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new(room),
    }
}

#[test]
fn forgetting_active_account_clears_its_editor_history_but_preserves_other_draft() {
    let a = scope(28, "!shared:fixture.invalid");
    let b = scope(29, "!shared:fixture.invalid");
    let mut drafts = RoomDrafts::new(a.clone());
    let mut editor = Composer::default();
    editor.replace(None, "alice private draft");
    drafts.switch(b.clone(), &mut editor);
    editor.replace(None, "bob caffè\nseconda riga");
    editor.select(0..3);
    drafts.switch(a.clone(), &mut editor);
    drafts.forget_account(AccountId::new(28), &mut editor);
    assert_eq!(editor.text(), "");
    editor.undo();
    assert_eq!(editor.text(), "", "logout must also clear undo history");
    drafts.switch(b, &mut editor);
    assert_eq!(editor.text(), "bob caffè\nseconda riga");
    assert_eq!(editor.selection(), 0..3);
    editor.undo();
    assert_eq!(editor.text(), "");
    editor.redo();
    assert_eq!(editor.text(), "bob caffè\nseconda riga");
    drafts.switch(a, &mut editor);
    assert_eq!(editor.text(), "");
}

#[test]
fn forgetting_saved_account_preserves_the_active_preedit_and_removes_every_saved_room() {
    let a1 = scope(28, "!one:fixture.invalid");
    let a2 = scope(28, "!two:fixture.invalid");
    let b = scope(29, "!one:fixture.invalid");
    let mut drafts = RoomDrafts::new(a1.clone());
    let mut editor = Composer::default();
    editor.replace(None, "alice first");
    drafts.switch(a2.clone(), &mut editor);
    editor.replace(None, "alice second");
    drafts.switch(b.clone(), &mut editor);
    editor.mark(None, "bob preedit", None);
    let marked = editor.marked();
    drafts.forget_account(AccountId::new(28), &mut editor);
    assert_eq!(editor.text(), "bob preedit");
    assert_eq!(editor.marked(), marked);
    drafts.switch(a1, &mut editor);
    assert_eq!(editor.text(), "");
    drafts.switch(a2, &mut editor);
    assert_eq!(editor.text(), "");
    drafts.switch(b, &mut editor);
    assert_eq!(editor.text(), "bob preedit");
}

#[test]
fn leaving_room_ends_preedit_without_losing_visible_text() {
    let a = scope(0, "!a:fixture.invalid");
    let mut rooms = RoomDrafts::new(a.clone());
    let mut editor = Composer::default();
    editor.mark(None, "preedit caffè", None);
    rooms.switch(scope(0, "!b:fixture.invalid"), &mut editor);
    rooms.switch(a, &mut editor);
    assert_eq!(editor.text(), "preedit caffè");
    assert_eq!(editor.marked(), None);
    assert_eq!(editor.enter(false).as_deref(), Some("preedit caffè"));
}

#[test]
fn same_room_name_in_other_account_has_independent_draft() {
    let a = scope(0, "!a:fixture.invalid");
    let other = scope(1, "!a:fixture.invalid");
    let mut rooms = RoomDrafts::new(a.clone());
    let mut editor = Composer::default();
    editor.replace(None, "account zero");
    rooms.switch(a.clone(), &mut editor);
    assert_eq!(editor.text(), "account zero");
    rooms.switch(other.clone(), &mut editor);
    assert_eq!(editor.text(), "");
    editor.replace(None, "account one");
    rooms.switch(a, &mut editor);
    assert_eq!(editor.enter(false).as_deref(), Some("account zero"));
    rooms.switch(other, &mut editor);
    assert_eq!(editor.text(), "account one");
}

#[test]
fn switching_rooms_restores_exact_draft_selection_and_undo() {
    let a = scope(0, "!a:fixture.invalid");
    let b = scope(0, "!b:fixture.invalid");
    let mut rooms = RoomDrafts::new(a.clone());
    let mut editor = Composer::default();
    editor.replace(None, "caffè 🦀\nseconda riga");
    editor.select(0..6);
    rooms.switch(b.clone(), &mut editor);
    assert_eq!(editor.text(), "");
    editor.replace(None, "draft B");
    rooms.switch(a, &mut editor);
    assert_eq!(editor.text(), "caffè 🦀\nseconda riga");
    assert_eq!(editor.selection(), 0..6);
    editor.undo();
    assert_eq!(editor.text(), "");
    editor.redo();
    assert_eq!(editor.text(), "caffè 🦀\nseconda riga");
    rooms.switch(b, &mut editor);
    assert_eq!(editor.text(), "draft B");
}

#[test]
fn draft_left_in_another_room_counts_as_unsent() {
    let (a, b) = (scope(1, "!a:fixture.invalid"), scope(1, "!b:fixture.invalid"));
    let mut drafts = RoomDrafts::new(a);
    let mut editor = Composer::default();
    assert!(!drafts.has_unsent());
    editor.replace(None, "non ancora inviato");
    drafts.switch(b.clone(), &mut editor);
    assert!(drafts.has_unsent(), "the draft saved with room A blocks the update");
    editor.replace(None, "");
    drafts.switch(scope(1, "!a:fixture.invalid"), &mut editor);
    assert!(!drafts.has_unsent(), "once A is the active editor nothing is saved");
}
