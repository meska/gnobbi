//! Left side of the shell: the room list, drawn from the toolkit-free `Sidebar`.
use crate::icons::Icon;
use eframe::egui::{self, Align2, Color32, FontId, Sense, Vec2, text::LayoutJob};
use matrix_desktop::{DesignTokens, Sidebar, i18n};

const ROW: f32 = 52.;

pub(crate) fn clean_single_line(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        if matches!(c, '\u{00ad}' | '\u{061c}' | '\u{200b}' | '\u{200c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}' | '\u{2066}'..='\u{2069}' | '\u{feff}' | '\u{fffc}' | '\u{fffd}') {
            continue;
        }
        if c.is_whitespace() || c.is_control() {
            space = !out.is_empty();
        } else {
            if space { out.push(' '); }
            out.push(c);
            space = false;
        }
    }
    out
}

#[cfg(test)]
mod mark_tests {
    use super::*;

    #[test]
    fn mark_follows_the_tags_and_pinned_wins() {
        assert_eq!(mark(true, false), Some(Mark::Pin));
        assert_eq!(mark(false, true), Some(Mark::Low));
        assert_eq!(mark(true, true), Some(Mark::Pin));
        assert_eq!(mark(false, false), None);
    }

    #[test]
    fn menu_offers_read_only_while_unread() {
        let (read, unread) = (i18n::t("Mark as read"), i18n::t("Mark as unread"));
        assert_eq!(mark_read_entry(3, false), read);
        assert_eq!(mark_read_entry(0, true), read);
        assert_eq!(mark_read_entry(0, false), unread);
    }

    #[test]
    fn every_menu_entry_has_its_icon_first_in_one_column() {
        let t = matrix_desktop::DARK;
        let glyph = |job: &LayoutJob| job.sections[0].format.font_id.clone();
        for (icon, label) in [(read_icon(1, false), "a"), (read_icon(0, false), "b"), (Icon::PushPin, "c"), (Icon::PushPinSlash, "d"), (Icon::Archive, "e"), (Icon::ArrowUp, "f"), (Icon::FolderSimple, "g"), (Icon::FolderSimpleMinus, "h"), (Icon::Check, "i"), (Icon::SignOut, "j")] {
            let job = item_job(icon, label, None, &t);
            assert_eq!(job.text.chars().next(), Some(icon.ch()));
            assert_eq!(glyph(&job), glyph(&item_job(Icon::Bell, "x", None, &t)), "one icon size for every entry");
            assert_eq!(job.sections[0].format.color, rgb(t.secondary));
        }
        assert_eq!(item_job(Icon::SignOut, "x", Some(rgb(t.error)), &t).sections[0].format.color, rgb(t.error));
    }

    #[test]
    fn mark_and_badge_both_reserve_their_own_width() {
        assert_eq!(right_reserved(false, None), 0.);
        assert_eq!(right_reserved(true, None), 28.);
        assert_eq!(right_reserved(false, Some(Mark::Pin)), MARK_SLOT);
        assert_eq!(right_reserved(true, Some(Mark::Low)), 28. + MARK_SLOT);
    }
}

#[cfg(test)]
mod single_line_tests {
    #[test]
    fn normalizes_line_breaks_and_preserves_emoji() {
        assert_eq!(super::clean_single_line("Gentile Marco Mescalchin,\r\nper velocità…"), "Gentile Marco Mescalchin, per velocità…");
        assert_eq!(super::clean_single_line("  A\u{200b}\u{feff}\t\u{85}\u{2028}\u{2029}\u{b}\u{c} B\u{a0}  👩\u{200d}💻❤️ "), "A B 👩\u{200d}💻❤️");
    }
}

pub fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn count(n: u64) -> String {
    if n > 99 { "99+".to_owned() } else { n.to_string() }
}

/// One line, cut with an ellipsis instead of wrapping.
pub fn one_line(ui: &egui::Ui, text: &str, size: f32, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(clean_single_line(text), FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}

/// Icon on the second line of a row: pinned (`m.favourite`) or low priority (`m.lowpriority`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mark { Pin, Low }

const MARK: f32 = 14.;
const MARK_SLOT: f32 = MARK + 4.;

/// Pinned wins when another client set both tags.
fn mark(pinned: bool, low: bool) -> Option<Mark> {
    if pinned { Some(Mark::Pin) } else if low { Some(Mark::Low) } else { None }
}

/// Width the preview must leave on the right: the unread badge and the mark sit side by side.
fn right_reserved(unread: bool, mark: Option<Mark>) -> f32 {
    (if unread { 28. } else { 0. }) + if mark.is_some() { MARK_SLOT } else { 0. }
}

fn mode_label(mode: matrix_domain::notifications::RoomNotifyMode) -> &'static str {
    use matrix_domain::notifications::RoomNotifyMode::*;
    match mode {
        AllMessages => i18n::t("All messages"),
        MentionsAndKeywordsOnly => i18n::t("Mentions only"),
        Mute => i18n::t("Muted"),
    }
}

/// Label of a room-menu entry: the icon in the secondary colour (`ink` for both when given, e.g. the error colour of «Leave»), then the text.
/// Every Phosphor glyph is 1 em wide at one size, so the text of all entries starts in the same column.
fn item_job(icon: crate::icons::Icon, label: &str, ink: Option<Color32>, t: &DesignTokens) -> LayoutJob {
    let mut job = crate::icons::with_label(icon, label);
    job.sections[0].format.color = ink.unwrap_or(rgb(t.secondary));
    if let Some(ink) = ink { job.sections[1].format.color = ink; }
    job
}

/// One entry of the room menu, always with its icon; `ink` recolours icon and text.
fn menu_item(ui: &mut egui::Ui, icon: crate::icons::Icon, label: &str, ink: Option<Color32>, t: &DesignTokens) -> egui::Response {
    ui.button(item_job(icon, label, ink, t))
}

/// Thin line between the groups of the room menu; with a title it is the group label (icon, secondary colour) and the line sits above it.
pub fn menu_divider(ui: &mut egui::Ui, title: Option<(crate::icons::Icon, &str)>, t: &DesignTokens) {
    ui.add_space(2.);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.min_rect().width().max(1.), 1.), Sense::hover()); // the widest entry so far: the line must not stretch the menu
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1., rgb(t.secondary).gamma_multiply(0.4)));
    let Some((icon, title)) = title else { return ui.add_space(2.) };
    let mut job = crate::icons::with_label(icon, title);
    for section in &mut job.sections { section.format.color = rgb(t.secondary); }
    // A button pads its text: the title takes the same inset, so its icon and text sit in the column of the entries.
    let label = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(ui.spacing().button_padding.x.round() as i8, 0)).show(ui, |ui| ui.add(egui::Label::new(job).selectable(false))).inner;
    label.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
}

/// The one read/unread entry: «Mark as read» while the room counts as unread (badge count or the manual flag), else «Mark as unread».
fn read_icon(unread: u64, marked_unread: bool) -> crate::icons::Icon {
    if unread > 0 || marked_unread { crate::icons::Icon::Checks } else { crate::icons::Icon::ChatCircle }
}

fn mark_read_entry(unread: u64, marked_unread: bool) -> &'static str {
    if unread > 0 || marked_unread { i18n::t("Mark as read") } else { i18n::t("Mark as unread") }
}

fn draw_mark(ui: &egui::Ui, id: egui::Id, m: Mark, center: egui::Pos2, t: &DesignTokens) {
    let rect = egui::Rect::from_center_size(center, Vec2::splat(MARK));
    let (label, color) = match m { Mark::Pin => (i18n::t("Pinned"), rgb(t.accent).gamma_multiply(0.8)), Mark::Low => (i18n::t("Low priority"), rgb(t.secondary).gamma_multiply(0.7)) };
    // Pinned is an active state (Fill); low priority is the plain down arrow.
    let (icon, fill) = match m { Mark::Pin => (crate::icons::Icon::PushPin, true), Mark::Low => (crate::icons::Icon::ArrowDown, false) };
    crate::icons::paint(ui.painter(), rect.center(), icon, fill, MARK, color);
    let resp = ui.interact(rect, id, Sense::hover()).on_hover_text(label);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
}

/// Draws the badge and returns its rectangle.
pub fn badge(ui: &egui::Ui, center: egui::Pos2, n: u64, t: &DesignTokens) -> egui::Rect {
    let g = ui.painter().layout_no_wrap(count(n), FontId::proportional(11.), rgb(t.on_accent));
    let w = (g.size().x + 10.).max(20.);
    let r = egui::Rect::from_center_size(center, Vec2::new(w, 20.));
    ui.painter().rect_filled(r, 10., rgb(t.accent));
    ui.painter().galley(r.center() - g.size() / 2., g, rgb(t.on_accent));
    r
}

/// The look of one room row (avatar, name, stamp, preview, badge, mark), the same in a single account's list and in the merged one;
/// `account` adds the small picture of the account over the room avatar (merged list with more than one account).
fn paint_row(ui: &mut egui::Ui, rect: egui::Rect, resp: &egui::Response, room: &matrix_desktop::SidebarRoom, mark_id: egui::Id, avatars: &crate::avatar::Avatars, selected: bool, account: Option<&AccountMark>, t: &DesignTokens) {
    let row_color = if selected { rgb(t.selected) } else if crate::touch_list::pressed(ui, resp) { rgb(t.input) } else { crate::list_fill(t) };
    if selected || crate::touch_list::pressed(ui, resp) {
        ui.painter().rect_filled(rect, 0., row_color);
    }
    // Worked out for LTR; `flip`/`place` put each piece on its side.
    let (flip, place) = (|p: egui::Pos2| crate::dir::flip_pos(rect, p), |p: egui::Pos2, g: &egui::Galley| crate::dir::place(rect, p, g.size().x));
    let c = flip(egui::pos2(rect.left() + 36., rect.center().y));
    crate::avatar::paint(ui, c, 40., rgb(room.color), &crate::avatar::lookup(avatars, room.info.avatar.as_deref()), &room.initials, 15., rgb(t.avatar_text));
    if let Some(m) = account {
        // A ring in the list colour keeps the small picture apart from the room's.
        let o = flip(egui::pos2(rect.left() + 36. + 14., rect.center().y + 14.));
        ui.painter().circle_filled(o, 10., row_color);
        crate::avatar::paint(ui, o, 17., rgb(crate::AVATAR), &m.avatar, &m.initial, 9., rgb(t.on_accent));
    }
    let p = ui.painter();
    let text_left = rect.left() + 68.;
    let stamp = room.stamp();
    let stamp_w = stamp.as_ref().map_or(0., |s| p.layout_no_wrap(s.clone(), FontId::proportional(12.), rgb(t.secondary)).size().x);
    let name = one_line(ui, &room.name, 15., rgb(t.text), (rect.right() - 12. - text_left - stamp_w - 8.).max(20.));
    ui.painter().galley(place(egui::pos2(text_left, rect.top() + 8.), &name), name, rgb(t.text));
    if let Some(s) = stamp {
        ui.painter().text(flip(egui::pos2(rect.right() - 12., rect.top() + 10.)), crate::dir::align(Align2::RIGHT_TOP), s, FontId::proportional(12.), rgb(t.secondary));
    }
    let row_mark = mark(room.pinned(), room.low_priority());
    let preview = one_line(ui, matrix_desktop::row_text::engine_placeholder(&room.preview), 13., rgb(t.secondary), (rect.right() - 12. - text_left - right_reserved(room.unread > 0, row_mark)).max(20.));
    ui.painter().galley(place(egui::pos2(text_left, rect.top() + 29.), &preview), preview, rgb(t.secondary));
    let mut mark_right = rect.right() - 12.;
    if room.unread > 0 {
        let b = badge(ui, flip(egui::pos2(rect.right() - 24., rect.top() + 38.)), room.unread, t);
        mark_right = rect.right() - 24. - b.width() / 2. - 4.;
    }
    if let Some(m) = row_mark {
        draw_mark(ui, mark_id, m, flip(egui::pos2(mark_right - MARK / 2., rect.top() + 38.)), t);
    }
}

/// A row whose «Esci» waits for the server: faded, with a spinner, until the answer removes it or brings the error.
fn dim_leaving(ui: &mut egui::Ui, rect: egui::Rect, leaving: bool, t: &DesignTokens) {
    if !leaving { return; }
    ui.painter().rect_filled(rect, 0., crate::list_fill(t).gamma_multiply(0.6));
    let at = crate::dir::flip_pos(rect, egui::pos2(rect.right() - 24., rect.center().y));
    ui.put(egui::Rect::from_center_size(at, Vec2::splat(18.)), egui::Spinner::new().size(18.));
}

/// The one confirmation modal: a question, what `body` adds, «Annulla» and a red button, both at button height (44 pt on touch).
/// `Some(true)` = confirmed, `Some(false)` = cancelled (button, Esc or a click outside). Shape of the log-out confirmation.
pub fn confirm_modal(ctx: &egui::Context, id: egui::Id, question: &str, go: &str, body: impl FnOnce(&mut egui::Ui)) -> Option<bool> {
    let mut step = None;
    let modal = egui::Modal::new(id).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        ui.set_max_width(crate::dir::fit(ui, 320.));
        ui.add(egui::Label::new(egui::RichText::new(question).heading()));
        body(ui);
        let tall = egui::vec2(0., crate::metrics::button());
        ui.horizontal_wrapped(|ui| {
            if ui.add(egui::Button::new(i18n::t("Cancel")).min_size(tall)).clicked() { step = Some(false); }
            // Text is the surface colour: tests/mention_contrast.rs holds surface-on-error at >=4.5:1 in every theme.
            let go = egui::Button::new(egui::RichText::new(go).color(ui.visuals().window_fill)).fill(ui.visuals().error_fg_color).min_size(tall);
            if ui.add(go).clicked() { step = Some(true); }
        });
    }));
    step.or_else(|| modal.should_close().then_some(false))
}

/// The question of the confirmation for a room or space: «Uscire da <nome>?».
pub fn leave_question(name: &str) -> String {
    i18n::t("Leave {name}?").replace("{name}", &clean_single_line(name))
}

/// «Esci» of the room menus asks here; drawn every frame for every session (a parked account asks too).
pub fn leave_modal(ctx: &egui::Context, session: &mut crate::session::Session) {
    let Some(scope) = session.leave_ask.clone() else { return };
    // Left or removed meanwhile: nothing to confirm.
    let Some(room) = session.sidebar.rooms().iter().find(|r| r.scope == scope) else {
        session.leave_ask = None;
        return;
    };
    let question = leave_question(&room.name);
    if let Some(go) = confirm_modal(ctx, egui::Id::new(("leave-room", &scope)), &question, i18n::t("Leave"), |_| {}) {
        session.leave_ask = None;
        if go { session.leave_room(scope); }
    }
}

/// The account a row of the merged list belongs to: its picture and initial, and its name for screen readers.
struct AccountMark {
    avatar: crate::avatar::Avatar,
    initial: String,
    name: String,
}

/// The other running accounts and the merged-list state, when «Non lette» or «Tutte le chat» lists every account: slot 0 is the shown
/// account, slot `n` is `parked[n - 1]`. A click on a row of another account leaves `(index in parked, room)` in `open`.
pub(crate) struct Unified<'a> {
    pub parked: &'a mut [crate::Parked],
    pub cache: &'a mut matrix_desktop::UnifiedList,
    pub open: &'a mut Option<(usize, matrix_domain::RoomScope)>,
}

impl Unified<'_> {
    /// The sidebars in slot order.
    fn sidebars<'s>(shown: (&'s Sidebar, &'s str), parked: &'s [crate::Parked]) -> impl Iterator<Item = (&'s Sidebar, &'s str)> + Clone {
        std::iter::once(shown).chain(parked.iter().map(|p| (&p.session.sidebar, p.session.user.as_str())))
    }
}

/// The shown account's row opens at once; another account's is handed to the app, which brings that account forward.
fn choose(open: &mut Option<(usize, matrix_domain::RoomScope)>, shown: &mut Sidebar, slot: usize, scope: matrix_domain::RoomScope) {
    if slot > 0 {
        *open = Some((slot - 1, scope));
    } else if !shown.select(&scope) {
        shown.select_outside_view(&scope);
    }
}

/// Header with the search field, then the rows. Selection goes through `Sidebar::select`; true when a room was chosen, the open one too.
pub fn rooms(ui: &mut egui::Ui, session: &mut crate::session::Session, new_chat: &mut bool, t: &DesignTokens, mut unified: Option<Unified>) -> bool {
    let (title, _, _) = session.sidebar.list_header();
    let in_space = session.sidebar.active_space().is_some();
    ui.add_space(12.);
    ui.horizontal(|ui| {
        ui.add_space(if in_space { 8. } else { 16. });
        crate::space::menu(ui, session, t);
        ui.heading(clean_single_line(&title));
        ui.with_layout(crate::dir::row_rev(egui::Align::Center), |ui| {
            ui.add_space(12.);
            let plus = crate::icons::button(ui, crate::icons::Icon::Plus, false, i18n::t("New"));
            egui::Popup::menu(&plus).show(|ui| crate::dir::popup(ui, |ui| {
                if ui.button(i18n::t("New chat")).clicked() {
                    *new_chat = true;
                    ui.close();
                }
                if ui.button(i18n::t("Explore public rooms")).clicked() {
                    session.open_explore();
                    ui.close();
                }
            }));
        });
    });
    let sidebar = &mut session.sidebar;
    let query = &mut session.query;
    let focus = &mut session.focus_search;
    let mut chosen = false;
    ui.add_space(8.);
    ui.horizontal(|ui| {
        ui.add_space(12.);
        let w = ui.available_width() - 12.;
        let r = crate::metrics::search(ui, [w, crate::metrics::button()], query, i18n::t("Search conversations"));
        if std::mem::take(focus) {
            r.request_focus();
        }
        if r.changed() {
            sidebar.set_query(query);
        }
        // Enter opens the first match, Escape clears the search: the keyboard way to change room.
        if r.has_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(Unified { parked, cache, open }) = unified.as_mut() {
                    let first = cache.rows(Unified::sidebars((&*sidebar, &session.user), parked), sidebar.unread_view(), query).first().copied();
                    if let Some((slot, i)) = first {
                        let scope = if slot == 0 { sidebar.rooms()[i].scope.clone() } else { parked[slot - 1].session.sidebar.rooms()[i].scope.clone() };
                        choose(open, sidebar, slot, scope);
                        chosen = true;
                    }
                } else if let Some(&first) = sidebar.visible_indices().first() {
                    let scope = sidebar.rooms()[first].scope.clone();
                    sidebar.select(&scope);
                    chosen = true;
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                query.clear();
                sidebar.set_query(query);
            }
        }
    });
    ui.add_space(8.);
    let selected = sidebar.selected().map(|r| r.scope.clone());
    let visible: Vec<usize> = sidebar.visible_indices().to_vec();
    let mut pick = None;
    let mut action = None;
    if let Some(error) = &session.room_error { ui.colored_label(rgb(t.error), error); }
    // Before the first sync answered an empty list is a list on its way: ghost rows instead of nothing.
    let waiting = crate::loading::list_waiting(sidebar.rooms().len(), !matches!(session.sync, None | Some(matrix_engine::RoomSyncStatus::Connecting)));
    if let Some(waited) = session.list_wait.elapsed(waiting, std::time::Instant::now()) {
        let motion = crate::loading::motion(ui);
        crate::loading::skeleton_list(ui, crate::list_fill(t), ROW, crate::loading::stage(waited).ghosts, motion, i18n::t("Loading conversations…"), t);
        // Still (Reduce Motion): nothing changes after the fade-in.
        if motion.is_some() || waited < crate::loading::LINE_DELAY + crate::loading::FADE {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(8));
        }
        return chosen;
    }
    let (press, swallow) = (crate::touch_menu::long_press(ui.ctx()), released_after_press(ui.ctx()));
    let (mut scroll_ui, bottom) = crate::mobile::extend_list(ui, crate::mobile::touch() && ui.ctx().content_rect().width() < crate::SINGLE_WIDTH);
    let mut margin = ui.spacing().scroll.content_margin;
    margin.bottom = margin.bottom.saturating_add(bottom.round() as i8);
    if let Some(Unified { parked, cache, open }) = unified {
        let rows = cache.rows(Unified::sidebars((&session.sidebar, &session.user), parked), session.sidebar.unread_view(), &session.query);
        let (mut pick, mut opened) = (None, None);
        // Taken out of the session while the rows draw: a row's menu needs its session mutably.
        let search = std::mem::replace(&mut session.all_search, matrix_engine::operations::MessageSearchState::Idle);
        let found = crate::message_hits::section(&search, session.query.trim());
        let shown = session.sidebar.selected().map(|r| r.scope.clone());
        let mut menu_pick = None;
        let draw = |ui: &mut egui::Ui, (slot, i): (usize, usize), pick: &mut Option<(usize, matrix_domain::RoomScope)>, menu_pick: &mut Option<(usize, matrix_domain::RoomScope, crate::session::RoomAction)>, session: &mut crate::session::Session, parked: &mut [crate::Parked]| {
            let owner = if slot == 0 { session } else { &mut parked[slot - 1].session };
            let room = &owner.sidebar.rooms()[i];
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
            let resp = crate::metrics::clickable(resp);
            let who = if owner.account_name.is_empty() { &owner.user } else { &owner.account_name };
            let mark = AccountMark { avatar: owner.own_avatar(), initial: who.trim_start_matches('@').chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_owned()), name: who.clone() };
            let spoken = format!("{}, {}", room.name, mark.name);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken));
            paint_row(ui, rect, &resp, room, egui::Id::new(("row-mark", slot, i)), &owner.avatars, slot == 0 && shown.as_ref() == Some(&room.scope), matrix_desktop::shows_account_mark(slot).then_some(&mark), t);
            dim_leaving(ui, rect, owner.sidebar.is_leaving(&room.scope), t);
            let long = pressed_here(ui, rect, press);
            if crate::touch_list::tapped(&resp) && !swallow {
                *pick = Some((slot, room.scope.clone()));
            }
            // Another account's room: no space entries (the space window belongs to the shown account), the rest works on its own session.
            if !room.info.invited && let Some(choice) = room_menu(row_popup(&resp, long, &room.scope), room, &owner.space_choices, &mut owner.space_pick, t) {
                *menu_pick = Some((slot, room.scope.clone(), choice));
            }
        };
        if matches!(found, crate::message_hits::Section::Hidden) {
            let area = egui::ScrollArea::vertical().content_margin(margin).auto_shrink(false).show_rows(&mut scroll_ui, ROW, rows.len(), |ui, range| {
                for &row in &rows[range] {
                    draw(ui, row, &mut pick, &mut menu_pick, &mut *session, &mut *parked);
                }
            });
            crate::touch_list::after(scroll_ui.ctx(), &area);
        } else {
            let area = egui::ScrollArea::vertical().content_margin(margin).auto_shrink(false).show(&mut scroll_ui, |ui| {
                for &row in rows {
                    draw(ui, row, &mut pick, &mut menu_pick, &mut *session, &mut *parked);
                }
                opened = crate::message_hits::show(ui, &found, |id| session.sidebar.rooms().iter().find(|r| r.scope.room_id.as_str() == id), &session.avatars, t);
            });
            crate::touch_list::after(scroll_ui.ctx(), &area);
        }
        session.all_search = search;
        match menu_pick {
            Some((0, scope, act)) => session.room_action(scope, act),
            Some((slot, scope, act)) => parked[slot - 1].session.room_action(scope, act),
            None => {}
        }
        if let Some((slot, scope)) = pick {
            choose(open, &mut session.sidebar, slot, scope);
            chosen = true;
        }
        match opened {
            Some(crate::message_hits::Out::Open(hit)) => {
                if session.open_message_hit(&hit) { chosen = true; }
            }
            Some(crate::message_hits::Out::More) => session.more_all_messages(),
            None => {}
        }
        return chosen;
    }
    // One room row; the same drawing serves the long virtual list and the short list that has messages under it.
    let mut draw = |ui: &mut egui::Ui, index: usize| {
            let room = &sidebar.rooms()[index];
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
            let resp = crate::metrics::clickable(resp);
            paint_row(ui, rect, &resp, room, egui::Id::new(("row-mark", index)), &session.avatars, selected.as_ref() == Some(&room.scope), None, t);
            dim_leaving(ui, rect, sidebar.is_leaving(&room.scope), t);
            let long = pressed_here(ui, rect, press);
            if crate::touch_list::tapped(&resp) && !swallow {
                pick = Some(room.scope.clone());
            }
            if (resp.secondary_clicked() && !crate::touch_list::stopped(ui.ctx())) || long {
                action = Some((room.scope.clone(), crate::session::RoomAction::Spaces));
            }
            if !room.info.invited {
                // Test hook: GNOBBI_EGUI_VIEW=room-menu shows the menu of the open room without a click, room-space-pick the window behind «Aggiungi a uno spazio…».
                let mut menu = row_popup(&resp, long, &room.scope);
                if selected.as_ref() == Some(&room.scope) && let Ok(view) = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW") && view.starts_with("room-") {
                    match &session.space_choices {
                        Some((asked, Some(_))) if asked == &room.scope => if view == "room-space-pick" && session.space_pick.is_none() { session.space_pick = Some((room.scope.clone(), None)); },
                        Some((asked, None)) if asked == &room.scope => {}
                        _ => action = Some((room.scope.clone(), crate::session::RoomAction::Spaces)),
                    }
                    // No pointer in a headless run: the menu is pinned to the row instead.
                    if view == "room-menu" { menu = menu.open(true).at_position(rect.center()); }
                }
                if let Some(choice) = room_menu(menu, room, &session.space_choices, &mut session.space_pick, t) { action = Some((room.scope.clone(), choice)); }
            }
    };
    // Messages of every room under the matching chats: the list is short then (a term filters it), so no virtual rows.
    let found = crate::message_hits::section(&session.all_search, session.query.trim());
    let mut opened = None;
    if matches!(found, crate::message_hits::Section::Hidden) {
        let area = egui::ScrollArea::vertical().content_margin(margin).auto_shrink(false).show_rows(&mut scroll_ui, ROW, visible.len(), |ui, range| {
            for &index in &visible[range] {
                draw(ui, index);
            }
        });
        crate::touch_list::after(scroll_ui.ctx(), &area);
    } else {
        let area = egui::ScrollArea::vertical().content_margin(margin).auto_shrink(false).show(&mut scroll_ui, |ui| {
            for &index in &visible {
                draw(ui, index);
            }
            opened = crate::message_hits::show(ui, &found, |id| sidebar.rooms().iter().find(|r| r.scope.room_id.as_str() == id), &session.avatars, t);
        });
        crate::touch_list::after(scroll_ui.ctx(), &area);
    }
    if let Some(scope) = pick {
        sidebar.select(&scope);
        chosen = true;
    }
    match opened {
        Some(crate::message_hits::Out::Open(hit)) => {
            if session.open_message_hit(&hit) { chosen = true; }
        }
        Some(crate::message_hits::Out::More) => session.more_all_messages(),
        None => {}
    }
    if let Some((scope, action)) = action { session.room_action(scope, action); }
    chosen
}

const SWALLOW: &str = "room-long-press";

/// A long press (touch) that landed on this row: the menu opens like a right click. The finger's release must not open the room (see `released_after_press`).
fn pressed_here(ui: &egui::Ui, rect: egui::Rect, press: Option<egui::Pos2>) -> bool {
    let hit = press.is_some_and(|p| !crate::touch_list::stopped(ui.ctx()) && rect.contains(p) && ui.is_rect_visible(rect) && ui.clip_rect().contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id()));
    if hit {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(SWALLOW), true));
        ui.ctx().stop_dragging(); // the list must not start scrolling under the held finger
        crate::mobile::haptic_tick();
    }
    hit
}

/// The frame in which the finger of a long press lifts: its click belongs to the menu, not to the row.
/// Same for a press (finger or mouse) that starts while a menu is open: it only closes the menu.
fn released_after_press(ctx: &egui::Context) -> bool {
    let id = egui::Id::new(SWALLOW);
    if egui::Popup::is_any_open(ctx) && ctx.input(|i| i.pointer.any_pressed()) {
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
    let up = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) && !ctx.input(|i| i.pointer.any_down());
    if up { ctx.data_mut(|d| d.remove_temp::<bool>(id)); }
    up
}

/// The room-menu popup of a row: opened by a right click, or by `long`. Its id belongs to the room, not to the row's place in the list:
/// a sync that moves the row (new message, pin) must not strand the open menu on a row that is no longer drawn there.
fn row_popup(resp: &egui::Response, long: bool, room: &matrix_domain::RoomScope) -> egui::Popup<'static> {
    // Not `Popup::context_menu`: that one closes the menu when its own row is clicked, and the release of a long press shorter than egui's click time is such a click.
    egui::Popup::menu(resp).id(egui::Id::new(("room-menu", room))).open_memory((long || resp.secondary_clicked()).then_some(egui::SetOpenCommand::Bool(true))).at_pointer_fixed()
}

/// The room menu on `menu` (a context menu or a pinned one); what the user picked. Shared by the single list and the multi-account list.
fn room_menu(menu: egui::Popup, room: &matrix_desktop::SidebarRoom, choices: &Option<(matrix_domain::RoomScope, Option<matrix_engine::operations::SpaceChoices>)>, pick: &mut Option<(matrix_domain::RoomScope, Option<matrix_domain::RoomScope>)>, t: &DesignTokens) -> Option<crate::session::RoomAction> {
    let mut action = None;
    // The area is kept inside the screen: the transparent margin leaves 8 pt to the edge, like the message menu.
    let frame = egui::Frame::popup(&menu.ctx().global_style()).outer_margin(8);
    menu.frame(frame).show(|ui| crate::dir::popup(ui, |ui| {
        use crate::session::RoomAction;
        ui.spacing_mut().item_spacing.y = 2.;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); // an entry never wraps: the menu grows (and the area shifts into the screen) instead
        if crate::mobile::touch() { ui.spacing_mut().interact_size.y = crate::touch_menu::ROW; } // finger-high rows, like the message menu
        let (pinned, low) = (room.pinned(), room.info.tags.iter().any(|t| t == "m.lowpriority"));
        for (icon, label, choice) in [
            (read_icon(room.unread, room.info.marked_unread), mark_read_entry(room.unread, room.info.marked_unread), RoomAction::Mark(room.unread > 0 || room.info.marked_unread)),
            if pinned { (Icon::PushPinSlash, i18n::t("Unpin"), RoomAction::Tag("m.favourite")) } else { (Icon::PushPin, i18n::t("Pin to top"), RoomAction::Tag("m.favourite")) },
            if low { (Icon::ArrowUp, i18n::t("Remove from low priority"), RoomAction::Tag("m.lowpriority")) } else { (Icon::Archive, i18n::t("Mark as low priority"), RoomAction::Tag("m.lowpriority")) },
        ] {
            if menu_item(ui, icon, label, None, t).clicked() { action = Some(choice); ui.close(); }
        }
        if let Some((asked, Some(choices))) = &choices && asked == &room.scope {
            let (add, remove) = space_entries(choices);
            if add && menu_item(ui, Icon::FolderSimple, i18n::t("Add to a space…"), None, t).clicked() { *pick = Some((room.scope.clone(), None)); ui.close(); }
            for space in remove {
                if menu_item(ui, Icon::FolderSimpleMinus, &i18n::t("Remove from {name}").replace("{name}", &space.name), None, t).clicked() { action = Some(RoomAction::RemoveFrom(space.scope.clone())); ui.close(); }
            }
        }
        let current = room.info.notification_mode;
        menu_divider(ui, Some((if current == matrix_domain::notifications::RoomNotifyMode::Mute { Icon::BellSlash } else { Icon::Bell }, i18n::t("Notifications"))), t);
        if let Some(mode) = notify_modes(ui, current, t) { action = Some(RoomAction::Mode(mode)); ui.close(); }
        menu_divider(ui, None, t);
        if menu_item(ui, Icon::SignOut, i18n::t("Leave"), Some(rgb(t.error)), t).clicked() {
            action = Some(RoomAction::Leave);
            ui.close();
        }
    }));
    action
}

/// «⋮» of the room header: the room menu of the list (right click, long press) for the open room, one body. Needs room for a 44 pt button.
pub fn header_menu(ui: &mut egui::Ui, session: &mut crate::session::Session, t: &DesignTokens) {
    let Some(room) = session.sidebar.selected().filter(|r| !r.info.invited) else { return };
    let scope = room.scope.clone();
    let (opened, choice) = header_button(ui, room, &session.space_choices, &mut session.space_pick, t);
    // Opening asks which spaces take the room, as the list does.
    let action = choice.or(opened.then_some(crate::session::RoomAction::Spaces));
    if let Some(action) = action { session.room_action(scope, action); }
}

/// The «⋮» button and its menu: whether it was clicked, and what was picked.
fn header_button(ui: &mut egui::Ui, room: &matrix_desktop::SidebarRoom, choices: &Option<(matrix_domain::RoomScope, Option<matrix_engine::operations::SpaceChoices>)>, pick: &mut Option<(matrix_domain::RoomScope, Option<matrix_domain::RoomScope>)>, t: &DesignTokens) -> (bool, Option<crate::session::RoomAction>) {
    let resp = crate::icons::button(ui, Icon::DotsThreeVertical, false, i18n::t("More"));
    (resp.clicked(), room_menu(egui::Popup::menu(&resp), room, choices, pick, t))
}

/// The three notification modes as indented entries, the `current` one checked; the mode tapped. Used by the room menu.
pub fn notify_modes(ui: &mut egui::Ui, current: matrix_domain::notifications::RoomNotifyMode, t: &DesignTokens) -> Option<matrix_domain::notifications::RoomNotifyMode> {
    let mut picked = None;
    ui.scope(|ui| {
        ui.style_mut().visuals.indent_has_left_vline = false;
        ui.indent("notify-modes", |ui| {
            for mode in matrix_domain::notifications::RoomNotifyMode::ALL {
                // The check slot is always there (invisible when not chosen), so the three texts line up.
                let mut job = item_job(Icon::Check, mode_label(mode), None, t);
                if current != mode { job.sections[0].format.color = Color32::TRANSPARENT; }
                if ui.button(job).clicked() { picked = Some(mode); }
            }
        });
    });
    picked
}

/// Space entries of the room menu: whether «Aggiungi a uno spazio…» shows, and the spaces behind «Togli da <spazio>».
/// A space the user may not edit gives no entry at all.
fn space_entries(choices: &matrix_engine::operations::SpaceChoices) -> (bool, Vec<&matrix_engine::operations::SpaceRef>) {
    (choices.addable.iter().any(|s| s.editable), choices.containing.iter().filter(|s| s.editable).collect())
}

/// «Aggiungi a uno spazio»: the spaces that can take the room, one choice, then «Aggiungi».
pub fn space_picker(ctx: &egui::Context, session: &mut crate::session::Session) {
    let Some((room, chosen)) = &mut session.space_pick else { return };
    let spaces: Vec<_> = match &session.space_choices {
        Some((asked, Some(choices))) if asked == room => choices.addable.iter().filter(|s| s.editable).collect(),
        _ => Vec::new(),
    };
    let (mut open, mut add, mut cancel) = (true, false, false);
    egui::Window::new(i18n::t("Add to a space")).open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| crate::dir::scope(ui, |ui| {
        ui.set_min_width(crate::dir::fit(ui, 300.));
        let area = egui::ScrollArea::vertical().max_height(260.).show(ui, |ui| {
            for space in spaces {
                crate::touch_list::radio(ui, chosen, Some(space.scope.clone()), &space.name);
            }
        });
        crate::touch_list::after(ui.ctx(), &area);
        ui.add_space(8.);
        ui.horizontal(|ui| {
            add = ui.add_enabled(chosen.is_some(), egui::Button::new(i18n::t("Add"))).clicked();
            cancel = ui.button(i18n::t("Cancel")).clicked();
        });
    }));
    if add && let Some((room, Some(space))) = session.space_pick.take() {
        session.room_action(room, crate::session::RoomAction::AddTo(space));
    } else if cancel || !open {
        session.space_pick = None;
    }
}

#[cfg(test)]
mod tests {
    use crate::touch_list::pad;
    use super::{header_button, pressed_here, released_after_press, room_menu, row_popup};
    use eframe::egui;

    /// A right click on a room row opens the room menu: the one body behind the single list and every row of the multi-account list.
    #[test]
    fn right_click_on_a_row_opens_the_room_menu() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        let room = matrix_desktop::SidebarRoom { scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:x") }, name: "a".into(), initials: String::new(), preview: String::new(), time: "", color: 0, unread: 0, info: Default::default() };
        let ctx = crate::icons::test_context();
        let mut pick = None;
        let at = egui::pos2(50., 30.);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
        for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]] {
            let input = eframe::egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 400.))), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                let (_, resp) = ui.allocate_exact_size(egui::vec2(300., 52.), egui::Sense::click());
                room_menu(egui::Popup::context_menu(&resp), &room, &None, &mut pick, &matrix_desktop::DARK);
            });
            out.textures_delta.clear();
        }
        assert!(egui::Popup::is_any_open(&ctx));
    }

    /// On touch a long press on a row opens the same room menu as the right click, in tall rows; its release does not open the room.
    #[test]
    fn long_press_on_a_row_opens_the_menu_and_not_the_room() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        crate::mobile::FORCE_TOUCH.set(true);
        let room = matrix_desktop::SidebarRoom { scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:x") }, name: "a".into(), initials: String::new(), preview: String::new(), time: "", color: 0, unread: 0, info: Default::default() };
        let ctx = crate::icons::test_context();
        let (mut pick, mut opened, mut swallowed, mut open_after_each_frame) = (None, false, false, Vec::new());
        let at = egui::pos2(50., 30.);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        // (time, events): down, still, held past the threshold, lifted.
        for (time, events) in [(0., vec![egui::Event::PointerMoved(at), button(true)]), (0.1, vec![]), (0.5, vec![]), (0.6, vec![button(false)]), (0.7, vec![])] {
            let input = egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 400.))), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                // From the long press on the list moves (a sync reorders the rows): the row has another place, so another automatic id.
                if time > 0.5 { ui.allocate_exact_size(egui::vec2(1., 1.), egui::Sense::click()); }
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(300., 52.), egui::Sense::click());
                let (press, swallow) = (crate::touch_menu::long_press(ui.ctx()), released_after_press(ui.ctx()));
                swallowed |= swallow;
                let long = pressed_here(ui, rect, press);
                opened |= resp.clicked() && !swallow;
                room_menu(row_popup(&resp, long, &room.scope), &room, &None, &mut pick, &matrix_desktop::DARK);
            });
            out.textures_delta.clear();
            open_after_each_frame.push(egui::Popup::is_any_open(&ctx));
        }
        crate::mobile::FORCE_TOUCH.set(false);
        assert!(open_after_each_frame.iter().skip(2).all(|o| *o), "the menu stays open from the long press on, also in the frame of the release and when the row moves: {open_after_each_frame:?}");
        assert!(egui::Popup::is_any_open(&ctx), "the long press opens the room menu");
        assert!(!opened && swallowed, "and the release of the finger is kept from opening the room");
    }

    /// A pad of chat rows drawn the way the list draws them: tap to open, long press for the menu, the release of a long press swallowed.
    fn chat_pad() -> crate::touch_list::pad::Pad {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        let mut room = matrix_desktop::SidebarRoom { scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:x") }, name: "a".into(), initials: String::new(), preview: String::new(), time: "", color: 0, unread: 0, info: Default::default() };
        let mut pick = None;
        crate::touch_list::pad::Pad::new(move |ui, range, seen| {
            let (press, swallow) = (crate::touch_menu::long_press(ui.ctx()), released_after_press(ui.ctx()));
            for i in range {
                room.scope.room_id = RoomId::new(format!("!{i}:x")); // every row its own room: the menu id is the room's
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), crate::touch_list::pad::ROW_H), egui::Sense::click());
                let long = pressed_here(ui, rect, press);
                seen.tapped.set(seen.tapped.get() | (crate::touch_list::tapped(&resp) && !swallow));
                room_menu(row_popup(&resp, long, &room.scope), &room, &None, &mut pick, &matrix_desktop::DARK);
            }
            seen.menu.set(egui::Popup::is_any_open(ui.ctx()));
        })
    }

    #[test]
    fn chat_row_still_tap_opens_without_the_menu() {
        let mut pad = chat_pad();
        let p = egui::pos2(100., 100.);
        pad.down(p);
        pad.wait(0.1);
        pad.up(p);
        assert!(pad.seen.tapped.get() && !pad.seen.menu.get());
    }

    #[test]
    fn chat_row_long_press_opens_the_menu_and_the_release_opens_no_chat() {
        let mut pad = chat_pad();
        let p = egui::pos2(100., 100.);
        pad.down(p);
        pad.wait(crate::touch_menu::HOLD + 0.1);
        assert!(pad.seen.menu.get(), "the menu opens while the finger is still down");
        pad.up(p);
        assert!(!pad.seen.tapped.get(), "the release does not open the chat");
    }

    #[test]
    fn chat_row_tap_that_closes_the_menu_opens_no_other_chat() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        let mut room = matrix_desktop::SidebarRoom { scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:x") }, name: "a".into(), initials: String::new(), preview: String::new(), time: "", color: 0, unread: 0, info: Default::default() };
        let mut pick = None;
        let first = std::cell::Cell::new(true);
        // The menu of the first row is open, the finger of its long press already lifted.
        let mut pad = crate::touch_list::pad::Pad::new(move |ui, range, seen| {
            let swallow = released_after_press(ui.ctx());
            for i in range {
                room.scope.room_id = RoomId::new(format!("!{i}:x"));
                let (_, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), crate::touch_list::pad::ROW_H), egui::Sense::click());
                seen.tapped.set(seen.tapped.get() | (crate::touch_list::tapped(&resp) && !swallow));
                room_menu(row_popup(&resp, i == 0 && first.replace(false), &room.scope), &room, &None, &mut pick, &matrix_desktop::DARK);
            }
            seen.menu.set(egui::Popup::is_any_open(ui.ctx()));
        });
        pad.wait(0.1);
        let q = egui::pos2(100., 300.); // another row, outside the menu
        pad.down(q);
        pad.wait(0.1);
        pad.up(q);
        pad.wait(0.1);
        assert!(!pad.seen.tapped.get(), "the tap that closes the menu opens no chat");
    }

    #[test]
    fn chat_row_movement_cancels_the_long_press_and_scrolls_without_opening() {
        let mut pad = chat_pad();
        let p = egui::pos2(100., 300.);
        pad.down(p);
        pad.wait(0.1);
        pad.drag(p, p - egui::vec2(0., 20.), 4); // leaves the slop before the hold completes
        pad.wait(crate::touch_menu::HOLD + 0.2);
        pad.up(p - egui::vec2(0., 20.));
        assert!(!pad.seen.menu.get() && !pad.seen.tapped.get());
        assert!(pad.offset > 0.);
    }

    #[test]
    fn chat_row_touch_that_catches_a_fling_opens_neither_chat_nor_menu() {
        let mut pad = chat_pad();
        pad.fling();
        let q = egui::pos2(100., 100.);
        pad.down(q);
        pad.wait(crate::touch_menu::HOLD + 0.2);
        pad.up(q);
        assert!(!pad.seen.menu.get() && !pad.seen.tapped.get());
    }

    /// With the room menu open, a tap on another row only closes the menu: it must not also open that chat.
    #[test]
    fn tap_outside_the_open_menu_does_not_open_the_row_below() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        crate::mobile::FORCE_TOUCH.set(true);
        let room = matrix_desktop::SidebarRoom { scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:x") }, name: "a".into(), initials: String::new(), preview: String::new(), time: "", color: 0, unread: 0, info: Default::default() };
        let ctx = crate::icons::test_context();
        let (mut pick, mut opened_second) = (None, false);
        let (first, second) = (egui::pos2(50., 30.), egui::pos2(50., 90.));
        let button = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        // down on the first row, held past the threshold, lifted; then a full tap on the second row.
        let script = [(0., vec![egui::Event::PointerMoved(first), button(first, true)]), (0.5, vec![]), (0.6, vec![button(first, false)]), (0.8, vec![egui::Event::PointerMoved(second)]), (0.9, vec![button(second, true)]), (1.0, vec![button(second, false)]), (1.1, vec![])];
        for (time, events) in script {
            let input = egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 400.))), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                let (r1, resp1) = ui.allocate_exact_size(egui::vec2(300., 52.), egui::Sense::click());
                let (_, resp2) = ui.allocate_exact_size(egui::vec2(300., 52.), egui::Sense::click());
                let (press, swallow) = (crate::touch_menu::long_press(ui.ctx()), released_after_press(ui.ctx()));
                let long = pressed_here(ui, r1, press);
                if time > 0.7 { opened_second |= resp2.clicked() && !swallow; }
                room_menu(row_popup(&resp1, long, &room.scope), &room, &None, &mut pick, &matrix_desktop::DARK);
            });
            out.textures_delta.clear();
        }
        crate::mobile::FORCE_TOUCH.set(false);
        assert!(!opened_second, "the tap that closes the menu must not open the chat under it");
    }

    /// The room header ends in «⋮» (a menu button of at least 44 pt) and has no bell; the menu behind it is the room menu.
    #[test]
    fn header_has_the_three_dots_and_no_bell() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        crate::mobile::FORCE_TOUCH.set(true);
        let room = matrix_desktop::SidebarRoom { scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!a:x") }, name: "a".into(), initials: String::new(), preview: String::new(), time: "", color: 0, unread: 0, info: Default::default() };
        let ctx = crate::icons::test_context();
        let (mut pick, mut size) = (None, egui::Vec2::ZERO);
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 400.))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            header_button(ui, &room, &None, &mut pick, &matrix_desktop::DARK);
            size = ui.min_rect().size();
        });
        out.textures_delta.clear();
        crate::mobile::FORCE_TOUCH.set(false);
        let glyphs: String = out.shapes.iter().filter_map(|s| match &s.shape { egui::epaint::Shape::Text(t) => Some(t.galley.text().to_owned()), _ => None }).collect();
        assert!(glyphs.contains(crate::icons::Icon::DotsThreeVertical.ch()), "«⋮» is drawn");
        assert!(!glyphs.contains(crate::icons::Icon::Bell.ch()) && !glyphs.contains(crate::icons::Icon::BellSlash.ch()), "no bell in the header");
        assert!(size.x >= 44. && size.y >= 44., "touch target {size:?}");
    }

    /// The sequence as the device sends it: the menu of the first row is opened (long press; right click on desktop), the finger lifts,
    /// then a press and a release land on another row, outside the menu. Returns (a chat opened, the menu is still open).
    fn tap_outside_the_menu(touch: bool, one_frame: bool) -> (bool, bool) {
        use egui::{Event, PointerButton};
        let mut pad = chat_pad();
        crate::mobile::FORCE_TOUCH.set(touch);
        let button = |pos, button, pressed| Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
        let p = egui::pos2(100., 20.);
        if touch {
            pad.down(p);
            pad.wait(crate::touch_menu::HOLD + 0.1);
            pad.up(p);
        } else {
            pad.frame(0.016, vec![Event::PointerMoved(p)]);
            pad.frame(0.016, vec![button(p, PointerButton::Secondary, true)]);
            pad.frame(0.016, vec![button(p, PointerButton::Secondary, false)]);
        }
        pad.wait(0.3);
        assert!(pad.seen.menu.get() && !pad.seen.tapped.get(), "the menu is open and no chat opened yet (touch {touch})");
        let q = (1..9).map(|k| egui::pos2(20., 20. + pad::ROW_H * k as f32)).find(|q| pad.ctx.layer_id_at(*q).is_some_and(|l| l.order == egui::Order::Background)).expect("a row the menu does not cover");
        pad.frame(0.016, vec![Event::PointerMoved(q)]);
        if one_frame {
            pad.frame(0.016, vec![button(q, PointerButton::Primary, true), button(q, PointerButton::Primary, false)]);
        } else {
            pad.frame(0.016, vec![button(q, PointerButton::Primary, true)]);
            pad.wait(0.12);
            pad.frame(0.016, vec![button(q, PointerButton::Primary, false)]);
        }
        pad.wait(0.1);
        let seen = (pad.seen.tapped.get(), pad.seen.menu.get());
        crate::mobile::FORCE_TOUCH.set(false);
        seen
    }

    /// A tap outside the open room menu only closes it, on the phone and on desktop, with a 120 ms tap and with press and release in one frame.
    #[test]
    fn tap_outside_the_menu_closes_it_and_opens_no_chat() {
        let cases = [(true, false), (true, true), (false, false), (false, true)];
        let seen: Vec<_> = cases.iter().map(|(touch, one_frame)| tap_outside_the_menu(*touch, *one_frame)).collect();
        assert_eq!(seen, [(false, false); 4], "(chat opened, menu open) for (touch, one frame) = {cases:?}");
    }

    #[test]
    fn a_space_without_the_right_gives_no_menu_entry() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        use matrix_engine::operations::{SpaceChoices, SpaceRef};
        let space = |id: &str, editable| SpaceRef { scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(id) }, name: id.to_owned(), editable };
        let choices = SpaceChoices { addable: vec![space("!a:x", true)], containing: vec![space("!b:x", true), space("!c:x", false)] };
        let (add, remove) = super::space_entries(&choices);
        assert!(add);
        assert_eq!(remove.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["!b:x"]);
        let choices = SpaceChoices { addable: Vec::new(), containing: vec![space("!c:x", false)] };
        let (add, remove) = super::space_entries(&choices);
        assert!(!add && remove.is_empty());
    }
}
