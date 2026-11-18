//! Bottom bar: multi-line text box. Enter sends, Shift+Enter adds a line, one draft per room.
use eframe::egui::{self, Key, Modifiers, RichText};

/// What the next Enter does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Send,
    Reply,
    Edit,
}

/// Returns the text to send when Enter was pressed on a non-empty draft; `cancel` is set by the X of the mode bar.
pub fn show(ui: &mut egui::Ui, draft: &mut String, mode: Mode, attachment: Option<&str>, cancel: &mut bool, drop_attachment: &mut bool) -> Option<String> {
    // Taken before the editor sees it, so a bare Enter never inserts a newline.
    let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    if mode != Mode::Send {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            ui.label(RichText::new(if mode == Mode::Reply { "Risposta" } else { "Modifica" }).strong());
            if ui.small_button("✕").clicked() {
                *cancel = true;
            }
        });
    }
    if let Some(name) = attachment {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            ui.label(format!("📎 {name}"));
            if ui.small_button("✕").clicked() {
                *drop_attachment = true;
            }
        });
    }
    ui.horizontal_centered(|ui| {
        ui.add_space(12.);
        let w = ui.available_width() - 12.;
        ui.add_sized([w, 36.], egui::TextEdit::multiline(draft).desired_rows(1).hint_text("Scrivi un messaggio").desired_width(w).lock_focus(true));
    });
    if enter && (attachment.is_some() || !draft.trim().is_empty()) {
        return Some(std::mem::take(draft));
    }
    None
}
