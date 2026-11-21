//! Bidi order and Arabic shaping come from the vendored egui fork (epaint: unicode-bidi + harfrust), so
//! labels, menus and messages need no extra layout step. These tests pin that behaviour; see the localization notes.
use eframe::egui::{Color32, FontId, Galley, RawInput};
use std::sync::Arc;

fn layout(text: &str) -> Arc<Galley> {
    let ctx = eframe::egui::Context::default();
    // Like the app (eframe `system_fonts`): Arabic and Hebrew come from a system font, the bundled ones lack them.
    ctx.add_font_provider(Arc::new(egui_system_font_provider::SystemFontProvider::new()));
    let mut galley = None;
    let mut out = ctx.run_ui(RawInput::default(), |ui| {
        galley = Some(ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(14.0), Color32::WHITE));
    });
    out.textures_delta.clear();
    galley.unwrap()
}

/// Logical char indices in the order they sit on screen, left to right.
fn visual(text: &str) -> Vec<usize> {
    let g = layout(text);
    assert_eq!(g.rows.len(), 1);
    assert_eq!(g.rows[0].row.text(), text, "the logical text stays intact (copy/select)");
    g.rows[0].row.visual_order().collect()
}

#[test]
fn hebrew_with_number_and_latin_is_reordered_per_run() {
    // RTL paragraph: words right to left, "42" and "Gnobbi" keep their own left-to-right order.
    let text = "שלום 42 Gnobbi";
    let order = visual(text);
    let pos = |i: usize| order.iter().position(|&o| o == i).unwrap();
    assert!(pos(13) < pos(5) && pos(6) < pos(3), "Gnobbi, 42, Hebrew from left to right: {order:?}");
    assert!(pos(5) < pos(6) && pos(8) < pos(13), "numbers and Latin keep their order: {order:?}");
    assert!(pos(3) < pos(2) && pos(2) < pos(1) && pos(1) < pos(0), "Hebrew word reversed: {order:?}");
}

#[test]
fn arabic_is_right_to_left_and_joined() {
    let order = visual("مرحبا");
    assert_eq!(order, vec![4, 3, 2, 1, 0]);
    // Contextual forms: a joined letter has the advance of its initial/medial form, not of the isolated one.
    let joined = layout("بيت").rows[0].row.glyphs.iter().map(|g| g.advance_width).collect::<Vec<_>>();
    let isolated: Vec<f32> = "بيت".chars().map(|c| layout(&c.to_string()).rows[0].row.glyphs[0].advance_width).collect();
    assert_ne!(joined, isolated, "no shaping: {joined:?}");
}

#[test]
fn latin_is_unchanged() {
    assert_eq!(visual("Hello 123 world"), (0..15).collect::<Vec<_>>());
    assert_eq!(visual("Ciao, come stai?"), (0..16).collect::<Vec<_>>());
}

#[test]
fn trailing_space_takes_width_in_every_script() {
    let w = |t: &str| layout(t).size().x;
    for word in ["abc", "ראו", "مرحبا"] {
        let space = w(&format!("{word} ")) - w(word);
        assert!(space > 2.0, "the space after {word:?} has no width: {space}");
    }
    assert!(w(" ראו") > w("ראו") + 2.0, "leading space lost");
}
