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
