//! List-column header: no buttons of its own (no ⋮ menu, no theme link, no «Gnobbi» title).
#![cfg(feature = "linux-a11y")]
use accesskit::Role;
use matrix_desktop::accessibility::theme_tree;

#[test]
fn the_column_header_exports_no_button() {
    for dark in [false, true] {
        assert!(!theme_tree(dark).nodes.iter().any(|(_, n)| n.role() == Role::Button), "dark={dark}");
    }
}
