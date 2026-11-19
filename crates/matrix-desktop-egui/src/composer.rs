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
pub fn show(ui: &mut egui::Ui, draft: &mut String, mode: Mode, attachment: Option<&str>, cancel: &mut bool, drop_attachment: &mut bool, poll: &mut bool, picked: &mut Option<std::path::PathBuf>, pasted: &mut Option<egui::ColorImage>, recording: Option<u64>, voice: &mut Option<VoiceAct>, members: &[matrix_engine::operations::Member], mentions: &mut Vec<matrix_engine::sending::Mention>) -> Option<String> {
    // Taken before the editor sees it, so a bare Enter never inserts a newline.
    // consume_key matches Shift loosely, so Shift+Enter is checked on the event and left to the TextEdit (it inserts the newline).
    let shift_enter = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.shift)));
    let enter = !shift_enter && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    // The Enter that confirms an IME / dead-key composition must not send: ignore it while composing and in the frame of the IME event.
    let ime_id = egui::Id::new("composer-ime");
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
        ui.horizontal(|ui| {
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
    // Not horizontal_centered: it fills the panel's height from the previous frame, so the bar never shrank after a narrow-window wrap.
    ui.horizontal_top(|ui| {
        ui.add_space(12.);
        // Same order as GPUI, all on the right: text, emoji, poll, attach, then the mic/send slot (32 px each).
        let w = (ui.available_width() - 4. * 32. - 5. * ui.spacing().item_spacing.x - 4.).max(80.); // 4: tuned by the right-gap test (icon box ends ~12 px from the panel edge, like the left pad); the row's spacings and the TextEdit's own inner margins (else the row overflows a narrow column)
        let id = egui::Id::new("composer-text");
        if ui.memory(|m| m.has_focus(id)) {
            *pasted = ui.input(|i| i.events.iter().find_map(|e| match e {
                egui::Event::PasteImage(image) => Some(image.as_ref().clone()),
                _ => None,
            }));
        }
        format_shortcuts(ui, id, draft);
        // Borderless like Telegram; grows with the text up to 8 rows, then scrolls. The 6 px vertical padding makes one row as tall as the 32 px icons.
        let row = ui.text_style_height(&egui::TextStyle::Body);
        let pad = ((32. - row) / 2.).max(0.); // centres one text row on the 32 px icons
        let max_h = 8. * row + 2. * pad;
        // A child ui with its own max height: inside a horizontal row the available height is one row, which would cap the scroll area.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(ui.cursor().min, egui::vec2(w + 8., max_h))));
        let scroll = egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(0.).id_salt("composer-scroll").show(&mut child, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, pad as i8)).show(ui, |ui| {
                ui.spacing_mut().interact_size.y = row; // else the editor is 20 px tall and the text sits at its top
                egui::TextEdit::multiline(draft).id(id).desired_rows(1).hint_text("Scrivi un messaggio").desired_width(w).lock_focus(true).frame(egui::Frame::NONE).margin(egui::Margin::ZERO).show(ui)
            }).inner
        });
        let text_h = scroll.content_size.y.min(max_h);
        ui.allocate_space(egui::vec2(w + 8., text_h));
        let out = scroll.inner;
        mention_popup(ui, id, &out, draft, members, mentions);
        // The 32 emoji of the GPUI picker; one is appended to the text and the box keeps the focus.
        // Icons stay at the bottom of a tall box.
        ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.; // else the space above adds a gap and the icons sit lower than the text
        ui.add_space((text_h - 32.).max(0.));
        ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(32., 32.), egui::Sense::hover());
        let emoji = icon_at(ui, rect, "composer-emoji", &emoji_strokes(), 1., true, "Emoji");
        egui::Popup::menu(&emoji).show(|ui| {
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
        let (rect, _) = ui.allocate_exact_size(egui::vec2(32., 32.), egui::Sense::hover());
        if icon_at(ui, rect, "composer-poll", &poll_strokes(), 1., true, "Sondaggio").clicked() {
            *poll = true;
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(32., 32.), egui::Sense::hover());
        if icon_at(ui, rect, "composer-attach", &attach_strokes(), 1., true, "Allega un file").clicked() {
            *picked = crate::pick::file("Allega un file", false);
        }
        // Voice and Send share one slot and cross-fade; the one fading out is inert.
        let has_text = !(draft.trim().is_empty() && attachment.is_none());
        let t = ui.ctx().animate_bool_with_time(egui::Id::new("composer-send-fade"), has_text, 0.15);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(32., 32.), egui::Sense::hover());
        if t < 1. && icon_at(ui, rect, "composer-voice", &voice_strokes(), 1. - t, !has_text, "Nota vocale").clicked() {
            *voice = Some(VoiceAct::Start);
        }
        if t > 0. && icon_at(ui, rect, "composer-send", &send_strokes(), t, has_text, "Invia").clicked() {
            send_click = true;
        }
        });
        });
    });
    if (enter || send_click) && (attachment.is_some() || !draft.trim().is_empty()) {
        return Some(std::mem::take(draft));
    }
    None
}

pub fn pasted_png(image: &egui::ColorImage) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let width = u32::try_from(image.size[0]).ok()?;
    let height = u32::try_from(image.size[1]).ok()?;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(&rgba, width, height, image::ExtendedColorType::Rgba8).ok()?;
    Some(png)
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
                let r = egui::Panel::bottom("composer").min_size(56.).show(ui, |ui| {
                    let (mut draft, mut a, mut b, mut c, mut d, mut e, mut f) = (text.to_owned(), false, false, false, None, None, None);
                    super::show(ui, &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new());
                });
                h = r.response.rect.height();
            });
            out.textures_delta.clear();
        }
        h
    }

    #[test]
    fn composer_height_returns_to_one_row_after_a_narrow_window() {
        let ctx = eframe::egui::Context::default();
        let wide = bar_height(&ctx, 900.);
        bar_height(&ctx, 120.);
        assert_eq!(bar_height(&ctx, 900.), wide);
    }

    #[test]
    fn composer_grows_with_newlines_up_to_a_cap_then_shrinks_back() {
        let h = |text: &str| bar_height_with(&eframe::egui::Context::default(), 900., text);
        let (one, three, many, more) = (h(""), h("a\nb\nc"), h(&"x\n".repeat(40)), h(&"x\n".repeat(80)));
        assert!(three > one && many > three, "{one} {three} {many} {more}");
        assert_eq!(many, more, "capped, scrolls inside");
        // Back to one row on the same context (no remembered height).
        let ctx = eframe::egui::Context::default();
        bar_height_with(&ctx, 900., "a\nb\nc");
        assert_eq!(bar_height(&ctx, 900.), one);
    }

    /// Runs the composer for a few frames with the text box focused; `events` go in the last frame. Returns (sent, draft, hint-text centre y, icon centre y, panel right, mic right).
    fn run(text: &str, events: Vec<eframe::egui::Event>) -> (Option<String>, String, f32, f32, f32, f32) {
        use eframe::egui;
        let ctx = egui::Context::default();
        let mut draft = text.to_owned();
        let (mut sent, mut hint_y, mut panel_right) = (None, 0., 0.);
        for frame in 0..5 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.))), events: if frame == 4 { events.clone() } else { vec![] }, ..Default::default() };
            if frame == 3 {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("composer-text")));
            }
            let mut out = ctx.run_ui(input, |ui| {
                let r = egui::Panel::bottom("composer").min_size(56.).show(ui, |ui| {
                    let (mut a, mut b, mut c, mut d, mut e, mut f) = (false, false, false, None, None, None);
                    sent = super::show(ui, &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new());
                });
                panel_right = r.response.rect.right();
            });
            out.textures_delta.clear();
            for s in &out.shapes {
                if let egui::epaint::Shape::Text(t) = &s.shape {
                    if t.galley.text() == "Scrivi un messaggio" {
                        hint_y = (t.pos + t.galley.rect.center().to_vec2()).y;
                    }
                }
            }
        }
                let mic = ctx.read_response(egui::Id::new("composer-voice")).or_else(|| ctx.read_response(egui::Id::new("composer-send"))).unwrap().rect;
        (sent, draft, hint_y, mic.center().y, panel_right, mic.right())
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
        let (_, _, text_y, icon_y, panel_right, mic_right) = run("", vec![]);
        assert!((text_y - icon_y).abs() <= 1., "text {text_y} icons {icon_y}");
        assert!((6. ..=16.).contains(&(panel_right - mic_right)), "right gap {}", panel_right - mic_right);
    }

    #[test]
    fn pasted_image_is_a_png_with_unpremultiplied_alpha() {
        let image = eframe::egui::ColorImage::from_rgba_unmultiplied([1, 1], &[200, 100, 50, 128]);
        let png = super::pasted_png(&image).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (1, 1));
        assert_eq!(decoded.as_raw()[3], 128);
        assert!((decoded.as_raw()[0] as i16 - 200).abs() <= 1);
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

// Glyph data of crates/matrix-desktop/src/composer_icons.rs (20x20 grid, painted at 1.2x, 1.6 px stroke); that file needs gpui, so it is copied here.
type Strokes = Vec<Vec<egui::Pos2>>;

fn arc(c: (f32, f32), r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    (0..=16).map(|i| { let a = a0 + (a1 - a0) * i as f32 / 16.; (c.0 + r * a.cos(), c.1 + r * a.sin()) }).collect()
}

fn pts(v: Vec<Vec<(f32, f32)>>) -> Strokes {
    v.into_iter().map(|l| l.into_iter().map(|(x, y)| egui::pos2(x, y)).collect()).collect()
}

fn attach_strokes() -> Strokes {
    use std::f32::consts::PI;
    let mut clip = vec![(9., 12.), (9., 6.)];
    clip.extend(arc((11., 6.), 2., PI, 2. * PI));
    clip.push((13., 13.));
    clip.extend(arc((10., 13.), 3., 0., PI));
    clip.push((7., 7.));
    pts(vec![clip])
}

fn voice_strokes() -> Strokes {
    use std::f32::consts::PI;
    let mut body = arc((10., 5.5), 2.5, PI, 2. * PI);
    body.extend(arc((10., 9.), 2.5, 0., PI));
    body.push((7.5, 5.5));
    pts(vec![body, arc((10., 9.), 5.5, 0., PI), vec![(10., 14.5), (10., 17.5)], vec![(7., 17.5), (13., 17.5)]])
}

fn send_strokes() -> Strokes {
    pts(vec![vec![(10., 16.), (10., 4.)], vec![(4.5, 9.5), (10., 4.), (15.5, 9.5)]])
}

fn poll_strokes() -> Strokes {
    pts(vec![vec![(4., 5.), (14., 5.)], vec![(4., 10.), (16., 10.)], vec![(4., 15.), (10., 15.)]])
}

fn emoji_strokes() -> Strokes {
    use std::f32::consts::PI;
    pts(vec![arc((10., 10.), 7.5, 0., 2. * PI), vec![(7.5, 8.), (7.5, 9.)], vec![(12.5, 8.), (12.5, 9.)], arc((10., 10.), 4., 0.25 * PI, 0.75 * PI)])
}

/// Paints `lines` centred (bounding box) in `rect` at opacity `alpha`; clickable only when `active`.
fn icon_at(ui: &mut egui::Ui, rect: egui::Rect, id: &str, lines: &Strokes, alpha: f32, active: bool, label: &str) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::new(id), if active { egui::Sense::click() } else { egui::Sense::hover() });
    let resp = if active { resp.on_hover_text(label) } else { resp };
    let all = || lines.iter().flatten();
    let (mut lo, mut hi) = (egui::pos2(f32::MAX, f32::MAX), egui::pos2(f32::MIN, f32::MIN));
    for p in all() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let origin = rect.center() - (lo.to_vec2() + hi.to_vec2()) / 2. * 1.2;
    let visuals = ui.visuals();
    let base = if id == "composer-send" { visuals.hyperlink_color } else { visuals.widgets.inactive.fg_stroke.color };
    let color = if resp.hovered() && active { visuals.strong_text_color() } else { base }.gamma_multiply(alpha);
    let painter = ui.painter();
    if resp.hovered() && active {
        painter.rect_filled(rect, 6., visuals.widgets.hovered.weak_bg_fill.gamma_multiply(alpha));
    }
    for line in lines {
        painter.add(egui::Shape::line(line.iter().map(|p| origin + p.to_vec2() * 1.2).collect(), egui::Stroke::new(1.6, color)));
    }
    resp
}
