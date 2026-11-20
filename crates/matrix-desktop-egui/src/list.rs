//! Left side of the shell: the room list, drawn from the toolkit-free `Sidebar`.
use eframe::egui::{self, Align2, Color32, FontId, Sense, Vec2, text::LayoutJob};
use matrix_desktop::{DesignTokens, Sidebar};

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

pub fn badge(ui: &egui::Ui, center: egui::Pos2, n: u64, t: &DesignTokens) {
    let g = ui.painter().layout_no_wrap(count(n), FontId::proportional(11.), rgb(t.on_accent));
    let w = (g.size().x + 10.).max(20.);
    let r = egui::Rect::from_center_size(center, Vec2::new(w, 20.));
    ui.painter().rect_filled(r, 10., rgb(t.accent));
    ui.painter().galley(r.center() - g.size() / 2., g, rgb(t.on_accent));
}

/// A round picture (or nothing, when there is none) centred at `c`; the caller draws the initials circle first.
pub fn photo(ui: &egui::Ui, avatars: &crate::timeline::Avatars, mxc: Option<&str>, c: egui::Pos2, size: f32) -> bool {
    let Some(tex) = mxc.and_then(|m| avatars.get(m)?.as_ref()) else { return false };
    let rect = egui::Rect::from_center_size(c, Vec2::splat(size));
    egui::Image::new(tex).fit_to_exact_size(rect.size()).corner_radius(egui::CornerRadius::same((size / 2.) as u8)).paint_at(ui, rect);
    true
}

/// Header with the search field, then the rows. Selection goes through `Sidebar::select`; true when a room was chosen, the open one too.
pub fn rooms(ui: &mut egui::Ui, session: &mut crate::session::Session, new_chat: &mut bool, t: &DesignTokens) -> bool {
    let (title, _, _) = session.sidebar.list_header();
    let in_space = session.sidebar.active_space().is_some();
    ui.add_space(12.);
    ui.horizontal(|ui| {
        ui.add_space(if in_space { 8. } else { 16. });
        crate::space::menu(ui, session, t);
        ui.heading(clean_single_line(&title));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.);
            let plus = ui.button("+");
            egui::Popup::menu(&plus).show(|ui| {
                if ui.button("Nuova chat").clicked() {
                    *new_chat = true;
                    ui.close();
                }
                if ui.button("Esplora stanze pubbliche").clicked() {
                    session.open_explore();
                    ui.close();
                }
            });
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
        let r = ui.add_sized([w, 32.], egui::TextEdit::singleline(query).hint_text("Cerca conversazioni").align(egui::Align2::LEFT_CENTER));
        if std::mem::take(focus) {
            r.request_focus();
        }
        if r.changed() {
            sidebar.set_query(query);
        }
        // Enter opens the first match, Escape clears the search: the keyboard way to change room.
        if r.has_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(&first) = sidebar.visible_indices().first()
            {
                let scope = sidebar.rooms()[first].scope.clone();
                sidebar.select(&scope);
                chosen = true;
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
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, visible.len(), |ui, range| {
        for &index in &visible[range] {
            let room = &sidebar.rooms()[index];
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
            if selected.as_ref() == Some(&room.scope) {
                ui.painter().rect_filled(rect, 0., rgb(t.selected));
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0., rgb(t.input));
            }
            let p = ui.painter();
            let c = egui::pos2(rect.left() + 36., rect.center().y);
            p.circle_filled(c, 20., rgb(room.color));
            p.text(c, Align2::CENTER_CENTER, &room.initials, FontId::proportional(15.), rgb(t.avatar_text));
            photo(ui, &session.avatars, room.info.avatar.as_deref(), c, 40.);
            let text_left = rect.left() + 68.;
            let stamp = room.stamp();
            let stamp_w = stamp.as_ref().map_or(0., |s| p.layout_no_wrap(s.clone(), FontId::proportional(12.), rgb(t.secondary)).size().x);
            let name = one_line(ui, &room.name, 15., rgb(t.text), (rect.right() - 12. - text_left - stamp_w - 8.).max(20.));
            ui.painter().galley(egui::pos2(text_left, rect.top() + 8.), name, rgb(t.text));
            if let Some(s) = stamp {
                ui.painter().text(egui::pos2(rect.right() - 12., rect.top() + 10.), Align2::RIGHT_TOP, s, FontId::proportional(12.), rgb(t.secondary));
            }
            let badge_w = if room.unread > 0 { 28. } else { 0. };
            let preview = one_line(ui, &room.preview, 13., rgb(t.secondary), (rect.right() - 12. - text_left - badge_w).max(20.));
            ui.painter().galley(egui::pos2(text_left, rect.top() + 29.), preview, rgb(t.secondary));
            if room.unread > 0 {
                badge(ui, egui::pos2(rect.right() - 24., rect.top() + 38.), room.unread, t);
            }
            if resp.clicked() {
                pick = Some(room.scope.clone());
            }
            if resp.secondary_clicked() {
                session.confirm_leave = None;
                action = Some((room.scope.clone(), crate::session::RoomAction::Spaces));
            }
            if !room.info.invited {
                // Test hook: GNOBBI_EGUI_VIEW=room-menu shows the menu of the open room without a click, room-space-pick the window behind «Aggiungi a uno spazio…».
                let mut menu = egui::Popup::context_menu(&resp);
                if selected.as_ref() == Some(&room.scope) && let Ok(view) = std::env::var("GNOBBI_EGUI_VIEW") && view.starts_with("room-") {
                    match &session.space_choices {
                        Some((asked, Some(_))) if asked == &room.scope => if view == "room-space-pick" && session.space_pick.is_none() { session.space_pick = Some((room.scope.clone(), None)); },
                        Some((asked, None)) if asked == &room.scope => {}
                        _ => action = Some((room.scope.clone(), crate::session::RoomAction::Spaces)),
                    }
                    // No pointer in a headless run: the menu is pinned to the row instead.
                    if view == "room-menu" { menu = menu.open(true).at_position(rect.center()); }
                }
                menu.show(|ui| {
                    use crate::session::RoomAction;
                    for (label, choice) in [("Segna come letto", RoomAction::Mark(true)), ("Segna come non letto", RoomAction::Mark(false)), (sidebar.favourite_label(&room.scope), RoomAction::Tag("m.favourite")), (sidebar.low_priority_label(&room.scope), RoomAction::Tag("m.lowpriority"))] {
                        if ui.button(label).clicked() { action = Some((room.scope.clone(), choice)); ui.close(); }
                    }
                    if let Some((asked, Some(choices))) = &session.space_choices && asked == &room.scope {
                        let (add, remove) = space_entries(choices);
                        if add && ui.button("Aggiungi a uno spazio…").clicked() { session.space_pick = Some((room.scope.clone(), None)); ui.close(); }
                        for space in remove {
                            if ui.button(format!("Togli da {}", space.name)).clicked() { action = Some((room.scope.clone(), RoomAction::RemoveFrom(space.scope.clone()))); ui.close(); }
                        }
                    }
                    for mode in matrix_domain::notifications::RoomNotifyMode::ALL {
                        let label = if sidebar.notify_mode(&room.scope) == mode { format!("✓ {}", mode.label()) } else { mode.label().to_owned() };
                        if ui.button(label).clicked() { action = Some((room.scope.clone(), RoomAction::Mode(mode))); ui.close(); }
                    }
                    let label = if session.confirm_leave.as_ref() == Some(&room.scope) { "Uscire?" } else { "Esci" };
                    if ui.button(label).clicked() && confirm_leave(&mut session.confirm_leave, &room.scope) {
                        action = Some((room.scope.clone(), RoomAction::Leave));
                        ui.close();
                    }
                });
            }
        }
    });
    if let Some(scope) = pick {
        sidebar.select(&scope);
        chosen = true;
    }
    if let Some((scope, action)) = action { session.room_action(scope, action); }
    chosen
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
    egui::Window::new("Aggiungi a uno spazio").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_min_width(300.);
        egui::ScrollArea::vertical().max_height(260.).show(ui, |ui| {
            for space in spaces {
                ui.radio_value(chosen, Some(space.scope.clone()), &space.name);
            }
        });
        ui.add_space(8.);
        ui.horizontal(|ui| {
            add = ui.add_enabled(chosen.is_some(), egui::Button::new("Aggiungi")).clicked();
            cancel = ui.button("Annulla").clicked();
        });
    });
    if add && let Some((room, Some(space))) = session.space_pick.take() {
        session.room_action(room, crate::session::RoomAction::AddTo(space));
    } else if cancel || !open {
        session.space_pick = None;
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn leaving_requires_two_clicks_on_the_same_room() {
        use matrix_domain::{AccountId, RoomId, RoomScope};
        let room = |id: &str| RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(id) };
        let (a, b) = (room("!a:x"), room("!b:x"));
        let mut confirm = None;
        assert!(!super::confirm_leave(&mut confirm, &a));
        assert!(!super::confirm_leave(&mut confirm, &b));
        assert!(super::confirm_leave(&mut confirm, &b));
        assert!(!super::confirm_leave(&mut confirm, &b));
    }
}

fn confirm_leave(confirm: &mut Option<matrix_domain::RoomScope>, scope: &matrix_domain::RoomScope) -> bool {
    if confirm.as_ref() == Some(scope) { *confirm = None; true } else { *confirm = Some(scope.clone()); false }
}
