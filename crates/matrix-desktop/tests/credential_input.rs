use matrix_desktop::{CredentialInput, LoginInput};

#[test]
fn public_clipboard_selection_preserves_unicode_boundaries() {
    let mut input = LoginInput::public();
    assert!(input.replace(None, "a🔑è"));
    assert_eq!(input.exportable_selection(), None);
    assert!(input.edit("left", false, false));
    assert!(input.edit("left", false, true));
    assert_eq!(input.selection_utf16(), 1..3);
    assert_eq!(input.exportable_selection().as_deref(), Some("🔑"));
    assert!(input.replace(None, ""));
    assert_eq!(input.display_text(), "aè");
}

#[test]
fn clipboard_export_refuses_protected_or_composing_selection() {
    for protected in [false, true] {
        let mut input = if protected {
            LoginInput::secret()
        } else {
            LoginInput::public()
        };
        assert!(input.replace(None, "synthetic🔑"));
        assert!(input.edit("a", true, false));
        if protected {
            assert_eq!(input.exportable_selection(), None);
            assert!(input.take().as_str() == "synthetic🔑");
        } else {
            assert!(input.mark(None, "日本🔑", Some(0..4)));
            assert_eq!(input.exportable_selection(), None);
            input.unmark();
            assert_eq!(input.exportable_selection().as_deref(), Some("日本🔑"));
        }
    }
}

#[test]
fn protected_preedit_is_masked_and_commit_replaces_the_whole_marked_range() {
    let mut input = LoginInput::secret();
    input.replace(None, "a");
    assert!(input.mark(None, "に🔑", Some(1..3)));
    assert_eq!(input.marked_utf16(), Some(1..4));
    assert_eq!(input.selection_utf16(), 2..4);
    assert_eq!(input.display_text(), "••••");
    assert_eq!(input.text_for_range(0..4).as_deref(), Some("••••"));
    assert!(!input.edit("backspace", false, false));
    assert!(input.mark(None, "日本", None));
    assert_eq!(input.marked_utf16(), Some(1..3));
    assert!(input.replace(None, "日🔑"));
    assert_eq!(input.marked_utf16(), None);
    assert!(input.take().as_str() == "a日🔑");
}

#[test]
fn invalid_preedit_selection_preserves_existing_secret_and_mark() {
    let mut input = LoginInput::secret();
    input.mark(None, "日🔑", None);
    for range in [0..100, 1..2, usize::MAX..usize::MAX] {
        assert!(!input.mark(None, "🔑", Some(range)));
        assert_eq!(input.marked_utf16(), Some(0..3));
        assert_eq!(input.display_text(), "•••");
    }
    assert!(!input.mark(None, &"x".repeat(4097), None));
    assert_eq!(input.marked_utf16(), Some(0..3));
    input.unmark();
    assert_eq!(input.marked_utf16(), None);
    assert!(input.edit("backspace", false, false));
    assert!(input.take().as_str() == "日");
}

#[test]
fn native_login_keyboard_edits_never_split_a_unicode_scalar() {
    let mut input = LoginInput::secret();
    assert!(input.replace(None, "a🔑è"));
    assert!(input.edit("left", false, false));
    assert_eq!(input.selection_utf16(), 3..3);
    assert!(input.edit("backspace", false, false));
    assert_eq!(input.selection_utf16(), 1..1);
    assert!(input.edit("home", false, false));
    assert!(input.edit("right", false, true));
    assert_eq!(input.selection_utf16(), 0..1);
    assert!(input.edit("delete", false, false));
    assert!(input.take().as_str() == "è");
    assert!(input.replace(None, "a🔑è"));
    assert!(input.edit("a", true, false));
    assert_eq!(input.selection_utf16(), 0..4);
    assert!(!input.edit("c", true, false));
    assert!(!input.edit("x", true, false));
    assert!(!input.edit("z", true, false));
    assert!(!input.edit("tab", false, false));
    assert!(input.edit("end", false, false));
    assert_eq!(input.selection_utf16(), 4..4);
    assert!(input.edit("home", false, true));
    assert_eq!(input.selection_utf16(), 0..4);
    assert!(input.replace(None, "new"));
    assert!(input.take().as_str() == "new");
}

#[test]
fn native_login_input_never_exports_secret_surrounding_text() {
    let mut password = LoginInput::secret();
    assert!(password.replace(None, "caffè🔑"));
    assert_eq!(password.display_text(), "•••••••");
    assert_eq!(password.text_for_range(0..7).as_deref(), Some("•••••••"));
    assert_eq!(password.text_for_range(6..8), None);
    assert!(password.take().as_str() == "caffè🔑");
    let mut user = LoginInput::public();
    assert!(user.replace(None, "@fixturè🔑:example.invalid"));
    assert_eq!(user.display_text(), "@fixturè🔑:example.invalid");
    assert_eq!(user.text_for_range(8..10).as_deref(), Some("🔑"));
    assert_eq!(user.text_for_range(9..10), None);
}

#[test]
fn secret_field_selection_uses_utf16_without_splitting_surrogate_pairs() {
    let mut field = CredentialInput::default();
    assert!(field.replace(None, "a🔑è"));
    assert!(!field.select_utf16(2..3));
    assert_eq!(field.selection_utf16(), 4..4);
    assert!(field.select_utf16(1..3));
    assert!(field.replace(None, "X"));
    assert_eq!(field.selection_utf16(), 2..2);
    assert!(!field.replace(Some(std::ops::Range { start: 3, end: 2 }), "broken"));
    assert!(!field.replace(Some(0..100), "broken"));
    assert!(field.take_for_authentication().as_str() == "aXè");
}

#[test]
fn secret_field_rejects_control_text_instead_of_silently_changing_the_password() {
    let mut field = CredentialInput::default();
    assert!(field.replace(None, "fixture"));
    for text in ["\n", "\r", "\t", "\0", "line\r\nline"] {
        assert!(!field.replace(None, text));
    }
    assert!(field.take_for_authentication().as_str() == "fixture");
}

#[test]
fn secret_field_rejects_oversize_replacements_without_losing_previous_input() {
    let mut field = CredentialInput::default();
    assert!(field.replace(None, "è"));
    assert!(!field.replace(None, &"x".repeat(4095)));
    assert_eq!(field.selection_utf16(), 1..1);
    assert!(field.take_for_authentication().as_str() == "è");
}

#[test]
fn secret_field_paints_only_a_mask_and_moves_ownership_on_submit() {
    let mut field = CredentialInput::default();
    assert!(field.replace(None, "caffè🔑"));
    assert_eq!(field.masked_text(), "•••••••");
    assert_eq!(field.selection_utf16(), 7..7);
    let submitted = field.take_for_authentication();
    assert!(submitted.as_str() == "caffè🔑");
    assert_eq!(field.masked_text(), "");
    assert_eq!(field.selection_utf16(), 0..0);
}
