//! Composer, header and timeline show only data; static captions are gone.
//! shortcut: source-level check, upgrade to a rendered-tree assert when one exists.

const COMPOSER: &str = include_str!("../src/composer_view.rs");
const MAIN: &str = include_str!("../src/main.rs");
const ACCESSIBILITY: &str = include_str!("../src/accessibility.rs");

#[test]
fn composer_placeholder_is_short_for_live_and_demo() {
    assert!(COMPOSER.contains("\"Scrivi un messaggio\""));
    assert!(!COMPOSER.contains("Scrivi un messaggio Matrix"));
    assert!(!COMPOSER.contains("Scrivi un messaggio · demo locale"));
}

#[test]
fn composer_editor_has_field_padding_and_placeholder_color() {
    let editor = COMPOSER.split(".id(\"composer-editor\")").nth(1).unwrap();
    let editor = editor.split(".child(body)").next().unwrap();
    assert!(editor.contains(".px_2()"), "editor must use the standard field padding");
    assert!(editor.contains("t.placeholder"), "empty editor must use the placeholder token");
}

#[test]
fn composer_has_no_shortcut_caption() {
    assert!(!COMPOSER.contains("draft in memoria"));
    assert!(!COMPOSER.contains("DEMO LOCALE · Invio"));
}

#[test]
fn live_header_has_no_constant_caption() {
    assert!(!MAIN.contains("Matrix · storico, invio e ricezione live"));
}

#[test]
fn composer_keeps_accessible_label() {
    assert!(ACCESSIBILITY.contains("set_label(\"Scrivi un messaggio\")"));
}

#[test]
fn composer_buttons_are_32px_icons_without_words() {
    for word in ["\"Allega\"", "\"Voce\"", "\"Emoji\"", "\"Invia\"", "Invia · demo"] {
        assert!(!COMPOSER.contains(word), "{word} must be an icon, not text");
    }
    assert_eq!(COMPOSER.matches(".size(px(32.))").count(), 2, "icon_button + Invia");
    for id in ["composer-emoji-toggle", "composer-poll", "composer-attach", "composer-record"] {
        assert!(COMPOSER.contains(&format!("icon_button(\"{id}\"")));
    }
}

#[test]
fn demo_send_arrow_is_muted_in_every_theme() {
    // t.secondary is a theme token, so light and dark both get the muted demo arrow.
    assert!(COMPOSER.contains("icon(Icon::Send, if self.live { t.accent } else { t.secondary })"));
}
