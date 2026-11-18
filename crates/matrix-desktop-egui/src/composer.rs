//! Bottom bar: multi-line text box. Enter sends, Shift+Enter adds a line, one draft per room.
use eframe::egui::{self, Key, Modifiers};

/// Returns the text to send when Enter was pressed on a non-empty draft.
pub fn show(ui: &mut egui::Ui, draft: &mut String) -> Option<String> {
    // Taken before the editor sees it, so a bare Enter never inserts a newline.
    let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    ui.horizontal_centered(|ui| {
        ui.add_space(12.);
        let w = ui.available_width() - 12.;
        ui.add_sized([w, 36.], egui::TextEdit::multiline(draft).desired_rows(1).hint_text("Scrivi un messaggio").desired_width(w).lock_focus(true));
    });
    if enter && !draft.trim().is_empty() {
        return Some(std::mem::take(draft));
    }
    None
}
