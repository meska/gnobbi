//! The space rail shows only data (initials, name, unread); the
//! "Spazio Matrix" caption lives in accessible labels only.
//! shortcut: source-level check, upgrade to a rendered-tree assert when one exists.

const RAIL: &str = include_str!("../src/rail_view.rs");

#[test]
fn rail_entry_has_no_visible_space_caption() {
    assert!(
        !RAIL.contains(".child(\"Spazio Matrix\")"),
        "visible caption under each space"
    );
}

#[test]
fn rail_keeps_accessible_space_label() {
    assert!(RAIL.contains("Spazio Matrix: {}"));
}
