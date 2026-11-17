//! «Ricerca rapida» overlay (Cmd+K / Ctrl+K, drawer entry): local search over the rooms already
//! known plus the «Visti di recente» / «Ricerche recenti» lists of `matrix_desktop::quick_search`.
//! Closes on Esc or a click outside, like the drawer. Keys are handled in `menu_key` (capture),
//! so room navigation underneath never sees them.
//! «Cerca» section: the «Persone» entry puts a chip in the field; server users («Utenti», from
//! `UserSearch` through `OperationCommand::SearchUsers`) follow the local results.
use crate::*;
use matrix_desktop::directory_servers::ServerPicker;
use matrix_desktop::new_chat::{Picker, Status};
use crate::directory_servers_view::render_server_picker;
use matrix_desktop::search_list::{Choice, Row, SearchList, more_text};
use matrix_desktop::quick_search::{self as qs, Category, Entry, Hit, Index, Recent};
use matrix_engine::operations::{DirectoryRoom, OperationCommand, SearchState};
use std::ops::Range;

const ROW_H: f32 = 44.;

#[derive(Clone, Copy, PartialEq)]
enum JoinKind {
    Working,
    Done,
    Failed,
    Retry,
}

/// The status line of a row: a pulsing dot while the join runs (the project's one animation primitive, as the
/// drawer), the error in the error colour, «Riprova» as the action of an unconfirmed join (a click on the row).
fn join_row_line(text: &'static str, kind: JoinKind, t: matrix_desktop::DesignTokens) -> gpui::AnyElement {
    use gpui::{Animation, AnimationExt};
    let color = match kind {
        JoinKind::Failed => t.error,
        _ => t.secondary,
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(rgb(color))
        .when(kind == JoinKind::Working, |d| {
            d.child(
                div().size(px(8.)).flex_shrink_0().rounded_full().bg(rgb(t.accent)).with_animation(
                    "join-dot",
                    Animation::new(std::time::Duration::from_secs(1)).repeat(),
                    |d, delta| d.opacity(gpui::pulsating_between(0.3, 1.0)(delta)),
                ),
            )
        })
        .child(div().truncate().child(text))
        .when(kind == JoinKind::Retry, |d| d.child(div().flex_shrink_0().text_color(rgb(t.accent)).child("· Riprova")))
        .into_any_element()
}

/// The one line a row shows for its join (the JOIN_STATE_MACHINE notes §6); `None`: the normal row.
fn join_line(state: Option<&matrix_desktop::joins::JoinState>) -> Option<(&'static str, JoinKind)> {
    use matrix_desktop::joins::JoinState::*;
    Some(match state? {
        Pending => ("Ingresso…", JoinKind::Working),
        Slow => ("Ancora in corso…", JoinKind::Working),
        Joined(_) => ("Già unito", JoinKind::Done),
        Refused(e) => (crate::new_room_view::error_text(false, true, *e), JoinKind::Failed),
        Unconfirmed => ("Non confermato", JoinKind::Retry),
    })
}
const LIST_MAX_H: f32 = 308.;
/// Local list height while «Utenti» and «Cerca» share the overlay.
const SHARED_LIST_MAX_H: f32 = 176.;
const LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Zone {
    /// The directory server selector, right of the field.
    Server,
    /// The horizontal «Visti di recente» row.
    Seen,
    /// The vertical list: «Ricerche recenti» while the field is empty, else the local results.
    Rows,
    /// «Utenti» from the server.
    Users,
    /// Public rooms or spaces from the directory.
    Rooms,
    /// The «Cerca» entries.
    Search,
}

impl Zone {
    fn idx(self) -> usize {
        self as usize
    }
    const ALL: [Zone; 6] =
        [Zone::Server, Zone::Seen, Zone::Rows, Zone::Users, Zone::Rooms, Zone::Search];
}

pub(crate) struct QuickSearch {
    field: gpui::Entity<crate::login_field::LoginField>,
    index: Index,
    term: String,
    hits: Vec<Hit>,
    pub(crate) chip: Option<Category>,
    /// Server users for `term`, and the last term handed to the engine.
    users: Picker,
    sent: Option<String>,
    /// Public directory: the hits, the last `(term, server, kind)` sent, and the server selector
    /// (the «Unisciti» one) with its «Aggiungi nuovo server…» name field.
    pub(crate) rooms: SearchList<DirectoryRoom>,
    rooms_sent: Option<qs::DirectoryRequest>,
    pub(crate) servers: Option<ServerPicker>,
    server_focus: FocusHandle,
    pub(crate) add_field: Option<gpui::Entity<crate::login_field::LoginField>>,
    pub(crate) add_focus: FocusHandle,
    /// A join or DM is waiting for the engine: the entry to remember on success, and whether it is a DM.
    busy: Option<(Recent, bool)>,
    /// The room joined by the last click on a directory row: its success opens the room only while this popup is open.
    /// Rooms never set `busy`: the field and the other rows stay usable while the join runs.
    pub(crate) joining: Option<Recent>,
    /// The one line shown under the field when that join or DM failed.
    error: Option<&'static str>,
    zone: Zone,
    active: usize,
    scroll: UniformListScrollHandle,
    seen_scroll: gpui::ScrollHandle,
    scrollbar: scrollbar_view::Scrollbar,
}

impl QuickSearch {
    pub(crate) fn set_users(&mut self, state: SearchState) {
        self.users.set_state(state, false);
    }

    pub(crate) fn set_rooms(&mut self, state: matrix_engine::search::SearchState<DirectoryRoom>) {
        self.rooms.set_state(state);
    }

    /// A directory chip is set: results come from the public directory.
    fn directory(&self) -> bool {
        self.chip.is_some_and(|c| c.kind().is_some())
    }

    pub(crate) fn focus_field(&self, window: &mut Window, cx: &gpui::App) {
        self.field.read(cx).focus.focus(window);
    }

    /// The directory hits, without the «Altri risultati» row.
    fn room_entries(&self) -> Vec<&DirectoryRoom> {
        self.rooms.rows().into_iter().filter_map(|r| if let Row::Hit(h) = r { Some(h) } else { None }).collect()
    }
}

/// Where a navigation key puts the highlight; `None` when it does not move. `lens` is the
/// item count of each zone (in `Zone::ALL` order): empty zones are skipped.
fn step(zone: Zone, active: usize, key: &str, lens: [usize; 6]) -> Option<(Zone, usize)> {
    let next = |from: usize| Zone::ALL[from + 1..].iter().copied().find(|z| lens[z.idx()] > 0);
    let prev = |from: usize| Zone::ALL[..from].iter().rev().copied().find(|z| lens[z.idx()] > 0);
    let last = |z: Zone| if z == Zone::Seen { 0 } else { lens[z.idx()] - 1 };
    let horizontal = zone == Zone::Seen;
    let moved = match key {
        "left" if horizontal && active > 0 => Some((zone, active - 1)),
        "right" if horizontal && active + 1 < lens[Zone::Seen.idx()] => Some((zone, active + 1)),
        "down" if !horizontal && active + 1 < lens[zone.idx()] => Some((zone, active + 1)),
        "down" => next(zone.idx()).map(|z| (z, 0)),
        "up" if !horizontal && active > 0 => Some((zone, active - 1)),
        "up" => prev(zone.idx()).map(|z| (z, last(z))),
        "tab" => next(zone.idx())
            .or_else(|| Zone::ALL.into_iter().find(|z| lens[z.idx()] > 0))
            .map(|z| (z, 0)),
        _ => None,
    };
    moved.filter(|&m| m != (zone, active))
}

/// Where the highlight starts once a chip is set (or removed).
fn chip_zone(chip: Option<Category>) -> Zone {
    match chip {
        None => Zone::Rows,
        Some(Category::People) => Zone::Users,
        Some(_) => Zone::Rooms,
    }
}

/// «Svuota» from the keyboard: Backspace/Delete on the highlighted searches, or Cmd/Ctrl+Backspace.
fn clears(zone: Zone, key: &str, plain: bool, secondary: bool, typing: bool, rows: usize) -> bool {
    !typing
        && rows > 0
        && ((plain && zone == Zone::Rows && matches!(key, "backspace" | "delete"))
            || (secondary && key == "backspace"))
}

/// Name under a «Visti» avatar: one line that fits the 64 px cell, ellipsis at the end.
fn seen_label(name: &str) -> String {
    let name = matrix_desktop::row_text::one_line(name);
    // shortcut: counts chars, not pixels (wide glyphs may still clip); upgrade with text measuring.
    if name.chars().count() > 8 {
        name.chars().take(7).chain(['…']).collect()
    } else {
        name
    }
}

fn seen_display<'a>(recent: &'a Recent, sidebar: &'a matrix_desktop::Sidebar) -> (&'a str, Option<&'a str>) {
    let room = sidebar.rooms().iter().find(|room| room.scope.room_id.as_str() == recent.id);
    let name = room.map(|r| r.name.as_str()).filter(|name| !name.is_empty() && !name.starts_with('!'))
        .or_else(|| (!recent.name.is_empty() && !recent.name.starts_with('!')).then_some(recent.name.as_str()))
        .unwrap_or("Chat");
    let avatar = room.and_then(|r| r.info.avatar.as_deref()).or(recent.avatar.as_deref());
    (name, avatar)
}

/// Row subtitle; a raw room ID (`!abc:server`) is never shown (the screen-reader label keeps it).
fn secondary_label(secondary: &str) -> &str {
    if secondary.starts_with('!') {
        ""
    } else {
        secondary
    }
}

/// «Nessun risultato» is text: `disabled` is a background token.
fn no_results_color(t: &matrix_desktop::DesignTokens) -> u32 {
    t.secondary
}

impl Shell {
    fn recents_account(&self) -> u64 {
        self.active_scope.account_id.as_u64()
    }

    fn save_recents(&self) {
        if let Some(dirs) = &self.prefs_dirs {
            let _ = self.recents.save(dirs);
        }
    }

    /// Any room that becomes the open one lands in «Visti di recente».
    pub(crate) fn note_seen(&mut self) {
        let Some(room) = self.sidebar.selected() else {
            return;
        };
        let item = Recent {
            id: room.scope.room_id.as_str().to_owned(),
            name: room.name.clone(),
            avatar: room.info.avatar.clone(),
        };
        let account = self.recents_account();
        let mut recents = self.recents.get(account);
        recents.opened(item);
        self.recents.set(account, recents);
        self.save_recents();
    }

    /// The local list as shown now (none while a chip limits the results).
    fn quick_rows(&self) -> Vec<Hit> {
        let Some(q) = self.quick.as_ref().filter(|q| q.chip.is_none()) else {
            return Vec::new();
        };
        if !q.term.is_empty() {
            return q.hits.clone();
        }
        self.recents
            .get(self.recents_account())
            .searched()
            .iter()
            .map(|r| Hit {
                id: r.id.clone(),
                name: r.name.clone(),
                avatar: r.avatar.clone(),
                secondary: r.id.clone(),
            })
            .collect()
    }

    /// «Utenti»: server hits for the typed term.
    fn quick_users(&self) -> Vec<Hit> {
        match self.quick.as_ref() {
            Some(q) if !q.term.is_empty() && !q.directory() => qs::user_hits(q.users.entries(), q.chip),
            _ => Vec::new(),
        }
    }

    /// «Stanze pubbliche» under the locals (no chip, a typed term, max 5), or every directory hit
    /// of the chip.
    fn quick_room_hits(&self) -> Vec<Hit> {
        match self.quick.as_ref() {
            Some(q) if q.directory() => qs::room_hits(q.room_entries(), usize::MAX),
            Some(q) if q.chip.is_none() && !q.term.is_empty() => qs::room_hits(q.room_entries(), qs::ROOMS_LIMIT),
            _ => Vec::new(),
        }
    }

    /// The «Altri risultati» row of a directory chip, when there is a next page.
    fn quick_more(&self) -> Option<matrix_engine::search::More> {
        let q = self.quick.as_ref().filter(|q| q.directory())?;
        q.rooms.rows().into_iter().find_map(|r| if let Row::More(m) = r { Some(m) } else { None })
    }

    /// «Cerca» entries; none once a chip is set.
    fn quick_categories(&self) -> &'static [Category] {
        match self.quick.as_ref() {
            Some(q) if q.chip.is_none() => &Category::ALL,
            _ => &[],
        }
    }

    /// Items per zone, in `Zone::ALL` order. «Visti» only shows while browsing.
    fn quick_lens(&self) -> [usize; 6] {
        let browsing = self.quick.as_ref().is_some_and(|q| q.term.is_empty() && q.chip.is_none());
        let seen = if browsing { self.recents.get(self.recents_account()).seen().len() } else { 0 };
        let server = self.quick.as_ref().is_some_and(|q| q.directory() && q.servers.is_some());
        [
            usize::from(server),
            seen,
            self.quick_rows().len(),
            self.quick_users().len(),
            self.quick_room_hits().len() + usize::from(self.quick_more().is_some()),
            self.quick_categories().len(),
        ]
    }

    /// Mxc references the overlay's user rows and searched list want from the cache.
    pub(crate) fn quick_avatars(&self) -> Vec<String> {
        let Some(q) = &self.quick else {
            return Vec::new();
        };
        let recents = self.recents.get(self.recents_account());
        q.users
            .entries()
            .into_iter()
            .filter_map(|e| e.avatar)
            .chain(q.room_entries().into_iter().filter_map(|r| r.avatar_url.clone()))
            .chain(recents.searched().iter().filter_map(|r| r.avatar.clone()))
            .collect()
    }

    /// Sends the typed term to the user search once; called from the operations tick. The
    /// «Nuova chat» dialog owns the search while it is open.
    pub(crate) fn flush_quick_users(&mut self) {
        let Some(q) = self.quick.as_mut().filter(|q| self.new_room.is_none() && !q.directory()) else {
            return;
        };
        let Some(term) = qs::user_request(&q.term, q.sent.as_deref()) else {
            return;
        };
        if let Some((sender, _)) = &mut self.routes.operation
            && sender
                .try_send(OperationCommand::SearchUsers { term: term.clone(), all: false })
                .is_ok()
        {
            q.sent = Some(term);
        }
    }

    /// Sends the directory request the overlay wants (see `qs::room_request`) once; called from
    /// the operations tick. Nothing goes out before a chip is chosen or a term is typed.
    pub(crate) fn flush_quick_rooms(&mut self) {
        let Some(q) = self.quick.as_mut().filter(|_| self.new_room.is_none()) else {
            return;
        };
        let wanted = qs::room_request(q.chip, &q.term, q.servers.as_ref().and_then(|p| p.value()));
        if wanted == q.rooms_sent {
            return;
        }
        let command = match &wanted {
            Some((term, server, kind)) => OperationCommand::SearchRooms { term: term.clone(), server: server.clone(), kind: *kind },
            None => OperationCommand::SearchRooms { term: String::new(), server: None, kind: None },
        };
        if let Some((sender, _)) = &mut self.routes.operation
            && sender.try_send(command).is_ok()
        {
            match &wanted {
                Some((term, ..)) if term.is_empty() => q.rooms.set_suggesting(),
                Some((term, ..)) => q.rooms.set_term(term),
                None => q.rooms.set_term(""),
            }
            q.rooms_sent = wanted;
        }
    }

    /// The overlay is gone: searches still in flight are cancelled.
    pub(crate) fn end_quick(&mut self) {
        let Some(q) = self.quick.take() else {
            return;
        };
        let Some((sender, _)) = &mut self.routes.operation else {
            return;
        };
        if q.sent.as_deref().is_some_and(|t| !t.is_empty()) {
            let _ = sender.try_send(OperationCommand::SearchUsers { term: String::new(), all: false });
        }
        if q.rooms_sent.is_some() {
            let _ = sender.try_send(OperationCommand::SearchRooms { term: String::new(), server: None, kind: None });
        }
    }

    pub(crate) fn open_quick_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected || self.quick.is_some() {
            return;
        }
        self.set_menu_open(false, window, cx);
        let field = cx.new(|cx| {
            let mut field = crate::login_field::LoginField::new(false, cx);
            field.enabled = true;
            field.placeholder = "Cerca";
            field
        });
        cx.observe(&field, |this, field, cx| {
            this.quick_typed(field.read(cx).input.display_text().trim().to_owned());
            cx.notify();
        })
        .detach();
        field.read(cx).focus.focus(window);
        // shortcut: the index is built once per opening (rooms arriving meanwhile are not
        // searched) and ordered like the list; upgrade with `Index::upsert` on room updates.
        let rooms = self.sidebar.rooms();
        let count = rooms.len() as u64;
        let index = Index::new(
            rooms
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.info.is_space && !r.info.invited)
                .map(|(i, r)| Entry::from_room(r, count - i as u64)),
        );
        let has_searched = !self
            .recents
            .get(self.recents_account())
            .searched()
            .is_empty();
        let has_seen = !self.recents.get(self.recents_account()).seen().is_empty();
        self.quick = Some(QuickSearch {
            field,
            index,
            term: String::new(),
            hits: Vec::new(),
            chip: None,
            users: Picker::default(),
            sent: None,
            rooms: SearchList::default(),
            rooms_sent: None,
            servers: self.directory_picker(),
            server_focus: cx.focus_handle(),
            add_field: None,
            add_focus: cx.focus_handle(),
            busy: None,
            joining: None,
            error: None,
            zone: if has_searched {
                Zone::Rows
            } else if has_seen {
                Zone::Seen
            } else {
                Zone::Search
            },
            active: 0,
            scroll: UniformListScrollHandle::new(),
            seen_scroll: gpui::ScrollHandle::new(),
            scrollbar: Default::default(),
        });
        cx.notify();
    }

    pub(crate) fn close_quick_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_quick();
        self.list_focus.focus(window);
        cx.notify();
    }

    fn quick_typed(&mut self, term: String) {
        let Some(q) = self.quick.as_mut().filter(|q| q.term != term) else {
            return;
        };
        q.hits = q.index.search(&term, LIMIT);
        // Old results no longer match: drop them until the engine answers.
        let state = if term.chars().count() < 2 {
            SearchState::Idle
        } else {
            SearchState::Searching
        };
        q.users.set_state(state, false);
        q.term = term;
        q.zone = chip_zone(q.chip);
        q.active = 0;
        q.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
    }

    /// Opens the room; a pick from the search lists (not from «Visti») is remembered too.
    pub(crate) fn quick_pick(&mut self, zone: Zone, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.recents_account();
        let mut recents = self.recents.get(account);
        let item = match zone {
            Zone::Seen => recents.seen().get(i).cloned(),
            Zone::Rows => self.quick_rows().get(i).map(Recent::from),
            Zone::Users => self.quick_users().get(i).map(Recent::from),
            Zone::Rooms => return self.quick_pick_room(i, window, cx),
            Zone::Server | Zone::Search => return,
        };
        if let Some(user) = item.as_ref().filter(|r| r.id.starts_with('@')) {
            return self.quick_pick_user(user.clone(), window, cx);
        }
        let Some(scope) = item.as_ref().and_then(|item| {
            let rooms = self.sidebar.rooms();
            rooms
                .iter()
                .find(|r| r.scope.room_id.as_str() == item.id)
                .map(|r| r.scope.clone())
        }) else {
            // The room is gone (left): drop the dead entry instead of a click that does nothing.
            if let Some(item) = item {
                recents.forget(&item.id);
                self.recents.set(account, recents);
                self.save_recents();
                if let Some(q) = self.quick.as_mut() {
                    q.active = q.active.saturating_sub(1);
                }
                cx.notify();
            }
            return;
        };
        if zone == Zone::Rows {
            recents.picked(item.unwrap());
            self.recents.set(account, recents);
            self.save_recents();
        }
        self.end_quick();
        self.open_scope(scope, window, cx);
    }

    /// Opens a room the account already has: lifts the search that would hide it.
    pub(crate) fn open_scope(&mut self, scope: RoomScope, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sidebar.select(&scope) {
            // Hidden by the text search: lift it rather than not opening.
            self.sidebar.set_query("");
            self.sidebar.select(&scope);
        }
        self.restore.cancel();
        self.sync_room(cx);
        self.set_compact_chat(true);
        self.list_focus.focus(window);
        cx.notify();
    }

    /// A directory row: «Altri risultati» asks the next page; a room the account is in opens, any
    /// other one is joined like «Unisciti» does (room ID through the server that listed it).
    fn quick_pick_room(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(q) = self.quick.as_mut() else {
            return;
        };
        q.rooms.set_active(i);
        let hit = match q.rooms.chosen() {
            Some(Choice::Hit(hit)) => hit.clone(),
            Some(Choice::More) => {
                if let Some((sender, _)) = &mut self.routes.operation {
                    let _ = sender.try_send(OperationCommand::MoreRooms);
                }
                return;
            }
            None => return,
        };
        let joined = self
            .sidebar
            .rooms()
            .iter()
            .find(|r| !r.info.invited && r.scope.room_id.as_str() == hit.room_id)
            .map(|r| r.scope.clone());
        if joined.is_none() && self.pending_operation_command.is_some() {
            return;
        }
        let recent = Recent { id: hit.room_id.clone(), name: hit.name.clone(), avatar: hit.avatar_url.clone() };
        match joined {
            Some(scope) => {
                self.remember_recent(recent);
                self.end_quick();
                self.open_scope(scope, window, cx)
            }
            None => {
                // A join already in flight for this room (double click): nothing to do.
                if self.joins.begin(&hit.room_id, &hit.name, None) == matrix_desktop::joins::Begin::Ignore {
                    return;
                }
                if let Some(q) = self.quick.as_mut() {
                    q.joining = Some(recent);
                    q.error = None;
                }
                self.pending_operation_command =
                    Some(OperationCommand::JoinRoom { target: hit.room_id, via: vec![hit.origin] });
                cx.notify();
            }
        }
    }

    /// A user: the same engine call as «Nuova chat» and the member profile, which opens the existing
    /// DM or creates one; the overlay stays until the answer and remembers the user only on success.
    pub(crate) fn quick_pick_user(&mut self, user: Recent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_operation_command.is_some() || self.quick.is_none() {
            return;
        }
        let user_id = user.id.clone();
        self.quick_busy(user, true, cx);
        self.pending_operation_command = Some(OperationCommand::StartDm { user_id });
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn quick_busy_for_test(&self) -> bool {
        self.quick.as_ref().is_some_and(|q| q.busy.is_some())
    }

    #[cfg(test)]
    pub(crate) fn quick_error_for_test(&self) -> Option<&'static str> {
        self.quick.as_ref().and_then(|q| q.error)
    }

    #[cfg(test)]
    pub(crate) fn quick_open_for_test(&self) -> bool {
        self.quick.is_some()
    }

    #[cfg(test)]
    pub(crate) fn quick_field_enabled_for_test(&self, cx: &gpui::App) -> bool {
        self.quick.as_ref().is_some_and(|q| q.field.read(cx).enabled)
    }

    pub(crate) fn remember_recent(&mut self, recent: Recent) {
        let account = self.recents_account();
        let mut recents = self.recents.get(account);
        recents.picked(recent);
        self.recents.set(account, recents);
        self.save_recents();
    }

    /// The overlay waits for the engine: field disabled, no old error.
    fn quick_busy(&mut self, recent: Recent, dm: bool, cx: &mut Context<Self>) {
        let Some(q) = self.quick.as_mut() else {
            return;
        };
        q.busy = Some((recent, dm));
        q.error = None;
        q.field.update(cx, |f, _| f.enabled = false);
    }

    /// The engine's answer to a join or DM started from the overlay; false when none is waiting.
    pub(crate) fn quick_answered(
        &mut self,
        result: &Result<RoomScope, matrix_engine::operations::OperationError>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(q) = self.quick.as_mut() else {
            return false;
        };
        let Some((recent, dm)) = q.busy.take() else {
            return false;
        };
        match result {
            Ok(_) => {
                self.remember_recent(recent);
                self.end_quick();
            }
            Err(e) => {
                q.error = Some(crate::new_room_view::error_text(dm, !dm, *e));
                q.field.update(cx, |f, _| f.enabled = true);
            }
        }
        true
    }

    /// A «Cerca» entry: the chip goes before the text and limits the results to its kind.
    fn quick_set_chip(&mut self, chip: Option<Category>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(q) = self.quick.as_mut() else {
            return;
        };
        q.chip = chip;
        q.add_field = None;
        if let Some(p) = &mut q.servers {
            p.close();
            p.cancel_add();
        }
        (q.zone, q.active) = (chip_zone(chip), 0);
        q.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        // The click took the focus off the field: give it back so typing continues.
        q.field.read(cx).focus.focus(window);
        cx.notify();
    }

    /// Runs from `menu_key` while the overlay is open; true when the key was consumed.
    pub(crate) fn quick_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let m = &event.keystroke.modifiers;
        let plain = !(m.control || m.alt || m.platform);
        let lens = self.quick_lens();
        let Some(q) = self.quick.as_mut() else {
            return false;
        };
        // Waiting for the engine: only Esc is handled, and it closes the overlay even with a chip.
        if q.busy.is_some() {
            if key == "escape" {
                self.close_quick_search(window, cx);
            }
            return true;
        }
        if let Some(field) = &q.add_field {
            let field_focused = field.read(cx).focus.is_focused(window);
            let button_focused = q.add_focus.is_focused(window);
            if key == "tab" && q.field.read(cx).focus.is_focused(window) {
                if m.shift { q.add_focus.focus(window); } else { field.read(cx).focus.focus(window); }
                cx.notify();
                return true;
            }
            if key == "tab" && (field_focused || button_focused) {
                if field_focused && !m.shift { q.add_focus.focus(window); }
                else if button_focused && m.shift { field.read(cx).focus.focus(window); }
                else { q.focus_field(window, cx); }
                cx.notify();
                return true;
            }
            if button_focused && matches!(key, "enter" | "space") {
                if q.servers.as_ref().is_some_and(|p| p.can_add(&field.read(cx).input.display_text())) {
                    self.confirm_server(window, cx);
                }
                return true;
            }
            if button_focused && key == "escape" { self.cancel_server_add(window, cx); return true; }
        }
        // «Aggiungi nuovo server…»: Enter probes the name, Esc drops it, other keys type.
        if q.add_field.as_ref().is_some_and(|f| f.read(cx).focus.is_focused(window)) {
            match key {
                "enter" => self.confirm_server(window, cx),
                "escape" => self.cancel_server_add(window, cx),
                _ => return false,
            }
            return true;
        }
        // The server menu takes the keys while open; Enter or Down on the selector opens it.
        let menu = q.servers.as_ref().is_some_and(|p| p.is_open());
        if plain && (menu || (q.zone == Zone::Server && matches!(key, "enter" | "down")))
            && let Some(choice) = q.servers.as_mut().and_then(|p| p.key(key))
        {
            self.server_choice(choice, window, cx);
            return true;
        }
        if key == "escape" {
            if qs::escape_closes(q.chip) {
                self.close_quick_search(window, cx);
            } else {
                self.quick_set_chip(None, window, cx);
            }
            return true;
        }
        if plain && key == "backspace" && qs::backspace_removes_chip(q.chip, &q.term) {
            self.quick_set_chip(None, window, cx);
            return true;
        }
        if clears(q.zone, key, plain, m.secondary(), !q.term.is_empty(), lens[Zone::Rows.idx()]) {
            self.quick_clear(window, cx);
            return true;
        }
        match key {
            "enter" if plain => {
                // Typed text found only on the server: Enter takes the first list that has rows.
                let zone = if lens[q.zone.idx()] == 0 {
                    [Zone::Rows, Zone::Users, Zone::Rooms].into_iter().find(|z| lens[z.idx()] > 0)
                } else {
                    Some(q.zone)
                };
                let i = if zone == Some(q.zone) { q.active } else { 0 };
                match zone {
                    Some(Zone::Search) => {
                        if let Some(&c) = Category::ALL.get(i) {
                            self.quick_set_chip(Some(c), window, cx);
                        }
                    }
                    Some(zone) => self.quick_pick(zone, i, window, cx),
                    None => {}
                }
            }
            "up" | "down" | "tab" | "left" | "right" if plain || key == "tab" => {
                // Left/Right belong to the text field unless the «Visti» row has the highlight.
                if matches!(key, "left" | "right") && q.zone != Zone::Seen {
                    return false;
                }
                if let Some((zone, active)) = step(q.zone, q.active, key, lens) {
                    (q.zone, q.active) = (zone, active);
                    match zone {
                        Zone::Rows | Zone::Users => q.scroll.scroll_to_item(active, ScrollStrategy::Top),
                        Zone::Rooms => {
                            q.rooms.set_active(active);
                            q.scroll.scroll_to_item(active, ScrollStrategy::Top)
                        }
                        Zone::Seen => q.seen_scroll.scroll_to_item(active),
                        Zone::Search | Zone::Server => {}
                    }
                }
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    fn quick_avatar(
        &self,
        t: matrix_desktop::DesignTokens,
        avatar: Option<&str>,
        name: &str,
        size: f32,
    ) -> gpui::AnyElement {
        match self.avatars.get(avatar) {
            Some(image) => avatar_view::round(image, size),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(size))
                .flex_shrink_0()
                .rounded_full()
                .bg(rgb(matrix_desktop::AVATAR_FALLBACK))
                .text_size(px(size * 0.4))
                .text_color(rgb(t.avatar_text))
                .child(matrix_desktop::initials(name))
                .into_any_element(),
        }
    }

    /// Screen-reader view: the field, the «Visti» row, then the list, name and Matrix ID each.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn semantic_quick_search(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<matrix_desktop::accessibility::new_room::Snapshot> {
        use accesskit::Role;
        use matrix_desktop::accessibility::new_room::{Item, Snapshot};
        let q = self.quick.as_ref()?;
        let field = q.field.read(cx);
        let mut items = vec![Item {
            role: Role::TextInput,
            label: field.placeholder.to_owned(),
            checked: None,
            focused: field.focus.is_focused(window),
            bounds: field.semantic_bounds,
        }];
        let item = |label: String, focused: bool| Item {
            role: Role::Button,
            label,
            checked: None,
            focused,
            bounds: None,
        };
        if let Some(chip) = q.chip {
            items.insert(1, item(chip.chip_label(), false));
        }
        let lens = self.quick_lens();
        if lens[Zone::Seen.idx()] > 0 {
            let recents = self.recents.get(self.recents_account());
            for (i, r) in recents.seen().iter().enumerate() {
                let (name, _) = seen_display(r, &self.sidebar);
                items.push(item(
                    format!("{}, {}", name, r.id),
                    q.zone == Zone::Seen && q.active == i,
                ));
            }
        }
        if q.term.is_empty() && q.chip.is_none() && lens[Zone::Rows.idx()] > 0 {
            items.push(item("Svuota".into(), false));
        }
        // A directory chip hangs the selector and its rows on the field (`Join`).
        let at = items.len();
        let join = q.servers.as_ref().filter(|_| q.directory()).map(|p| {
            use matrix_desktop::accessibility::directory_servers::ServerMenu;
            matrix_desktop::accessibility::new_room::Join::new(
                at,
                { let mut m = ServerMenu::of(p);
                  if let Some(f) = &q.add_field { m.add_enabled = p.can_add(&f.read(cx).input.display_text()); }
                  m.add_focused = q.add_focus.is_focused(window); m },
                (q.zone == Zone::Server || p.is_open()) && !q.add_field.as_ref().is_some_and(|f| f.read(cx).focus.is_focused(window)) && !q.add_focus.is_focused(window),
                &q.rooms.rows(),
                q.rooms.active(),
                q.rooms.status(),
            )
        });
        let hits_of = |zone| match zone {
            Zone::Rooms if q.directory() => Vec::new(),
            Zone::Rooms => self.quick_room_hits(),
            Zone::Users => self.quick_users(),
            _ => self.quick_rows(),
        };
        for zone in [Zone::Rows, Zone::Users, Zone::Rooms] {
            let hits = hits_of(zone);
            for (i, hit) in hits.iter().enumerate() {
                items.push(item(
                    format!("{}, {}", hit.name, hit.secondary),
                    q.zone == zone && q.active == i,
                ));
            }
        }
        // The state of a join and its result, as a label next to the rows (the same words as the row).
        let rooms = if q.directory() { qs::room_hits(q.room_entries(), usize::MAX) } else { self.quick_room_hits() };
        for hit in &rooms {
            if let Some(text) = self.joins.state(&hit.id).and_then(|s| join_line(Some(s))).map(|(t, _)| t) {
                let label = format!("{}, {}", hit.name, text.to_lowercase().replace('…', ""));
                items.push(Item { role: Role::Label, label, checked: None, focused: false, bounds: None });
            }
        }
        for (i, c) in self.quick_categories().iter().enumerate() {
            items.push(item(c.label().into(), q.zone == Zone::Search && q.active == i));
        }
        if let Some(add) = &q.add_field {
            let f = add.read(cx);
            items.push(Item {
                role: Role::TextInput,
                label: f.placeholder.to_owned(),
                checked: None,
                focused: f.focus.is_focused(window),
                bounds: f.semantic_bounds,
            });
        }
        if let Some(e) = q.error {
            items.push(Item { role: Role::Label, label: e.to_owned(), checked: None, focused: false, bounds: None });
        }
        Some(Snapshot { items, join })
    }

    /// Screen-reader actions on the directory selector and rows of the overlay.
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub(crate) fn apply_semantic_quick(
        &mut self,
        action: matrix_desktop::accessibility::new_room::NewRoomAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use matrix_desktop::accessibility::{directory_servers::ServerAction, new_room::NewRoomAction};
        let Some(q) = self.quick.as_mut() else {
            return;
        };
        match action {
            NewRoomAction::Server(ServerAction::Toggle) => {
                q.zone = Zone::Server;
                if let Some(p) = &mut q.servers {
                    if p.is_open() { p.close() } else { p.open() }
                }
            }
            NewRoomAction::Server(ServerAction::Add) => self.confirm_server(window, cx),
            NewRoomAction::Server(ServerAction::Choose(i)) => {
                if let Some(choice) = q.servers.as_mut().map(|p| p.choose(i)) {
                    self.server_choice(choice, window, cx);
                }
            }
            NewRoomAction::Join(i) => self.quick_pick(Zone::Rooms, i, window, cx),
        }
        cx.notify();
    }

    pub(crate) fn render_quick_search(
        &self,
        t: matrix_desktop::DesignTokens,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let q = self.quick.as_ref()?;
        q.field.update(cx, |f, _| f.theme = t);
        if let Some(field) = &q.add_field {
            field.update(cx, |f, _| f.theme = t);
        }
        let typing = !q.term.is_empty();
        let recents = self.recents.get(self.recents_account());
        let lens = self.quick_lens();
        let (zone, active) = (q.zone, q.active);
        let rows = lens[Zone::Rows.idx()];
        let browsing = !typing && q.chip.is_none();
        let header = |label: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(rgb(t.secondary))
                .child(label)
        };
        let seen = (lens[Zone::Seen.idx()] > 0).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(header("Visti di recente"))
                .child(
                    div()
                        .id("quick-seen")
                        .flex()
                        .overflow_x_scroll()
                        .track_scroll(&q.seen_scroll)
                        .children(recents.seen().iter().enumerate().map(|(i, r)| {
                            let (name, avatar) = seen_display(r, &self.sidebar);
                            div()
                                .id(("quick-seen-item", i))
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .w(px(64.))
                                .flex_shrink_0()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .when(zone == Zone::Seen && active == i, |d| d.bg(rgb(t.input)))
                                .child(self.quick_avatar(t, avatar, name, 40.))
                                .child(
                                    div()
                                        .w_full()
                                        .px_1()
                                        .truncate()
                                        .text_center()
                                        .text_size(px(11.))
                                        .text_color(rgb(t.text))
                                        .child(seen_label(name)),
                                )
                                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                    this.quick_highlight(Zone::Seen, i, cx)
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.quick_pick(Zone::Seen, i, window, cx)
                                }))
                        })),
                )
        });
        let searched_header = (browsing && rows > 0).then(|| {
            div()
                .flex()
                .justify_between()
                .child(header("Ricerche recenti"))
                .child(
                    div()
                        .id("quick-clear")
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(t.accent))
                        .child("Svuota")
                        .on_click(cx.listener(|this, _, window, cx| this.quick_clear(window, cx))),
                )
        });
        let room_hits = self.quick_room_hits();
        let local_cap = if lens[Zone::Users.idx()] > 0 || q.users.status().is_some() || !room_hits.is_empty() {
            SHARED_LIST_MAX_H
        } else {
            LIST_MAX_H
        };
        let list = (rows > 0).then(|| self.quick_list(t, Zone::Rows, rows, local_cap, cx));
        let users = self.quick_users();
        let status = q.users.status().filter(|_| typing);
        let status_line = |text: &'static str| {
            div().text_size(px(13.)).text_color(rgb(no_results_color(&t))).child(text)
        };
        let users_section = match q.chip {
            // The directory has its own section below.
            Some(c) if c.kind().is_some() => None,
            // The whole result: a scrolling list, or one status line.
            Some(_) => (!users.is_empty())
                .then(|| self.quick_list(t, Zone::Users, users.len(), LIST_MAX_H, cx))
                .or_else(|| status.map(|s| status_line(s.text()).into_any_element())),
            None => (!users.is_empty() || matches!(status, Some(Status::Searching | Status::Error(_))))
                .then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .child(header("Utenti"))
                        .children(users.iter().enumerate().map(|(i, hit)| {
                            self.quick_row(t, Zone::Users, i, hit, zone == Zone::Users && active == i, cx)
                        }))
                        .children(
                            status
                                .filter(|_| users.is_empty())
                                .map(|s| status_line(s.text())),
                        )
                        .into_any_element()
                }),
        };
        let rooms_status = q.rooms.status();
        let rooms_section = if q.directory() {
            // «Suggerimenti» is the first page for an empty field; typed text shows only the hits.
            let more = self.quick_more();
            Some(
                div()
                    .flex()
                    .flex_col()
                    .children((!typing && !room_hits.is_empty()).then(|| header("Suggerimenti")))
                    .children((!room_hits.is_empty()).then(|| self.quick_list(t, Zone::Rooms, room_hits.len(), LIST_MAX_H, cx)))
                    .children(more.map(|m| {
                        div()
                            .id("quick-more")
                            .flex()
                            .items_center()
                            .px_2()
                            .h(px(ROW_H))
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(rgb(t.secondary))
                            .when(zone == Zone::Rooms && active == room_hits.len(), |d| d.bg(rgb(t.input)))
                            .child(more_text(m))
                            .on_click(cx.listener({
                                let n = room_hits.len();
                                move |this, _, window, cx| this.quick_pick(Zone::Rooms, n, window, cx)
                            }))
                    }))
                    .children(rooms_status.filter(|_| room_hits.is_empty()).map(|s| status_line(s.text())))
                    .into_any_element(),
            )
        } else if !room_hits.is_empty() {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .child(header("Stanze pubbliche"))
                    .children(room_hits.iter().enumerate().map(|(i, hit)| {
                        self.quick_row(t, Zone::Rooms, i, hit, zone == Zone::Rooms && active == i, cx)
                    }))
                    .into_any_element(),
            )
        } else {
            None
        };
        let categories = self.quick_categories();
        let search_section = (!categories.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .child(header("Cerca"))
                .children(categories.iter().enumerate().map(|(i, c)| {
                    let c = *c;
                    div()
                        .id(("quick-cat", i))
                        .flex()
                        .items_center()
                        .px_2()
                        .h(px(ROW_H))
                        .rounded_md()
                        .cursor_pointer()
                        .text_color(rgb(t.text))
                        .when(zone == Zone::Search && active == i, |d| d.bg(rgb(t.input)))
                        .child(c.label())
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            this.quick_highlight(Zone::Search, i, cx)
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.quick_set_chip(Some(c), window, cx)
                        }))
                }))
        });
        let chip = q.chip.map(|c| {
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap_1()
                .px_2()
                .h(px(28.))
                .rounded_full()
                .bg(rgb(t.input))
                .text_size(px(13.))
                .text_color(rgb(t.text))
                .child(c.label())
                .child(
                    div()
                        .id("quick-chip-remove")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(crate::composer_icons::icon(crate::composer_icons::Icon::Close, t.text))
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.quick_set_chip(None, window, cx)
                        })),
                )
        });
        let nothing = typing
            && q.chip.is_none()
            && rows == 0
            && users.is_empty()
            && room_hits.is_empty()
            && !matches!(status, Some(Status::Searching | Status::Error(_)))
            && !matches!(rooms_status, Some(matrix_desktop::search_list::Status::Searching));
        Some(
            div()
                .id("quick-search-layer")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(matrix_desktop::SCRIM))
                .on_click(cx.listener(|this, _, window, cx| this.close_quick_search(window, cx)))
                .child(
                    div()
                        .id("quick-search")
                        .w(px(480.))
                        .max_w_full()
                        .p_3()
                        .gap_2()
                        .flex()
                        .flex_col()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.border))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .children(chip)
                                .child(div().flex_1().min_w_0().child(q.field.clone()))
                                .children(self.quick_server_picker(t, cx)),
                        )
                        .children(q.add_field.clone().zip(q.servers.as_ref()).map(|(f, p)| self.add_server_block(f, p, &q.add_focus, t, cx)))
                        .children(q.error.map(|e| div().text_size(px(13.)).text_color(rgb(t.error)).child(e)))
                        .children(seen)
                        .children(searched_header)
                        .children(list)
                        .children(users_section)
                        .children(rooms_section)
                        .children(search_section)
                        .when(nothing, |d| {
                            d.child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(no_results_color(&t)))
                                    .child("Nessun risultato"),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    /// The «Unisciti» server selector, right of the field while a directory chip is set. It never
    /// takes the keyboard focus: the field keeps typing, the highlight zone carries the keys.
    fn quick_server_picker(&self, t: matrix_desktop::DesignTokens, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let q = self.quick.as_ref().filter(|q| q.directory())?;
        let picker = q.servers.as_ref()?;
        let this = cx.entity();
        let (toggle, choose, close) = (this.clone(), this.clone(), this);
        Some(render_server_picker(
            picker,
            &q.server_focus,
            q.zone == Zone::Server,
            t,
            move |window, cx| {
                toggle.update(cx, |this, cx| {
                    if let Some(q) = &mut this.quick {
                        q.zone = Zone::Server;
                        if let Some(p) = &mut q.servers {
                            if p.is_open() { p.close() } else { p.open() }
                        }
                        q.focus_field(window, cx);
                    }
                    cx.notify();
                })
            },
            move |i, window, cx| {
                choose.update(cx, |this, cx| {
                    let choice = this.quick.as_mut().and_then(|q| q.servers.as_mut()).map(|p| p.choose(i));
                    if let Some(choice) = choice {
                        this.server_choice(choice, window, cx);
                    }
                })
            },
            move |_, cx| {
                close.update(cx, |this, cx| {
                    if let Some(p) = this.quick.as_mut().and_then(|q| q.servers.as_mut()) {
                        p.close();
                    }
                    cx.notify();
                })
            },
        ))
    }

    /// A scrolling list of `zone`'s rows (`Rows` or `Users`).
    fn quick_list(
        &self,
        t: matrix_desktop::DesignTokens,
        zone: Zone,
        count: usize,
        max_h: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let q = self.quick.as_ref().expect("overlay open");
        div()
            .relative()
            .h(px((count as f32 * ROW_H).min(max_h)))
            .child(
                uniform_list(
                    "quick-rows",
                    count,
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        let hits = match zone {
                            Zone::Users => this.quick_users(),
                            Zone::Rooms => this.quick_room_hits(),
                            _ => this.quick_rows(),
                        };
                        let (z, active) = this.quick.as_ref().map_or((zone, 0), |q| (q.zone, q.active));
                        range
                            .filter_map(|i| hits.get(i).map(|hit| (i, hit)))
                            .map(|(i, hit)| this.quick_row(t, zone, i, hit, z == zone && active == i, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(q.scroll.clone())
                .h_full(),
            )
            .child(q.scrollbar.render(
                "quick-scrollbar",
                scrollbar_view::ScrollTarget::Rooms(q.scroll.clone()),
                t,
            ))
            .into_any_element()
    }

    /// One result row: avatar, name, secondary label (Matrix ID for users).
    fn quick_row(
        &self,
        t: matrix_desktop::DesignTokens,
        zone: Zone,
        i: usize,
        hit: &Hit,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let id = match zone {
            Zone::Users => "quick-user",
            Zone::Rooms => "quick-room",
            _ => "quick-row",
        };
        div()
            .id((id, i))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(ROW_H))
            .rounded_md()
            .cursor_pointer()
            .when(active, |d| d.bg(rgb(t.input)))
            .child(self.quick_avatar(t, hit.avatar.as_deref(), &hit.name, 28.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_color(rgb(t.text))
                            .child(matrix_desktop::row_text::one_line(&hit.name)),
                    )
                    .children(match join_line(self.joins.state(&hit.id).filter(|_| zone == Zone::Rooms)) {
                        Some((text, kind)) => Some(join_row_line(text, kind, t)),
                        None => Some(secondary_label(&hit.secondary))
                            .filter(|s| !s.is_empty())
                            .map(|s| {
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(t.secondary))
                                    .child(s.to_owned())
                                    .into_any_element()
                            }),
                    }),
            )
            .on_mouse_move(cx.listener(move |this, _, _, cx| this.quick_highlight(zone, i, cx)))
            .on_click(cx.listener(move |this, _, window, cx| this.quick_pick(zone, i, window, cx)))
    }

    /// The mouse moves the same highlight as the keyboard.
    fn quick_highlight(&mut self, zone: Zone, i: usize, cx: &mut Context<Self>) {
        if let Some(q) = self
            .quick
            .as_mut()
            .filter(|q| (q.zone, q.active) != (zone, i))
        {
            (q.zone, q.active) = (zone, i);
            if zone == Zone::Rooms {
                q.rooms.set_active(i);
            }
            cx.notify();
        }
    }

    /// «Svuota»: only the search list; «Visti di recente» is history and stays.
    fn quick_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.recents_account();
        let mut recents = self.recents.get(account);
        recents.clear_searched();
        let has_seen = !recents.seen().is_empty();
        self.recents.set(account, recents);
        self.save_recents();
        if let Some(q) = self.quick.as_mut() {
            (q.zone, q.active) = (if has_seen { Zone::Seen } else { Zone::Rows }, 0);
            // The click took the focus off the field: give it back so typing continues.
            q.field.read(cx).focus.focus(window);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Zone::*;

    #[test]
    fn highlight_moves_within_and_between_the_zones() {
        // Lens order = `Zone::ALL`: the selector, «Visti», locals, users, directory, «Cerca».
        assert_eq!(Zone::ALL, [Server, Seen, Rows, Users, Rooms, Search]);
        // Typing: «Visti» is hidden (0); locals, users, rooms, «Cerca» in that order.
        let typing = [0, 0, 5, 3, 2, 1];
        assert_eq!(step(Rows, 0, "down", typing), Some((Rows, 1)));
        assert_eq!(step(Rows, 4, "down", typing), Some((Users, 0)));
        assert_eq!(step(Users, 2, "down", typing), Some((Rooms, 0)));
        assert_eq!(step(Rooms, 1, "down", typing), Some((Search, 0)));
        assert_eq!(step(Search, 0, "down", typing), None);
        assert_eq!(step(Rooms, 0, "up", typing), Some((Users, 2)));
        assert_eq!(step(Rows, 0, "up", typing), None);
        // Tab goes section to section and wraps.
        assert_eq!(step(Rows, 3, "tab", typing), Some((Users, 0)));
        assert_eq!(step(Users, 1, "tab", typing), Some((Rooms, 0)));
        assert_eq!(step(Search, 0, "tab", typing), Some((Rows, 0)));
        // Empty zones are skipped: no local hit, «Visti» row on top of the searches.
        assert_eq!(step(Rows, 0, "down", [0, 0, 1, 0, 0, 1]), Some((Search, 0)));
        let browsing = [0, 3, 5, 0, 0, 1];
        assert_eq!(step(Seen, 0, "right", browsing), Some((Seen, 1)));
        assert_eq!(step(Seen, 2, "right", browsing), None);
        assert_eq!(step(Seen, 0, "left", browsing), None);
        assert_eq!(step(Seen, 1, "down", browsing), Some((Rows, 0)));
        assert_eq!(step(Rows, 0, "up", browsing), Some((Seen, 0)));
        assert_eq!(step(Rows, 2, "up", browsing), Some((Rows, 1)));
        assert_eq!(step(Seen, 1, "tab", browsing), Some((Rows, 0)));
        assert_eq!(step(Search, 0, "tab", browsing), Some((Seen, 0)));
        assert_eq!(step(Seen, 0, "down", [0, 3, 0, 0, 0, 0]), None);
        // People chip: only the users list.
        assert_eq!(step(Users, 0, "tab", [0, 0, 0, 4, 0, 0]), None);
        assert_eq!(step(Users, 0, "down", [0, 0, 0, 4, 0, 0]), Some((Users, 1)));
        // Directory chip: the selector sits above the hits, Tab goes between them and wraps.
        let chip = [1, 0, 0, 0, 3, 0];
        assert_eq!(step(Rooms, 0, "up", chip), Some((Server, 0)));
        assert_eq!(step(Server, 0, "down", chip), Some((Rooms, 0)));
        assert_eq!(step(Rooms, 1, "tab", chip), Some((Server, 0)));
        assert_eq!(step(Server, 0, "tab", chip), Some((Rooms, 0)));
    }

    #[test]
    fn svuota_from_the_keyboard() {
        // Backspace/Delete on the highlighted searches, empty field.
        assert!(clears(Rows, "backspace", true, false, false, 2));
        assert!(clears(Rows, "delete", true, false, false, 2));
        assert!(!clears(Seen, "backspace", true, false, false, 2));
        // Cmd/Ctrl+Backspace from any zone.
        assert!(clears(Seen, "backspace", false, true, false, 2));
        // Typing: Backspace edits the field. Nothing to clear: no-op.
        assert!(!clears(Rows, "backspace", true, false, true, 2));
        assert!(!clears(Rows, "backspace", true, false, false, 0));
    }

    #[test]
    fn seen_label_is_one_short_line_with_final_ellipsis() {
        assert_eq!(seen_label("Anna"), "Anna");
        assert_eq!(seen_label("12345678"), "12345678");
        assert_eq!(seen_label("017 Progetto lungo"), "017 Pro…");
        assert_eq!(seen_label("a\nb"), "a b");
        assert_eq!(seen_label("àèìòùàèìòù").chars().count(), 8);
    }

    #[test]
    fn seen_uses_the_current_sidebar_name_after_resolution() {
        let mut recents = matrix_desktop::quick_search::Recents::default();
        recents.opened(Recent { id: "!room:server".into(), name: "!room:server".into(), avatar: None });
        use matrix_domain::{AccountId, RoomId, RoomScope};
        use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};
        let mut sidebar = matrix_desktop::Sidebar::connected(AccountId::new(1));
        let mut room = RoomSummary {
            scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!room:server") },
            name: "!room:server".into(), unread: 0, info: RoomInfo::default(),
        };
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(room.clone())));
        assert_eq!(seen_display(&recents.seen()[0], &sidebar).0, "Chat");
        room.name = "Dora Bianchi".into();
        room.info.avatar = Some("mxc://server/avatar".into());
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(room)));
        let (name, avatar) = seen_display(&recents.seen()[0], &sidebar);
        assert_eq!(seen_label(name), "Dora Bi…");
        assert_eq!(avatar, Some("mxc://server/avatar"));
    }

    #[test]
    fn raw_room_ids_are_not_shown_but_other_secondary_text_is() {
        assert_eq!(secondary_label("!KBYhOBtiJiD1au:matrix.org"), "");
        assert_eq!(secondary_label("@bob:matrix.org"), "@bob:matrix.org");
        assert_eq!(secondary_label("WhatsApp"), "WhatsApp");
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let l = |c: u32| {
            let v = |s: u32| {
                let x = f64::from((c >> s) & 255) / 255.;
                if x <= 0.03928 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * v(16) + 0.7152 * v(8) + 0.0722 * v(0)
        };
        (l(a).max(l(b)) + 0.05) / (l(a).min(l(b)) + 0.05)
    }

    #[test]
    fn no_results_text_is_readable_in_every_theme() {
        for t in [
            matrix_desktop::LIGHT,
            matrix_desktop::DARK,
            matrix_desktop::DAY,
            matrix_desktop::NIGHT,
        ] {
            let r = contrast(no_results_color(&t), t.surface);
            assert!(r >= 4.5, "{r:.2}");
        }
    }

    #[test]
    fn a_left_room_leaves_both_recent_lists() {
        let item = |id: &str| Recent {
            id: id.into(),
            name: id.into(),
            avatar: None,
        };
        let mut r = matrix_desktop::quick_search::Recents::default();
        r.opened(item("a"));
        r.opened(item("b"));
        r.picked(item("a"));
        r.picked(item("b"));
        r.forget("a");
        assert_eq!(r.seen().len(), 1);
        assert_eq!(r.searched().len(), 1);
        assert_eq!(r.seen()[0].id, "b");
    }
}
