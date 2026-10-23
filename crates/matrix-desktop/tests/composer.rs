use matrix_desktop::Composer;

#[test]
fn undo_retains_only_the_last_32_edits() {
    let mut draft = Composer::default();
    for _ in 0..40 {
        draft.replace(None, "é");
    }
    for _ in 0..40 {
        draft.undo();
    }
    assert_eq!(draft.text(), "é".repeat(8));
    for _ in 0..40 {
        draft.redo();
    }
    assert_eq!(draft.text(), "é".repeat(40));
}

#[test]
fn invalid_or_oversized_edits_are_rejected_without_losing_draft_or_undo() {
    let mut draft = Composer::default();
    draft.replace(None, "è🦀");
    let reversed = std::ops::Range { start: 5, end: 3 };
    for range in [1..2, reversed, 0..99] {
        assert!(!draft.replace(Some(range.clone()), "invalid"));
        draft.select(range);
        assert_eq!(draft.selection(), 6..6);
        assert_eq!(draft.text(), "è🦀");
    }
    assert!(!draft.replace(None, &"x".repeat(65536)));
    assert_eq!(draft.text(), "è🦀");
    draft.undo();
    assert_eq!(draft.text(), "");
    assert!(draft.replace(None, &"é".repeat(32768)));
    assert_eq!(draft.text().len(), 65536);
    assert!(!draft.replace(None, "x"));
    assert!(!draft.mark(None, "x", None));
    assert_eq!(draft.marked(), None);
}

#[test]
fn utf16_adapter_preserves_scalar_boundaries_and_relative_preedit_selection() {
    let mut draft = Composer::default();
    draft.replace(None, "a🦀è\nz");
    assert_eq!(draft.from_utf16(1..3), 1..5);
    assert_eq!(draft.to_utf16(1..5), 1..3);
    assert_eq!(draft.from_utf16(2..2), 1..1, "inside surrogate clamps down");
    assert_eq!(draft.from_utf16(999..999), 9..9);
    draft.mark(Some(1..5), "🌍é", Some(2..3));
    assert_eq!(draft.selection(), 5..7);
    assert_eq!(draft.to_utf16(draft.selection()), 3..4);
    draft.unmark();
    assert_eq!(draft.marked(), None);
    draft.undo();
    assert_eq!(draft.text(), "a🦀è\nz");
    draft.mark(None, "", None);
    assert_eq!(draft.marked(), None);
}

#[test]
fn unicode_multiline_replacement_can_be_undone_and_redone() {
    let mut draft = Composer::default();
    draft.replace(None, "caffè 🦀\ne\u{301}");
    draft.select(6..11);
    draft.replace(None, "🌍\n");
    assert_eq!(draft.text(), "caffè🌍\n\ne\u{301}");
    draft.undo();
    assert_eq!(draft.text(), "caffè 🦀\ne\u{301}");
    assert_eq!(draft.selection(), 6..11);
    draft.redo();
    assert_eq!(draft.text(), "caffè🌍\n\ne\u{301}");
    draft.undo();
    draft.replace(None, "nuovo");
    draft.redo();
    assert_eq!(draft.text(), "caffènuovo\ne\u{301}");
    draft.enter(false);
    draft.undo();
    assert_eq!(
        draft.text(),
        "",
        "undo must not restore an already submitted message"
    );
}

#[test]
fn composition_enter_cannot_send_or_insert_newline_until_commit() {
    let mut draft = Composer::default();
    draft.replace(None, "prefisso ");
    draft.mark(None, "に", Some(1..1));
    assert_eq!(draft.marked(), Some(9..12));
    assert_eq!(draft.enter(false), None);
    assert_eq!(draft.enter(true), None);
    assert_eq!(draft.text(), "prefisso に");
    draft.mark(None, "日本語", Some(2..3));
    assert_eq!(draft.text(), "prefisso 日本語");
    assert_eq!(draft.selection(), 15..18);
    draft.replace(None, "日本語");
    assert_eq!(draft.marked(), None);
    assert_eq!(draft.enter(false), Some("prefisso 日本語".into()));
}

#[test]
fn whitespace_is_not_sendable_and_is_preserved() {
    let mut draft = Composer::default();
    for text in ["", " \t\n\r", "\u{2003}\u{00a0}"] {
        draft.replace(Some(0..draft.text().len()), text);
        assert_eq!(draft.enter(false), None);
        assert_eq!(draft.text(), text);
    }
}

#[test]
fn shift_enter_inserts_newline_at_selection() {
    let mut draft = Composer::default();
    draft.replace(None, "caffè🦀fine");
    assert_eq!(draft.enter(true), None);
    assert_eq!(draft.text(), "caffè🦀fine\n");
    draft.replace(Some(6..10), "");
    draft.enter(true);
    assert_eq!(draft.text(), "caffè\nfine\n");
}

#[test]
fn enter_returns_exact_nonempty_draft_once() {
    let mut draft = Composer::default();
    draft.replace(None, "  caffè 🦀\nseconda riga  ");
    assert_eq!(
        draft.enter(false),
        Some("  caffè 🦀\nseconda riga  ".into())
    );
    assert_eq!(draft.text(), "");
    assert_eq!(draft.enter(false), None);
}
