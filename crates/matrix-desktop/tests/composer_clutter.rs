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
    for id in ["composer-emoji-toggle", "composer-attach", "composer-record"] {
        assert!(COMPOSER.contains(&format!("icon_button(\"{id}\"")));
    }
}
