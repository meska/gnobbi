//! List-column header: the ⋮ button is named for screen readers; no theme link, no «Gnobbi» title in the column.
#![cfg(feature = "linux-a11y")]
use accesskit::Role;
use matrix_desktop::accessibility::theme_tree;

#[test]
fn menu_button_is_named_and_the_column_has_no_theme_link_or_app_title() {
    for dark in [false, true] {
        let buttons: Vec<_> = theme_tree(dark)
            .nodes
            .into_iter()
            .filter(|(_, n)| n.role() == Role::Button)
            .map(|(_, n)| n.label().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(buttons, ["Menu elenco"], "dark={dark}: one button, no «Tema» link, no «Gnobbi» button");
    }
}
