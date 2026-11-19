//! Spaces rail, folder-bar style: «Non lette», «Tutte le chat», the joined spaces as a tree (icon above its name) and «Modifica» at the bottom.
use crate::list::{badge, one_line, photo, rgb};
use eframe::egui::{self, Align2, FontId, Sense, Stroke, Vec2};
use matrix_desktop::{AppDirs, DesignTokens, NavigationEntry, RailLayout, RailLayouts, Sidebar};
use matrix_domain::RoomScope;
use std::collections::{HashMap, HashSet};

pub const WIDTH: f32 = 76.;
const ITEM: f32 = 58.;

#[derive(Default)]
pub struct State {
    pub layouts: RailLayouts,
    /// Parent spaces shown with their subspaces (room ids); this run only.
    pub open: HashSet<String>,
    pub edit: bool,
}

/// One space of the rail tree.
pub struct Row {
    pub scope: RoomScope,
    pub name: String,
    avatar: Option<String>,
    selected: bool,
    pub unread: u64,
    pub depth: usize,
    /// Room id of the space this one sits under.
    pub parent: Option<String>,
    pub has_children: bool,
    open: bool,
    pub hidden: bool,
}

struct Walk<'a> {
    sidebar: &'a Sidebar,
    /// Spaces in the user's order; taken out as they are placed, so each shows once.
    entries: Vec<(String, Option<(NavigationEntry, bool)>)>,
    /// Subspaces of each space.
    kids: HashMap<String, Vec<String>>,
    open: &'a HashSet<String>,
    all: bool,
    rows: Vec<Row>,
}

impl Walk<'_> {
    /// Places the space and everything under it; `show` is false below a hidden or folded space (placed, not listed).
    fn push(&mut self, at: usize, depth: usize, parent: Option<&str>, show: bool) {
        let Some((entry, hidden)) = self.entries[at].1.take() else { return };
        let Some(scope) = entry.scope else { return };
        let id = self.entries[at].0.clone();
        let show = show && (self.all || !hidden);
        let children: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].1.is_some() && self.kids[&id].contains(&self.entries[i].0)).collect();
        let open = self.all || self.open.contains(&id);
        if show {
            // Folded parent: its badge also counts what sits in the subspaces, each room once.
            let unread = if children.is_empty() || open { entry.unread } else { self.deep_unread(&id) };
            self.rows.push(Row { scope, name: entry.name, avatar: entry.avatar, selected: entry.selected, unread, depth, parent: parent.map(str::to_owned), has_children: !children.is_empty(), open, hidden });
        }
        for child in children {
            self.push(child, depth + 1, Some(&id), show && open);
        }
    }

    fn deep_unread(&self, id: &str) -> u64 {
        let (mut spaces, mut todo) = (HashSet::new(), vec![id]);
        while let Some(s) = todo.pop() {
            if spaces.insert(s) {
                todo.extend(self.kids.get(s).into_iter().flatten().map(String::as_str));
            }
        }
        let rooms: HashSet<&str> = self.sidebar.spaces().filter(|s| spaces.contains(s.scope.room_id.as_str())).flat_map(|s| s.info.children.iter().map(|c| c.as_str())).collect();
        self.sidebar.rooms().iter().filter(|r| !r.info.muted() && rooms.contains(r.scope.room_id.as_str())).fold(0, |n, r| n.saturating_add(r.unread))
    }
}

/// The joined spaces as a depth-first tree in the user's order: a space that is a child of another joined space sits under it.
/// `all` (the edit window) lists every space; otherwise hidden spaces drop with their subspaces and closed parents keep theirs folded.
pub fn tree(sidebar: &Sidebar, layout: &RailLayout, open: &HashSet<String>, all: bool) -> Vec<Row> {
    let entries: Vec<(String, Option<(NavigationEntry, bool)>)> = layout
        .arrange(sidebar.navigation_entries())
        .into_iter()
        .filter_map(|e| Some((e.0.scope.as_ref()?.room_id.as_str().to_owned(), Some(e))))
        .collect();
    let ids: HashSet<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
    let kids: HashMap<String, Vec<String>> = sidebar
        .spaces()
        .map(|s| {
            let own = s.scope.room_id.as_str();
            (own.to_owned(), s.info.children.iter().map(|c| c.as_str()).filter(|c| *c != own && ids.contains(c)).map(str::to_owned).collect())
        })
        .collect();
    let nested: HashSet<String> = kids.values().flatten().cloned().collect();
    let count = entries.len();
    let mut walk = Walk { sidebar, entries, kids, open, all, rows: Vec::new() };
    // Second pass: spaces that only reach each other in a circle have no root; listed at the top level instead of lost.
    for pass in 0..2 {
        for at in 0..count {
            if pass == 1 || !nested.contains(&walk.entries[at].0) {
                walk.push(at, 0, None, true);
            }
        }
    }
    walk.rows
}

enum Icon<'a> {
    Unread,
    All,
    Edit,
    Space(&'a Row),
}

/// One rail entry: icon, name below, unread badge. The whole cell is the target.
fn item(ui: &mut egui::Ui, avatars: &crate::timeline::Avatars, t: &DesignTokens, icon: Icon, label: &str, n: u64, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(WIDTH, ITEM), Sense::click());
    let cell = rect.shrink2(Vec2::new(4., 1.));
    if selected {
        ui.painter().rect_filled(cell, 10., rgb(t.selected));
    } else if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(cell, 10., rgb(t.input));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(cell, 10., Stroke::new(1.5, rgb(t.accent)), egui::StrokeKind::Inside);
    }
    let depth = if let Icon::Space(row) = &icon { row.depth.min(2) as f32 } else { 0. };
    let c = egui::pos2(rect.center().x + depth * 4., rect.top() + 22.);
    let ink = rgb(if selected { t.accent } else { t.secondary });
    let p = ui.painter();
    let bubble = |at: egui::Pos2, color| {
        let body = egui::Rect::from_center_size(at, Vec2::new(22., 16.));
        p.rect_filled(body, 6., color);
        p.add(egui::Shape::convex_polygon(vec![body.left_bottom() + Vec2::new(3., -5.), body.left_bottom() + Vec2::new(10., -1.), body.left_bottom() + Vec2::new(1., 4.)], color, Stroke::NONE));
    };
    match &icon {
        Icon::Unread => bubble(c, ink),
        Icon::All => {
            let back = rgb(if selected { t.selected } else { t.surface });
            bubble(c + Vec2::new(4., -4.), ink);
            p.rect_filled(egui::Rect::from_center_size(c + Vec2::new(-3., 3.), Vec2::new(25., 19.)), 7., back);
            bubble(c + Vec2::new(-3., 3.), ink);
        }
        Icon::Edit => {
            for (dy, knob) in [(-5., 5.), (5., -5.)] {
                p.line_segment([c + Vec2::new(-11., dy), c + Vec2::new(11., dy)], Stroke::new(2., ink));
                p.circle_filled(c + Vec2::new(knob, dy), 4., ink);
            }
        }
        Icon::Space(row) => {
            let size = if row.depth > 0 { 28. } else { 34. };
            let initial: String = row.name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
            p.circle_filled(c, size / 2., rgb(crate::AVATAR));
            p.text(c, Align2::CENTER_CENTER, initial, FontId::proportional(size * 0.45), rgb(t.on_accent));
            photo(ui, avatars, row.avatar.as_deref(), c, size);
            if row.depth > 0 {
                p.line_segment([egui::pos2(rect.left() + 11., rect.top()), egui::pos2(rect.left() + 11., rect.top() + 40.)], Stroke::new(1., rgb(t.border)));
            }
            if row.has_children {
                // Chevron, drawn: right when folded, down when open.
                let a = egui::pos2(rect.left() + 12. + depth * 4., c.y);
                let pts = if row.open { [a + Vec2::new(-4., -2.), a + Vec2::new(0., 2.), a + Vec2::new(4., -2.)] } else { [a + Vec2::new(-2., -4.), a + Vec2::new(2., 0.), a + Vec2::new(-2., 4.)] };
                p.line_segment([pts[0], pts[1]], Stroke::new(1.5, rgb(t.secondary)));
                p.line_segment([pts[1], pts[2]], Stroke::new(1.5, rgb(t.secondary)));
            }
        }
    }
    let name = one_line(ui, label, 11., rgb(if selected { t.text } else { t.secondary }), WIDTH - 12.);
    p.galley(egui::pos2(rect.center().x - name.size().x / 2., rect.top() + 41.), name, rgb(t.text));
    if n > 0 {
        badge(ui, c + Vec2::new(17., -12.), n, t);
    }
    let spoken = if n > 0 { format!("{label}, {n} non letti") } else { label.to_owned() };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &spoken));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The rail below the account picture. `account` is the catalog id the layout is stored under.
pub fn show(ui: &mut egui::Ui, sidebar: &mut Sidebar, avatars: &crate::timeline::Avatars, state: &mut State, account: u64, t: &DesignTokens) {
    // Test hooks: GNOBBI_EGUI_VIEW=rail-open unfolds every parent space, rail-edit shows the «Modifica» window.
    let view = std::env::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
    state.edit |= view == "rail-edit";
    let rows = tree(sidebar, &state.layouts.get(account), &state.open, view == "rail-open");
    let all = sidebar.navigation_entries().first().map_or(0, |e| e.unread);
    let (unread_rooms, unread_view, in_space) = (sidebar.unread_rooms() as u64, sidebar.unread_view(), sidebar.active_space().is_some());
    ui.spacing_mut().item_spacing.y = 0.;
    egui::ScrollArea::vertical().max_height(ui.available_height() - ITEM - 4.).auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.);
        if item(ui, avatars, t, Icon::Unread, "Non lette", unread_rooms, unread_view).clicked() {
            sidebar.set_unread_view(true);
        }
        if item(ui, avatars, t, Icon::All, "Tutte le chat", all, !unread_view && !in_space).clicked() {
            sidebar.set_space(None);
        }
        for row in &rows {
            let resp = item(ui, avatars, t, Icon::Space(row), &row.name, row.unread, row.selected);
            let id = row.scope.room_id.as_str();
            // Same click on a parent: pick it and unfold it; on the picked one, fold or unfold.
            if resp.clicked() {
                if row.has_children && !(row.selected && state.open.remove(id)) {
                    state.open.insert(id.to_owned());
                }
                sidebar.set_space(Some(row.scope.clone()));
            }
        }
    });
    if item(ui, avatars, t, Icon::Edit, "Modifica", 0, state.edit).clicked() {
        state.edit = !state.edit;
    }
}

/// Small drawn arrow button of the edit window.
fn arrow(ui: &mut egui::Ui, t: &DesignTokens, up: bool, enabled: bool, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && (resp.hovered() || resp.has_focus()) {
        ui.painter().rect_filled(rect, 8., rgb(t.input));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, 8., Stroke::new(1.5, rgb(t.accent)), egui::StrokeKind::Inside);
    }
    let (c, dy) = (rect.center(), if up { -3. } else { 3. });
    let stroke = Stroke::new(2., rgb(if enabled { t.text } else { t.disabled }));
    ui.painter().line_segment([c + Vec2::new(-5., -dy), c + Vec2::new(0., dy)], stroke);
    ui.painter().line_segment([c + Vec2::new(0., dy), c + Vec2::new(5., -dy)], stroke);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    resp.clicked()
}

/// What the edit window asks for.
enum Edit {
    /// Put the first space where the second one is, in the flat order.
    Move(String, String),
    Toggle(RoomScope),
}

/// «Modifica»: order (among the spaces of the same parent) and visibility of the spaces in the rail; saved at once, locally.
pub fn edit_window(ctx: &egui::Context, sidebar: &mut Sidebar, state: &mut State, account: u64, dirs: Option<&AppDirs>, t: &DesignTokens) {
    if !state.edit {
        return;
    }
    let mut layout = state.layouts.get(account);
    let rows = tree(sidebar, &layout, &state.open, true);
    let (mut open, mut act) = (true, None);
    egui::Window::new("Modifica barra").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0., 0.]).show(ctx, |ui| {
        ui.set_width(340.);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if rows.is_empty() {
            ui.label("Nessuno spazio");
        }
        egui::ScrollArea::vertical().max_height(420.).show(ui, |ui| {
            for (i, row) in rows.iter().enumerate() {
                let sibling = |r: &&Row| r.parent == row.parent;
                let (before, after) = (rows[..i].iter().rfind(sibling), rows[i + 1..].iter().find(sibling));
                let id = |r: &Row| r.scope.room_id.as_str().to_owned();
                ui.horizontal(|ui| {
                    ui.add_space(row.depth as f32 * 16.);
                    if arrow(ui, t, true, before.is_some(), &format!("Sposta su {}", row.name)) && let Some(b) = before {
                        act = Some(Edit::Move(id(row), id(b)));
                    }
                    if arrow(ui, t, false, after.is_some(), &format!("Sposta giù {}", row.name)) && let Some(a) = after {
                        act = Some(Edit::Move(id(row), id(a)));
                    }
                    let mut shown = !row.hidden;
                    if crate::settings::switch(ui, &row.name, &mut shown, t) {
                        act = Some(Edit::Toggle(row.scope.clone()));
                    }
                });
            }
        });
    });
    state.edit = open;
    let Some(act) = act else { return };
    match act {
        Edit::Move(id, target) => {
            let current: Vec<String> = layout.arrange(sidebar.navigation_entries()).into_iter().filter_map(|(e, _)| Some(e.scope?.room_id.as_str().to_owned())).collect();
            if let Some(to) = current.iter().position(|c| *c == target) {
                layout.move_to(&current, &id, to);
            }
        }
        Edit::Toggle(scope) => {
            layout.toggle_hidden(scope.room_id.as_str());
            // A space taken off the rail cannot stay the shown one.
            if sidebar.active_space() == Some(&scope) {
                sidebar.set_space(None);
            }
        }
    }
    state.layouts.set(account, layout);
    if let Some(dirs) = dirs {
        let _ = state.layouts.save(dirs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};
    use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};

    fn sidebar(rooms: &[(&str, u64, Option<&[&str]>)]) -> Sidebar {
        let mut s = Sidebar::connected(AccountId::new(1));
        for (id, unread, children) in rooms {
            s.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
                scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(format!("!{id}:x")) },
                name: (*id).into(),
                unread: *unread,
                info: RoomInfo { is_space: children.is_some(), children: children.unwrap_or_default().iter().map(|c| RoomId::new(format!("!{c}:x"))).collect(), ..Default::default() },
            })));
        }
        s
    }

    fn shape(rows: &[Row]) -> Vec<(&str, usize, u64)> {
        rows.iter().map(|r| (r.name.as_str(), r.depth, r.unread)).collect()
    }

    #[test]
    fn subspaces_nest_under_their_parent_and_a_folded_parent_counts_them() {
        // work > team > (r2), work > (r1); home apart. r1 is in both work and team: counted once.
        let s = sidebar(&[("r1", 2, None), ("r2", 3, None), ("r3", 5, None), ("work", 0, Some(&["team", "r1"])), ("team", 0, Some(&["r1", "r2"])), ("home", 0, Some(&["r3"]))]);
        let layout = RailLayout::default();
        let folded = tree(&s, &layout, &HashSet::new(), false);
        assert_eq!(shape(&folded), [("work", 0, 5), ("home", 0, 5)]);
        assert!(folded[0].has_children && !folded[1].has_children);
        let open = HashSet::from(["!work:x".to_owned()]);
        let rows = tree(&s, &layout, &open, false);
        assert_eq!(shape(&rows), [("work", 0, 2), ("team", 1, 5), ("home", 0, 5)]);
        assert_eq!(rows[1].parent.as_deref(), Some("!work:x"));
    }

    #[test]
    fn hidden_spaces_leave_the_rail_with_their_subspaces_and_stay_in_the_edit_list() {
        let s = sidebar(&[("work", 0, Some(&["team"])), ("team", 0, Some(&["r"])), ("home", 0, Some(&["r"]))]);
        let mut layout = RailLayout::default();
        layout.toggle_hidden("!work:x");
        let open = HashSet::from(["!work:x".to_owned()]);
        assert_eq!(shape(&tree(&s, &layout, &open, false)), [("home", 0, 0)]);
        let all = tree(&s, &layout, &HashSet::new(), true);
        assert_eq!(shape(&all), [("work", 0, 0), ("team", 1, 0), ("home", 0, 0)]);
        assert!(all[0].hidden && !all[1].hidden);
    }

    #[test]
    fn the_saved_order_applies_among_siblings_and_a_cycle_loses_no_space() {
        let s = sidebar(&[("a", 0, Some(&["b"])), ("b", 0, Some(&["a"])), ("c", 0, Some(&["r"]))]);
        let mut layout = RailLayout::default();
        let current = ["!a:x".to_owned(), "!b:x".to_owned(), "!c:x".to_owned()];
        assert!(layout.move_to(&current, "!c:x", 0));
        let names: Vec<String> = tree(&s, &layout, &HashSet::new(), true).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["c", "a", "b"]);
    }
}
