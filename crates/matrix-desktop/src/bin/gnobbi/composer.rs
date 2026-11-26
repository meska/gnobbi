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
pub fn show(ui: &mut egui::Ui, salt: &str, draft: &mut String, mode: Mode, attachment: Option<(&str, &[u8], u64)>, cancel: &mut bool, drop_attachment: &mut bool, poll: &mut bool, picked: &mut Option<std::path::PathBuf>, pasted: &mut Option<Pasted>, recording: Option<u64>, voice: &mut Option<VoiceAct>, members: &[matrix_engine::operations::Member], mentions: &mut Vec<matrix_engine::sending::Mention>, mut stickers: Option<(&mut crate::stickers::Panel, &crate::avatar::Avatars)>, t: &matrix_desktop::DesignTokens) -> Option<String> {
    let avatars = stickers.as_ref().map(|(_, a)| *a);
    // The suggestions take Up/Down/Enter/Tab/Esc first, so Enter picks instead of sending.
    crate::mention_popup::keys(ui, egui::Id::new(format!("composer-text{salt}")), draft, members, mentions);
    // Taken before the editor sees it, so a bare Enter never inserts a newline.
    // consume_key matches Shift loosely, so Shift+Enter is checked on the event and left to the TextEdit (it inserts the newline).
    let shift_enter = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.shift)));
    let nid = |base: &str| egui::Id::new(format!("{base}{salt}"));
    let thread = !salt.is_empty();
    // Two composers on screen: Enter belongs to the focused one; the room one also keeps it when nothing is focused.
    let focus = ui.memory(|m| m.focused());
    let mine = focus == Some(nid("composer-text")) || (!thread && focus != Some(egui::Id::new(format!("composer-text{THREAD}"))));
    // On touch there is no Shift: Enter stays with the TextEdit (newline) and only the Send button sends.
    let enter = mine && !shift_enter && !crate::mobile::touch() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
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
    if let Some(gif) = take_gif_input(ui, nid("composer-text"), mode != Mode::Edit && recording.is_none() && attachment.is_none()) {
        *pasted = Some(Pasted::Gif(gif));
    }
    let image_input = take_image_input(ui, nid("composer-text"), if recording.is_some() { Mode::Edit } else { mode }, attachment.is_some(), draft);
    let inserting_image = image_input.is_some();
    if let Some(result) = image_input {
        match result {
            Ok(image) => {
                *pasted = Some(Pasted::Picture(image));
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
            let picture = attachment_thumbnail(ui, nid("composer-thumbnail"), data, revision);
            // The chip never outgrows the bar: the name is cut to what is left after the icon and the buttons that follow, so Send stays on screen.
            let m = crate::metrics::get();
            let gap = ui.spacing().item_spacing.x;
            let buttons = 1 + (picture && stickers.is_some()) as usize;
            let room = ui.available_width() - (m.icon + m.gap + 2. * gap) - buttons as f32 * (crate::metrics::button() + gap) - 12.;
            let font = egui::FontId::proportional(m.body);
            let shown = fit_name(name, |s| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE).size().x <= room);
            ui.label(crate::icons::with_label(crate::icons::Icon::Paperclip, &shown));
            // A picture (pasted, picked, or a Memoji from the iOS keyboard) can become a sticker instead of a message.
            if picture && let Some((panel, _)) = stickers.as_mut() && crate::icons::button(ui, Icon::Sticker, false, i18n::t("Save as sticker")).clicked() {
                panel.add_picture(data, &std::path::Path::new(name).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                *drop_attachment = true;
            }
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
    let gap = ui.spacing().item_spacing.x;
    // Wide desktop (room composer only): one Emoji button; hover shows a small popup, click toggles the drawer above the bar.
    let extra = if thread { 0 } else { wide_extra_slots(crate::mobile::touch(), ui.available_width() - 12. - gap, slot, gap) };
    let dock_id = egui::Id::new(DOCK_ID);
    if extra == 1 && ui.data(|d| d.get_temp::<bool>(dock_id)).unwrap_or(false) {
        dock(ui, dock_id, nid("emoji-dock-scroll"), nid("emoji-dock-grid"), nid("composer-text"), draft, &mut stickers, t);
    }
    // Not horizontal_centered: it fills the panel's height from the previous frame, so the bar never shrank after a narrow-window wrap.
    ui.horizontal_top(|ui| {
        ui.add_space(12.);
        // All on the right: text, attach (menu), then the mic/send slot.
        let slots = 2. + extra as f32;
        let w = (ui.available_width() - slots * slot - (slots + 1.) * gap - 12.).max(80.); // Keep the right gap close to the 12 px left pad.
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
        crate::mention_popup::show(ui, id, out.response.rect, draft, members, mentions, avatars, t);
        // The picker restores focus to the composer after inserting at its saved selection.
        // Icons stay at the bottom of a tall box.
        ui.with_layout(crate::dir::col(), |ui| {
        ui.spacing_mut().item_spacing.y = 0.; // else the space above adds a gap and the icons sit lower than the text
        ui.add_space((text_h - slot).max(0.));
        ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(slot), egui::Sense::hover());
        // Wide desktop: the emoji button sits before the attach one.
        let (rect, wide_emoji) = if extra > 0 { (ui.allocate_exact_size(egui::Vec2::splat(slot), egui::Sense::hover()).0, Some(rect)) } else { (rect, None) };
        // One button opens a menu with the rest: emoji, poll, attach. Keeps the field wide at any text scale (Telegram keeps attach and send/mic only).
        let attach = icon_at(ui, rect, &format!("composer-attach{salt}"), Icon::Paperclip, 1., true, i18n::t("Attach a file"));
        // Invisible twin of the button: the emoji popup hangs from it, opened by the menu entry.
        let emoji = ui.interact(wide_emoji.unwrap_or(rect), egui::Id::new(format!("composer-emoji{salt}")), egui::Sense::hover());
        // The picker answers by its title, the same for both bars: remember which one asked.
        let asker = egui::Id::new("composer-attach-asker");
        let tab_id = egui::Id::new(crate::stickers::TAB_ID);
        let mut emoji_hover = None;
        if let Some(r) = wide_emoji {
            let button = icon_at(ui, r, &format!("composer-emoji-button{salt}"), Icon::Smiley, 1., true, i18n::t("Emoji"));
            if button.clicked() {
                let open = !ui.data(|d| d.get_temp::<bool>(dock_id)).unwrap_or(false);
                ui.data_mut(|d| d.insert_temp(dock_id, open));
                if open { open_popup(ui.ctx(), Popup::Drawer); }
            }
            emoji_hover = Some(button);
        }
        if attach.clicked() { open_popup(ui.ctx(), Popup::Attach); }
        egui::Popup::menu(&attach).show(|ui| {
            if extra == 0 && ui.button(i18n::t("Emoji")).clicked() {
                open_popup(ui.ctx(), Popup::Emoji);
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&emoji));
            }
            if !thread && ui.button(i18n::t("Poll")).clicked() {
                *poll = true;
                ui.close();
            }
            if ui.button(i18n::t("Attach a file")).clicked() {
                ui.data_mut(|d| d.insert_temp(asker, thread));
                crate::pick::start(ui.ctx(), i18n::t("Attach a file"), false);
                ui.close();
            }
        });
        // Stays open on clicks inside (tabs, search, pack bar); a picked emoji or sticker closes it itself.
        let mut popup = egui::Popup::menu(&emoji).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        // One box for both tabs, so switching tabs moves and resizes nothing.
        let safe = ui.ctx().content_rect();
        let width = crate::stickers::width(safe.width());
        let x = (emoji.rect.center().x - width / 2.).clamp(safe.left() + 4., (safe.right() - width - 16.).max(safe.left() + 4.));
        popup = popup.anchor(egui::Rect::from_min_max(egui::pos2(x, emoji.rect.top()), egui::pos2(x, emoji.rect.bottom())));
        popup.show(|ui| crate::dir::popup(ui, |ui| {
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
                crate::stickers::show(ui, panel, textures, t, crate::stickers::width(ui.ctx().content_rect().width()), crate::stickers::ROWS);
                return;
            }
            let width = crate::stickers::width(ui.ctx().content_rect().width());
            if emoji_grid(ui, nid("emoji-scroll"), nid("emoji-grid"), id, draft, t, width, crate::stickers::body_height(width, crate::stickers::ROWS, ui.spacing().item_spacing.y)) {
                ui.close();
            }
        }));
        // Wide desktop only (the button does not exist on touch): hovering the Emoji button shows the grid alone; clicking it toggles the drawer.
        if let Some(button) = emoji_hover {
            let hid = nid("emoji-hover");
            let now = ui.input(|i| i.time);
            let drawer = ui.data(|d| d.get_temp::<bool>(dock_id)).unwrap_or(false);
            let (open, last) = ui.data(|d| d.get_temp::<(bool, f64)>(hid)).unwrap_or((false, 0.));
            let (mut open, last) = hover_step((open, last), button.hovered(), drawer, now);
            if open {
                open_popup(ui.ctx(), Popup::EmojiHover);
                let safe = ui.ctx().content_rect();
                let (cols, cell) = (POPUP_COLUMNS, POPUP_CELL);
                let (_, height) = popup_rows(matrix_desktop::EMOJI_SET.len(), cols, cell);
                // Same frame as the context menus; its 8 px margin is the same on all four sides.
                let frame = egui::Frame::popup(ui.style()).inner_margin(8).corner_radius(10).stroke(egui::Stroke::NONE).fill(crate::list::rgb(t.surface));
                let area = egui::Area::new(hid).order(egui::Order::Foreground).pivot(egui::Align2::RIGHT_BOTTOM).fixed_pos(egui::pos2(button.rect.right().min(safe.right() - 4.), button.rect.top() - 2.)).show(ui.ctx(), |ui| {
                    frame.show(ui, |ui| crate::dir::popup(ui, |ui| {
                        ui.spacing_mut().scroll.floating = true;
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        ui.set_width(cols as f32 * cell);
                        ui.set_min_height(height); // whole rows: the area must not shrink them
                        egui::ScrollArea::vertical().id_salt(nid("emoji-hover-scroll")).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
                            emoji_cells(ui, nid("emoji-hover-grid"), cols, egui::Vec2::splat(cell), id, draft, t)
                        }).inner
                    })).inner
                });
                let on_popup = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| area.response.rect.union(button.rect).contains(p));
                let (still, last) = hover_step((true, last), on_popup, false, now);
                open = still && !area.inner;
                ui.data_mut(|d| d.insert_temp(hid, (open, last)));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            } else {
                ui.data_mut(|d| d.insert_temp(hid, (false, last)));
            }
        }
        if ui.data(|d| d.get_temp::<bool>(asker)) == Some(thread) && let Some(p) = crate::pick::done(i18n::t("Attach a file")) {
            *picked = Some(p);
        }
        // Voice and Send share one slot and cross-fade; the one fading out is inert.
        let has_text = !(draft.trim().is_empty() && attachment.is_none());
        let t = ui.ctx().animate_bool_with_time(nid("composer-send-fade"), has_text, 0.15);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(slot), egui::Sense::hover());
        if t < 1. && icon_at(ui, rect, &format!("composer-voice{salt}"), Icon::Microphone, 1. - t, !has_text, i18n::t("Voice message")).clicked() {
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

/// Ctx flag: the Emoji | Sticker drawer above the bar is open (mirrored to the preferences by the caller).
pub const DOCK_ID: &str = "emoji-dock-open";
const POPUP_COLUMNS: usize = 8;
const POPUP_CELL: f32 = 40.;
const POPUP_MAX_ROWS: usize = 4;
/// Sticker rows the docked drawer shows (half a row hints at the scroll); the popup shows `stickers::ROWS`.
const DOCK_ROWS: f32 = 2.5;

/// Emoji button shown beside the attach one: 0 on touch or when the field would fall under `MIN_WIDE_FIELD` with it, else 1.
fn wide_extra_slots(touch: bool, available: f32, slot: f32, gap: f32) -> usize {
    const MIN_WIDE_FIELD: f32 = 240.;
    let field = available - 3. * slot - 4. * gap - 12.;
    if touch || field < MIN_WIDE_FIELD { 0 } else { 1 }
}

/// Rows the hover popup shows (whole rows only, at most `POPUP_MAX_ROWS`, scroll beyond) and the height they take.
fn popup_rows(count: usize, columns: usize, cell: f32) -> (usize, f32) {
    let rows = count.div_ceil(columns).clamp(1, POPUP_MAX_ROWS);
    (rows, rows as f32 * cell)
}

/// `columns` x `cell` grid of the emoji set, no margins. True when an emoji was inserted.
fn emoji_cells(ui: &mut egui::Ui, grid_id: egui::Id, columns: usize, cell: egui::Vec2, id: egui::Id, draft: &mut String, t: &matrix_desktop::DesignTokens) -> bool {
    let glyph = egui::TextStyle::Body.resolve(ui.style()).size * 1.4;
    let mut inserted = false;
    egui::Grid::new(grid_id).spacing(egui::Vec2::ZERO).min_col_width(cell.x).min_row_height(cell.y).show(ui, |ui| { // else a column is the 48 px button size
        for (i, (name, emoji)) in matrix_desktop::EMOJI_SET.iter().enumerate() {
            let resp = crate::metrics::emoji_cell(ui, cell, glyph, name, emoji, false, false, t);
            if resp.on_hover_text(i18n::t(name)).clicked() {
                insert_emoji(ui.ctx(), id, draft, emoji);
                inserted = true;
            }
            if i % columns == columns - 1 { ui.end_row(); }
        }
    });
    inserted
}

/// The drawer docked above the bar: as wide as the bar, Emoji | Sticker tabs, one fixed height for both; X or Esc closes it.
#[allow(clippy::too_many_arguments)]
fn dock(ui: &mut egui::Ui, dock_id: egui::Id, scroll_id: egui::Id, grid_id: egui::Id, id: egui::Id, draft: &mut String, stickers: &mut Option<(&mut crate::stickers::Panel, &crate::avatar::Avatars)>, t: &matrix_desktop::DesignTokens) {
    let tab = egui::Id::new(crate::stickers::TAB_ID);
    let mut close = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    ui.separator();
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
        let w = ui.available_width();
        let mut sticker_tab = stickers.is_some() && ui.data(|d| d.get_temp::<bool>(tab)).unwrap_or(false);
        ui.horizontal(|ui| {
            if stickers.is_some() {
                ui.selectable_value(&mut sticker_tab, false, i18n::t("Emoji"));
                ui.selectable_value(&mut sticker_tab, true, i18n::t("Sticker"));
            }
            ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| close |= crate::info::close_icon(ui, i18n::t("Close")).clicked());
        });
        ui.data_mut(|d| d.insert_temp(tab, sticker_tab));
        if let (true, Some((panel, textures))) = (sticker_tab, stickers.as_mut()) {
            crate::stickers::show(ui, panel, textures, t, w, DOCK_ROWS);
        } else {
            dock_grid(ui, scroll_id, grid_id, id, draft, t, crate::stickers::body_height(w, DOCK_ROWS, ui.spacing().item_spacing.y));
        }
    });
    if close {
        ui.data_mut(|d| d.insert_temp(dock_id, false));
    }
}

/// The emoji set centred in the drawer's width (what is left of the row), in a box `height` tall.
fn dock_grid(ui: &mut egui::Ui, scroll_id: egui::Id, grid_id: egui::Id, id: egui::Id, draft: &mut String, t: &matrix_desktop::DesignTokens, height: f32) {
    let cell = POPUP_CELL;
    let columns = ((ui.available_width() / cell).floor() as usize).max(1);
    let margin = ((ui.available_width() - columns as f32 * cell) / 2.).max(0.);
    ui.spacing_mut().scroll.floating = true;
    egui::ScrollArea::vertical().id_salt(scroll_id).min_scrolled_height(height).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        ui.horizontal(|ui| {
            ui.add_space(margin);
            emoji_cells(ui, grid_id, columns, egui::Vec2::splat(cell), id, draft, t);
        });
    });
}

/// The emoji grid centred in `width`, in a box `height` tall. True when an emoji was inserted.
fn emoji_grid(ui: &mut egui::Ui, scroll_id: egui::Id, grid_id: egui::Id, id: egui::Id, draft: &mut String, t: &matrix_desktop::DesignTokens, width: f32, height: f32) -> bool {
    let zoom = ui.ctx().zoom_factor();
    let (columns, cell, margin) = emoji_grid_geometry(width * zoom, zoom);
    let (cell, margin) = (cell / zoom, margin / zoom);
    ui.set_width(width);
    let mut inserted = false;
    ui.spacing_mut().scroll.floating = true;
    egui::ScrollArea::vertical().id_salt(scroll_id).min_scrolled_height(height).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        ui.horizontal(|ui| {
            ui.add_space(margin);
            inserted = emoji_cells(ui, grid_id, columns, egui::vec2(cell, cell.max(44. / zoom)), id, draft, t);
        });
    });
    inserted
}

/// The composer popup that is open. Every opening goes through `open_popup`, the one place that closes the others.
/// The docked drawer is a bar above the composer, not a popup: opening the attach menu leaves it, opening it closes the menu.
#[derive(Clone, Copy, PartialEq)]
pub enum Popup { Attach, Emoji, EmojiHover, Drawer, Mention }

/// Id of the flag set when another popup took over from the mention list: it stays shut until the `@` is gone.
pub const MENTION_HIDDEN: &str = "composer-mention-hidden";

pub fn open_popup(ctx: &egui::Context, which: Popup) {
    let current = egui::Id::new("composer-popup");
    if ctx.data(|d| d.get_temp::<Popup>(current)) == Some(which) { return; }
    ctx.data_mut(|d| {
        d.insert_temp(current, which);
        if which != Popup::EmojiHover { d.insert_temp(egui::Id::new("emoji-hover"), (false, 0.)); }
        if which != Popup::Mention { d.insert_temp(egui::Id::new(MENTION_HIDDEN), true); }
    });
    // egui keeps a single menu open (attach or emoji) by itself; the other three are not egui menus.
    if !matches!(which, Popup::Attach | Popup::Emoji) { egui::Popup::close_all(ctx); }
}

/// Hover state of the small emoji popup: (open, last time the pointer was on the button or the popup).
const HOVER_CLOSE_DELAY: f64 = 0.3;
fn hover_step((open, last): (bool, f64), over: bool, drawer_open: bool, now: f64) -> (bool, f64) {
    if drawer_open { return (false, last); }
    if over { return (true, now); }
    (open && now - last <= HOVER_CLOSE_DELAY, last)
}

/// `name` cut with «…» until `fits` accepts it, keeping a short extension: `documento_molto_lu….pdf`.
fn fit_name(name: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(name) { return name.to_owned(); }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 6 => name.split_at(i),
        _ => (name, ""),
    };
    let chars: Vec<char> = stem.chars().collect();
    let cut = |n: usize| format!("{}…{ext}", chars[..n].iter().collect::<String>());
    // Longest prefix that fits (widths only grow with n); at least one character stays.
    let (mut lo, mut hi) = (1, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&cut(mid)) { lo = mid } else { hi = mid - 1 }
    }
    cut(lo)
}

fn emoji_grid_geometry(width: f32, zoom: f32) -> (usize, f32, f32) {
    let cell = 40. * zoom;
    let columns = (((width - 24.) / cell).floor() as usize).max(1);
    (columns, cell, (width - columns as f32 * cell) / 2.)
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

/// What a paste or a keyboard insertion hands over: a picture (sent as PNG) or the bytes of a GIF (sent as they are, still animated).
pub enum Pasted {
    Picture(egui::ColorImage),
    Gif(Vec<u8>),
}

/// A GIF pasted into the focused composer: iOS hands over the pasteboard's own bytes; elsewhere a copied `.gif` file
/// is read from the clipboard's file list (arboard has no raw GIF data, and a decoded paste would lose the animation).
/// `free`: nothing else is attached and the composer is not in edit mode; otherwise the usual error is left to `take_image_input`.
fn take_gif_input(ui: &egui::Ui, id: egui::Id, free: bool) -> Option<Vec<u8>> {
    type Batch = (Option<egui::Id>, Vec<Result<(Vec<u8>, Option<String>), String>>);
    if !free || !ui.memory(|m| m.has_focus(id)) { return None; }
    let key = egui::Id::new("winit_image_insertions");
    if let Some((Some(owner), items)) = ui.data(|d| d.get_temp::<Batch>(key))
        && owner == id
        && let [Ok((bytes, _))] = items.as_slice()
        && bytes.starts_with(b"GIF8")
    {
        ui.data_mut(|d| d.remove::<Batch>(key));
        return Some(bytes.clone());
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let asked = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_))
            || matches!(e, egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command)));
        if asked && let Some(bytes) = clipboard_gif() {
            ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_))));
            return Some(bytes);
        }
    }
    None
}

/// The one `.gif` file on the clipboard (copied in a file manager), if it is one.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn clipboard_gif() -> Option<Vec<u8>> {
    use std::io::Read as _;
    let files = arboard::Clipboard::new().ok()?.get().file_list().ok()?;
    let [path] = files.as_slice() else { return None };
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif")) { return None; }
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(matrix_desktop::composer_image::MAX_IMAGE_BYTES as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.starts_with(b"GIF8") && bytes.len() <= matrix_desktop::composer_image::MAX_IMAGE_BYTES).then_some(bytes)
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

/// Draws the thumbnail; false when the attachment is not a picture.
fn attachment_thumbnail(ui: &mut egui::Ui, id: egui::Id, data: &[u8], revision: u64) -> bool {
    let mut cached = ui.data(|d| d.get_temp::<Thumbnail>(id));
    if cached.as_ref().is_none_or(|c| c.revision != revision) {
        let texture = matrix_desktop::composer_image::thumbnail(data).map(|image| ui.ctx().load_texture("composer-attachment", image, Default::default()));
        cached = Some(Thumbnail { revision, texture });
        ui.data_mut(|d| d.insert_temp(id, cached.clone().unwrap()));
    }
    let texture = cached.and_then(|c| c.texture);
    if let Some(texture) = &texture {
        ui.add(egui::Image::new(texture).max_size(egui::vec2(64., 64.)));
    }
    texture.is_some()
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

    #[test]
    fn a_pasted_gif_keeps_its_bytes_and_a_png_stays_on_the_image_path() {
        let gif = b"GIF89a\x01\x00\x01\x00 whatever follows".to_vec();
        let run = |bytes: Vec<u8>, free: bool| {
            let ctx = crate::icons::test_context();
            let (mut gif_out, mut left) = (None, true);
            let id = egui::Id::new("composer-text");
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.memory_mut(|m| m.request_focus(id));
                ui.data_mut(|d| d.insert_temp::<Batch>(egui::Id::new("winit_image_insertions"), (Some(id), vec![Ok((bytes.clone(), None))])));
                gif_out = take_gif_input(ui, id, free);
                left = ui.data(|d| d.get_temp::<Batch>(egui::Id::new("winit_image_insertions"))).is_some();
            });
            output.textures_delta.clear();
            (gif_out, left)
        };
        assert_eq!(run(gif.clone(), true), (Some(gif.clone()), false));
        assert_eq!(run(gif, false), (None, true)); // occupied: take_image_input reports it
        let png = matrix_desktop::composer_image::prepare(&image(), 1 << 20).unwrap();
        assert_eq!(run(png, true), (None, true));
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
                if frame == 3 { assert!(matches!(pasted, Some(Pasted::Picture(_)))); }
            });
            output.textures_delta.clear();
        }
        assert_eq!(draft, "caption");
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
    fn hover_opens_the_small_popup_and_closes_after_a_delay_but_not_over_the_drawer() {
        let s = super::hover_step((false, 0.), true, false, 1.);
        assert_eq!(s, (true, 1.));
        assert_eq!(super::hover_step(s, false, false, 1.2), (true, 1.), "still open inside the delay");
        assert_eq!(super::hover_step(s, false, false, 1.5).0, false);
        assert!(!super::hover_step((false, 0.), true, true, 1.).0, "the clicked drawer replaces the small popup");
        // Touch has no Emoji button, so nothing can hover.
        assert_eq!(super::wide_extra_slots(true, 900., 44., 8.), 0);
    }

    #[test]
    fn long_attachment_names_are_cut_keeping_the_extension() {
        let long = format!("{}.pdf", "documento_molto_lungo_".repeat(4));
        let cut = super::fit_name(&long, |s| s.chars().count() <= 24);
        assert_eq!(cut.chars().count(), 24);
        assert!(cut.ends_with("….pdf") && cut.starts_with("documento_molto_lu"), "{cut}");
        assert_eq!(super::fit_name("a.txt", |_| true), "a.txt");
        assert_eq!(super::fit_name("nome_lungo", |s| s.chars().count() <= 5), "nome…");
        assert_eq!(super::fit_name("x.jpeg", |_| false), "x….jpeg", "never under one character");
    }

    #[test]
    fn one_emoji_button_only_on_wide_desktop() {
        assert_eq!(super::wide_extra_slots(false, 900., 36., 8.), 1);
        assert_eq!(super::wide_extra_slots(false, 340., 36., 8.), 0, "narrow window keeps the menu");
        assert_eq!(super::wide_extra_slots(true, 900., 44., 8.), 0, "phone keeps the menu");
        // The field keeps at least the threshold width with the button.
        assert!((240..900).all(|a| { let e = super::wide_extra_slots(false, a as f32, 36., 8.); e == 0 || a as f32 - 3. * 36. - 4. * 8. - 12. >= 240. }));
    }

    #[test]
    fn hover_popup_shows_whole_rows_and_scrolls_only_beyond_four() {
        assert_eq!(super::popup_rows(32, 8, 40.), (4, 160.), "the 32 emoji are exactly four rows: no cut, no scroll");
        assert_eq!(super::popup_rows(3, 8, 40.), (1, 40.));
        assert_eq!(super::popup_rows(0, 8, 40.), (1, 40.));
        assert_eq!(super::popup_rows(100, 8, 40.), (4, 160.), "more rows scroll inside four whole ones");
    }

    #[test]
    fn emoji_grid_geometry_fits_and_centres_at_each_text_scale() {
        for width in [300_f32, 360., 402., 440., 600.] {
            for zoom in [1_f32, 1.15, 1.34] {
                let (columns, cell, margin) = super::emoji_grid_geometry(width, zoom);
                assert_eq!(columns, (((width - 24.) / (40. * zoom)).floor() as usize).max(1));
                assert!(columns >= 1 && margin >= 12., "width={width} zoom={zoom} columns={columns} margin={margin}");
                assert!((width - margin - columns as f32 * cell - margin).abs() <= 1.);
            }
        }
    }

    #[test]
    fn emoji_popup_paints_inside_the_safe_screen() {
        use eframe::egui;
        for zoom in [1., 1.15, 1.34] {
            let ctx = crate::icons::test_context();
            ctx.set_zoom_factor(zoom);
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(402. / zoom, 800. / zoom))), ..Default::default() };
            let mut count = 0;
            for frame in 0..6 {
                let mut out = ctx.run_ui(input.clone(), |ui| {
                    super::panel(egui::Frame::NONE).show(ui, |ui| {
                        let (mut draft, mut a, mut b, mut c, mut d, mut e, mut f) = (String::new(), false, false, false, None, None, None);
                        super::show(ui, "", &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new(), None, &matrix_desktop::LIGHT);
                    });
                });
                if frame == 0 {
                    let button = ctx.read_response(egui::Id::new("composer-emoji")).unwrap();
                    egui::Popup::open_id(&ctx, egui::Popup::default_response_id(&button));
                }
                if frame == 5 {
                    let button = ctx.read_response(egui::Id::new("composer-emoji")).unwrap();
                    let popup = ctx.read_response(egui::Popup::default_response_id(&button)).unwrap().rect;
                    let safe = ctx.content_rect();
                    assert!(popup.left() >= safe.left() && popup.right() <= safe.right(), "zoom={zoom} popup={popup:?} safe={safe:?}");
                    for clipped in &out.shapes {
                        if let egui::epaint::Shape::Text(text) = &clipped.shape {
                            if matrix_desktop::EMOJI_SET.iter().any(|(_, emoji)| *emoji == text.galley.text()) && clipped.clip_rect.intersects(text.visual_bounding_rect()) {
                                count += 1;
                                let rect = text.visual_bounding_rect();
                                assert!(rect.left() >= clipped.clip_rect.left() - 1. && rect.right() <= clipped.clip_rect.right() + 1., "zoom={zoom} rect={rect:?} clip={:?}", clipped.clip_rect);
                                assert!(rect.left() >= ctx.content_rect().left() && rect.right() <= ctx.content_rect().right(), "zoom={zoom} rect={rect:?}");
                            }
                        }
                    }
                }
                out.textures_delta.clear();
            }
            assert!(count > 0, "popup had no visible emoji at zoom {zoom}");
        }
    }

    /// The wide desktop bar driven with a pointer; `width` 900 has the Emoji button, 436 (touch) only the attach menu.
    struct Bar { ctx: eframe::egui::Context, width: f32 }

    impl Bar {
        fn new(width: f32) -> Self {
            let bar = Bar { ctx: crate::icons::test_context(), width };
            for _ in 0..4 { bar.frame(vec![]); }
            bar
        }
        fn frame(&self, events: Vec<eframe::egui::Event>) {
            use eframe::egui;
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(self.width, 1200.))), events, ..Default::default() }; // tall: the open drawer pushes the bar down
            let mut out = self.ctx.run_ui(input, |ui| {
                super::panel(egui::Frame::NONE).show(ui, |ui| {
                    let (mut draft, mut a, mut b, mut c, mut d, mut e, mut f) = (String::new(), false, false, false, None, None, None);
                    let (mut panel, avatars) = (crate::stickers::Panel::default(), crate::avatar::Avatars::new());
                    super::show(ui, "", &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new(), Some((&mut panel, &avatars)), &matrix_desktop::LIGHT);
                });
            });
            out.textures_delta.clear();
        }
        fn center(&self, id: &str) -> eframe::egui::Pos2 { self.ctx.read_response(eframe::egui::Id::new(id)).unwrap().rect.center() }
        fn hover(&self, id: &str) {
            let pos = self.center(id);
            for _ in 0..3 { self.frame(vec![eframe::egui::Event::PointerMoved(pos)]); }
        }
        fn click(&self, id: &str) {
            use eframe::egui::{Event, Modifiers, PointerButton};
            for _ in 0..6 { self.frame(vec![]); }
            let pos = self.center(id);
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(vec![]);
        }
        fn menu(&self) -> bool {
            let attach = self.ctx.read_response(eframe::egui::Id::new("composer-attach")).unwrap();
            eframe::egui::Popup::is_id_open(&self.ctx, eframe::egui::Popup::default_response_id(&attach))
        }
        fn small_popup(&self) -> bool { self.ctx.data(|d| d.get_temp::<(bool, f64)>(eframe::egui::Id::new("emoji-hover"))).is_some_and(|s| s.0) }
        fn drawer(&self) -> bool { self.ctx.data(|d| d.get_temp::<bool>(eframe::egui::Id::new(super::DOCK_ID))).unwrap_or(false) }
    }

    #[test]
    fn hovering_emoji_closes_the_attach_menu() {
        let bar = Bar::new(900.);
        bar.click("composer-attach");
        assert!(bar.menu() && !bar.small_popup());
        bar.hover("composer-emoji-button");
        assert!(bar.small_popup() && !bar.menu(), "menu {} popup {}", bar.menu(), bar.small_popup());
    }

    #[test]
    fn clicking_attach_closes_the_small_emoji_popup() {
        let bar = Bar::new(900.);
        bar.hover("composer-emoji-button");
        assert!(bar.small_popup() && !bar.menu());
        bar.click("composer-attach");
        assert!(bar.menu() && !bar.small_popup());
    }

    #[test]
    fn the_drawer_stays_when_attach_opens_and_opening_it_closes_the_menu() {
        let bar = Bar::new(900.);
        bar.click("composer-emoji-button");
        assert!(bar.drawer());
        bar.click("composer-attach");
        assert!(bar.menu() && bar.drawer(), "the drawer is not a popup: menu {} drawer {}", bar.menu(), bar.drawer());
        bar.click("composer-emoji-button");
        bar.click("composer-emoji-button");
        assert!(bar.drawer() && !bar.menu(), "opening the drawer closes the menu");
    }

    #[test]
    fn the_drawer_reopens_on_the_sticker_tab() {
        let bar = Bar::new(900.);
        let tab = eframe::egui::Id::new(crate::stickers::TAB_ID);
        bar.ctx.data_mut(|d| d.insert_temp(tab, true));
        for _ in 0..2 { bar.click("composer-emoji-button"); } // open, close
        assert!(!bar.drawer());
        bar.click("composer-emoji-button");
        assert!(bar.drawer() && bar.ctx.data(|d| d.get_temp::<bool>(tab)) == Some(true));
    }

    #[test]
    fn phone_has_no_hover_popup_and_the_menu_still_opens() {
        crate::mobile::FORCE_TOUCH.set(true);
        let bar = Bar::new(436.);
        bar.click("composer-attach");
        assert!(bar.menu() && !bar.small_popup());
        crate::mobile::FORCE_TOUCH.set(false);
    }

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
    fn on_touch_enter_adds_a_line_and_never_sends() {
        crate::mobile::FORCE_TOUCH.set(true);
        let (sent, draft, ..) = run("ab", enter(false));
        crate::mobile::FORCE_TOUCH.set(false);
        assert_eq!((sent, draft.as_str()), (None, "ab\n"));
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
            (r("composer-attach-thread"), r("composer-voice-thread"), r("composer-send-thread"), r("composer-poll"))
        };
        let (attach, voice, send, poll) = rects("");
        let (attach2, voice2, send2, _) = rects("ciao");
        assert!(poll.is_none(), "no poll in a thread");
        assert_eq!(attach.unwrap().0, attach2.unwrap().0, "attach does not move");
        let (mic, plane) = (voice.unwrap(), send2.unwrap());
        assert!(mic.1 && plane.1, "microphone clickable when empty, send with text");
        assert_eq!(mic.0, plane.0, "one slot for both");
        assert!(send.is_none_or(|s| !s.1) && voice2.is_none_or(|v| !v.1), "the faded one is inert");
        assert!(320. - mic.0.right() <= 16., "no empty slot at the right edge");
    }

    /// Telegram layout at every width and text scale: the field takes the remaining width, attach and mic/send sit right on the field's last row, nothing leaves the panel.
    #[test]
    fn field_takes_the_remaining_width_with_attach_and_send_anchored_right_at_every_scale() {
        use eframe::egui;
        for (width, zoom) in [(240., 1.5), (390., 1.), (436., 1.5), (900., 1.)] {
            for text in ["", "a\nb\nc\nd"] {
                let ctx = crate::icons::test_context();
                ctx.set_zoom_factor(zoom);
                let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.))), ..Default::default() };
                for _ in 0..4 {
                    let mut out = ctx.run_ui(input.clone(), |ui| {
                        super::panel(egui::Frame::NONE).show(ui, |ui| {
                            let (mut draft, mut a, mut b, mut c, mut d, mut e, mut f) = (text.to_owned(), false, false, false, None, None, None);
                            super::show(ui, "", &mut draft, super::Mode::Send, None, &mut a, &mut b, &mut c, &mut d, &mut e, None, &mut f, &[], &mut Vec::new(), None, &matrix_desktop::LIGHT);
                        });
                    });
                    out.textures_delta.clear();
                }
                let r = |id: &str| ctx.read_response(egui::Id::new(id)).unwrap_or_else(|| panic!("{id} missing")).rect;
                let (field, attach) = (r("composer-text"), r("composer-attach"));
                let mic = ctx.read_response(egui::Id::new("composer-voice")).or_else(|| ctx.read_response(egui::Id::new("composer-send"))).unwrap().rect;
                let slot = crate::metrics::button();
                let ctx_msg = format!("width {width} zoom {zoom} text {text:?}");
                assert!(field.width() >= 0.4 * width, "field too narrow: {} {ctx_msg}", field.width());
                assert!(field.right() <= attach.left() + 1., "field overlaps attach {ctx_msg}");
                assert!(attach.right() <= mic.left() + 1. && mic.right() <= width, "buttons leave the panel {ctx_msg}");
                assert!((mic.left() - attach.right()).abs() <= 8. && attach.size() == mic.size() && attach.bottom() == mic.bottom(), "buttons are one uniform row {ctx_msg}");
                let below = mic.bottom() - field.bottom(); // the field has its vertical padding outside its rect
                assert!((0. ..=slot / 2. + 1.).contains(&below), "buttons stay on the field's last row: {below} {ctx_msg}");
            }
        }
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
