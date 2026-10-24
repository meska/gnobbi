//! Bubble footer: the time plus a one-glyph delivery state, no state words.
use matrix_desktop::{DemoSendState, bubble_stamp};

#[test]
fn pending_outgoing_shows_clock_glyph_not_words() {
    let s = bubble_stamp("10:42", true, Some(DemoSendState::Pending));
    assert_eq!(s, "10:42 ◷");
}

#[test]
fn failed_outgoing_shows_alert_glyph_not_words() {
    assert_eq!(bubble_stamp("10:42", true, Some(DemoSendState::Failed)), "10:42 !");
}

#[test]
fn confirmed_outgoing_shows_check() {
    assert_eq!(bubble_stamp("10:42", true, None), "10:42 ✓");
}

#[test]
fn incoming_shows_only_time() {
    assert_eq!(bubble_stamp("10:42", false, None), "10:42");
}
