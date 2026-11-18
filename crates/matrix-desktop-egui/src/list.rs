//! Left side of the shell: the spaces rail and the room list, drawn from the toolkit-free `Sidebar`.
use eframe::egui::{self, Align2, Color32, FontId, Sense, Vec2, text::LayoutJob};
use matrix_desktop::{DesignTokens, Sidebar};

const ROW: f32 = 52.;
const RAIL: f32 = 64.;

pub fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn count(n: u64) -> String {
    if n > 99 { "99+".to_owned() } else { n.to_string() }
}

/// One line, cut with an ellipsis instead of wrapping.
fn one_line(ui: &egui::Ui, text: &str, size: f32, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}

fn badge(ui: &egui::Ui, center: egui::Pos2, n: u64, t: &DesignTokens) {
    let g = ui.painter().layout_no_wrap(count(n), FontId::proportional(11.), rgb(t.on_accent));
    let w = (g.size().x + 10.).max(20.);
    let r = egui::Rect::from_center_size(center, Vec2::new(w, 20.));
    ui.painter().rect_filled(r, 10., rgb(t.accent));
    ui.painter().galley(r.center() - g.size() / 2., g, rgb(t.on_accent));
}

/// Spaces rail: «Tutte» and the joined spaces.
pub fn rail(ui: &mut egui::Ui, sidebar: &mut Sidebar, t: &DesignTokens) {
    let mut pick = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(8.);
        for entry in sidebar.navigation_entries() {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(RAIL, 52.), Sense::click());
            let c = rect.center();
            if entry.selected {
                ui.painter().rect_filled(egui::Rect::from_center_size(c, Vec2::new(RAIL - 12., 48.)), 10., rgb(t.selected));
            } else if resp.hovered() {
                ui.painter().rect_filled(egui::Rect::from_center_size(c, Vec2::new(RAIL - 12., 48.)), 10., rgb(t.input));
            }
            let initial: String = entry.name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
            ui.painter().circle_filled(c, 18., if entry.scope.is_none() { rgb(t.accent) } else { rgb(crate::AVATAR) });
            ui.painter().text(c, Align2::CENTER_CENTER, initial, FontId::proportional(16.), rgb(t.on_accent));
            if entry.unread > 0 {
                badge(ui, c + Vec2::new(16., -16.), entry.unread, t);
            }
            if resp.clicked() {
                pick = Some(entry.scope);
            }
        }
    });
    if let Some(scope) = pick {
        sidebar.set_space(scope);
    }
}

/// Header with the search field, then the rows. Returns nothing: selection goes through `Sidebar::select`.
pub fn rooms(ui: &mut egui::Ui, sidebar: &mut Sidebar, query: &mut String, new_chat: &mut bool, focus: &mut bool, t: &DesignTokens) {
    let (title, _, _) = sidebar.list_header();
    ui.add_space(12.);
    ui.horizontal(|ui| {
        ui.add_space(16.);
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.);
            if ui.button("+").clicked() {
                *new_chat = true;
            }
        });
    });
    ui.add_space(8.);
    ui.horizontal(|ui| {
        ui.add_space(12.);
        let w = ui.available_width() - 12.;
        let r = ui.add_sized([w, 32.], egui::TextEdit::singleline(query).hint_text("Cerca").align(egui::Align2::LEFT_CENTER));
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
        }
    });
    if let Some(scope) = pick {
        sidebar.select(&scope);
    }
}
