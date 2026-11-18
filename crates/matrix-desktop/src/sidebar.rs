use crate::ROOMS;
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::RoomListDiff;

/// Presentation row, either explicit offline fixture or server projection.
pub struct SidebarRoom {
    pub scope: RoomScope,
    pub name: String,
    pub initials: String,
    pub preview: String,
    pub time: &'static str,
    pub color: u32,
    pub unread: u64,
    pub info: matrix_engine::RoomInfo,
}

impl SidebarRoom {
    /// Right-hand stamp of the row: from the last event, else the offline fixture's; None shows nothing.
    pub fn stamp(&self) -> Option<String> {
        crate::time_fmt::last_message_stamp(self.info.last_event_ts)
            .or_else(|| (!self.time.is_empty()).then(|| self.time.to_owned()))
    }

    /// Screen-reader form of `stamp()`.
    pub fn stamp_spoken(&self) -> Option<String> {
        match self.info.last_event_ts {
            Some(ms) => Some(crate::time_fmt::last_message_spoken_ms(ms)),
            None => self.stamp().map(|s| crate::time_fmt::last_message_spoken(&s)),
        }
    }

    /// Pinned (`m.favourite`) row: it carries the pin where the unread badge goes.
    pub fn pinned(&self) -> bool {
        !self.info.invited && self.info.tags.iter().any(|t| t == FAVOURITE)
    }

    /// Low-priority row (never also pinned: pinned wins when another client set both).
    pub fn low_priority(&self) -> bool {
        !self.info.invited && !self.pinned() && self.info.tags.iter().any(|t| t == LOW_PRIORITY)
    }
}

/// Session-local navigation projection. `Some` is a joined Matrix Space, not a local folder.
pub struct NavigationEntry {
    pub scope: Option<RoomScope>,
    pub name: String,
    pub unread: u64,
    pub mentions: u64,
    pub selected: bool,
    /// mxc URI of the space avatar; None for «Tutte» and for spaces without one.
    pub avatar: Option<String>,
}

const FAVOURITE: &str = "m.favourite";
const LOW_PRIORITY: &str = "m.lowpriority";

/// Label of a tag: the part after `u.`, or the Italian name of the two standard ones.
pub fn group_label(tag: &str) -> String {
    match tag {
        "m.favourite" => "Preferiti".into(),
        "m.lowpriority" => "Bassa priorità".into(),
        _ => tag.strip_prefix("u.").unwrap_or(tag).to_owned(),
    }
}

/// Where the answer to an invitation stands; drives the buttons of its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteState {
    Idle,
    /// «Rifiuta» was clicked once: the next one leaves the room.
    ConfirmReject,
    Busy,
    /// The last answer failed; «Accetta» / «Rifiuta» try again.
    Failed,
}

pub enum Navigation {
    Previous,
    Next,
    First,
    Last,
}

pub struct Sidebar {
    navigation_revision: u64,
    navigation_context: Option<(AccountId, bool)>,
    navigation_visibility: (bool, bool, bool),
    account_id: AccountId,
    rooms: Vec<SidebarRoom>,
    /// Position of each room in the `RoomListService` order (most recent first).
    order: std::collections::HashMap<RoomScope, usize>,
    visible: Vec<usize>,
    query: String,
    space: Option<RoomScope>,
    /// «Non lette» view: only joined rooms with unread messages or a manual mark. Exclusive with `space`.
    unread_view: bool,
    selected: Option<usize>,
    keyboard_focus: Option<RoomScope>,
    /// Invitations whose answer is on its way, and those whose last answer failed.
    answering: std::collections::HashSet<RoomScope>,
    failed: std::collections::HashSet<RoomScope>,
    reject_armed: Option<RoomScope>,
    /// The room whose last «Esci» failed, with the one-line reason shown in its row.
    leave_error: Option<(RoomScope, &'static str)>,
    /// The tag toggle in flight: the room and ALL its tags and pin order before the optimistic flip.
    tag: Option<(RoomScope, Vec<String>, Option<u64>)>,
    /// The space whose «Esci dallo spazio» request is in flight (disables the control, nothing else).
    space_leaving: Option<RoomScope>,
    /// One-line reason of the last failed toggle, until the next one.
    tag_error: Option<&'static str>,
    /// A notification-mode write in flight: the room and the mode it had before.
    mode: Option<(RoomScope, matrix_domain::notifications::RoomNotifyMode)>,
    mode_error: Option<&'static str>,
}

impl Sidebar {
    pub fn connected(account_id: AccountId) -> Self {
        Self {
            navigation_revision: 0,
            navigation_context: None,
            navigation_visibility: (true, true, false),
            account_id,
            rooms: Vec::new(),
            order: std::collections::HashMap::new(),
            visible: Vec::new(),
            query: String::new(),
            selected: None,
            keyboard_focus: None,
            space: None,
            unread_view: false,
            answering: Default::default(),
            failed: Default::default(),
            reject_armed: None,
            leave_error: None,
            space_leaving: None,
            tag: None,
            tag_error: None,
            mode: None,
            mode_error: None,
        }
    }

    pub fn apply_diff(&mut self, diff: RoomListDiff) -> bool {
        let summary = match diff {
            RoomListDiff::Upsert(summary) => summary,
            RoomListDiff::Remove(scope) => {
                if scope.account_id != self.account_id {
                    return false;
                }
                let selected = self.selected.map(|i| self.rooms[i].scope.clone());
                self.answering.remove(&scope);
                self.failed.remove(&scope);
                let old_len = self.rooms.len();
                self.rooms.retain(|room| room.scope != scope);
                if old_len != self.rooms.len() {
                    self.retire_navigation();
                }
                self.selected = selected
                    .and_then(|scope| self.rooms.iter().position(|room| room.scope == scope));
                self.set_query(&self.query.clone());
                // Fallback: first visible row, never a space (spaces are not in `visible`).
                self.selected = self.selected.or_else(|| self.visible.first().copied());
                return old_len != self.rooms.len();
            }
        };
        if summary.scope.account_id != self.account_id {
            return false;
        }
        if !summary.info.invited {
            self.answering.remove(&summary.scope);
            self.failed.remove(&summary.scope);
        }
        let row = SidebarRoom {
            scope: summary.scope,
            initials: summary.name.chars().take(2).collect(),
            name: summary.name,
            preview: summary.info.preview.clone().unwrap_or_default(),
            time: "",
            color: crate::AVATAR_FALLBACK,
            unread: summary.unread,
            info: summary.info,
        };
        let visible = self.visible.clone();
        if let Some(index) = self.rooms.iter().position(|room| room.scope == row.scope) {
            if self.rooms[index].info.is_space != row.info.is_space {
                self.retire_navigation();
            }
            self.rooms[index] = row;
        } else {
            if row.info.is_space {
                self.retire_navigation();
            }
            self.rooms.push(row);
        }
        self.set_query(&self.query.clone());
        if self.selected.is_none() {
            self.selected = self.visible.first().copied();
        }
        if self.visible != visible {
            self.retire_navigation();
        }
        true
    }

    /// New SDK order (most recent first); rooms it does not know yet go last until the next one.
    pub fn set_order(&mut self, order: Vec<RoomScope>) {
        self.order = order.into_iter().zip(0..).collect();
        let visible = self.visible.clone();
        self.set_query(&self.query.clone());
        if self.visible != visible {
            self.retire_navigation();
        }
    }

    pub fn demo(account_id: AccountId) -> Self {
        let rooms = (0..300)
            .map(|i| {
                let seed = &ROOMS[i % ROOMS.len()];
                SidebarRoom {
                    scope: RoomScope {
                        account_id,
                        room_id: RoomId::new(format!("!offline-demo-{}:fixture.invalid", i + 1)),
                    },
                    name: if i < ROOMS.len() {
                        seed.name.to_owned()
                    } else {
                        format!("Stanza demo {:03}", i + 1)
                    },
                    initials: if i < ROOMS.len() {
                        seed.initials.to_owned()
                    } else {
                        format!("{:02}", (i + 1) % 100)
                    },
                    preview: seed.preview.to_owned(),
                    time: seed.time,
                    color: seed.color,
                    unread: 0,
                    // The offline demo has no server: its rooms stay editable so the sheet and the dialog can be shown.
                    info: matrix_engine::RoomInfo { edit_rights: matrix_engine::EditRights { name: true, topic: true, avatar: true }, ..Default::default() },
                }
            })
            .collect();
        Self {
            navigation_revision: 0,
            navigation_context: None,
            navigation_visibility: (true, true, false),
            account_id,
            rooms,
            order: Default::default(),
            visible: (0..300).collect(),
            query: String::new(),
            selected: Some(0),
            keyboard_focus: None,
            space: None,
            unread_view: false,
            answering: Default::default(),
            failed: Default::default(),
            reject_armed: None,
            leave_error: None,
            space_leaving: None,
            tag: None,
            tag_error: None,
            mode: None,
            mode_error: None,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn navigation_revision(&self) -> u64 {
        self.navigation_revision
    }

    /// Capture the painted source before queuing a native navigation callback.
    pub fn navigation_callback(&self) -> impl Fn(&Self) -> bool + use<> {
        let account = self.account_id;
        let revision = self.navigation_revision;
        move |current| {
            current.navigation_visibility.1
                && current.account_id == account
                && current.navigation_revision == revision
        }
    }

    pub fn set_navigation_context(&mut self, account: AccountId, connected: bool) {
        let context = Some((account, connected));
        if self.navigation_context != context {
            self.keyboard_focus = None;
            self.retire_navigation();
            self.navigation_context = context;
        }
    }

    pub fn set_navigation_visibility(&mut self, persistent: bool, visible: bool, menu_open: bool) {
        let visibility = (persistent, visible, !persistent && visible && menu_open);
        if self.navigation_visibility != visibility {
            if !visible {
                self.keyboard_focus = None;
            }
            self.retire_navigation();
            self.navigation_visibility = visibility;
        }
    }

    fn retire_navigation(&mut self) {
        self.reject_armed = None;
        self.navigation_revision = self
            .navigation_revision
            .checked_add(1)
            .expect("navigation revision exhausted");
    }

    pub fn spaces(&self) -> impl Iterator<Item = &SidebarRoom> {
        self.rooms.iter().filter(|room| room.info.is_space && !room.info.invited)
    }

    pub fn invite_count(&self) -> usize {
        self.rooms.iter().filter(|room| room.info.invited).count()
    }

    pub fn invite_state(&self, scope: &RoomScope) -> InviteState {
        if self.answering.contains(scope) {
            InviteState::Busy
        } else if self.reject_armed.as_ref() == Some(scope) {
            InviteState::ConfirmReject
        } else if self.failed.contains(scope) {
            InviteState::Failed
        } else {
            InviteState::Idle
        }
    }

    /// The user clicked «Accetta» (`accept`) or «Rifiuta» on an invitation. Returns the answer to send:
    /// accepting is one click, rejecting needs a second click on the same row. Nothing is ever sent unasked.
    pub fn answer_invite(&mut self, scope: &RoomScope, accept: bool) -> Option<bool> {
        let open = self.rooms.iter().any(|r| r.scope == *scope && r.info.invited);
        if !open || self.answering.contains(scope) {
            return None;
        }
        if !accept && self.reject_armed.as_ref() != Some(scope) {
            self.reject_armed = Some(scope.clone());
            return None;
        }
        self.reject_armed = None;
        self.failed.remove(scope);
        self.answering.insert(scope.clone());
        Some(accept)
    }

    /// Outcome of an answer. Success changes nothing here: the next sync upserts the joined room or removes the left one.
    pub fn invite_answered(&mut self, scope: &RoomScope, ok: bool) {
        self.answering.remove(scope);
        if !ok {
            self.failed.insert(scope.clone());
        }
    }

    /// Outcome of «Esci». The engine confirmed the leave with the server (or the SDK no longer has the room joined:
    /// the sync projection has dropped it), so the row goes now and the selection moves like a sync `Remove`.
    pub fn room_left(&mut self, scope: &RoomScope, result: Result<(), matrix_engine::operations::OperationError>) {
        use matrix_engine::operations::OperationError::{Forbidden, Unavailable};
        if matches!(result, Ok(()) | Err(Unavailable)) && !self.rooms.iter().any(|r| r.scope == *scope && r.info.invited) {
            self.leave_error = None;
            self.apply_diff(RoomListDiff::Remove(scope.clone()));
            return;
        }
        self.leave_error = result.err().map(|e| {
            let text = match e {
                Forbidden => "Non consentito dal server",
                Unavailable => "Stanza non trovata",
                _ => "Connessione non riuscita",
            };
            (scope.clone(), text)
        });
    }

    /// Marks the leave of `scope` as in flight; false (do not send) when one already is.
    pub fn begin_space_leave(&mut self, scope: RoomScope) -> bool {
        self.leave_error = None;
        self.space_leaving.replace(scope).is_none()
    }

    /// True only while the request runs: the control stays disabled exactly that long.
    pub fn space_leaving(&self) -> bool {
        self.space_leaving.is_some()
    }

    /// Outcome of «Esci dallo spazio», any result frees the control. Success drops the space from the rail and,
    /// when it was selected, returns the list to «Tutte» (as `room_left`); failure leaves the one-line error.
    pub fn space_left(&mut self, scope: &RoomScope, result: Result<(), matrix_engine::operations::OperationError>) {
        self.space_leaving = None;
        self.room_left(scope, result);
    }

    /// The failure of the last «Esci» on this row, until the next click or menu.
    pub fn leave_error(&self, scope: &RoomScope) -> Option<&'static str> {
        self.leave_error.as_ref().filter(|(s, _)| s == scope).map(|(_, t)| *t)
    }

    pub fn clear_leave_error(&mut self) {
        self.leave_error = None;
    }

    /// The one state every UI and the list order read: the room carries `tag`.
    pub fn has_tag(&self, scope: &RoomScope, tag: &str) -> bool {
        self.rooms.iter().any(|r| r.scope == *scope && r.info.tags.iter().any(|t| t == tag))
    }

    pub fn is_favourite(&self, scope: &RoomScope) -> bool {
        self.has_tag(scope, FAVOURITE)
    }

    /// Room-menu entry for the room's current state.
    pub fn favourite_label(&self, scope: &RoomScope) -> &'static str {
        if self.is_favourite(scope) { "Stacca" } else { "Fissa in alto" }
    }

    pub fn low_priority_label(&self, scope: &RoomScope) -> &'static str {
        if self.has_tag(scope, LOW_PRIORITY) { "Togli dalla bassa priorità" } else { "Segna come bassa priorità" }
    }

    pub fn tag_pending(&self) -> bool {
        self.tag.is_some()
    }

    pub fn tag_error(&self) -> Option<&'static str> {
        self.tag_error
    }

    fn set_tags(&mut self, scope: &RoomScope, tags: Vec<String>, order: Option<u64>) {
        if let Some(room) = self.rooms.iter_mut().find(|r| r.scope == *scope) {
            room.info.tags = tags;
            room.info.favourite_order = order;
        }
        self.set_query(&self.query.clone());
    }

    /// The single favourite / low-priority action (context menu and drawer): flips the room now so the list
    /// follows, and returns the tag edit to send. Turning one on drops the other, as the SDK does, so the write
    /// stays ONE edit. None while another toggle waits or for an unknown room.
    pub fn toggle_tag(&mut self, scope: &RoomScope, tag: &'static str) -> Option<matrix_engine::operations::TagEdit> {
        if self.tag.is_some() {
            return None;
        }
        let room = self.rooms.iter().find(|r| r.scope == *scope)?;
        let (before, before_order) = (room.info.tags.clone(), room.info.favourite_order);
        let on = !before.iter().any(|t| t == tag);
        let mut tags = before.clone();
        if on {
            let other = if tag == FAVOURITE { LOW_PRIORITY } else { FAVOURITE };
            tags.retain(|t| t != other);
            tags.push(tag.into());
            tags.sort();
        } else {
            tags.retain(|t| t != tag);
        }
        self.tag = Some((scope.clone(), before, before_order));
        self.tag_error = None;
        // Pinned now = above every pinned room (order 0.0); the server's real `order` replaces it with the next sync.
        let order = if on && tag == FAVOURITE { Some(0) } else { None };
        self.set_tags(scope, tags, order);
        Some(matrix_engine::operations::TagEdit::Assign {
            room_id: scope.room_id.as_str().to_owned(),
            tag: tag.into(),
            on,
        })
    }

    /// The engine's answer: on failure the room gets all its real tags back and one line says why.
    pub fn tag_done(&mut self, result: Result<(), matrix_engine::operations::OperationError>) {
        let Some((scope, before, before_order)) = self.tag.take() else { return };
        if let Err(e) = result {
            self.set_tags(&scope, before, before_order);
            self.tag_error = Some(match e {
                matrix_engine::operations::OperationError::Forbidden => "Non consentito dal server",
                _ => "Connessione non riuscita",
            });
        }
    }

    pub fn notify_mode(&self, scope: &RoomScope) -> matrix_domain::notifications::RoomNotifyMode {
        self.rooms.iter().find(|r| r.scope == *scope).map(|r| r.info.notification_mode).unwrap_or_default()
    }

    pub fn mode_error(&self) -> Option<&'static str> {
        self.mode_error
    }

    /// The single notification-mode action (context menu, drawer, Notifications panel): the row changes now.
    /// True when the command must be sent; false for an unknown room, an unchanged mode or a write in flight.
    pub fn set_mode(&mut self, scope: &RoomScope, mode: matrix_domain::notifications::RoomNotifyMode) -> bool {
        let Some(room) = self.rooms.iter_mut().find(|r| r.scope == *scope) else { return false };
        if self.mode.is_some() || room.info.notification_mode == mode {
            return false;
        }
        self.mode = Some((scope.clone(), room.info.notification_mode));
        self.mode_error = None;
        room.info.notification_mode = mode;
        true
    }

    /// The engine's answer: on failure the room gets its real mode back and one line says why.
    pub fn mode_done(&mut self, result: Result<(), matrix_engine::operations::OperationError>) {
        let Some((scope, before)) = self.mode.take() else { return };
        if let Err(e) = result {
            if let Some(room) = self.rooms.iter_mut().find(|r| r.scope == scope) {
                room.info.notification_mode = before;
            }
            self.mode_error = Some(match e {
                matrix_engine::operations::OperationError::Forbidden => "Non consentito dal server",
                _ => "Connessione non riuscita",
            });
        }
    }

    /// The invitation under the cursor, if the cursor is on one.
    pub fn selected_invite(&self) -> Option<&SidebarRoom> {
        self.selected.map(|i| &self.rooms[i]).filter(|r| r.info.invited)
    }

    pub fn is_joined(&self, scope: &RoomScope) -> bool {
        self.rooms.iter().any(|r| r.scope == *scope && !r.info.invited)
    }

    /// List header: name, initials and colour of the selected space; «Tutte» (no colour) otherwise.
    pub fn list_header(&self) -> (String, String, Option<u32>) {
        match self
            .space
            .as_ref()
            .and_then(|s| self.spaces().find(|r| r.scope == *s))
        {
            Some(space) => (
                space.name.clone(),
                space.initials.clone(),
                Some(space.color),
            ),
            None => ((if self.unread_view { "Non lette" } else { "Tutte" }).into(), "▦".into(), None),
        }
    }

    /// Avatar (mxc) of the space shown in the list header; none for «Tutte».
    pub fn list_header_avatar(&self) -> Option<&str> {
        let scope = self.space.as_ref()?;
        self.spaces().find(|r| r.scope == *scope)?.info.avatar.as_deref()
    }

    /// Counts each joined row once; independent of transient search/unread filters.
    pub fn navigation_entries(&self) -> Vec<NavigationEntry> {
        let total = |keep: &dyn Fn(&SidebarRoom) -> bool| {
            self.rooms
                .iter()
                // Muted rooms never count: same rule for the list, the drawer and the tray.
                .filter(|room| !room.info.muted() && keep(room))
                .fold((0u64, 0u64), |(u, m), room| {
                    (
                        u.saturating_add(room.unread),
                        m.saturating_add(room.info.mentions),
                    )
                })
        };
        let (unread, mentions) = total(&|_| true);
        let mut entries = vec![NavigationEntry {
            scope: None,
            name: "Tutte".into(),
            unread,
            mentions,
            selected: self.space.is_none() && !self.unread_view,
            avatar: None,
        }];
        entries.extend(self.spaces().map(|space| {
            let (unread, mentions) = total(&|room| {
                room.scope == space.scope || space.info.children.contains(&room.scope.room_id)
            });
            NavigationEntry {
                scope: Some(space.scope.clone()),
                name: space.name.clone(),
                unread,
                mentions,
                selected: self.space.as_ref() == Some(&space.scope),
                avatar: space.info.avatar.clone(),
            }
        }));
        entries
    }

    /// Optimistic drop once the read receipt is sent: zero this room's counters now,
    /// the next server projection of the room (apply_diff) overwrites them. Other accounts are ignored.
    pub fn mark_read_optimistic(&mut self, scope: &RoomScope) -> bool {
        let Some(room) = self.rooms.iter_mut().find(|room| room.scope == *scope) else {
            return false;
        };
        if room.unread == 0 && room.info.mentions == 0 {
            return false;
        }
        room.unread = 0;
        room.info.mentions = 0;
        self.set_query(&self.query.clone());
        self.retire_navigation();
        true
    }

    pub fn active_space(&self) -> Option<&RoomScope> {
        self.space.as_ref()
    }

    /// Only locally joined Spaces. Filtering never changes the active conversation.
    pub fn set_space(&mut self, scope: Option<RoomScope>) -> bool {
        if scope
            .as_ref()
            .is_some_and(|s| !self.spaces().any(|room| room.scope == *s))
        {
            return false;
        }
        if self.space != scope || self.unread_view {
            self.retire_navigation();
        }
        self.unread_view = false;
        self.space = scope;
        self.set_query(&self.query.clone());
        true
    }

    /// A joined, unmuted room that counts for «Non lette» (same muted rule as the other badges).
    fn in_unread_view(room: &SidebarRoom) -> bool {
        !room.info.invited && !room.info.is_space && !room.info.muted() && (room.unread > 0 || room.info.marked_unread)
    }

    /// How many rooms «Non lette» holds (rooms, not messages, like Telegram's chat counter).
    pub fn unread_rooms(&self) -> usize {
        self.rooms.iter().filter(|r| Self::in_unread_view(r)).count()
    }

    pub fn unread_view(&self) -> bool {
        self.unread_view
    }

    /// Shows «Non lette» (leaving any Space) or, with false, «Tutte le chat». True when something changed.
    pub fn set_unread_view(&mut self, on: bool) -> bool {
        if self.unread_view == on {
            return false;
        }
        self.retire_navigation();
        self.unread_view = on;
        self.space = None;
        self.set_query(&self.query.clone());
        true
    }

    /// Search room names locally; hidden selection is retained until explicit selection.
    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            self.retire_navigation();
        }
        self.query = query.to_owned();
        if self
            .space
            .as_ref()
            .is_some_and(|scope| !self.spaces().any(|r| r.scope == *scope))
        {
            self.space = None;
        }
        let needle = query.trim().to_lowercase();
        let space = self
            .space
            .as_ref()
            .and_then(|scope| self.rooms.iter().find(|r| r.scope == *scope));
        self.visible = self
            .rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| {
                // Joined spaces live in the rail only; an invitation to one stays an invite row.
                (room.info.invited || !room.info.is_space)
                    && room.name.to_lowercase().contains(&needle)
                    && (room.info.invited || space.is_none_or(|s| s.info.children.contains(&room.scope.room_id)))
                    && (!self.unread_view || Self::in_unread_view(room))
            })
            .map(|(i, _)| i)
            .collect();
        // Search keeps match order; the plain list follows the SDK order, rooms not yet ordered last by name.
        if needle.is_empty() {
            let (rooms, order) = (&self.rooms, &self.order);
            self.visible.sort_by_cached_key(|&i| {
                (
                    order.get(&rooms[i].scope).copied().unwrap_or(usize::MAX),
                    rooms[i].name.to_lowercase(),
                )
            });
        }
        // Invitations wait for an answer: first. Then the pinned (lowest `order` first, those without one last in
        // activity order), the rest, and low priority at the bottom: whatever the search. Stable: ties keep the order above.
        let rooms = &self.rooms;
        self.visible.sort_by_key(|&i| {
            let r = &rooms[i];
            let has = |t: &str| r.info.tags.iter().any(|x| x == t);
            let (class, pin) = if has(FAVOURITE) { (0, r.info.favourite_order.unwrap_or(u64::MAX)) } else if has(LOW_PRIORITY) { (2, 0) } else { (1, 0) };
            (!r.info.invited, class, pin)
        });
        if self.keyboard_focus.as_ref().is_some_and(|scope| self.position_of(scope).is_none()) {
            self.keyboard_focus = None;
        }
    }

    pub fn clear_focus(&mut self) {
        self.keyboard_focus = None;
    }

    pub fn focus_room(&mut self, scope: &RoomScope) -> bool {
        if self.position_of(scope).is_none() { return false; }
        self.keyboard_focus = Some(scope.clone());
        true
    }

    pub fn focused_room(&self) -> Option<&SidebarRoom> {
        let position = self.position_of(self.keyboard_focus.as_ref()?)?;
        Some(&self.rooms[self.visible[position]])
    }

    pub fn navigate_focus(&mut self, direction: Navigation) -> Option<usize> {
        let last = self.visible.len().checked_sub(1)?;
        let current = self.keyboard_focus.as_ref().and_then(|s| self.position_of(s));
        let position = match direction {
            Navigation::First => 0,
            Navigation::Last => last,
            Navigation::Next => current.map_or(0, |i| (i + 1).min(last)),
            Navigation::Previous => current.map_or(last, |i| i.saturating_sub(1)),
        };
        self.keyboard_focus = Some(self.rooms[self.visible[position]].scope.clone());
        Some(position)
    }

    /// Discard automatic selection without moving keyboard focus.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Reject foreign account/room identities and rows outside the current filter.
    pub fn select(&mut self, scope: &RoomScope) -> bool {
        self.leave_error = None;
        if let Some(&i) = self
            .visible
            .iter()
            .find(|&&i| self.rooms[i].scope == *scope)
        {
            if self.selected != Some(i) {
                self.retire_navigation();
            }
            self.selected = Some(i);
            self.keyboard_focus = Some(scope.clone());
            true
        } else {
            false
        }
    }

    /// Opens a joined room the active view (Unread, Space, search) hides: the restored room stays open.
    pub fn select_outside_view(&mut self, scope: &RoomScope) -> bool {
        let Some(i) = self.rooms.iter().position(|r| r.scope == *scope && !r.info.invited) else {
            return false;
        };
        self.retire_navigation();
        self.selected = Some(i);
        true
    }

    /// Returns the filtered position for the view to reveal, not a global room index.
    pub fn navigate(&mut self, direction: Navigation) -> Option<usize> {
        let last = self.visible.len().checked_sub(1)?;
        let current = self.visible.iter().position(|&i| Some(i) == self.selected);
        let position = match direction {
            Navigation::First => 0,
            Navigation::Last => last,
            Navigation::Next => current.map_or(0, |i| (i + 1).min(last)),
            Navigation::Previous => current.map_or(last, |i| i.saturating_sub(1)),
        };
        if self.selected != Some(self.visible[position]) {
            self.retire_navigation();
        }
        self.selected = Some(self.visible[position]);
        self.keyboard_focus = Some(self.rooms[self.visible[position]].scope.clone());
        Some(position)
    }

    /// Position of a room in the visible order, if it is shown.
    pub fn position_of(&self, scope: &RoomScope) -> Option<usize> {
        self.visible
            .iter()
            .position(|&i| self.rooms[i].scope == *scope)
    }

    pub fn rooms(&self) -> &[SidebarRoom] {
        &self.rooms
    }
    pub fn visible_indices(&self) -> &[usize] {
        &self.visible
    }
    /// The open conversation; an invitation under the cursor is not one (see `selected_invite`).
    pub fn selected(&self) -> Option<&SidebarRoom> {
        self.selected.map(|i| &self.rooms[i]).filter(|r| !r.info.invited)
    }
}

/// Scroll distance that keeps the row at the top of the viewport in place when it moves
/// from `old` to `new` in the list; at the very top the list follows the new order.
pub fn reading_offset(offset: f32, row_height: f32, old: Option<usize>, new: Option<usize>) -> f32 {
    match (old, new) {
        (Some(old), Some(new)) if offset > 0. && row_height > 0. => {
            (offset + (new as f32 - old as f32) * row_height).max(0.)
        }
        _ => offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_engine::{RoomInfo, RoomSummary};

    fn names(sidebar: &Sidebar) -> Vec<&str> {
        sidebar
            .visible_indices()
            .iter()
            .map(|&i| sidebar.rooms()[i].name.as_str())
            .collect()
    }

    thread_local! {
        static ACTIVITY: std::cell::RefCell<Vec<(String, Option<u64>)>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Upserts the rooms and sets the order the SDK sorter would give: newest first, then name,
    /// rooms without activity last (this stands in for `RoomListEvent::Order`).
    fn fill(sidebar: &mut Sidebar, account: &AccountId, rooms: &[(&str, Option<u64>)]) {
        for (name, last_activity) in rooms {
            ACTIVITY.with(|a| {
                let mut a = a.borrow_mut();
                a.retain(|(n, _)| n != name);
                a.push(((*name).into(), *last_activity));
            });
            sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
                scope: RoomScope {
                    account_id: *account,
                    room_id: RoomId::new(format!("!{name}:t.invalid")),
                },
                name: (*name).into(),
                unread: 0,
                info: RoomInfo {
                    preview: Some("x".into()),
                    ..Default::default()
                },
            })));
        }
        let mut all = ACTIVITY.with(|a| a.borrow().clone());
        all.sort_by_key(|(n, at)| (std::cmp::Reverse(*at), n.to_lowercase()));
        sidebar.set_order(
            all.into_iter()
                .map(|(n, _)| RoomScope {
                    account_id: *account,
                    room_id: RoomId::new(format!("!{n}:t.invalid")),
                })
                .collect(),
        );
    }

    #[test]
    fn muted_rooms_do_not_count_in_the_account_unread_total() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        for (id, unread, muted) in [("a", 2, false), ("b", 5, true)] {
            sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
                scope: RoomScope { account_id: account, room_id: RoomId::new(format!("!{id}:t.invalid")) },
                name: id.into(),
                unread,
                info: RoomInfo { notification_mode: if muted { matrix_domain::notifications::RoomNotifyMode::Mute } else { Default::default() }, mentions: unread, ..Default::default() },
            })));
        }
        let all = &sidebar.navigation_entries()[0];
        assert_eq!((all.unread, all.mentions), (2, 2));
    }

    #[test]
    fn rooms_are_ordered_by_recent_activity_then_name_with_idle_rooms_last() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(
            &mut sidebar,
            &account,
            &[
                ("zeta", None),
                ("beta", Some(10)),
                ("alfa", None),
                ("old", Some(5)),
                ("Gamma", Some(10)),
                ("new", Some(99)),
            ],
        );
        assert_eq!(
            names(&sidebar),
            ["new", "beta", "Gamma", "old", "alfa", "zeta"]
        );
        // a newer message moves the room to the top
        fill(&mut sidebar, &account, &[("old", Some(100))]);
        assert_eq!(names(&sidebar)[0], "old");
        // search keeps the unsorted match order
        sidebar.set_query("a");
        assert_eq!(names(&sidebar), ["zeta", "beta", "alfa", "Gamma"]);
    }

    #[test]
    fn row_stamp_is_present_with_a_timestamp_and_absent_without() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(&mut sidebar, &account, &[("no-ts", None)]);
        assert_eq!(sidebar.rooms()[0].stamp(), None);
        let now = chrono::Utc::now().timestamp_millis() as u64;
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: scope(&account, "ts"),
            name: "ts".into(),
            unread: 0,
            info: matrix_engine::RoomInfo { last_event_ts: Some(now), ..Default::default() },
        })));
        let room = sidebar.rooms().iter().find(|r| r.name == "ts").unwrap();
        assert_eq!(room.stamp().map(|s| s.len()), Some(5)); // «HH:MM»
    }

    fn scope(account: &AccountId, name: &str) -> RoomScope {
        RoomScope {
            account_id: *account,
            room_id: RoomId::new(format!("!{name}:t.invalid")),
        }
    }

    #[test]
    fn a_new_message_keeps_the_open_room_selected() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(
            &mut sidebar,
            &account,
            &[("a", Some(3)), ("b", Some(2)), ("c", Some(1))],
        );
        let open = scope(&account, "c");
        assert!(sidebar.select(&open));
        // another room gets the newest message, the open one is pushed down
        fill(&mut sidebar, &account, &[("b", Some(50))]);
        assert_eq!(names(&sidebar), ["b", "a", "c"]);
        assert_eq!(
            sidebar.selected().map(|r| r.scope.clone()),
            Some(open.clone())
        );
        // the open room itself moving to the top stays selected too
        fill(&mut sidebar, &account, &[("c", Some(60))]);
        assert_eq!(names(&sidebar)[0], "c");
        assert_eq!(sidebar.selected().map(|r| r.scope.clone()), Some(open));
    }

    #[test]
    fn a_new_message_reorders_inside_a_selected_space() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(
            &mut sidebar,
            &account,
            &[("x", Some(1)), ("y", Some(2)), ("z", Some(3))],
        );
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: scope(&account, "space"),
            name: "space".into(),
            unread: 0,
            info: RoomInfo {
                is_space: true,
                children: vec![RoomId::new("!x:t.invalid"), RoomId::new("!y:t.invalid")],
                ..Default::default()
            },
        })));
        assert!(sidebar.set_space(Some(scope(&account, "space"))));
        assert!(sidebar.select(&scope(&account, "x")));
        fill(&mut sidebar, &account, &[("x", Some(9))]);
        assert_eq!(
            names(&sidebar),
            ["x", "y", "space"][..names(&sidebar).len()]
        );
        assert_eq!(sidebar.selected().map(|r| r.name.as_str()), Some("x"));
    }

    #[test]
    fn space_leave_disables_only_while_running_and_success_returns_to_tutte() {
        use matrix_engine::operations::OperationError::Forbidden;
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: scope(&account, "bots"),
            name: "Bots".into(),
            unread: 0,
            info: RoomInfo { is_space: true, ..Default::default() },
        })));
        let bots = scope(&account, "bots");
        assert!(sidebar.set_space(Some(bots.clone())));
        assert!(sidebar.begin_space_leave(bots.clone()) && sidebar.space_leaving());
        assert!(!sidebar.begin_space_leave(bots.clone()), "no second request while one runs");
        sidebar.space_left(&bots, Err(Forbidden));
        assert!(!sidebar.space_leaving() && sidebar.leave_error(&bots) == Some("Non consentito dal server"));
        assert_eq!(sidebar.active_space(), Some(&bots));
        assert!(sidebar.begin_space_leave(bots.clone()));
        sidebar.space_left(&bots, Ok(()));
        assert!(!sidebar.space_leaving() && sidebar.leave_error(&bots).is_none());
        assert_eq!((sidebar.active_space(), sidebar.spaces().count()), (None, 0));
    }

    #[test]
    fn list_header_names_the_selected_space_with_initials_or_tutte() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        assert_eq!(sidebar.list_header(), ("Tutte".into(), "▦".into(), None));
        assert_eq!(sidebar.list_header_avatar(), None);
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: scope(&account, "bots"),
            name: "Bots".into(),
            unread: 0,
            info: RoomInfo {
                is_space: true,
                avatar: Some("mxc://s/bots".into()),
                ..Default::default()
            },
        })));
        assert!(sidebar.set_space(Some(scope(&account, "bots"))));
        let (name, initials, color) = sidebar.list_header();
        assert_eq!((name.as_str(), initials.as_str()), ("Bots", "Bo"));
        assert!(color.is_some());
        assert_eq!(sidebar.list_header_avatar(), Some("mxc://s/bots"));
    }

    #[test]
    fn list_header_without_avatar_falls_back_to_deterministic_initials_and_colour() {
        let header = || {
            let account = AccountId::new(1);
            let mut sidebar = Sidebar::connected(account);
            sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
                scope: scope(&account, "bots"),
                name: "Bots".into(),
                unread: 0,
                info: RoomInfo { is_space: true, ..Default::default() },
            })));
            assert!(sidebar.set_space(Some(scope(&account, "bots"))));
            assert_eq!(sidebar.list_header_avatar(), None);
            sidebar.list_header()
        };
        let (name, initials, color) = header();
        assert_eq!((name.as_str(), initials.as_str()), ("Bots", "Bo"));
        assert!(color.is_some());
        assert_eq!(header(), (name, initials, color), "same input, same fallback");
    }

    #[test]
    fn position_of_gives_the_row_position_in_the_visible_order() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(&mut sidebar, &account, &[("a", Some(3)), ("b", Some(2))]);
        assert_eq!(sidebar.position_of(&scope(&account, "b")), Some(1));
        assert_eq!(sidebar.position_of(&scope(&account, "nope")), None);
    }

    #[test]
    fn reading_offset_follows_the_row_that_was_at_the_top() {
        // at the top the list shows the new room: no shift
        assert_eq!(reading_offset(0., 56., Some(0), Some(1)), 0.);
        // scrolled 3 rows and a half; the anchor row moved down one place: scroll one row more
        assert_eq!(reading_offset(196., 56., Some(3), Some(4)), 252.);
        // anchor moved up (room removed ahead of it)
        assert_eq!(reading_offset(196., 56., Some(3), Some(2)), 140.);
        // anchor vanished or no row height: untouched; never negative
        assert_eq!(reading_offset(196., 56., Some(3), None), 196.);
        assert_eq!(reading_offset(196., 0., Some(3), Some(4)), 196.);
        assert_eq!(reading_offset(10., 56., Some(1), Some(0)), 0.);
    }

    #[test]
    fn optimistic_read_lowers_room_space_and_mention_counters() {
        let other = AccountId::new(2);
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let upsert = |sidebar: &mut Sidebar, id: &str, unread, mentions, children: &[&str]| {
            sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
                scope: scope(&account, id),
                name: id.into(),
                unread,
                info: RoomInfo {
                    mentions,
                    is_space: !children.is_empty(),
                    children: children
                        .iter()
                        .map(|c| RoomId::new(format!("!{c}:t.invalid")))
                        .collect(),
                    ..Default::default()
                },
            })));
        };
        upsert(&mut sidebar, "a", 3, 2, &[]);
        upsert(&mut sidebar, "b", 4, 1, &[]);
        upsert(&mut sidebar, "s", 0, 0, &["a"]);
        assert_eq!(names(&sidebar), ["a", "b"]);
        let counts = |s: &Sidebar| {
            s.navigation_entries()
                .iter()
                .map(|e| (e.unread, e.mentions))
                .collect::<Vec<_>>()
        };
        assert_eq!(counts(&sidebar), [(7, 3), (3, 2)]);

        // a foreign account's room with the same id is not touched
        assert!(!sidebar.mark_read_optimistic(&scope(&other, "a")));
        assert_eq!(counts(&sidebar), [(7, 3), (3, 2)]);

        assert!(sidebar.mark_read_optimistic(&scope(&account, "a")));
        assert_eq!(counts(&sidebar), [(4, 1), (0, 0)]); // all, space
        assert_eq!(names(&sidebar), ["a", "b"]); // no unread filter: the row stays
        assert!(!sidebar.mark_read_optimistic(&scope(&account, "a"))); // idempotent

        // the next server projection wins
        upsert(&mut sidebar, "a", 1, 0, &[]);
        assert_eq!(counts(&sidebar), [(5, 1), (1, 0)]);
    }

    fn pinned(sidebar: &mut Sidebar, account: &AccountId, name: &str, unread: u64, tags: &[&str], order: Option<f64>) {
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: scope(account, name),
            name: name.into(),
            unread,
            info: RoomInfo {
                preview: Some("x".into()),
                tags: tags.iter().map(|t| (*t).into()).collect(),
                favourite_order: order.map(f64::to_bits),
                ..Default::default()
            },
        })));
    }

    #[test]
    fn pinned_rooms_lead_by_order_then_unordered_by_activity_and_low_priority_ends_the_list() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        // SDK order (newest first): n1, f_none1, low1, f_old, f_new, f_none2, n2, low2
        fill(&mut sidebar, &account, &[("n1", Some(90)), ("f_none1", Some(80)), ("low1", Some(70)), ("f_old", Some(60)), ("f_new", Some(50)), ("f_none2", Some(40)), ("n2", Some(30)), ("low2", Some(20))]);
        pinned(&mut sidebar, &account, "f_old", 0, &["m.favourite"], Some(0.5));
        pinned(&mut sidebar, &account, "f_new", 0, &["m.favourite"], Some(0.25));
        pinned(&mut sidebar, &account, "f_none1", 0, &["m.favourite"], None);
        pinned(&mut sidebar, &account, "f_none2", 7, &["m.favourite"], None);
        pinned(&mut sidebar, &account, "low1", 0, &["m.lowpriority"], None);
        pinned(&mut sidebar, &account, "low2", 0, &["m.lowpriority"], None);
        // Unread has no special rank: n2 stays below n1.
        assert_eq!(names(&sidebar), ["f_new", "f_old", "f_none1", "f_none2", "n1", "n2", "low1", "low2"]);
        // Search: only the matches, pinned still first, low priority last.
        sidebar.set_query("o");
        assert_eq!(names(&sidebar), ["f_old", "f_none1", "f_none2", "low1", "low2"]);
        sidebar.set_query("");
        let (pin, low) = (|n: &str| sidebar.rooms().iter().find(|r| r.name == n).unwrap().pinned(), |n: &str| sidebar.rooms().iter().find(|r| r.name == n).unwrap().low_priority());
        assert!(pin("f_new") && !pin("n1") && !pin("low1") && low("low1") && !low("f_new"));
        // Both tags set by another client: pinned wins.
        pinned(&mut sidebar, &account, "n1", 0, &["m.favourite", "m.lowpriority"], Some(0.9));
        assert_eq!(names(&sidebar)[..4], ["f_new", "f_old", "n1", "f_none1"]);
    }

    #[test]
    fn unread_view_lists_rooms_with_unread_or_manual_mark_in_list_order_and_counts_rooms() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(&mut sidebar, &account, &[("a", Some(4)), ("b", Some(3)), ("c", Some(2)), ("d", Some(1))]);
        pinned(&mut sidebar, &account, "b", 5, &[], None);
        pinned(&mut sidebar, &account, "d", 2, &["m.favourite"], Some(0.5));
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: scope(&account, "c"),
            name: "c".into(),
            unread: 0,
            info: RoomInfo { marked_unread: true, preview: Some("x".into()), ..Default::default() },
        })));
        assert_eq!(sidebar.unread_rooms(), 3);
        assert!(sidebar.set_unread_view(true) && !sidebar.set_unread_view(true));
        assert_eq!(sidebar.list_header().0, "Non lette");
        assert_eq!(names(&sidebar), ["d", "b", "c"]); // pinned first, then activity
        assert!(!sidebar.navigation_entries()[0].selected);
        sidebar.set_query("c");
        assert_eq!(names(&sidebar), ["c"]); // search inside the view
        // Reading a room drops it from the view and the counter; all read: empty, no badge.
        sidebar.set_query("");
        for n in ["b", "d"] {
            sidebar.mark_read_optimistic(&scope(&account, n));
        }
        assert_eq!(names(&sidebar), ["c"]);
        assert_eq!(sidebar.unread_rooms(), 1);
        pinned(&mut sidebar, &account, "c", 0, &[], None);
        assert_eq!((sidebar.unread_rooms(), names(&sidebar).len()), (0, 0));
        // Leaving the view: «Tutte le chat» is selected again.
        assert!(sidebar.set_unread_view(false));
        assert_eq!(names(&sidebar).len(), 4);
        assert!(sidebar.navigation_entries()[0].selected);
    }

    #[test]
    fn views_are_exclusive_a_space_shows_its_rooms_unread_shows_unread_of_every_space_and_all_shows_everything() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let upsert = |sidebar: &mut Sidebar, id: &str, unread, children: &[&str]| {
            sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
                scope: scope(&account, id),
                name: id.into(),
                unread,
                info: RoomInfo {
                    is_space: !children.is_empty(),
                    children: children.iter().map(|c| RoomId::new(format!("!{c}:t.invalid"))).collect(),
                    ..Default::default()
                },
            })));
        };
        upsert(&mut sidebar, "a", 2, &[]);
        upsert(&mut sidebar, "b", 3, &[]);
        upsert(&mut sidebar, "c", 0, &[]);
        upsert(&mut sidebar, "s", 0, &["a", "c"]);
        let s = scope(&account, "s");
        assert!(sidebar.set_space(Some(s.clone())));
        assert_eq!(names(&sidebar), ["a", "c"]);
        sidebar.set_unread_view(true);
        assert_eq!((sidebar.active_space(), names(&sidebar)), (None, vec!["a", "b"]));
        assert!(sidebar.set_space(Some(s)) && !sidebar.unread_view());
        assert_eq!(names(&sidebar), ["a", "c"]);
        sidebar.set_unread_view(true);
        assert!(sidebar.set_space(None) && !sidebar.unread_view());
        assert_eq!(names(&sidebar), ["a", "b", "c"]);
        assert!(sidebar.navigation_entries()[0].selected);
    }

    #[test]
    fn pinning_puts_the_room_above_the_pinned_and_unpinning_returns_it_to_its_place() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(&mut sidebar, &account, &[("a", Some(3)), ("b", Some(2)), ("c", Some(1))]);
        let (b, c) = (scope(&account, "b"), scope(&account, "c"));
        sidebar.toggle_tag(&c, FAVOURITE);
        sidebar.tag_done(Ok(()));
        // The server answers with the real order of c; then b is pinned: optimistic order beats it.
        pinned(&mut sidebar, &account, "c", 0, &["m.favourite"], Some(0.5));
        sidebar.toggle_tag(&b, FAVOURITE);
        assert_eq!(names(&sidebar), ["b", "c", "a"]);
        sidebar.tag_done(Ok(()));
        sidebar.toggle_tag(&b, FAVOURITE);
        sidebar.tag_done(Ok(()));
        assert_eq!(names(&sidebar), ["c", "a", "b"]);
    }

    #[test]
    fn a_rejected_pin_gives_the_room_back_its_order_and_place() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(&mut sidebar, &account, &[("a", Some(3)), ("b", Some(2))]);
        pinned(&mut sidebar, &account, "a", 0, &["m.favourite"], Some(0.5));
        let b = scope(&account, "b");
        sidebar.toggle_tag(&b, FAVOURITE);
        assert_eq!(names(&sidebar), ["b", "a"]);
        sidebar.tag_done(Err(matrix_engine::operations::OperationError::Forbidden));
        assert_eq!(names(&sidebar), ["a", "b"]);
        assert_eq!(sidebar.rooms()[sidebar.visible_indices()[1]].info.favourite_order, None);
    }

    #[test]
    fn there_is_no_limit_to_the_pinned_rooms() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        for i in 0..40 {
            pinned(&mut sidebar, &account, &format!("r{i:02}"), 0, &["m.favourite"], Some(1.0 / (i + 2) as f64));
        }
        assert_eq!(names(&sidebar).len(), 40);
        assert_eq!(names(&sidebar)[0], "r39");
    }
}
#[cfg(test)]
mod invite_tests {
    use super::*;
    use matrix_engine::{RoomInfo, RoomSummary};

    fn scope(account: AccountId, id: &str) -> RoomScope {
        RoomScope { account_id: account, room_id: RoomId::new(id) }
    }

    fn upsert(sidebar: &mut Sidebar, account: AccountId, id: &str, name: &str, info: RoomInfo) {
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: scope(account, id),
            name: name.into(),
            unread: 0,
            info,
        })));
    }

    fn invite() -> RoomInfo {
        RoomInfo { invited: true, ..Default::default() }
    }

    fn names(sidebar: &Sidebar) -> Vec<&str> {
        sidebar.visible_indices().iter().map(|&i| sidebar.rooms()[i].name.as_str()).collect()
    }

    /// The reproduction of the federated invite: the engine projects it as `invited`
    /// (also over federation) and the list used to drop it.
    #[test]
    fn a_federated_invite_gets_a_row_first_in_the_list_and_never_opens_a_conversation() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let chat = RoomInfo { preview: Some("x".into()), joined_members: 2, ..Default::default() };
        upsert(&mut sidebar, account, "!a:t.invalid", "chat", chat);
        upsert(&mut sidebar, account, "!inv:127.0.0.1:8448", "gnobbi-test-fed-invite", invite());
        assert_eq!(names(&sidebar), ["gnobbi-test-fed-invite", "chat"]);
        assert_eq!(sidebar.invite_count(), 1);
        sidebar.select(&scope(account, "!inv:127.0.0.1:8448"));
        assert!(sidebar.selected().is_none(), "an invitation is not an open conversation");
        assert!(sidebar.selected_invite().is_some());
    }

    #[test]
    fn invites_ignore_the_empty_rule_spaces_and_survive_a_restart_projection() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        // Alone in the room and nothing unread: a joined room like this is listed too, and an invitation is.
        let lonely = RoomInfo { invited: true, joined_members: 1, ..Default::default() };
        upsert(&mut sidebar, account, "!dm:t.invalid", "Anna", lonely);
        let space = RoomInfo { invited: true, is_space: true, ..Default::default() };
        upsert(&mut sidebar, account, "!sp:t.invalid", "Spazio", space);
        assert_eq!(names(&sidebar), ["Anna", "Spazio"]);
        assert_eq!(sidebar.spaces().count(), 0, "an invited space is not in the rail");
        sidebar.set_query("ann");
        assert_eq!(names(&sidebar), ["Anna"]);
    }

    #[test]
    fn a_room_without_a_name_is_listed_under_the_label_the_engine_gives() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        upsert(&mut sidebar, account, "!x:t.invalid", "!x:t.invalid", invite());
        assert_eq!(names(&sidebar), ["!x:t.invalid"]);
    }

    #[test]
    fn many_invites_stay_ahead_of_more_than_twenty_rooms() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        for i in 0..30 {
            let info = RoomInfo { preview: Some("x".into()), joined_members: 2, ..Default::default() };
            upsert(&mut sidebar, account, &format!("!r{i}:t.invalid"), &format!("room{i:02}"), info);
        }
        for i in 0..3 {
            upsert(&mut sidebar, account, &format!("!i{i}:t.invalid"), &format!("inv{i}"), invite());
        }
        assert_eq!(sidebar.visible_indices().len(), 33);
        assert!(names(&sidebar)[..3].iter().all(|n| n.starts_with("inv")));
        assert!(sidebar.rooms()[sidebar.visible_indices()[3]].name.starts_with("room"));
    }

    #[test]
    fn a_room_outside_the_active_view_can_still_be_opened() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let info = || RoomInfo { joined_members: 2, preview: Some("x".into()), ..Default::default() };
        upsert(&mut sidebar, account, "!a:t.invalid", "a", info());
        upsert(&mut sidebar, account, "!b:t.invalid", "b", info());
        sidebar.set_query("b");
        let a = scope(account, "!a:t.invalid");
        assert!(!sidebar.select(&a));
        assert!(sidebar.select_outside_view(&a));
        assert_eq!(sidebar.selected().map(|r| &r.scope), Some(&a));
    }

    #[test]
    fn successful_leave_removes_the_row_without_waiting_for_sync() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let room = scope(account, "!a:t.invalid");
        upsert(&mut sidebar, account, "!a:t.invalid", "a", RoomInfo { joined_members: 2, preview: Some("x".into()), ..Default::default() });
        assert_eq!(sidebar.selected().map(|r| &r.scope), Some(&room));
        sidebar.room_left(&room, Ok(()));
        assert!(sidebar.rooms().iter().all(|r| r.scope != room));
        assert!(sidebar.selected().is_none());
    }

    #[test]
    fn a_room_the_sdk_no_longer_has_joined_is_dropped_not_reported_as_not_found() {
        use matrix_engine::operations::OperationError::Unavailable;
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let a = scope(account, "!a:t.invalid");
        upsert(&mut sidebar, account, "!a:t.invalid", "a", RoomInfo { joined_members: 2, preview: Some("x".into()), ..Default::default() });
        sidebar.room_left(&a, Err(Unavailable));
        assert!(sidebar.rooms().is_empty() && sidebar.leave_error(&a).is_none());
    }

    #[test]
    fn an_empty_room_is_listed_and_leaving_it_removes_it() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let e = scope(account, "!e:t.invalid");
        upsert(&mut sidebar, account, "!e:t.invalid", "vuota", RoomInfo { joined_members: 1, ..Default::default() });
        assert_eq!(names(&sidebar), ["vuota"]);
        upsert(&mut sidebar, account, "!t:t.invalid", "t", RoomInfo { joined_members: 2, preview: Some("x".into()), tags: vec!["u.work".into()], ..Default::default() });
        sidebar.room_left(&e, Ok(()));
        assert_eq!(names(&sidebar), ["t"]);
    }

    #[test]
    fn a_failed_leave_shows_one_error_line_on_its_row_until_the_next_click() {
        use matrix_engine::operations::OperationError::{Forbidden, Transport};
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let (a, b) = (scope(account, "!a:t.invalid"), scope(account, "!b:t.invalid"));
        upsert(&mut sidebar, account, "!a:t.invalid", "a", RoomInfo { joined_members: 2, preview: Some("x".into()), ..Default::default() });
        upsert(&mut sidebar, account, "!b:t.invalid", "b", RoomInfo { joined_members: 2, preview: Some("x".into()), ..Default::default() });
        sidebar.room_left(&a, Err(Transport));
        assert_eq!(sidebar.leave_error(&a), Some("Connessione non riuscita"));
        assert_eq!(sidebar.leave_error(&b), None);
        sidebar.room_left(&a, Err(Forbidden));
        assert_eq!(sidebar.leave_error(&a), Some("Non consentito dal server"));
        assert!(sidebar.rooms().iter().any(|r| r.scope == a), "a refused leave never removes the row");
        assert!(sidebar.select(&b));
        assert_eq!(sidebar.leave_error(&a), None);
        sidebar.room_left(&a, Err(Transport));
        sidebar.room_left(&a, Ok(()));
        assert_eq!(sidebar.leave_error(&a), None);
    }

    #[test]
    fn accepting_is_one_click_rejecting_needs_a_second_and_nothing_goes_out_unasked() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let (inv, other) = (scope(account, "!i:t.invalid"), scope(account, "!o:t.invalid"));
        upsert(&mut sidebar, account, "!i:t.invalid", "inv", invite());
        upsert(&mut sidebar, account, "!o:t.invalid", "other", invite());
        assert_eq!(sidebar.invite_state(&inv), InviteState::Idle);
        // reject: first click only asks; a click elsewhere disarms
        assert_eq!(sidebar.answer_invite(&inv, false), None);
        assert_eq!(sidebar.invite_state(&inv), InviteState::ConfirmReject);
        assert_eq!(sidebar.answer_invite(&other, false), None);
        assert_eq!(sidebar.invite_state(&inv), InviteState::Idle);
        assert_eq!(sidebar.answer_invite(&other, false), Some(false));
        assert_eq!(sidebar.invite_state(&other), InviteState::Busy);
        assert_eq!(sidebar.answer_invite(&other, true), None, "busy rows take no second answer");
        // accept fails: the row stays, shows the error, and Riprova sends again
        assert_eq!(sidebar.answer_invite(&inv, true), Some(true));
        sidebar.invite_answered(&inv, false);
        assert_eq!(sidebar.invite_state(&inv), InviteState::Failed);
        assert_eq!(sidebar.answer_invite(&inv, true), Some(true));
        // the sync then joins the room: the row becomes a conversation and the state is gone
        upsert(&mut sidebar, account, "!i:t.invalid", "inv", RoomInfo { joined_members: 2, preview: Some("x".into()), ..Default::default() });
        assert_eq!(sidebar.invite_state(&inv), InviteState::Idle);
        assert!(sidebar.is_joined(&inv));
        assert_eq!(sidebar.invite_count(), 1);
        // the other one is left: the sync removes it
        sidebar.apply_diff(RoomListDiff::Remove(other.clone()));
        assert_eq!(sidebar.invite_count(), 0);
        assert_eq!(sidebar.answer_invite(&other, true), None, "no invitation, no answer");
    }
}

#[cfg(test)]
mod favourite_tests {
    use super::*;
    use matrix_engine::operations::{OperationError, TagEdit};

    fn one() -> (Sidebar, RoomScope) {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let scope = RoomScope { account_id: account, room_id: RoomId::new("!r:x") };
        sidebar.rooms.push(SidebarRoom {
            scope: scope.clone(),
            name: "r".into(),
            initials: "R".into(),
            preview: String::new(),
            time: "",
            color: 0,
            unread: 0,
            info: Default::default(),
        });
        sidebar.set_query("");
        (sidebar, scope)
    }

    #[test]
    fn mode_flips_at_once_and_a_failed_write_restores_it_with_one_line() {
        use matrix_domain::notifications::RoomNotifyMode::*;
        let (mut s, scope) = one();
        assert!(s.set_mode(&scope, Mute) && s.rooms()[0].info.muted());
        assert!(!s.set_mode(&scope, AllMessages), "one write at a time");
        s.mode_done(Err(matrix_engine::operations::OperationError::Forbidden));
        assert_eq!((s.notify_mode(&scope), s.mode_error()), (AllMessages, Some("Non consentito dal server")));
        assert!(!s.set_mode(&scope, AllMessages), "unchanged mode sends nothing");
        assert!(s.set_mode(&scope, MentionsAndKeywordsOnly) && s.mode_error().is_none());
        s.mode_done(Ok(()));
        assert_eq!(s.notify_mode(&scope), MentionsAndKeywordsOnly);
    }

    #[test]
    fn toggle_flips_at_once_and_the_row_pins() {
        let (mut s, scope) = one();
        assert!(!s.rooms()[0].pinned());
        let edit = s.toggle_tag(&scope, FAVOURITE);
        assert_eq!(edit, Some(TagEdit::Assign { room_id: "!r:x".into(), tag: FAVOURITE.into(), on: true }));
        assert!(s.is_favourite(&scope) && s.rooms()[0].pinned());
        assert!(s.toggle_tag(&scope, FAVOURITE).is_none(), "one toggle at a time");
        s.tag_done(Ok(()));
        assert!(s.is_favourite(&scope) && s.tag_error().is_none());
        let off = s.toggle_tag(&scope, FAVOURITE);
        assert!(matches!(off, Some(TagEdit::Assign { on: false, .. })));
        assert!(!s.rooms()[0].pinned());
    }

    #[test]
    fn menu_label_follows_the_state() {
        let (mut s, scope) = one();
        assert_eq!(s.favourite_label(&scope), "Fissa in alto");
        s.toggle_tag(&scope, FAVOURITE);
        assert_eq!(s.favourite_label(&scope), "Stacca");
        s.tag_done(Ok(()));
        s.toggle_tag(&scope, FAVOURITE);
        assert_eq!(s.favourite_label(&scope), "Fissa in alto");
    }

    #[test]
    fn failure_restores_the_real_state_with_one_line() {
        let (mut s, scope) = one();
        s.toggle_tag(&scope, FAVOURITE);
        s.tag_done(Err(OperationError::Transport));
        assert!(!s.is_favourite(&scope));
        assert_eq!(s.tag_error(), Some("Connessione non riuscita"));
        assert!(!s.tag_pending());
        // the same from the favourite side
        s.tag_done(Ok(()));
        s.toggle_tag(&scope, FAVOURITE);
        s.tag_done(Ok(()));
        s.toggle_tag(&scope, FAVOURITE);
        s.tag_done(Err(OperationError::Forbidden));
        assert!(s.is_favourite(&scope));
        assert_eq!(s.tag_error(), Some("Non consentito dal server"));
    }

    fn tags(s: &Sidebar, scope: &RoomScope) -> Vec<String> {
        s.rooms.iter().find(|r| r.scope == *scope).unwrap().info.tags.clone()
    }

    #[test]
    fn low_priority_label_and_row_flip_at_once() {
        let (mut s, scope) = one();
        assert_eq!(s.low_priority_label(&scope), "Segna come bassa priorità");
        assert!(!s.rooms()[0].low_priority());
        let edit = s.toggle_tag(&scope, LOW_PRIORITY);
        assert_eq!(edit, Some(TagEdit::Assign { room_id: "!r:x".into(), tag: LOW_PRIORITY.into(), on: true }));
        assert_eq!(s.low_priority_label(&scope), "Togli dalla bassa priorità");
        assert!(s.rooms()[0].low_priority());
        s.tag_done(Ok(()));
        let off = s.toggle_tag(&scope, LOW_PRIORITY);
        assert!(matches!(off, Some(TagEdit::Assign { on: false, .. })));
        assert!(!s.rooms()[0].low_priority());
    }

    #[test]
    fn each_tag_turns_the_other_off_in_the_same_flip() {
        let (mut s, scope) = one();
        s.toggle_tag(&scope, FAVOURITE);
        s.tag_done(Ok(()));
        let edit = s.toggle_tag(&scope, LOW_PRIORITY);
        assert!(matches!(edit, Some(TagEdit::Assign { on: true, .. })), "one edit, no second request");
        assert!(s.has_tag(&scope, LOW_PRIORITY) && !s.is_favourite(&scope));
        s.tag_done(Ok(()));
        s.toggle_tag(&scope, FAVOURITE);
        assert!(s.is_favourite(&scope) && !s.has_tag(&scope, LOW_PRIORITY));
    }

    #[test]
    fn failure_restores_both_tags() {
        let (mut s, scope) = one();
        s.toggle_tag(&scope, FAVOURITE);
        s.tag_done(Ok(()));
        let before = tags(&s, &scope);
        s.toggle_tag(&scope, LOW_PRIORITY);
        s.tag_done(Err(OperationError::Transport));
        assert_eq!(tags(&s, &scope), before);
        assert!(s.is_favourite(&scope) && !s.has_tag(&scope, LOW_PRIORITY));
        assert_eq!(s.tag_error(), Some("Connessione non riuscita"));
    }
}
