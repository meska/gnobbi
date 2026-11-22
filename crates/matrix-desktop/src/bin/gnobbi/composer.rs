//! Bottom bar: multi-line text box. Enter sends, Shift+Enter adds a line, one draft per room.
use crate::icons::Icon;
use eframe::egui::{self, Key, Modifiers, RichText};
use matrix_desktop::i18n;

pub fn panel(frame: egui::Frame) -> egui::Panel {
    egui::Panel::bottom("composer").min_size(40.).show_separator_line(false).frame(frame.inner_margin(egui::Margin::symmetric(0, 4)))
}

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
/// Suffix of the widget ids of the thread drawer's composer, so it can sit beside the main one.
pub const THREAD: &str = "-thread";

/// `salt` is `""` for the room composer and `THREAD` for the drawer: the same bar minus the poll (attach and voice go into the thread).
pub fn show(ui: &mut egui::Ui, salt: &str, draft: &mut String, mode: Mode, attachment: Option<(&str, &[u8], u64)>, cancel: &mut bool, drop_attachment: &mut bool, poll: &mut bool, picked: &mut Option<std::path::PathBuf>, pasted: &mut Option<egui::ColorImage>, recording: Option<u64>, voice: &mut Option<VoiceAct>, members: &[matrix_engine::operations::Member], mentions: &mut Vec<matrix_engine::sending::Mention>, mut stickers: Option<(&mut crate::stickers::Panel, &crate::avatar::Avatars)>, t: &matrix_desktop::DesignTokens) -> Option<String> {
    // Taken before the editor sees it, so a bare Enter never inserts a newline.
    // consume_key matches Shift loosely, so Shift+Enter is checked on the event and left to the TextEdit (it inserts the newline).
    let shift_enter = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.shift)));
    let nid = |base: &str| egui::Id::new(format!("{base}{salt}"));
    let thread = !salt.is_empty();
    // Two composers on screen: Enter belongs to the focused one; the room one also keeps it when nothing is focused.
    let focus = ui.memory(|m| m.focused());
    let mine = focus == Some(nid("composer-text")) || (!thread && focus != Some(egui::Id::new(format!("composer-text{THREAD}"))));
    let enter = mine && !shift_enter && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    // The Enter that confirms an IME / dead-key composition must not send: ignore it while composing and in the frame of the IME event.
    let ime_id = nid("composer-ime");
    let ime_event = ui.input(|i| i.events.iter().find_map(|e| match e {
        egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => Some(!text.is_empty()),
        egui::Event::Ime(egui::ImeEvent::Commit(_)) => Some(false),
        _ => None,
    }));
    let composing = ui.data(|d| d.get_temp::<bool>(ime_id)).unwrap_or(false);
    ui.data_mut(|d| d.insert_temp(ime_id, ime_event.unwrap_or(composing)));
    let enter = enter && !composing && ime_event.is_none();
    if mode != Mode::Send {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            ui.label(RichText::new(if mode == Mode::Reply { i18n::t_ctx("banner", "Reply") } else { i18n::t("Edit") }).strong());
            if crate::info::close_icon(ui, i18n::t("Cancel")).clicked() {
                *cancel = true;
            }
        });
    }
    let error_id = nid("composer-image-error");
    let image_input = take_image_input(ui, nid("composer-text"), if recording.is_some() { Mode::Edit } else { mode }, attachment.is_some(), draft);
    let inserting_image = image_input.is_some();
    if let Some(result) = image_input {
        match result {
            Ok(image) => {
                *pasted = Some(image);
                ui.data_mut(|d| d.remove::<String>(error_id));
            }
            Err(error) => { ui.data_mut(|d| d.insert_temp(error_id, error)); }
        }
    }
    if let Some(error) = ui.data(|d| d.get_temp::<String>(error_id)) {
        ui.horizontal(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, i18n::t(&error));
            if crate::info::close_icon(ui, i18n::t("Close")).clicked() {
                ui.data_mut(|d| d.remove::<String>(error_id));
            }
        });
    }
    if let Some((name, data, revision)) = attachment {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            attachment_thumbnail(ui, nid("composer-thumbnail"), data, revision);
            ui.label(crate::icons::with_label(crate::icons::Icon::Paperclip, name));
            if crate::info::close_icon(ui, i18n::t("Remove attachment")).clicked() {
                *drop_attachment = true;
            }
        });
    } else {
        ui.data_mut(|d| d.remove::<Thumbnail>(nid("composer-thumbnail")));
    }
    if let Some(secs) = recording {
        ui.horizontal(|ui| {
            ui.add_space(12.);
            let red = ui.visuals().error_fg_color;
            ui.label(crate::icons::rich(Icon::Record, true, crate::metrics::get().icon).color(red));
            ui.colored_label(red, format!("{}:{:02}", secs / 60, secs % 60));
            if ui.button(i18n::t("Send note")).clicked() {
                *voice = Some(VoiceAct::Send);
            }
            if ui.button(i18n::t("Cancel")).clicked() {
                *voice = Some(VoiceAct::Cancel);
            }
        });
        return None;
    }
    let mut send_click = false;
    let slot = crate::metrics::button(); // side of an icon button: the scale's button height, 44 on touch
    // Not horizontal_centered: it fills the panel's height from the previous frame, so the bar never shrank after a narrow-window wrap.
    ui.horizontal_top(|ui| {
        ui.add_space(12.);
        // Same order as GPUI, all on the right: text, emoji, poll, attach, then the mic/send slot (32 px each).
        let slots = if thread { 3. } else { 4. };
        let w = (ui.available_width() - slots * slot - (slots + 1.) * ui.spacing().item_spacing.x - 12.).max(80.); // Keep the right gap close to the 12 px left pad.
        let id = nid("composer-text");
        format_shortcuts(ui, id, draft);
        // Borderless like Telegram; grows with the text up to 8 rows, then scrolls. The 6 px vertical padding makes one row as tall as the icon buttons.
        let row = ui.text_style_height(&egui::TextStyle::Body);
        let pad = ((slot - row) / 2.).max(0.); // centres one text row on the icon buttons
        let max_h = 8. * row + 2. * pad;
        // A child ui with its own max height: inside a horizontal row the available height is one row, which would cap the scroll area.
        // RTL: the row runs from the right, the box ends at the cursor's right edge.
        let corner = if crate::dir::rtl() { egui::pos2(ui.cursor().right() - (w + 8.), ui.cursor().top()) } else { ui.cursor().min };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(corner, egui::vec2(w + 8., max_h))));
        let scroll = egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(0.).id_salt(nid("composer-scroll")).show(&mut child, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, pad as i8)).show(ui, |ui| {
                ui.spacing_mut().interact_size.y = row; // else the editor is 20 px tall and the text sits at its top
                egui::TextEdit::multiline(draft).id(id).ime_purpose(if mode == Mode::Edit { egui::IMEPurpose::Normal } else { egui::IMEPurpose::Message }).desired_rows(1).hint_text(if thread { i18n::t("Reply in thread") } else { i18n::t("Write a message") }).desired_width(w).horizontal_align(crate::dir::start()).lock_focus(true).frame(egui::Frame::NONE).margin(egui::Margin::ZERO).show(ui)
            }).inner
        });
        let text_h = scroll.content_size.y.min(max_h);
        ui.allocate_space(egui::vec2(w + 8., text_h));
        let out = scroll.inner;
        mention_popup(ui, id, &out, draft, members, mentions, t);
        // The picker restores focus to the composer after inserting at its saved selection.
        // Icons stay at the bottom of a tall box.
        ui.with_layout(crate::dir::col(), |ui| {
        ui.spacing_mut().item_spacing.y = 0.; // else the space above adds a gap and the icons sit lower than the text
        ui.add_space((text_h - slot).max(0.));
        ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(slot), egui::Sense::hover());
        let emoji = icon_at(ui, rect, &format!("composer-emoji{salt}"), Icon::Smiley, 1., true, i18n::t("Emoji"));
        egui::Popup::menu(&emoji).show(|ui| crate::dir::popup(ui, |ui| {
            let tab = egui::Id::new(crate::stickers::TAB_ID);
            let mut sticker_tab = stickers.is_some() && ui.data(|d| d.get_temp::<bool>(tab)).unwrap_or(false);
            if stickers.is_some() {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut sticker_tab, false, i18n::t("Emoji"));
                    ui.selectable_value(&mut sticker_tab, true, i18n::t("Sticker"));
                });
                ui.data_mut(|d| d.insert_temp(tab, sticker_tab));
            }
            if let (true, Some((panel, textures))) = (sticker_tab, stickers.as_mut()) {
                crate::stickers::show(ui, panel, textures);
                return;
            }
            egui::Grid::new(nid("emoji-grid")).show(ui, |ui| {
                for (i, (name, emoji)) in matrix_desktop::EMOJI_SET.iter().enumerate() {
                    if ui.button(*emoji).on_hover_text(i18n::t(name)).clicked() {
                        insert_emoji(ui.ctx(), id, draft, emoji);
                        ui.close();
                    }
                    if i % 8 == 7 {
                        ui.end_row();
                    }
                }
            });
        }));
        if !thread {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(slot), egui::Sense::hover());
        if icon_at(ui, rect, "composer-poll", Icon::ChartBarHorizontal, 1., true, i18n::t("Poll")).clicked() {
            *poll = true;
        }
        }
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(slot), egui::Sense::hover());
        // The picker answers by its title, the same for both bars: remember which one asked.
        let asker = egui::Id::new("composer-attach-asker");
        if icon_at(ui, rect, &format!("composer-attach{salt}"), Icon::Paperclip, 1., true, i18n::t("Attach a file")).clicked() {
            ui.data_mut(|d| d.insert_temp(asker, thread));
            crate::pick::start(ui.ctx(), i18n::t("Attach a file"), false);
        }
        if ui.data(|d| d.get_temp::<bool>(asker)) == Some(thread) && let Some(p) = crate::pick::done(i18n::t("Attach a file")) {
            *picked = Some(p);
        }
        // Voice and Send share one slot and cross-fade; the one fading out is inert.
        let has_text = !(draft.trim().is_empty() && attachment.is_none());
        let t = ui.ctx().animate_bool_with_time(nid("composer-send-fade"), has_text, 0.15);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(slot), egui::Sense::hover());
        if t < 1. && icon_at(ui, rect, &format!("composer-voice{salt}"), Icon::Microphone, 1. - t, !has_text, i18n::t("Voice note")).clicked() {
            *voice = Some(VoiceAct::Start);
        }
        if t > 0. && icon_at(ui, rect, &format!("composer-send{salt}"), Icon::PaperPlaneRight, t, has_text, i18n::t("Send")).clicked() {
            send_click = true;
        }
        });
        });
    });
    // A native insertion is first validated and attached by the session after this call.
    // Never send a caption before that validation succeeds in the same frame.
    if !inserting_image && (enter || send_click) && (attachment.is_some() || !draft.trim().is_empty()) {
        return Some(std::mem::take(draft));
    }
    None
}

fn insert_emoji(ctx: &egui::Context, id: egui::Id, draft: &mut String, emoji: &str) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let len = draft.chars().count();
    let (a, b) = state.cursor.char_range().map_or((len, len), |r| {
        let (p, q): (usize, usize) = (r.primary.index.into(), r.secondary.index.into());
        (p.min(q).min(len), p.max(q).min(len))
    });
    let byte = |n| draft.char_indices().nth(n).map_or(draft.len(), |(i, _)| i);
    draft.replace_range(byte(a)..byte(b), emoji);
    let end = a + emoji.chars().count();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Image events belong only to the focused composer. Text paste remains in egui's event stream.
fn take_image_input(ui: &egui::Ui, id: egui::Id, mode: Mode, occupied: bool, draft: &mut String) -> Option<Result<egui::ColorImage, String>> {
    if !ui.memory(|m| m.has_focus(id)) { return None; }
    let mut images = Vec::new();
    ui.input_mut(|i| i.events.retain(|event| {
        if let egui::Event::PasteImage(image) = event {
            images.push(image.clone());
            false
        } else { true }
    }));
    let native = ui.data_mut(|d| d.remove_temp::<(Option<egui::Id>, Vec<Result<(Vec<u8>, Option<String>), String>>)>(egui::Id::new("winit_image_insertions")));
    let mut native = native.filter(|(owner, _)| *owner == Some(id)).map(|(_, items)| items).unwrap_or_default();
    let count = images.len() + native.len();
    if count == 0 { return None; }
    if mode == Mode::Edit { return Some(Err("Image cannot be attached".to_owned())); }
    if occupied || count != 1 { return Some(Err("Only one attachment is supported".to_owned())); }
    if let Some(result) = native.pop() {
        return Some(result.and_then(|(bytes, description)| {
            let image = matrix_desktop::composer_image::decode(&bytes).map_err(|_| "Image cannot be attached".to_owned())?;
            // A flattened adaptive glyph becomes one ordinary attachment. Preserve its
            // accessibility description as editable caption text, never as a filesystem path.
            if let Some(description) = description {
                let caption: String = description.chars().filter(|c| {
                    !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                }).collect();
                let caption = caption.trim();
                if !caption.is_empty() {
                    if !draft.is_empty() { draft.push('\n'); }
                    draft.push_str(caption);
                }
            }
            Ok(image)
        }));
    }
    let image = images.pop().unwrap();
    let pixels = image.size[0].checked_mul(image.size[1]);
    if pixels.is_none_or(|p| p == 0 || p > matrix_desktop::composer_image::MAX_IMAGE_PIXELS || p != image.pixels.len()) {
        return Some(Err("Image cannot be attached".to_owned()));
    }
    Some(Ok(image.as_ref().clone()))
}

#[derive(Clone)]
struct Thumbnail {
    revision: u64,
    texture: Option<egui::TextureHandle>,
}

fn attachment_thumbnail(ui: &mut egui::Ui, id: egui::Id, data: &[u8], revision: u64) {
    let mut cached = ui.data(|d| d.get_temp::<Thumbnail>(id));
    if cached.as_ref().is_none_or(|c| c.revision != revision) {
        let texture = matrix_desktop::composer_image::thumbnail(data).map(|image| ui.ctx().load_texture("composer-attachment", image, Default::default()));
        cached = Some(Thumbnail { revision, texture });
        ui.data_mut(|d| d.insert_temp(id, cached.clone().unwrap()));
    }
    if let Some(texture) = cached.and_then(|c| c.texture) {
        ui.add(egui::Image::new(&texture).max_size(egui::vec2(64., 64.)));
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    type Batch = (Option<egui::Id>, Vec<Result<(Vec<u8>, Option<String>), String>>);

    fn image_input(salt: &str, focus: &str, mode: Mode, occupied: bool, events: Vec<egui::Event>, native: Option<Batch>) -> (Option<Result<egui::ColorImage, String>>, bool) {
        let ctx = crate::icons::test_context();
        let mut result = None;
        let mut text_paste = false;
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            ui.memory_mut(|m| m.request_focus(egui::Id::new(focus)));
            if let Some(batch) = native.clone() {
                ui.data_mut(|d| d.insert_temp(egui::Id::new("winit_image_insertions"), batch));
            }
            result = take_image_input(ui, egui::Id::new(format!("composer-text{salt}")), mode, occupied, &mut String::new());
            text_paste = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        });
        output.textures_delta.clear();
        (result, text_paste)
    }

    fn image() -> egui::ColorImage { egui::ColorImage::filled([2, 1], egui::Color32::RED) }
    fn event() -> egui::Event { egui::Event::PasteImage(std::sync::Arc::new(image())) }

    #[test]
    fn image_paste_requires_the_focused_room_or_thread_composer_and_preserves_text_paste() {
        for (salt, focus) in [("", "composer-text"), (THREAD, "composer-text-thread")] {
            let (result, text) = image_input(salt, focus, Mode::Send, false, vec![event(), egui::Event::Paste("caption".into())], None);
            assert_eq!(result.unwrap().unwrap().size, [2, 1]);
            assert!(text);
        }
        for focus in ["password", "search", "composer-text-thread"] {
            assert!(image_input("", focus, Mode::Send, false, vec![event()], None).0.is_none());
        }
    }

    #[test]
    fn native_png_is_routed_to_its_owner_and_errors_are_explicit() {
        let png = matrix_desktop::composer_image::prepare(&image(), 1024).unwrap();
        let owner = egui::Id::new("composer-text");
        let input = |items| image_input("", "composer-text", Mode::Send, false, vec![], Some((Some(owner), items))).0.unwrap();
        assert_eq!(input(vec![Ok((png.clone(), None))]).unwrap().size, [2, 1]);
        assert_eq!(input(vec![Err("Image is too large".into())]).err().as_deref(), Some("Image is too large"));
        assert_eq!(input(vec![Ok((b"invalid PNG".to_vec(), None))]).err().as_deref(), Some("Image cannot be attached"));
        assert!(image_input(THREAD, "composer-text-thread", Mode::Send, false, vec![], Some((Some(owner), vec![Ok((png, None))]))).0.is_none());
    }

    #[test]
    fn multiple_images_existing_attachment_and_edit_are_refused() {
        let error = |mode, occupied, events| image_input("", "composer-text", mode, occupied, events, None).0.unwrap().err().unwrap();
        assert_eq!(error(Mode::Send, false, vec![event(), event()]), "Only one attachment is supported");
        assert_eq!(error(Mode::Reply, true, vec![event()]), "Only one attachment is supported");
        assert_eq!(error(Mode::Edit, false, vec![event()]), "Image cannot be attached");
    }

    #[test]
    fn image_and_send_in_same_frame_keep_caption_until_attachment_is_validated() {
        let ctx = crate::icons::test_context();
        let mut draft = "caption".to_owned();
        for frame in 0..4 {
            let mut events = vec![];
            if frame == 3 {
                events.push(event());
                events.push(egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            }
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.))), events, ..Default::default() }, |ui| {
                ui.memory_mut(|m| m.request_focus(egui::Id::new("composer-text")));
                let (mut cancel, mut drop, mut poll, mut picked, mut pasted, mut voice) = (false, false, false, None, None, None);
                let sent = show(ui, "", &mut draft, Mode::Send, None, &mut cancel, &mut drop, &mut poll, &mut picked, &mut pasted, None, &mut voice, &[], &mut vec![], None, &matrix_desktop::LIGHT);
                assert!(sent.is_none());
                if frame == 3 { assert!(pasted.is_some()); }
            });
            output.textures_delta.clear();
        }
        assert_eq!(draft, "caption");
    }
}

/// «@name»: members matching what is typed after the `@` before the cursor; a click (or Tab) puts «@Name » in the text
/// and remembers who it is, so the send turns it into a real mention.
fn mention_popup(ui: &mut egui::Ui, id: egui::Id, out: &egui::text_edit::TextEditOutput, draft: &mut String, members: &[matrix_engine::operations::Member], mentions: &mut Vec<matrix_engine::sending::Mention>, t: &matrix_desktop::DesignTokens) {
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
    let duplicates = crate::rows::duplicate_names(matches.iter().map(|m| m.name.as_str()));
    let mut pick = None;
    let corner = if crate::dir::rtl() { out.response.rect.right_top() } else { out.response.rect.left_top() };
    egui::Area::new(id.with("mentions")).order(egui::Order::Foreground).pivot(crate::dir::align(egui::Align2::LEFT_TOP)).fixed_pos(corner - egui::vec2(0., 8. + crate::rows::ROW * matches.len() as f32)).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(260.);
            for (m, duplicate) in matches.iter().zip(duplicates) {
                if ui.push_id(&m.user_id, |ui| crate::rows::person_row(ui, &m.name, &m.user_id, duplicate, false, t)).inner.clicked() {
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

    /// Height of the composer bar after laying it out in a window `width` wide, reusing `ctx` so panel memory carries over.
    fn bar_height(ctx: &eframe::egui::Context, width: f32) -> f32 {
        bar_height_with(ctx, width, "")
    }

    fn bar_height_with(ctx: &eframe::egui::Context, width: f32, text: &str) -> f32 {
        use eframe::egui;
        let mut h = 0.;
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.))), ..Default::default() };
        for _ in 0..4 {
            let mut out = ctx.run_ui(input.clone(), |ui| {
                let r = super::panel(egui::Frame::NONE).show(ui, |ui| {
                    let (mut draft, mut a, mut b, mut c, mut d, mut e, mut f) = (text.to_owned(), false, false, false, None, None, None);
                    super::show(ui, "", &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new(), None, &matrix_desktop::LIGHT);
                });
                h = r.response.rect.height();
            });
            out.textures_delta.clear();
        }
        h
    }

    #[test]
    fn composer_height_returns_to_one_row_after_a_narrow_window() {
        let ctx = crate::icons::test_context();
        let wide = bar_height(&ctx, 900.);
        assert!(wide <= 44., "single row bar too tall: {wide}");
        bar_height(&ctx, 120.);
        assert_eq!(bar_height(&ctx, 900.), wide);
    }

    #[test]
    fn composer_grows_with_newlines_up_to_a_cap_then_shrinks_back() {
        let h = |text: &str| bar_height_with(&crate::icons::test_context(), 900., text);
        let (one, three, many, more) = (h(""), h("a\nb\nc"), h(&"x\n".repeat(40)), h(&"x\n".repeat(80)));
        assert!(three > one && many > three, "{one} {three} {many} {more}");
        assert_eq!(many, more, "capped, scrolls inside");
        // Back to one row on the same context (no remembered height).
        let ctx = crate::icons::test_context();
        bar_height_with(&ctx, 900., "a\nb\nc");
        assert_eq!(bar_height(&ctx, 900.), one);
    }

    /// Runs the composer for a few frames with the text box focused; `events` go in the last frame. Returns (sent, draft, hint-text centre y, icon centre y, panel right, mic right).
    fn run(text: &str, events: Vec<eframe::egui::Event>) -> (Option<String>, String, f32, f32, f32, f32, f32, f32) {
        use eframe::egui;
        let ctx = crate::icons::test_context();
        let mut draft = text.to_owned();
        let (mut sent, mut hint_y, mut panel_right, mut panel_bottom) = (None, 0., 0., 0.);
        for frame in 0..5 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.))), events: if frame == 4 { events.clone() } else { vec![] }, ..Default::default() };
            if frame == 3 {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text")));
            }
            let mut out = ctx.run_ui(input, |ui| {
                let r = super::panel(egui::Frame::NONE).show(ui, |ui| {
                    let (mut a, mut b, mut c, mut d, mut e, mut f) = (false, false, false, None, None, None);
                    sent = super::show(ui, "", &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new(), None, &matrix_desktop::LIGHT);
                });
                panel_right = r.response.rect.right();
                panel_bottom = r.response.rect.bottom();
            });
            out.textures_delta.clear();
            for s in &out.shapes {
                if let egui::epaint::Shape::Text(t) = &s.shape {
                    if t.galley.text() == matrix_desktop::i18n::t("Write a message") {
                        hint_y = (t.pos + t.galley.rect.center().to_vec2()).y;
                    }
                }
            }
        }
                let mic = ctx.read_response(egui::Id::new("composer-voice")).or_else(|| ctx.read_response(egui::Id::new("composer-send"))).unwrap().rect;
        (sent, draft, hint_y, mic.center().y, panel_right, mic.right(), panel_bottom, mic.bottom())
    }

    fn enter(shift: bool) -> Vec<eframe::egui::Event> {
        let modifiers = eframe::egui::Modifiers { shift, ..Default::default() };
        vec![eframe::egui::Event::Key { key: eframe::egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers }]
    }

    #[test]
    fn shift_enter_adds_a_line_and_plain_enter_sends() {
        let (sent, draft, ..) = run("ab", enter(true));
        assert_eq!((sent, draft.as_str()), (None, "ab\n"));
        let (sent, ..) = run("ab", enter(false));
        assert_eq!(sent.as_deref(), Some("ab"));
    }

    #[test]
    fn single_row_text_and_icons_share_a_centre_line_with_a_small_right_margin() {
        let (_, _, text_y, icon_y, panel_right, mic_right, panel_bottom, mic_bottom) = run("", vec![]);
        assert!((text_y - icon_y).abs() <= 1., "text {text_y} icons {icon_y}");
        assert!((6. ..=16.).contains(&(panel_right - mic_right)), "right gap {}", panel_right - mic_right);
        let (_, _, _, _, _, _, tall_bottom, tall_icon_bottom) = run("a\nb\nc", vec![]);
        assert!((panel_bottom - mic_bottom - (tall_bottom - tall_icon_bottom)).abs() <= 1., "multiline icons must stay at the bottom");
    }

    /// The thread bar is the room bar: microphone in the last slot while empty, send in the same slot with text, emoji and attach where they were.
    #[test]
    fn thread_bar_swaps_microphone_and_send_in_one_slot_like_the_room_bar() {
        use eframe::egui;
        let rects = |text: &str| {
            let ctx = crate::icons::test_context();
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320., 600.))), predicted_dt: 1., ..Default::default() };
            for _ in 0..4 {
                let mut out = ctx.run_ui(input.clone(), |ui| {
                    let (mut draft, mut a, mut b, mut c, mut d, mut e, mut f) = (text.to_owned(), false, false, false, None, None, None);
                    super::show(ui, super::THREAD, &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new(), None, &matrix_desktop::LIGHT);
                });
                out.textures_delta.clear();
            }
            let r = |id: &str| ctx.read_response(egui::Id::new(id)).map(|r| (r.rect, r.sense.senses_click()));
            (r("composer-emoji-thread"), r("composer-attach-thread"), r("composer-voice-thread"), r("composer-send-thread"), r("composer-poll"))
        };
        let (emoji, attach, voice, send, poll) = rects("");
        let (emoji2, attach2, voice2, send2, _) = rects("ciao");
        assert!(poll.is_none(), "no poll in a thread");
        assert_eq!((emoji.unwrap().0, attach.unwrap().0), (emoji2.unwrap().0, attach2.unwrap().0), "the other icons do not move");
        let (mic, plane) = (voice.unwrap(), send2.unwrap());
        assert!(mic.1 && plane.1, "microphone clickable when empty, send with text");
        assert_eq!(mic.0, plane.0, "one slot for both");
        assert!(send.is_none_or(|s| !s.1) && voice2.is_none_or(|v| !v.1), "the faded one is inert");
        assert!(320. - mic.0.right() <= 16., "no empty slot at the right edge");
    }

    #[test]
    fn pasted_image_is_a_png_with_unpremultiplied_alpha() {
        let image = eframe::egui::ColorImage::from_rgba_unmultiplied([1, 1], &[200, 100, 50, 128]);
        let png = matrix_desktop::composer_image::prepare(&image, 25 << 20).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (1, 1));
        assert_eq!(decoded.as_raw()[3], 128);
        assert!((decoded.as_raw()[0] as i16 - 200).abs() <= 1);
    }

    #[test]
    fn emoji_replaces_selection_and_advances_cursor_by_chars() {
        use eframe::egui::{self, text::{CCursor, CCursorRange}};
        let ctx = crate::icons::test_context();
        let id = egui::Id::new("composer-text");
        let mut state = egui::text_edit::TextEditState::default();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(3), CCursor::new(1))));
        state.store(&ctx, id);
        let mut text = "èabc".to_owned();
        super::insert_emoji(&ctx, id, &mut text, "👩‍💻");
        assert_eq!(text, "è👩‍💻c");
        let range = egui::TextEdit::load_state(&ctx, id).unwrap().cursor.char_range().unwrap();
        assert_eq!(usize::from(range.primary.index), 4);
        assert_eq!(usize::from(range.secondary.index), 4);
        super::insert_emoji(&ctx, id, &mut text, "❤️");
        assert_eq!(text, "è👩‍💻❤️c");
        let range = egui::TextEdit::load_state(&ctx, id).unwrap().cursor.char_range().unwrap();
        assert_eq!(usize::from(range.primary.index), 6);
    }

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

/// A composer icon button in `rect` at opacity `alpha`; clickable only when `active`. The send icon takes the link colour.
fn icon_at(ui: &mut egui::Ui, rect: egui::Rect, id: &str, icon: Icon, alpha: f32, active: bool, label: &str) -> egui::Response {
    let color = id.starts_with("composer-send").then(|| ui.visuals().hyperlink_color);
    let resp = crate::icons::button_at(ui, rect, egui::Id::new(id), icon, id.starts_with("composer-send"), crate::metrics::get().icon, label, color, active, alpha);
    if active { resp.on_hover_text(label) } else { resp }
}
