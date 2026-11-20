//! Spaces rail, folder-bar style: «Non lette», «Tutte le chat», the joined spaces as a tree (icon above its name) and, at the bottom,
//! «Organizza spazi» (the drawer where the spaces are dragged into order and into each other) and «Impostazioni» (the settings drawer).
use crate::list::{badge, one_line, photo, rgb};
use eframe::egui::{self, Align2, FontId, Sense, Stroke, Vec2};
use matrix_desktop::{AppDirs, DesignTokens, NavigationEntry, RailLayout, RailLayouts, Sidebar};
use matrix_domain::RoomScope;
use matrix_engine::operations::OperationError;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const WIDTH: f32 = 76.;
const ITEM: f32 = 58.;
const DRAWER: f32 = 380.;
const ROW: f32 = 48.;
const INDENT: f32 = 20.;

/// A move sent to the server: shown at once, undone when the server refuses.
pub struct Move {
    id: String,
    parent: Option<String>,
    before: Option<String>,
    sent: Instant,
    /// When the server accepted; the row then waits for the sync to bring the same tree.
    done: Option<Instant>,
}

#[derive(Default)]
pub struct State {
    pub layouts: RailLayouts,
    /// Parent spaces shown with their subspaces (room ids); this run only.
    pub open: HashSet<String>,
    pub edit: bool,
    /// The settings and devices drawers are out. One drawer at a time: set them through `set_edit` / `set_settings` / `set_devices`.
    pub settings: bool,
    pub devices: bool,
    /// The devices drawer was just opened: the caller refreshes the list once and clears this.
    pub refresh_devices: bool,
    pub moving: Option<Move>,
    /// Why the last move did not happen.
    pub error: Option<String>,
    /// Where the rail was drawn last; the drawer slides out from its right edge.
    pub panel: Option<egui::Rect>,
}

impl State {
    pub fn set_edit(&mut self, on: bool) {
        self.edit = on;
        self.settings &= !on;
        self.devices &= !on;
    }

    pub fn set_settings(&mut self, on: bool) {
        self.settings = on;
        self.edit &= !on;
        self.devices &= !on;
    }

    pub fn set_devices(&mut self, on: bool) {
        self.refresh_devices |= on && !self.devices;
        self.devices = on;
        self.edit &= !on;
        self.settings &= !on;
    }
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
    /// The user may change the children of this space (`m.space.child`).
    pub editable: bool,
    /// Its move is on the way to the server.
    pub busy: bool,
}

impl Row {
    fn id(&self) -> &str {
        self.scope.room_id.as_str()
    }
}

struct Walk<'a> {
    sidebar: &'a Sidebar,
    /// Spaces, the top level in the user's order; taken out as they are placed, so each shows once.
    entries: Vec<(String, Option<(NavigationEntry, bool)>)>,
    /// Subspaces of each space, in the order of the space.
    kids: HashMap<String, Vec<String>>,
    editable: HashSet<String>,
    busy: Option<&'a str>,
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
        let children: Vec<usize> =
            self.kids.get(&id).into_iter().flatten().filter_map(|kid| self.entries.iter().position(|(e, slot)| e == kid && slot.is_some())).collect();
        let open = self.all || self.open.contains(&id);
        if show {
            // Folded parent: its badge also counts what sits in the subspaces, each room once.
            let unread = if children.is_empty() || open { entry.unread } else { self.deep_unread(&id) };
            let (editable, busy) = (self.editable.contains(&id), self.busy == Some(id.as_str()));
            self.rows.push(Row { scope, name: entry.name, avatar: entry.avatar, selected: entry.selected, unread, depth, parent: parent.map(str::to_owned), has_children: !children.is_empty(), open, hidden, editable, busy });
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

/// The joined spaces as a depth-first tree: a space that is a child of another joined space sits under it, in the order of that
/// space (server); the top level follows the user's local order. `moving` is drawn where it was sent, ahead of the sync.
/// `all` (the edit drawer) lists every space; otherwise hidden spaces drop with their subspaces and closed parents keep theirs folded.
pub fn tree(sidebar: &Sidebar, layout: &RailLayout, open: &HashSet<String>, all: bool, moving: Option<&Move>) -> Vec<Row> {
    let entries: Vec<(String, Option<(NavigationEntry, bool)>)> = layout
        .arrange(sidebar.navigation_entries())
        .into_iter()
        .filter_map(|e| Some((e.0.scope.as_ref()?.room_id.as_str().to_owned(), Some(e))))
        .collect();
    let ids: HashSet<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
    let mut kids: HashMap<String, Vec<String>> = sidebar
        .spaces()
        .map(|s| {
            let own = s.scope.room_id.as_str();
            (own.to_owned(), s.info.children.iter().map(|c| c.as_str()).filter(|c| *c != own && ids.contains(c)).map(str::to_owned).collect())
        })
        .collect();
    if let Some(m) = moving.filter(|m| ids.contains(m.id.as_str())) {
        kids.values_mut().for_each(|list| list.retain(|c| *c != m.id));
        if let Some(list) = m.parent.as_ref().and_then(|p| kids.get_mut(p)) {
            let at = m.before.as_ref().and_then(|b| list.iter().position(|c| c == b)).unwrap_or(list.len());
            list.insert(at, m.id.clone());
        }
    }
    let nested: HashSet<String> = kids.values().flatten().cloned().collect();
    let editable = sidebar.spaces().filter(|s| s.info.child_rights).map(|s| s.scope.room_id.as_str().to_owned()).collect();
    let count = entries.len();
    let mut walk = Walk { sidebar, entries, kids, editable, busy: moving.map(|m| m.id.as_str()), open, all, rows: Vec::new() };
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

/// The row and every row under it.
fn subtree(rows: &[Row], at: usize) -> std::ops::Range<usize> {
    at..at + 1 + rows[at + 1..].iter().take_while(|r| r.depth > rows[at].depth).count()
}

/// Why the space at `at` cannot go under `parent` (`None` = the top level); `None` when it can. A space never goes inside itself
/// or one of its subspaces, and both ends are written on the server: the new parent gets the child, the old one loses it.
pub fn refusal(rows: &[Row], at: usize, parent: Option<&str>) -> Option<String> {
    let find = |id: &str| rows.iter().position(|r| r.id() == id);
    if parent.and_then(find).is_some_and(|p| subtree(rows, at).contains(&p)) {
        return Some("Uno spazio non può stare dentro sé stesso".into());
    }
    [parent, rows[at].parent.as_deref()].into_iter().flatten().filter_map(find).find(|&i| !rows[i].editable).map(|i| format!("Non hai i permessi per modificare «{}»", rows[i].name))
}

/// What the drawer shows while a row hovers.
#[derive(Debug, PartialEq, Eq)]
pub enum Mark {
    /// The row the space goes into.
    Into(usize),
    /// A line above this row index (the row count = below the last), at this depth.
    Line(usize, usize),
}

/// Where a dragged space lands: under `parent` (`None` = top level), right before the sibling `before` (`None` = last).
#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub parent: Option<String>,
    pub before: Option<String>,
    pub mark: Mark,
}

/// The landing spot of the row `at` held over the row `over`, `frac` (0..1) down its height: the middle half nests it into that
/// row, the edges put it between two rows at the level `depth` asks for (kept within what the two neighbours allow).
/// `None` over the dragged row's own subtree.
pub fn drop_target(rows: &[Row], at: usize, over: usize, frac: f32, depth: usize) -> Option<Target> {
    let own = subtree(rows, at);
    if (0.25..0.75).contains(&frac) {
        return (!own.contains(&over)).then(|| Target { parent: Some(rows[over].id().to_owned()), before: None, mark: Mark::Into(over) });
    }
    let gap = if frac < 0.5 { over } else { over + 1 };
    if gap > own.start && gap < own.end {
        return None;
    }
    // Neighbours as if the dragged subtree were already lifted out.
    let above = rows[..gap].iter().enumerate().rfind(|(i, _)| !own.contains(i)).map(|(i, _)| i);
    let below = rows.iter().enumerate().skip(gap).find(|(i, _)| !own.contains(i)).map(|(_, r)| r);
    let depth = depth.clamp(below.map_or(0, |r| r.depth), above.map_or(0, |i| rows[i].depth + 1));
    let parent = above.filter(|_| depth > 0).and_then(|a| rows[..=a].iter().enumerate().rfind(|(i, r)| !own.contains(i) && r.depth == depth - 1)).map(|(_, r)| r.id().to_owned());
    Some(Target { parent, before: below.filter(|r| r.depth == depth).map(|r| r.id().to_owned()), mark: Mark::Line(gap, depth) })
}

/// Takes the engine's answer to the move in flight: a refusal undoes it and says why; an accepted one stays drawn until the
/// sync brings the same tree.
fn settle(state: &mut State, sidebar: &Sidebar, layout: &RailLayout, placed: Option<Result<(), OperationError>>) {
    let Some(mut m) = state.moving.take() else { return };
    let failed = match placed {
        Some(Err(e)) => Some(e),
        Some(Ok(())) => {
            m.done = Some(Instant::now());
            None
        }
        // shortcut: an answer lost with its session (account switched meanwhile) ends as a plain failure after 30 s.
        None => (m.done.is_none() && m.sent.elapsed() > Duration::from_secs(30)).then_some(OperationError::Transport),
    };
    if let Some(e) = failed {
        state.error = Some(matrix_engine::operations::space_error_line(e).into());
        return;
    }
    let shape = |moving| tree(sidebar, layout, &HashSet::new(), true, moving).into_iter().map(|r| (r.scope.room_id, r.parent)).collect::<Vec<_>>();
    if !m.done.is_some_and(|at| at.elapsed() > Duration::from_secs(8) || shape(None) == shape(Some(&m))) {
        state.moving = Some(m);
    }
}

enum Icon<'a> {
    Unread,
    All,
    Edit,
    Settings,
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
            let back = if selected { rgb(t.selected) } else if resp.hovered() || resp.has_focus() { rgb(t.input) } else { crate::chat_background(t) };
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
        Icon::Settings => {
            // Gear: eight teeth around a ring.
            for i in 0..8 {
                let d = Vec2::angled(i as f32 * std::f32::consts::TAU / 8.);
                p.line_segment([c + d * 7., c + d * 11.], Stroke::new(4., ink));
            }
            p.circle_stroke(c, 6., Stroke::new(3.5, ink));
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
    let spoken = if matches!(icon, Icon::Edit) { "Organizza spazi".to_owned() } else if n > 0 { format!("{label}, {n} non letti") } else { label.to_owned() };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &spoken));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The rail below the account picture. `account` is the catalog id the layout is stored under; `placed` the engine's answer to a move.
pub fn show(ui: &mut egui::Ui, sidebar: &mut Sidebar, avatars: &crate::timeline::Avatars, state: &mut State, account: u64, placed: Option<Result<(), OperationError>>, t: &DesignTokens) {
    // Test hooks: GNOBBI_EGUI_VIEW=rail-open unfolds every parent space, rail-edit shows «Organizza spazi», rail-drag / rail-line also a held row.
    let view = std::env::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
    if matches!(view.as_str(), "rail-edit" | "rail-drag" | "rail-line") {
        state.set_edit(true);
    }
    let layout = state.layouts.get(account);
    settle(state, sidebar, &layout, placed);
    state.panel = Some(ui.max_rect());
    let rows = tree(sidebar, &layout, &state.open, view == "rail-open", state.moving.as_ref());
    let all = sidebar.navigation_entries().first().map_or(0, |e| e.unread);
    let (unread_rooms, unread_view, in_space) = (sidebar.unread_rooms() as u64, sidebar.unread_view(), sidebar.active_space().is_some());
    ui.spacing_mut().item_spacing.y = 0.;
    egui::ScrollArea::vertical().max_height(ui.available_height() - 2. * ITEM - 4.).auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.);
        if item(ui, avatars, t, Icon::Unread, "Non lette", unread_rooms, unread_view).clicked() {
            sidebar.set_unread_view(true);
        }
        if item(ui, avatars, t, Icon::All, "Tutte le chat", all, !unread_view && !in_space).clicked() {
            sidebar.set_space(None);
        }
        for row in &rows {
            let resp = item(ui, avatars, t, Icon::Space(row), &row.name, row.unread, row.selected);
            let id = row.id();
            // Same click on a parent: pick it and unfold it; on the picked one, fold or unfold.
            if resp.clicked() {
                if row.has_children && !(row.selected && state.open.remove(id)) {
                    state.open.insert(id.to_owned());
                }
                sidebar.set_space(Some(row.scope.clone()));
            }
        }
    });
    if item(ui, avatars, t, Icon::Edit, "Spazi", 0, state.edit).clicked() {
        state.set_edit(!state.edit);
    }
    if item(ui, avatars, t, Icon::Settings, "Impostazioni", 0, state.settings).clicked() {
        state.set_settings(!state.settings);
    }
}

/// Small drawn arrow button of the edit window.
fn arrow(ui: &mut egui::Ui, t: &DesignTokens, up: bool, enabled: bool, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(crate::mobile::target(28.)), if enabled { Sense::click() } else { Sense::hover() });
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

/// Hover and focus look of a drawn control.
fn lit(ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, radius: f32, t: &DesignTokens) {
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(rect, radius, rgb(t.input));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.5, rgb(t.accent)), egui::StrokeKind::Inside);
    }
}

/// Initial on the fallback colour, the picture over it when there is one.
fn face(ui: &egui::Ui, p: &egui::Painter, avatars: &crate::timeline::Avatars, row: &Row, c: egui::Pos2, size: f32, t: &DesignTokens) {
    let initial: String = row.name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
    p.circle_filled(c, size / 2., rgb(crate::AVATAR));
    p.text(c, Align2::CENTER_CENTER, initial, FontId::proportional(size * 0.45), rgb(t.on_accent));
    photo(ui, avatars, row.avatar.as_deref(), c, size);
}

/// The row being dragged (egui drag-and-drop payload): its room id.
struct Dragged(String);

/// What the drawer asks for.
enum Edit {
    /// Put the row at this index under `parent`, before `before`.
    Place(usize, Option<String>, Option<String>),
    Toggle(RoomScope),
    Close,
}

/// How far the drawer `name` is out (0..1), sliding towards `shown`.
fn slide(ctx: &egui::Context, name: &str, shown: bool) -> f32 {
    ctx.animate_bool_with_time(egui::Id::new((name, "open")), shown, 0.18)
}

/// A drawer beside the rail (`panel`): it slides out from the rail's right edge over the dimmed window, with its title and an X.
/// `body` gets the rectangle below the title. True when the X or the dimmed part was clicked; Esc is the caller's.
pub fn drawer(ctx: &egui::Context, panel: Option<egui::Rect>, name: &'static str, title: &str, shown: bool, t: &DesignTokens, body: impl FnOnce(&mut egui::Ui, egui::Rect)) -> bool {
    drawer_back(ctx, panel, name, title, shown, t, None, body)
}

/// `drawer` with a «‹» left of the title when `back` is given; a click on it sets `*back`.
pub fn drawer_back(ctx: &egui::Context, panel: Option<egui::Rect>, name: &'static str, title: &str, shown: bool, t: &DesignTokens, back: Option<&mut bool>, body: impl FnOnce(&mut egui::Ui, egui::Rect)) -> bool {
    let open = slide(ctx, name, shown);
    let Some(panel) = panel.filter(|_| open > 0.) else { return false };
    let width = DRAWER.min(ctx.content_rect().right() - panel.right());
    let clip = egui::Rect::from_min_max(panel.right_top(), egui::pos2(ctx.content_rect().right(), panel.bottom()));
    let rect = egui::Rect::from_min_size(panel.right_top() - Vec2::new((1. - open) * width, 0.), Vec2::new(width, panel.height()));
    let mut closed = false;
    // The rest of the window is dimmed: a click there closes the drawer and reaches nothing else. The rail stays usable.
    egui::Area::new(egui::Id::new((name, "scrim"))).order(egui::Order::Middle).fixed_pos(clip.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(clip.size(), Sense::click());
        ui.painter().rect_filled(r, 0., egui::Color32::from_black_alpha((open * 102.) as u8));
        closed |= resp.clicked();
    });
    egui::Area::new(egui::Id::new(name)).order(egui::Order::Foreground).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        ui.set_clip_rect(clip);
        ui.set_min_size(rect.size());
        ui.interact(rect, egui::Id::new((name, "back")), Sense::click());
        let p = ui.painter().clone();
        p.rect_filled(rect, 0., rgb(t.surface));
        let head = egui::Rect::from_min_size(rect.min, Vec2::new(width, 56.));
        let mut title_x = 20.;
        if let Some(back) = back {
            let b = egui::Rect::from_center_size(egui::pos2(head.left() + 30., head.center().y), Vec2::splat(36.));
            let r = ui.interact(b, egui::Id::new((name, "return")), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            lit(ui, &r, b, 18., t);
            let c = b.center();
            p.add(egui::Shape::line(vec![c + Vec2::new(3., -7.), c + Vec2::new(-4., 0.), c + Vec2::new(3., 7.)], Stroke::new(2., rgb(t.secondary))));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Indietro"));
            *back |= r.clicked();
            title_x = 52.;
        }
        p.text(head.left_center() + Vec2::new(title_x, 0.), Align2::LEFT_CENTER, title, FontId::proportional(17.), rgb(t.text));
        let x = egui::Rect::from_center_size(egui::pos2(head.right() - 30., head.center().y), Vec2::splat(36.));
        let close = ui.interact(x, egui::Id::new((name, "close")), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        lit(ui, &close, x, 18., t);
        for d in [Vec2::new(6., 6.), Vec2::new(6., -6.)] {
            p.line_segment([x.center() - d, x.center() + d], Stroke::new(2., rgb(t.secondary)));
        }
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Chiudi"));
        closed |= close.clicked();
        body(ui, egui::Rect::from_min_max(head.left_bottom(), rect.max));
    });
    closed
}

/// «Organizza spazi»: a drawer beside the rail. Rows are dragged (or moved with the arrow keys and buttons) into order and into
/// each other: inside a space that is `m.space.child` on the server, at the top level a local order. The switch hides a
/// space from the rail, locally.
pub fn edit_window(ctx: &egui::Context, session: &mut crate::session::Session, form: &mut crate::newchat::Form, state: &mut State, account: u64, dirs: Option<&AppDirs>, t: &DesignTokens) {
    if slide(ctx, "rail-drawer", state.edit) == 0. {
        return;
    }
    let mut layout = state.layouts.get(account);
    let rows = tree(&session.sidebar, &layout, &state.open, true, state.moving.as_ref());
    let frozen = state.moving.is_some();
    let mut act = None;
    let closed = drawer(ctx, state.panel, "rail-drawer", "Organizza spazi", state.edit, t, |ui, rect| {
        let (p, width) = (ui.painter().clone(), rect.width());
        let action = egui::Rect::from_min_size(egui::pos2(rect.right() - 176., rect.top() - 46.), Vec2::new(120., 36.));
        if ui.put(action, egui::Button::new(egui::RichText::new("+ Nuovo spazio").color(rgb(t.accent))).frame(false)).clicked() {
            form.open_creation(true);
            session.new_open = true;
            act = Some(Edit::Close);
        }
        let mut top = rect.top() + 8.;
        if let Some(error) = &state.error {
            let line = one_line(ui, error, 13., rgb(t.error), width - 40.);
            p.galley(egui::pos2(rect.left() + 20., top + 18. - line.size().y / 2.), line, rgb(t.error));
            top += 36.;
        }
        if rows.is_empty() {
            p.text(egui::pos2(rect.center().x, top + 40.), Align2::CENTER_CENTER, "Nessuno spazio", FontId::proportional(15.), rgb(t.secondary));
            return;
        }
        let body = egui::Rect::from_min_max(egui::pos2(rect.left(), top + 4.), rect.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let (top, left) = (ui.cursor().top(), ui.max_rect().left());
                let view = std::env::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
                // The held row and where the pointer is; the test hooks hold the last row over the first.
                let (held, pos, from) = match view.as_str() {
                    "rail-drag" | "rail-line" => (Some(rows.len() - 1), Some(egui::pos2(left + 150., top + if view == "rail-drag" { ROW / 2. } else { ROW })), None),
                    _ => (
                        egui::DragAndDrop::payload::<Dragged>(ctx).and_then(|d| rows.iter().position(|r| r.id() == d.0)),
                        ctx.pointer_interact_pos(),
                        ctx.input(|i| i.pointer.press_origin()),
                    ),
                };
                let target = held.zip(pos).and_then(|(at, pos)| {
                    let y = ((pos.y - top) / ROW).clamp(0., rows.len() as f32 - 0.001);
                    // Sideways from where the row was grabbed: right goes a level in, left a level out.
                    let depth = rows[at].depth as f32 + from.map_or(0., |o| ((pos.x - o.x) / INDENT).round());
                    drop_target(&rows, at, y as usize, y.fract(), depth.max(0.) as usize)
                });
                let refused = held.zip(target.as_ref()).and_then(|(at, tg)| refusal(&rows, at, tg.parent.as_deref()));
                let ink = rgb(if refused.is_some() { t.error } else { t.accent });
                let siblings = |i: usize| {
                    let (prev, next) = (rows[..i].iter().rposition(|r| r.depth <= rows[i].depth), subtree(&rows, i).end);
                    (prev.filter(|&j| rows[j].depth == rows[i].depth), (next < rows.len() && rows[next].depth == rows[i].depth).then_some(next))
                };
                let id = |i: usize| rows[i].id().to_owned();
                for (i, row) in rows.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::hover());
                    let right = egui::Rect::from_min_max(egui::pos2(r.right() - 132., r.top()), egui::pos2(r.right() - 20., r.bottom()));
                    let grab = egui::Rect::from_min_max(r.min, egui::pos2(right.left(), r.bottom())).shrink2(Vec2::new(6., 2.));
                    let resp = ui.interact(grab, egui::Id::new(("rail-row", row.id())), if frozen { Sense::hover() } else { Sense::click_and_drag() });
                    if target.as_ref().is_some_and(|tg| tg.mark == Mark::Into(i)) {
                        ui.painter().rect(r.shrink2(Vec2::new(6., 2.)), 10., ink.gamma_multiply(0.18), Stroke::new(2., ink), egui::StrokeKind::Inside);
                    } else if held.is_none() {
                        lit(ui, &resp, grab, 10., t);
                    }
                    if resp.drag_started() {
                        resp.dnd_set_drag_payload(Dragged(row.id().to_owned()));
                    }
                    let (prev, next) = siblings(i);
                    let allowed = !frozen && refusal(&rows, i, row.parent.as_deref()).is_none();
                    if resp.has_focus() && !frozen {
                        // Arrow keys move the row instead of the focus: up/down among its siblings, right into the one above, left out of its parent.
                        ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
                        let key = |k| ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
                        if let Some(b) = prev.filter(|_| key(egui::Key::ArrowUp)) {
                            act = Some(Edit::Place(i, row.parent.clone(), Some(id(b))));
                        } else if let Some(n) = next.filter(|_| key(egui::Key::ArrowDown)) {
                            act = Some(Edit::Place(i, row.parent.clone(), siblings(n).1.map(id)));
                        } else if let Some(b) = prev.filter(|_| key(egui::Key::ArrowRight)) {
                            act = Some(Edit::Place(i, Some(id(b)), None));
                        } else if let Some(p) = row.parent.as_ref().filter(|_| key(egui::Key::ArrowLeft)).and_then(|p| rows.iter().position(|r| r.id() == p)) {
                            act = Some(Edit::Place(i, rows[p].parent.clone(), siblings(p).1.map(id)));
                        }
                    }
                    let mut p = ui.painter().clone();
                    if held == Some(i) {
                        p.set_opacity(0.35);
                    }
                    let (mut x, cy) = (grab.left() + 14., r.center().y);
                    if row.busy {
                        egui::Spinner::new().color(rgb(t.accent)).paint_at(ui, egui::Rect::from_center_size(egui::pos2(x, cy), Vec2::splat(16.)));
                    } else {
                        // Grip: six dots.
                        for (dx, dy) in [(-3., -6.), (3., -6.), (-3., 0.), (3., 0.), (-3., 6.), (3., 6.)] {
                            p.circle_filled(egui::pos2(x + dx, cy + dy), 1.5, rgb(if frozen { t.disabled } else { t.placeholder }));
                        }
                    }
                    x += 18. + row.depth as f32 * INDENT;
                    face(ui, &p, &session.avatars, row, egui::pos2(x + 14., cy), 28., t);
                    let name = one_line(ui, &row.name, 15., rgb(if row.hidden { t.placeholder } else { t.text }), grab.right() - x - 44.);
                    p.galley(egui::pos2(x + 38., cy - name.size().y / 2.), name, rgb(t.text));
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !frozen, format!("Sposta {}", row.name)));
                    if !frozen {
                        resp.on_hover_cursor(egui::CursorIcon::Grab);
                    }
                    ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.;
                        let mut shown = !row.hidden;
                        if crate::settings::toggle(ui, &row.name, &mut shown, t) {
                            act = Some(Edit::Toggle(row.scope.clone()));
                        }
                        ui.add_space(14.);
                        if arrow(ui, t, false, allowed && next.is_some(), &format!("Sposta giù {}", row.name)) && let Some(n) = next {
                            act = Some(Edit::Place(i, row.parent.clone(), siblings(n).1.map(id)));
                        }
                        if arrow(ui, t, true, allowed && prev.is_some(), &format!("Sposta su {}", row.name)) && let Some(b) = prev {
                            act = Some(Edit::Place(i, row.parent.clone(), Some(id(b))));
                        }
                    });
                }
                let (Some(at), Some(pos)) = (held, pos) else { return };
                if let Some(Target { mark: Mark::Line(gap, depth), .. }) = &target {
                    let a = egui::pos2(left + 26. + *depth as f32 * INDENT, top + *gap as f32 * ROW);
                    ui.painter().hline(a.x..=ui.max_rect().right() - 20., a.y, Stroke::new(2., ink));
                    ui.painter().circle_filled(a, 4., ink);
                }
                // The held row follows the pointer, above everything.
                let g = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("rail-held")));
                let card = egui::Rect::from_min_size(pos + Vec2::new(14., -20.), Vec2::new(220., 40.));
                g.rect_filled(card.translate(Vec2::new(0., 3.)), 10., egui::Color32::from_black_alpha(48));
                g.rect(card, 10., rgb(t.input), Stroke::new(1., rgb(t.border)), egui::StrokeKind::Inside);
                face(ui, &g, &session.avatars, &rows[at], card.left_center() + Vec2::new(24., 0.), 26., t);
                let name = one_line(ui, &rows[at].name, 15., rgb(t.text), 160.);
                g.galley(egui::pos2(card.left() + 46., card.center().y - name.size().y / 2.), name, rgb(t.text));
                if let Some(why) = &refused {
                    let line = one_line(ui, why, 13., rgb(t.error), 300.);
                    let note = egui::Rect::from_min_size(card.left_bottom() + Vec2::new(0., 6.), line.size() + Vec2::new(20., 12.));
                    g.rect(note, 8., rgb(t.surface), Stroke::new(1., rgb(t.error)), egui::StrokeKind::Inside);
                    g.galley(note.min + Vec2::new(10., 6.), line, rgb(t.error));
                }
                ctx.set_cursor_icon(if refused.is_some() { egui::CursorIcon::NotAllowed } else { egui::CursorIcon::Grabbing });
                // Near the top or bottom edge the list scrolls under the held row.
                let edge = ui.clip_rect();
                if pos.y < edge.top() + 36. || pos.y > edge.bottom() - 36. {
                    ui.scroll_with_delta(Vec2::new(0., if pos.y < edge.center().y { 10. } else { -10. }));
                    ctx.request_repaint();
                }
                if ctx.input(|i| i.pointer.any_released()) {
                    egui::DragAndDrop::clear_payload(ctx);
                    if let Some(tg) = target {
                        act = Some(Edit::Place(at, tg.parent, tg.before));
                    }
                }
            });
        });
    });
    match act.or(closed.then_some(Edit::Close)) {
        None => return,
        Some(Edit::Close) => {
            state.edit = false;
            return;
        }
        Some(Edit::Toggle(scope)) => {
            layout.toggle_hidden(scope.room_id.as_str());
            // A space taken off the rail cannot stay the shown one.
            if session.sidebar.active_space() == Some(&scope) {
                session.sidebar.set_space(None);
            }
        }
        Some(Edit::Place(at, parent, before)) => {
            let row = &rows[at];
            state.error = refusal(&rows, at, parent.as_deref());
            let next = rows.get(subtree(&rows, at).end).filter(|r| r.depth == row.depth).map(Row::id);
            if state.error.is_some() || (parent == row.parent && before.as_deref() == next) {
                return;
            }
            let scope = |id: &Option<String>| rows.iter().find(|r| Some(r.id()) == id.as_deref()).map(|r| r.scope.clone());
            if parent.is_some() || row.parent.is_some() {
                session.place_space(row.scope.clone(), scope(&row.parent), scope(&parent), scope(&before));
                state.moving = Some(Move { id: row.id().to_owned(), parent: parent.clone(), before: before.clone(), sent: Instant::now(), done: None });
            }
            if parent.is_none() {
                // The top level has no order on the server: it is the user's own, kept here.
                let current: Vec<String> = layout.arrange(session.sidebar.navigation_entries()).into_iter().filter_map(|(e, _)| Some(e.scope?.room_id.as_str().to_owned())).collect();
                let spot = |id: &str| current.iter().position(|c| c == id);
                if let Some(from) = spot(row.id()) {
                    let to = before.as_deref().and_then(spot).map_or(current.len() - 1, |b| if from < b { b - 1 } else { b });
                    layout.move_to(&current, row.id(), to);
                }
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

    /// Rooms as (name, unread, children when it is a space); every space is editable but those named `ro*`.
    fn sidebar(rooms: &[(&str, u64, Option<&[&str]>)]) -> Sidebar {
        let mut s = Sidebar::connected(AccountId::new(1));
        for (id, unread, children) in rooms {
            s.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
                scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(format!("!{id}:x")) },
                name: (*id).into(),
                unread: *unread,
                info: RoomInfo { is_space: children.is_some(), child_rights: children.is_some() && !id.starts_with("ro"), children: children.unwrap_or_default().iter().map(|c| RoomId::new(format!("!{c}:x"))).collect(), ..Default::default() },
            })));
        }
        s
    }

    fn shape(rows: &[Row]) -> Vec<(&str, usize, u64)> {
        rows.iter().map(|r| (r.name.as_str(), r.depth, r.unread)).collect()
    }

    fn all(s: &Sidebar, moving: Option<&Move>) -> Vec<Row> {
        tree(s, &RailLayout::default(), &HashSet::new(), true, moving)
    }

    fn at(rows: &[Row], name: &str) -> usize {
        rows.iter().position(|r| r.name == name).unwrap()
    }

    #[test]
    fn one_drawer_at_a_time() {
        let mut s = State::default();
        s.set_edit(true);
        s.set_settings(true);
        assert!(s.settings && !s.edit);
        s.set_edit(true);
        assert!(s.edit && !s.settings);
        s.set_settings(false);
        assert!(s.edit, "closing one leaves the other");
        s.set_devices(true);
        assert!(s.devices && !s.edit && !s.settings && s.refresh_devices);
        s.set_settings(true);
        assert!(s.settings && !s.devices);
    }

    #[test]
    fn subspaces_nest_under_their_parent_and_a_folded_parent_counts_them() {
        // work > team > (r2), work > (r1); home apart. r1 is in both work and team: counted once.
        let s = sidebar(&[("r1", 2, None), ("r2", 3, None), ("r3", 5, None), ("work", 0, Some(&["team", "r1"])), ("team", 0, Some(&["r1", "r2"])), ("home", 0, Some(&["r3"]))]);
        let layout = RailLayout::default();
        let folded = tree(&s, &layout, &HashSet::new(), false, None);
        assert_eq!(shape(&folded), [("work", 0, 5), ("home", 0, 5)]);
        assert!(folded[0].has_children && !folded[1].has_children);
        let open = HashSet::from(["!work:x".to_owned()]);
        let rows = tree(&s, &layout, &open, false, None);
        assert_eq!(shape(&rows), [("work", 0, 2), ("team", 1, 5), ("home", 0, 5)]);
        assert_eq!(rows[1].parent.as_deref(), Some("!work:x"));
    }

    #[test]
    fn hidden_spaces_leave_the_rail_with_their_subspaces_and_stay_in_the_edit_list() {
        let s = sidebar(&[("work", 0, Some(&["team"])), ("team", 0, Some(&["r"])), ("home", 0, Some(&["r"]))]);
        let mut layout = RailLayout::default();
        layout.toggle_hidden("!work:x");
        let open = HashSet::from(["!work:x".to_owned()]);
        assert_eq!(shape(&tree(&s, &layout, &open, false, None)), [("home", 0, 0)]);
        let all = tree(&s, &layout, &HashSet::new(), true, None);
        assert_eq!(shape(&all), [("work", 0, 0), ("team", 1, 0), ("home", 0, 0)]);
        assert!(all[0].hidden && !all[1].hidden);
    }

    #[test]
    fn the_saved_order_applies_at_the_top_level_and_a_cycle_loses_no_space() {
        let s = sidebar(&[("a", 0, Some(&["b"])), ("b", 0, Some(&["a"])), ("c", 0, Some(&["r"]))]);
        let mut layout = RailLayout::default();
        let current = ["!a:x".to_owned(), "!b:x".to_owned(), "!c:x".to_owned()];
        assert!(layout.move_to(&current, "!c:x", 0));
        let names: Vec<String> = tree(&s, &layout, &HashSet::new(), true, None).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["c", "a", "b"]);
    }

    #[test]
    fn subspaces_follow_the_order_of_their_space_not_the_local_one() {
        // The engine hands the children in the space's order (`order`, age): here c before b before a.
        let s = sidebar(&[("a", 0, Some(&[])), ("b", 0, Some(&[])), ("c", 0, Some(&[])), ("top", 0, Some(&["c", "b", "a"]))]);
        let names: Vec<String> = all(&s, None).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["top", "c", "b", "a"]);
    }

    #[test]
    fn a_move_in_flight_is_drawn_where_it_was_sent_and_marked_busy() {
        let s = sidebar(&[("a", 0, Some(&["a1", "a2"])), ("a1", 0, Some(&[])), ("a2", 0, Some(&[])), ("b", 0, Some(&[]))]);
        let mv = |parent: Option<&str>, before: Option<&str>| Move { id: "!b:x".into(), parent: parent.map(str::to_owned), before: before.map(str::to_owned), sent: Instant::now(), done: None };
        let rows = all(&s, Some(&mv(Some("!a:x"), Some("!a2:x"))));
        assert_eq!(shape(&rows), [("a", 0, 0), ("a1", 1, 0), ("b", 1, 0), ("a2", 1, 0)]);
        assert!(rows[2].busy && !rows[1].busy);
        assert_eq!(shape(&all(&s, Some(&mv(Some("!a:x"), None)))).last(), Some(&("b", 1, 0)));
        // Out of its parent, and a move of a space that is gone changes nothing.
        let out = Move { id: "!a1:x".into(), parent: None, before: None, sent: Instant::now(), done: None };
        assert_eq!(shape(&all(&s, Some(&out))), [("a", 0, 0), ("a2", 1, 0), ("a1", 0, 0), ("b", 0, 0)]);
        let gone = Move { id: "!zz:x".into(), parent: Some("!a:x".into()), before: None, sent: Instant::now(), done: None };
        assert_eq!(shape(&all(&s, Some(&gone))), shape(&all(&s, None)));
    }

    #[test]
    fn the_middle_of_a_row_nests_and_the_edges_place_between_rows_at_the_asked_level() {
        // a > (a1 > a11, a2), b, c
        let s = sidebar(&[("a", 0, Some(&["a1", "a2"])), ("a1", 0, Some(&["a11"])), ("a11", 0, Some(&[])), ("a2", 0, Some(&[])), ("b", 0, Some(&[])), ("c", 0, Some(&[]))]);
        let rows = all(&s, None);
        assert_eq!(shape(&rows).iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(), [("a", 0), ("a1", 1), ("a11", 2), ("a2", 1), ("b", 0), ("c", 0)]);
        let (c, b, a1) = (at(&rows, "c"), at(&rows, "b"), at(&rows, "a1"));
        let id = |n: &str| Some(format!("!{n}:x"));
        // Middle of «b»: c becomes its child.
        assert_eq!(drop_target(&rows, c, b, 0.5, 0), Some(Target { parent: id("b"), before: None, mark: Mark::Into(b) }));
        // Top edge of «a2» at level 1: sibling inside «a», before a2; pulled a level in: last child of «a1»; two in: under «a11».
        assert_eq!(drop_target(&rows, c, 3, 0.1, 1), Some(Target { parent: id("a"), before: id("a2"), mark: Mark::Line(3, 1) }));
        assert_eq!(drop_target(&rows, c, 3, 0.1, 2), Some(Target { parent: id("a1"), before: None, mark: Mark::Line(3, 2) }));
        assert_eq!(drop_target(&rows, c, 3, 0.1, 9), Some(Target { parent: id("a11"), before: None, mark: Mark::Line(3, 3) }));
        // The level is kept within the neighbours: above «a2» nothing is shallower than 1.
        assert_eq!(drop_target(&rows, c, 3, 0.1, 0).unwrap().mark, Mark::Line(3, 1));
        // Bottom edge of «a2»: level 0 = top level before «b»; level 1 = last child of «a».
        assert_eq!(drop_target(&rows, c, 3, 0.9, 0), Some(Target { parent: None, before: id("b"), mark: Mark::Line(4, 0) }));
        assert_eq!(drop_target(&rows, c, 3, 0.9, 1), Some(Target { parent: id("a"), before: None, mark: Mark::Line(4, 1) }));
        // Out of its parent: «a1» (with a11) below the last row, level 0.
        assert_eq!(drop_target(&rows, a1, c, 0.9, 0), Some(Target { parent: None, before: None, mark: Mark::Line(6, 0) }));
        // Very top.
        assert_eq!(drop_target(&rows, c, 0, 0.0, 0), Some(Target { parent: None, before: id("a"), mark: Mark::Line(0, 0) }));
        // The neighbours are read without the held subtree: «a1» dropped right below itself stays a child of «a», before «a2».
        assert_eq!(drop_target(&rows, a1, 3, 0.1, 0), Some(Target { parent: id("a"), before: id("a2"), mark: Mark::Line(3, 1) }));
    }

    #[test]
    fn a_space_never_lands_inside_itself_or_its_subspaces() {
        let s = sidebar(&[("a", 0, Some(&["a1"])), ("a1", 0, Some(&["a11"])), ("a11", 0, Some(&[])), ("b", 0, Some(&[]))]);
        let rows = all(&s, None);
        let a = at(&rows, "a");
        for over in [0, 1, 2] {
            assert_eq!(drop_target(&rows, a, over, 0.5, 0), None, "into row {over}");
        }
        assert_eq!(drop_target(&rows, a, 1, 0.9, 2), None, "between its own subspaces");
        // Asked anyway (keyboard, stale target): refused with the reason.
        for parent in ["!a:x", "!a1:x", "!a11:x"] {
            assert_eq!(refusal(&rows, a, Some(parent)).as_deref(), Some("Uno spazio non può stare dentro sé stesso"));
        }
        assert_eq!(refusal(&rows, a, Some("!b:x")), None);
        assert_eq!(refusal(&rows, at(&rows, "a11"), Some("!a:x")), None, "a subspace may move up to its grandparent");
    }

    #[test]
    fn a_move_needs_the_right_on_the_new_and_on_the_old_parent() {
        // «ro» and «ro2» are spaces whose children the user may not change; «rw» may. kid sits in «ro», free at the top.
        let s = sidebar(&[("ro", 0, Some(&["kid"])), ("kid", 0, Some(&[])), ("ro2", 0, Some(&[])), ("rw", 0, Some(&["rwkid"])), ("rwkid", 0, Some(&[])), ("free", 0, Some(&[]))]);
        let rows = all(&s, None);
        let (kid, free, rwkid) = (at(&rows, "kid"), at(&rows, "free"), at(&rows, "rwkid"));
        assert!(!rows[at(&rows, "ro")].editable && rows[at(&rows, "rw")].editable);
        assert_eq!(refusal(&rows, free, Some("!ro2:x")).as_deref(), Some("Non hai i permessi per modificare «ro2»"), "new parent");
        assert_eq!(refusal(&rows, kid, Some("!rw:x")).as_deref(), Some("Non hai i permessi per modificare «ro»"), "old parent");
        assert_eq!(refusal(&rows, kid, None).as_deref(), Some("Non hai i permessi per modificare «ro»"), "out to the top level");
        assert_eq!(refusal(&rows, kid, Some("!ro:x")).as_deref(), Some("Non hai i permessi per modificare «ro»"), "reorder inside");
        assert_eq!(refusal(&rows, free, Some("!rw:x")), None);
        assert_eq!(refusal(&rows, rwkid, None), None);
        assert_eq!(refusal(&rows, free, None), None, "the top level is the user's own order");
    }
}
