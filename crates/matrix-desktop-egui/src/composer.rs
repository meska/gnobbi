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
pub fn show(ui: &mut egui::Ui, draft: &mut String, mode: Mode, attachment: Option<&str>, cancel: &mut bool, drop_attachment: &mut bool, poll: &mut bool, members: &[matrix_engine::operations::Member], mentions: &mut Vec<matrix_engine::sending::Mention>) -> Option<String> {
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
        if ui.button("📊").clicked() {
            *poll = true;
        }
        let w = ui.available_width() - 12.;
        let id = egui::Id::new("composer-text");
        let out = egui::TextEdit::multiline(draft).id(id).desired_rows(1).hint_text("Scrivi un messaggio").desired_width(w).lock_focus(true).show(ui);
        mention_popup(ui, id, &out, draft, members, mentions);
    });
    if enter && (attachment.is_some() || !draft.trim().is_empty()) {
        return Some(std::mem::take(draft));
    }
    None
}

/// «@name»: members matching what is typed after the `@` before the cursor; a click (or Tab) puts «@Name » in the text
/// and remembers who it is, so the send turns it into a real mention.
fn mention_popup(ui: &mut egui::Ui, id: egui::Id, out: &egui::text_edit::TextEditOutput, draft: &mut String, members: &[matrix_engine::operations::Member], mentions: &mut Vec<matrix_engine::sending::Mention>) {
    // A window the system never focused (headless seat) keeps no text focus: the list then follows the text alone.
    let focused = out.response.has_focus() || !ui.input(|i| i.focused);
    if !focused || members.is_empty() {
        return;
    }
    // No cursor yet (text set from outside, nothing clicked): the end of the text.
    let cursor = out.cursor_range.map_or(draft.len(), |r| draft.char_indices().nth(r.primary.index.into()).map_or(draft.len(), |(b, _)| b));
    let Some((at, query)) = matrix_desktop::mention_token(draft, cursor) else { return };
    let matches: Vec<matrix_engine::operations::Member> = matrix_desktop::mention_matches(members, query).into_iter().cloned().collect();
    if matches.is_empty() {
        return;
    }
    let mut pick = None;
    egui::Area::new(id.with("mentions")).order(egui::Order::Foreground).fixed_pos(out.response.rect.left_top() - egui::vec2(0., 8. + 26. * matches.len() as f32)).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            for m in &matches {
                if ui.selectable_label(false, &m.name).clicked() {
                    pick = Some(m.clone());
                }
            }
        });
    });
    if pick.is_none() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
        pick = matches.first().cloned();
    }
    if let Some(m) = pick {
        let label = format!("@{}", m.name.trim_start_matches('@'));
        draft.replace_range(at..cursor, &format!("{label} "));
        mentions.push(matrix_engine::sending::Mention { user_id: m.user_id, label: label.clone() });
        let end = draft[..at].chars().count() + label.chars().count() + 1;
        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
        state.store(ui.ctx(), id);
        ui.ctx().memory_mut(|m| m.request_focus(id));
    }
}
