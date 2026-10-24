//! Room rows and the details panel show only data; static captions are gone.
//! shortcut: source-level check, upgrade to a rendered-tree assert when one exists.

const SIDEBAR: &str = include_str!("../src/sidebar.rs");
const DETAILS: &str = include_str!("../src/room_details_view.rs");
const ACCESSIBLE_DETAILS: &str = include_str!("../src/accessibility/details.rs");

#[test]
fn live_room_rows_have_no_constant_preview_caption() {
    assert!(!SIDEBAR.contains("Storico Matrix · messaggi live"));
}

#[test]
fn details_panel_has_no_visible_kind_caption() {
    assert!(!DETAILS.contains("\"Spazio Matrix\""));
    assert!(!DETAILS.contains("\"Stanza Matrix\""));
}

#[test]
fn details_keep_accessible_kind_label() {
    assert!(ACCESSIBLE_DETAILS.contains("\"Spazio Matrix\""));
    assert!(ACCESSIBLE_DETAILS.contains("\"Stanza Matrix\""));
}
