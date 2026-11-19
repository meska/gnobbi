//! Bottom bar: multi-line text box. Enter sends, Shift+Enter adds a line, one draft per room.
use eframe::egui::{self, Key, Modifiers, RichText};

/// What the user asked of the microphone this frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VoiceAct {
    Start,
    Send,
    Cancel,
}

/// What the next Enter does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Send,
    Reply,
    Edit,
}

/// Returns the text to send when Enter was pressed on a non-empty draft; `cancel` is set by the X of the mode bar.
pub fn show(ui: &mut egui::Ui, draft: &mut String, mode: Mode, attachment: Option<&str>, cancel: &mut bool, drop_attachment: &mut bool, poll: &mut bool, picked: &mut Option<std::path::PathBuf>, recording: Option<u64>, voice: &mut Option<VoiceAct>, members: &[matrix_engine::operations::Member], mentions: &mut Vec<matrix_engine::sending::Mention>) -> Option<String> {
    // Taken before the editor sees it, so a bare Enter never inserts a newline.
    let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    if mode != Mode::Send {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            ui.label(RichText::new(if mode == Mode::Reply { "Risposta" } else { "Modifica" }).strong());
            if crate::info::close_icon(ui).clicked() {
                *cancel = true;
            }
        });
    }
    if let Some(name) = attachment {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            ui.label(format!("📎 {name}"));
            if crate::info::close_icon(ui).clicked() {
                *drop_attachment = true;
            }
        });
    }
    if let Some(secs) = recording {
        ui.horizontal_centered(|ui| {
            ui.add_space(12.);
            ui.colored_label(ui.visuals().error_fg_color, format!("● {}:{:02}", secs / 60, secs % 60));
            if ui.button("Invia nota").clicked() {
                *voice = Some(VoiceAct::Send);
            }
            if ui.button("Annulla").clicked() {
                *voice = Some(VoiceAct::Cancel);
            }
        });
        return None;
    }
    let mut send_click = false;
    ui.horizontal_centered(|ui| {
        ui.add_space(12.);
        if ui.button("📊").clicked() {
            *poll = true;
        }
        if ui.button("📎").clicked() {
            *picked = crate::pick::file("Allega un file", false);
        }
        let w = ui.available_width() - 96.;
        let id = egui::Id::new("composer-text");
        format_shortcuts(ui, id, draft);
        let out = egui::TextEdit::multiline(draft).id(id).desired_rows(1).hint_text("Scrivi un messaggio").desired_width(w).lock_focus(true).show(ui);
        mention_popup(ui, id, &out, draft, members, mentions);
        if draft.trim().is_empty() && attachment.is_none() {
            if ui.button("🎤").on_hover_text("Nota vocale").clicked() {
                *voice = Some(VoiceAct::Start);
            }
        } else if ui.button("➤").on_hover_text("Invia").clicked() {
            send_click = true;
        }
        // The 32 emoji of the GPUI picker; one is appended to the text and the box keeps the focus.
        ui.menu_button("☺", |ui| {
            egui::Grid::new("emoji-grid").show(ui, |ui| {
                for (i, (name, emoji)) in matrix_desktop::EMOJI_SET.iter().enumerate() {
                    if ui.button(*emoji).on_hover_text(*name).clicked() {
                        draft.push_str(emoji);
                        ui.memory_mut(|m| m.request_focus(id));
                        ui.close();
                    }
                    if i % 8 == 7 {
                        ui.end_row();
                    }
                }
            });
        });
    });
    if (enter || send_click) && (attachment.is_some() || !draft.trim().is_empty()) {
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

/// Markdown marks the send path already turns into `formatted_body`: Cmd/Ctrl+B bold, +I italic, +Shift+X strike, +E code.
/// Wraps the selection (or inserts an empty pair with the cursor inside).
fn format_shortcuts(ui: &egui::Ui, id: egui::Id, draft: &mut String) {
    if !ui.memory(|m| m.has_focus(id)) {
        return;
    }
    let mark = ui.input_mut(|i| {
        [(Modifiers::COMMAND, Key::B, "**"), (Modifiers::COMMAND, Key::I, "*"), (Modifiers::COMMAND | Modifiers::SHIFT, Key::X, "~~"), (Modifiers::COMMAND, Key::E, "`")]
            .into_iter()
            .find(|(m, k, _)| i.consume_key(*m, *k))
            .map(|(_, _, mark)| mark)
    });
    let Some(mark) = mark else { return };
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let (a, b) = state.cursor.char_range().map_or((draft.chars().count(), draft.chars().count()), |r| {
        let (p, q): (usize, usize) = (r.primary.index.into(), r.secondary.index.into());
        (p.min(q), p.max(q))
    });
    let (a, b) = wrap(draft, a, b, mark);
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
    state.store(ui.ctx(), id);
}

/// Puts `mark` around the char range `a..b`; returns the new selection (inside the marks).
/// shortcut: no toggle-off of an already wrapped selection, upgrade if users ask.
fn wrap(text: &mut String, a: usize, b: usize, mark: &str) -> (usize, usize) {
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    let (ba, bb) = (byte(a), byte(b));
    text.insert_str(bb, mark);
    text.insert_str(ba, mark);
    let n = mark.chars().count();
    (a + n, b + n)
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn wraps_selection_and_empty_cursor_by_chars() {
        let mut t = "è ciao mondo".to_owned();
        assert_eq!(wrap(&mut t, 2, 6, "**"), (4, 8));
        assert_eq!(t, "è **ciao** mondo");
        let mut t = "ab".to_owned();
        assert_eq!(wrap(&mut t, 1, 1, "`"), (2, 2));
        assert_eq!(t, "a``b");
    }
}
