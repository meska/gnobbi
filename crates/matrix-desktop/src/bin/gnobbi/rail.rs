//! Spaces rail, folder-bar style: «Non lette», «Tutte le chat», the joined spaces as a tree (icon above its name) and, at the bottom,
//! «Organizza spazi» (the drawer where the spaces are dragged into order and into each other) and «Impostazioni» (the settings drawer).
use matrix_desktop::i18n;
use crate::icons;
use crate::list::{badge, one_line, rgb};
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
    /// The list shows only the shown account (picked in the account menu); «Non lette» and «Tutte le chat» clear it and list every account.
    pub one_account: bool,
    /// Parent spaces shown with their subspaces (room ids); this run only.
    pub open: HashSet<String>,
    pub edit: bool,
    /// The settings, devices and accounts drawers are out. One drawer at a time: set them through `set_edit` / `set_settings` / `set_devices` / `set_accounts`.
    pub settings: bool,
    pub devices: bool,
    pub accounts: bool,
    /// «Personalizza»: look and size drawer, opened from the settings.
    pub customize: bool,
    /// The devices drawer was just opened: the caller refreshes the list once and clears this.
    pub refresh_devices: bool,
    /// «Send feedback» was tapped; the shell hands it to the session and clears it.
    pub feedback: bool,
    pub moving: Option<Move>,
    /// Why the last move did not happen.
    pub error: Option<String>,
    /// Where the rail was drawn last; the drawer slides out from its right edge.
    pub panel: Option<egui::Rect>,
}

impl State {
    /// Closes every drawer but the one being opened (`keep`: "edit", "settings", "devices", "accounts" or "customize").
    fn only(&mut self, keep: &str, on: bool) {
        for (name, flag) in [("edit", &mut self.edit), ("settings", &mut self.settings), ("devices", &mut self.devices), ("accounts", &mut self.accounts), ("customize", &mut self.customize)] {
            if name == keep { *flag = on } else { *flag &= !on }
        }
    }

    pub fn set_edit(&mut self, on: bool) { self.only("edit", on) }

    pub fn set_settings(&mut self, on: bool) { self.only("settings", on) }

    pub fn set_devices(&mut self, on: bool) {
        self.refresh_devices |= on && !self.devices;
        self.only("devices", on)
    }

    pub fn set_accounts(&mut self, on: bool) { self.only("accounts", on) }

    pub fn set_customize(&mut self, on: bool) { self.only("customize", on) }

    pub fn any_drawer(&self) -> bool { self.edit || self.settings || self.devices || self.accounts || self.customize }
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
        return Some(i18n::t("A space cannot go inside itself").into());
    }
    [parent, rows[at].parent.as_deref()].into_iter().flatten().filter_map(find).find(|&i| !rows[i].editable).map(|i| i18n::t("You do not have permission to edit «{name}»").replace("{name}", &rows[i].name))
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
        state.error = Some(matrix_desktop::i18n::t(matrix_engine::operations::space_error_line(e)).into());
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
    let c = crate::dir::flip_pos(rect, egui::pos2(rect.center().x + depth * 4., rect.top() + 22.));
    let ink = rgb(if selected { t.accent } else { t.secondary });
    let p = ui.painter();
    let glyph = crate::metrics::get().icon + 6.;
    match &icon {
        Icon::Unread => icons::paint(p, c, icons::Icon::ChatCircleDots, selected, glyph, ink),
        Icon::All => icons::paint(p, c, icons::Icon::Chats, selected, glyph, ink),
        Icon::Edit => icons::paint(p, c, icons::Icon::SlidersHorizontal, selected, glyph, ink),
        Icon::Settings => icons::paint(p, c, icons::Icon::GearSix, selected, glyph, ink),
        Icon::Space(row) => {
            let size = if row.depth > 0 { 28. } else { 34. };
            let initial: String = row.name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
            crate::avatar::paint(ui, c, size, rgb(crate::AVATAR), &crate::avatar::lookup(avatars, row.avatar.as_deref()), &initial, size * 0.45, rgb(t.on_accent));
            if row.depth > 0 {
                let x = crate::dir::flip_pos(rect, egui::pos2(rect.left() + 11., 0.)).x;
                p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + 40.)], Stroke::new(1., rgb(t.border)));
            }
            if row.has_children {
                // Chevron: right when folded, down when open.
                icons::paint(p, crate::dir::flip_pos(rect, egui::pos2(rect.left() + 12. + depth * 4., c.y)), if row.open { icons::Icon::CaretDown } else { icons::Icon::CaretRight }, false, crate::metrics::get().icon - 4., rgb(t.secondary));
            }
        }
    }
    let name = one_line(ui, label, 11., rgb(if selected { t.text } else { t.secondary }), WIDTH - 12.);
    p.galley(egui::pos2(rect.center().x - name.size().x / 2., rect.top() + 41.), name, rgb(t.text));
    if n > 0 {
        badge(ui, c + Vec2::new(if crate::dir::rtl() { -17. } else { 17. }, -12.), n, t);
    }
    let spoken = if matches!(icon, Icon::Edit) { i18n::t("Organise spaces").to_owned() } else if n > 0 { i18n::t("{label}, {n} unread").replace("{label}", label).replace("{n}", &n.to_string()) } else { label.to_owned() };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &spoken));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The rail below the account picture. `account` is the catalog id the layout is stored under; `placed` the engine's answer to a move;
/// `totals` the badges of «Non lette» (rooms) and «Tutte le chat» (messages), summed over every account.
pub fn show(ui: &mut egui::Ui, sidebar: &mut Sidebar, avatars: &crate::timeline::Avatars, state: &mut State, account: u64, placed: Option<Result<(), OperationError>>, totals: (u64, u64), t: &DesignTokens) {
    // Test hooks: GNOBBI_EGUI_VIEW=rail-open unfolds every parent space, rail-edit shows «Organizza spazi», rail-drag / rail-line also a held row.
    let view = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
    if matches!(view.as_str(), "rail-edit" | "rail-drag" | "rail-line") {
        state.set_edit(true);
    }
    let layout = state.layouts.get(account);
    settle(state, sidebar, &layout, placed);
    state.panel = Some(ui.max_rect());
    let rows = tree(sidebar, &layout, &state.open, view == "rail-open", state.moving.as_ref());
    let (unread_rooms, all) = totals;
    let (unread_view, in_space) = (sidebar.unread_view(), sidebar.active_space().is_some());
    ui.spacing_mut().item_spacing.y = 0.;
    egui::ScrollArea::vertical().max_height(ui.available_height() - 2. * ITEM - 4.).auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.);
        if item(ui, avatars, t, Icon::Unread, i18n::t("Unread"), unread_rooms, unread_view).clicked() {
            sidebar.set_unread_view(true);
            state.one_account = false;
        }
        if item(ui, avatars, t, Icon::All, i18n::t("All chats"), all, !unread_view && !in_space).clicked() {
            sidebar.set_space(None);
            state.one_account = false;
        }
        for row in &rows {
            let resp = item(ui, avatars, t, Icon::Space(row), &row.name, row.unread, row.selected);
            if sidebar.is_leaving(&row.scope) {
                // «Esci» sent, the server not yet answered: faded with a spinner, painted so the next item stays put.
                ui.painter().rect_filled(resp.rect, 0., ui.visuals().panel_fill.gamma_multiply(0.6));
                egui::Spinner::new().size(18.).paint_at(ui, egui::Rect::from_center_size(resp.rect.center_top() + Vec2::new(0., 22.), Vec2::splat(18.)));
            }
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
    if item(ui, avatars, t, Icon::Edit, i18n::t("Spaces"), 0, state.edit).clicked() {
        state.set_edit(!state.edit);
    }
    if item(ui, avatars, t, Icon::Settings, i18n::t("Settings"), 0, state.settings).clicked() {
        state.set_settings(!state.settings);
    }
}

/// Small drawn arrow button of the edit window.
fn arrow(ui: &mut egui::Ui, t: &DesignTokens, up: bool, enabled: bool, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(crate::metrics::button()), if enabled { Sense::click() } else { Sense::hover() });
    let resp = crate::metrics::clickable(resp);
    if enabled && (resp.hovered() || resp.has_focus()) {
        ui.painter().rect_filled(rect, 8., rgb(t.input));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, 8., Stroke::new(1.5, rgb(t.accent)), egui::StrokeKind::Inside);
    }
    icons::paint(ui.painter(), rect.center(), if up { icons::Icon::CaretUp } else { icons::Icon::CaretDown }, false, crate::metrics::get().icon, rgb(if enabled { t.text } else { t.disabled }));
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
fn face(ui: &egui::Ui, avatars: &crate::timeline::Avatars, row: &Row, c: egui::Pos2, size: f32, t: &DesignTokens) {
    let initial: String = row.name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
    crate::avatar::paint(ui, c, size, rgb(crate::AVATAR), &crate::avatar::lookup(avatars, row.avatar.as_deref()), &initial, size * 0.45, rgb(t.on_accent));
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
    if ctx.content_rect().width() < crate::SINGLE_WIDTH {
        // One column: `page` moves it; here only «is it still on screen».
        return if shown { 1. } else { 1. - ctx.data(|d| d.get_temp::<crate::mobile::Nav>(egui::Id::new((name, "page")))).map_or(1., |n| n.progress) };
    }
    ctx.animate_bool_with_time(egui::Id::new((name, "open")), shown, 0.18)
}

/// How far out the one-column drawers are, as of this frame or the one before: (those with a «‹», any). Read by what lies under them.
pub fn pages_open(ctx: &egui::Context) -> (f32, f32) {
    let now = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(u64, f32, f32)>(egui::Id::new("pages-open"))).filter(|(at, ..)| at + 1 >= now).map_or((0., 0.), |(_, child, any)| (child, any))
}

fn publish_page(ctx: &egui::Context, child: bool, open: f32) {
    let (now, id) = (ctx.cumulative_frame_nr(), egui::Id::new("pages-open"));
    let (_, c, a) = ctx.data(|d| d.get_temp::<(u64, f32, f32)>(id)).filter(|(at, ..)| *at == now).unwrap_or_default();
    ctx.data_mut(|d| d.insert_temp(id, (now, if child { c.max(open) } else { c }, a.max(open))));
}

/// Shift of a one-column drawer from its place: off to the side while it is not fully out, and back by 30 % of what the page above it shows.
fn page_shift(open: f32, above: f32, width: f32) -> f32 {
    (1. - open) * width - 0.3 * above * width
}

/// One column: the drawer `name` is a page that comes in from the side and follows the back swipe from the screen edge.
/// (how far it is out 0..1, how far the page above it is out, the swipe was let go as «back»).
fn page(ctx: &egui::Context, name: &str, shown: bool, child: bool) -> (f32, f32, bool) {
    use crate::mobile::BackDrag;
    let id = egui::Id::new((name, "page"));
    let mut nav: crate::mobile::Nav = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    // A drawer without «‹» stays under the ones that return to it; only the one on top takes the gesture.
    let above = if child { 0. } else { pages_open(ctx).0 };
    let drag = if shown && above == 0. { crate::mobile::back_drag(ctx) } else { None };
    let swiped = drag == Some(BackDrag::Released(true));
    let hidden = !shown || swiped;
    nav.step(true, hidden, match drag { Some(BackDrag::Moving(p)) => Some(p), _ => None }, std::time::Instant::now(), crate::mobile::reduce_motion());
    if let Some(p) = crate::mobile::nav_forced().filter(|_| shown) {
        nav.progress = if above > 0. { 0. } else { p };
    }
    if nav.wants_repaint(hidden) {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, nav));
    if nav.progress < 1. {
        publish_page(ctx, child, 1. - nav.progress);
    }
    (1. - nav.progress, above, swiped)
}

/// Left edge and width of a drawer. In the single-column mobile layout the list fills the window, so the rail's right edge leaves
/// no room: the drawer takes the whole content rect (already inside the safe area) instead of 0 px.
fn drawer_span(rail_right: f32, content: egui::Rect) -> (f32, f32) {
    if content.width() < crate::NARROW_WIDTH { (content.left(), content.width()) } else { (rail_right, DRAWER.min(content.right() - rail_right)) }
}

/// A drawer beside the rail (`panel`): it slides out from the rail's right edge over the dimmed window, with its title and an X.
/// `body` gets the rectangle below the title. True when the X or the dimmed part was clicked; Esc is the caller's.
pub fn drawer(ctx: &egui::Context, panel: Option<egui::Rect>, name: &'static str, title: &str, shown: bool, t: &DesignTokens, body: impl FnOnce(&mut egui::Ui, egui::Rect)) -> bool {
    drawer_back(ctx, panel, name, title, shown, t, None, body)
}

/// `drawer` with a «‹» left of the title when `back` is given; a click on it sets `*back`.
pub fn drawer_back(ctx: &egui::Context, panel: Option<egui::Rect>, name: &'static str, title: &str, shown: bool, t: &DesignTokens, back: Option<&mut bool>, body: impl FnOnce(&mut egui::Ui, egui::Rect)) -> bool {
    let content = ctx.content_rect();
    // One column (a phone): a page from the far side, over the list or over the drawer it came from, that follows the back swipe.
    let phone = content.width() < crate::SINGLE_WIDTH;
    let (open, above, swiped) = if phone { page(ctx, name, shown, back.is_some()) } else { (slide(ctx, name, shown), 0., false) };
    let Some(panel) = panel.filter(|_| open > 0.) else { return false };
    // Worked out as if the rail were on the left, then mirrored in the window: in RTL the drawer slides out of the rail's left edge.
    let flip = |r| crate::dir::flip(content, r);
    let panel = flip(panel);
    let (left, width) = drawer_span(panel.right(), content);
    let clip = egui::Rect::from_min_max(egui::pos2(left, panel.top()), egui::pos2(content.right(), panel.bottom()));
    let shift = if phone { page_shift(open, above, width) } else { -(1. - open) * width };
    let rect = flip(egui::Rect::from_min_size(clip.min + Vec2::new(shift, 0.), Vec2::new(width, panel.height())));
    let clip = flip(clip);
    let mut closed = false;
    let mut back = back;
    // Under the page that returns to it: its own layer below, so the one on top stays on top whichever was opened first.
    let under = above > 0.;
    // The rest of the window is dimmed: a click there closes the drawer and reaches nothing else. The rail stays usable.
    egui::Area::new(egui::Id::new((name, "scrim"))).order(egui::Order::Middle).fixed_pos(clip.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(clip.size(), Sense::click());
        ui.painter().rect_filled(r, 0., egui::Color32::from_black_alpha(if phone { 0 } else { (open * 102.) as u8 }));
        closed |= resp.clicked();
    });
    let area = if under { egui::Area::new(egui::Id::new((name, "under"))).order(egui::Order::Middle) } else { egui::Area::new(egui::Id::new(name)).order(egui::Order::Foreground) };
    area.fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        ui.set_clip_rect(clip);
        ui.set_min_size(rect.size());
        ui.interact(rect, egui::Id::new((name, "back")), Sense::click());
        let p = ui.painter().clone();
        if phone {
            // Dim and shadow on what is under, and the page's base up into the safe-area bands.
            crate::mobile::nav_cover(ui, 1. - open, rect.left() - content.left(), rgb(t.surface));
        }
        p.rect_filled(rect, 0., rgb(t.surface));
        let head = egui::Rect::from_min_size(rect.min, Vec2::new(width, 56.));
        let flip = |r| crate::dir::flip(head, r);
        let mut title_x = 20.;
        if let Some(back) = back.as_deref_mut() {
            let b = flip(egui::Rect::from_center_size(egui::pos2(head.left() + 30., head.center().y), Vec2::splat(crate::metrics::button() + 4.)));
            let r = ui.interact(b, egui::Id::new((name, "return")), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            lit(ui, &r, b, 18., t);
            icons::paint(&p, b.center(), icons::Icon::CaretLeft, false, crate::metrics::get().icon, rgb(t.secondary));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, matrix_desktop::i18n::t("Back")));
            *back |= r.clicked();
            title_x = 52.;
        }
        p.text(crate::dir::flip_pos(head, head.left_center() + Vec2::new(title_x, 0.)), crate::dir::align(Align2::LEFT_CENTER), title, FontId::proportional(17.), rgb(t.text));
        let x = flip(egui::Rect::from_center_size(egui::pos2(head.right() - 30., head.center().y), Vec2::splat(crate::metrics::button() + 4.)));
        let close = ui.interact(x, egui::Id::new((name, "close")), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        lit(ui, &close, x, 18., t);
        icons::paint(&p, x.center(), icons::Icon::X, false, crate::metrics::get().icon, rgb(t.secondary));
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, matrix_desktop::i18n::t("Close")));
        closed |= close.clicked();
        let body_rect = egui::Rect::from_min_max(head.left_bottom(), rect.max);
        if phone {
            // One scope for both layers of the page: its scroll position survives the change.
            ui.scope_builder(egui::UiBuilder::new().scope_id(egui::Id::new((name, "body"))), |ui| crate::dir::scope(ui, |ui| body(ui, body_rect)));
        } else {
            crate::dir::scope(ui, |ui| body(ui, body_rect));
        }
    });
    // The swipe is the «‹» where there is one, else the X.
    if swiped {
        match back {
            Some(back) => *back = true,
            None => closed = true,
        }
    }
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
    let closed = drawer(ctx, state.panel, "rail-drawer", i18n::t("Organise spaces"), state.edit, t, |ui, rect| {
        let (p, width) = (ui.painter().clone(), rect.width());
        let size = crate::metrics::button() + 4.;
        let action = crate::dir::flip(rect, egui::Rect::from_center_size(egui::pos2(rect.right() - 30. - size, rect.top() - 28.), Vec2::splat(size)));
        let label = matrix_desktop::i18n::t("New space");
        if icons::button_at(ui, action, egui::Id::new("new-space"), icons::Icon::Plus, false, crate::metrics::get().icon, label, Some(rgb(t.accent)), true, 1.).on_hover_text(label).clicked() {
            form.open_creation(true);
            session.new_open = true;
            act = Some(Edit::Close);
        }
        let mut top = rect.top() + 8.;
        if let Some(error) = &state.error {
            let line = one_line(ui, error, 13., rgb(t.error), width - 40.);
            p.galley(crate::dir::place(rect, egui::pos2(rect.left() + 20., top + 18. - line.size().y / 2.), line.size().x), line, rgb(t.error));
            top += 36.;
        }
        if rows.is_empty() {
            p.text(egui::pos2(rect.center().x, top + 40.), Align2::CENTER_CENTER, i18n::t("No spaces"), FontId::proportional(15.), rgb(t.secondary));
            return;
        }
        let body = egui::Rect::from_min_max(egui::pos2(rect.left(), top + 4.), rect.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let (top, left) = (ui.cursor().top(), ui.max_rect().left());
                let view = matrix_desktop::hooks::var("GNOBBI_EGUI_VIEW").unwrap_or_default();
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
                    let depth = rows[at].depth as f32 + from.map_or(0., |o| ((pos.x - o.x) * if crate::dir::rtl() { -1. } else { 1. } / INDENT).round());
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
                    // LTR maths; `flip` puts each piece on its side of the row.
                    let flip = |p: egui::Pos2| crate::dir::flip_pos(r, p);
                    let right = egui::Rect::from_min_max(egui::pos2(r.right() - 132., r.top()), egui::pos2(r.right() - 20., r.bottom()));
                    let grab = egui::Rect::from_min_max(r.min, egui::pos2(right.left(), r.bottom())).shrink2(Vec2::new(6., 2.));
                    let resp = ui.interact(crate::dir::flip(r, grab), egui::Id::new(("rail-row", row.id())), if frozen { Sense::hover() } else { Sense::click_and_drag() });
                    if target.as_ref().is_some_and(|tg| tg.mark == Mark::Into(i)) {
                        ui.painter().rect(r.shrink2(Vec2::new(6., 2.)), 10., ink.gamma_multiply(0.18), Stroke::new(2., ink), egui::StrokeKind::Inside);
                    } else if held.is_none() {
                        lit(ui, &resp, crate::dir::flip(r, grab), 10., t);
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
                        let (inward, outward) = if crate::dir::rtl() { (egui::Key::ArrowLeft, egui::Key::ArrowRight) } else { (egui::Key::ArrowRight, egui::Key::ArrowLeft) };
                        if let Some(b) = prev.filter(|_| key(egui::Key::ArrowUp)) {
                            act = Some(Edit::Place(i, row.parent.clone(), Some(id(b))));
                        } else if let Some(n) = next.filter(|_| key(egui::Key::ArrowDown)) {
                            act = Some(Edit::Place(i, row.parent.clone(), siblings(n).1.map(id)));
                        } else if let Some(b) = prev.filter(|_| key(inward)) {
                            act = Some(Edit::Place(i, Some(id(b)), None));
                        } else if let Some(p) = row.parent.as_ref().filter(|_| key(outward)).and_then(|p| rows.iter().position(|r| r.id() == p)) {
                            act = Some(Edit::Place(i, rows[p].parent.clone(), siblings(p).1.map(id)));
                        }
                    }
                    let mut p = ui.painter().clone();
                    if held == Some(i) {
                        p.set_opacity(0.35);
                    }
                    let (mut x, cy) = (grab.left() + 14., r.center().y);
                    if row.busy {
                        egui::Spinner::new().color(rgb(t.accent)).paint_at(ui, egui::Rect::from_center_size(flip(egui::pos2(x, cy)), Vec2::splat(16.)));
                    } else {
                        icons::paint(&p, flip(egui::pos2(x, cy)), icons::Icon::DotsSixVertical, false, crate::metrics::get().icon, rgb(if frozen { t.disabled } else { t.placeholder }));
                    }
                    x += 18. + row.depth as f32 * INDENT;
                    face(ui, &session.avatars, row, flip(egui::pos2(x + 14., cy)), 28., t);
                    let name = one_line(ui, &row.name, 15., rgb(if row.hidden { t.placeholder } else { t.text }), grab.right() - x - 44.);
                    p.galley(crate::dir::place(r, egui::pos2(x + 38., cy - name.size().y / 2.), name.size().x), name, rgb(t.text));
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !frozen, i18n::t("Move {name}").replace("{name}", &row.name)));
                    if !frozen {
                        resp.on_hover_cursor(egui::CursorIcon::Grab);
                    }
                    ui.scope_builder(egui::UiBuilder::new().max_rect(crate::dir::flip(r, right)).layout(crate::dir::row_rev(egui::Align::Center)), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.;
                        let mut shown = !row.hidden;
                        if crate::settings::toggle(ui, &row.name, &mut shown, t) {
                            act = Some(Edit::Toggle(row.scope.clone()));
                        }
                        ui.add_space(14.);
                        if arrow(ui, t, false, allowed && next.is_some(), &i18n::t("Move {name} down").replace("{name}", &row.name)) && let Some(n) = next {
                            act = Some(Edit::Place(i, row.parent.clone(), siblings(n).1.map(id)));
                        }
                        if arrow(ui, t, true, allowed && prev.is_some(), &i18n::t("Move {name} up").replace("{name}", &row.name)) && let Some(b) = prev {
                            act = Some(Edit::Place(i, row.parent.clone(), Some(id(b))));
                        }
                    });
                }
                let (Some(at), Some(pos)) = (held, pos) else { return };
                if let Some(Target { mark: Mark::Line(gap, depth), .. }) = &target {
                    let line = crate::dir::flip(ui.max_rect(), egui::Rect::from_x_y_ranges((left + 26. + *depth as f32 * INDENT)..=(ui.max_rect().right() - 20.), (top + *gap as f32 * ROW)..=(top + *gap as f32 * ROW)));
                    let a = if crate::dir::rtl() { line.right_top() } else { line.left_top() };
                    ui.painter().hline(line.x_range(), a.y, Stroke::new(2., ink));
                    ui.painter().circle_filled(a, 4., ink);
                }
                // The held row follows the pointer, above everything.
                let g = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("rail-held")));
                let card = egui::Rect::from_min_size(pos + Vec2::new(14., -20.), Vec2::new(220., 40.));
                g.rect_filled(card.translate(Vec2::new(0., 3.)), 10., egui::Color32::from_black_alpha(48));
                g.rect(card, 10., rgb(t.input), Stroke::new(1., rgb(t.border)), egui::StrokeKind::Inside);
                face(ui, &session.avatars, &rows[at], card.left_center() + Vec2::new(24., 0.), 26., t);
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
    #[test]
    fn drawer_fills_a_narrow_window_and_stays_beside_the_rail_on_desktop() {
        let r = |w| egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(w, 800.));
        assert_eq!(super::drawer_span(393., r(393.)), (0., 393.));
        assert_eq!(super::drawer_span(72., r(1000.)), (72., 380.));
    }

    #[test]
    fn one_column_drawer_is_a_page_from_the_side_that_follows_the_back_swipe() {
        assert_eq!(super::page_shift(1., 0., 400.), 0., "out: in place");
        assert_eq!(super::page_shift(0.25, 0., 400.), 300., "coming in from the far side");
        assert!((super::page_shift(1., 0.5, 400.) + 60.).abs() < 1e-3, "under a page half out: trails at 30 %");
        let ctx = egui::Context::default();
        let mut frame = |shown, child, events: Vec<egui::Event>| {
            let mut got = (0., 0., false);
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400., 800.))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| got = super::page(ui.ctx(), "t", shown, child));
            out.textures_delta.clear();
            got
        };
        let button = |x: f32, pressed| egui::Event::PointerButton { pos: egui::pos2(x, 300.), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let moved = |x: f32| egui::Event::PointerMoved(egui::pos2(x, 300.));
        assert_eq!(frame(true, true, vec![]), (1., 0., false), "first frame: as it is, no move");
        assert_eq!(super::pages_open(&ctx), (1., 1.), "what is under knows a page is out");
        frame(true, true, vec![moved(5.), button(5., true)]);
        assert_eq!(frame(true, true, vec![moved(105.)]), (0.75, 0., false), "the page follows the finger");
        assert_eq!(frame(true, true, vec![moved(205.)]).0, 0.5);
        assert_eq!(frame(true, true, vec![button(205., false)]), (0.5, 0., true), "let go past a third: back, from where it was");
        // The drawer under it (no «‹») sees the page above and leaves the gesture to it.
        let mut frame = |name, child, events: Vec<egui::Event>| {
            let mut got = (0., 0., false);
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(400., 800.))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| got = super::page(ui.ctx(), name, true, child));
            out.textures_delta.clear();
            got
        };
        frame("t", true, vec![moved(5.), button(5., true)]);
        frame("t", true, vec![]);
        let root = frame("u", false, vec![moved(205.)]);
        assert!(root.0 == 1. && root.1 > 0., "{root:?}");
    }

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
        s.set_accounts(true);
        assert!(s.accounts && !s.settings && !s.devices && !s.edit);
        s.set_devices(true);
        assert!(!s.accounts);
        s.set_customize(true);
        assert!(s.customize && !s.devices && !s.settings && !s.accounts && !s.edit);
        s.set_settings(true);
        assert!(s.settings && !s.customize);
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
            assert_eq!(refusal(&rows, a, Some(parent)).as_deref(), Some("A space cannot go inside itself"));
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
        assert_eq!(refusal(&rows, free, Some("!ro2:x")).as_deref(), Some("You do not have permission to edit «ro2»"), "new parent");
        assert_eq!(refusal(&rows, kid, Some("!rw:x")).as_deref(), Some("You do not have permission to edit «ro»"), "old parent");
        assert_eq!(refusal(&rows, kid, None).as_deref(), Some("You do not have permission to edit «ro»"), "out to the top level");
        assert_eq!(refusal(&rows, kid, Some("!ro:x")).as_deref(), Some("You do not have permission to edit «ro»"), "reorder inside");
        assert_eq!(refusal(&rows, free, Some("!rw:x")), None);
        assert_eq!(refusal(&rows, rwkid, None), None);
        assert_eq!(refusal(&rows, free, None), None, "the top level is the user's own order");
    }
}
