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
    /// Only me in the room, nothing unread and no live message. Unknown member count (0) is never empty.
    fn is_empty(&self) -> bool {
        !self.info.is_space
            && self.info.joined_members == 1
            && self.unread == 0
            && self.info.preview.is_none()
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

/// Personal group filter (from the standard room tags), shown like a Telegram folder.
pub struct GroupEntry {
    /// Tag name; None is «Tutte» (no group filter).
    pub tag: Option<String>,
    pub name: String,
    pub unread: u64,
    pub selected: bool,
}

/// Label of a tag: the part after `u.`, or the Italian name of the two standard ones.
pub fn group_label(tag: &str) -> String {
    match tag {
        "m.favourite" => "Preferiti".into(),
        "m.lowpriority" => "Bassa priorità".into(),
        _ => tag.strip_prefix("u.").unwrap_or(tag).to_owned(),
    }
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
    unread_only: bool,
    show_empty: bool,
    /// Rooms the user just created: shown although still empty.
    kept: std::collections::HashSet<RoomScope>,
    space: Option<RoomScope>,
    group: Option<String>,
    selected: Option<usize>,
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
            unread_only: false,
            show_empty: false,
            kept: Default::default(),
            selected: None,
            space: None,
            group: None,
        }
    }

    pub fn apply_diff(&mut self, diff: RoomListDiff) -> bool {
        let summary = match diff {
            RoomListDiff::Upsert(summary) => summary,
            RoomListDiff::Remove(scope) => {
                if scope.account_id != self.account_id {
                    return false;
                }
                let selected = self.selected().map(|room| room.scope.clone());
                let old_len = self.rooms.len();
                self.rooms.retain(|room| room.scope != scope);
                if old_len != self.rooms.len() {
                    self.retire_navigation();
                }
                self.selected = selected
                    .and_then(|scope| self.rooms.iter().position(|room| room.scope == scope))
                    .or_else(|| (!self.rooms.is_empty()).then_some(0));
                self.set_query(&self.query.clone());
                return old_len != self.rooms.len();
            }
        };
        // shortcut: invitations are projected by the engine but have no row yet (no accept/reject UI).
        if summary.scope.account_id != self.account_id || summary.info.invited {
            return false;
        }
        let row = SidebarRoom {
            scope: summary.scope,
            initials: summary.name.chars().take(2).collect(),
            name: summary.name,
            preview: summary.info.preview.clone().unwrap_or_default(),
            time: "",
            color: 0x5288b5,
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
        if self.selected.is_none() {
            self.selected = Some(0);
        }
        self.set_query(&self.query.clone());
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
                    info: Default::default(),
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
            unread_only: false,
            show_empty: false,
            kept: Default::default(),
            selected: Some(0),
            space: None,
            group: None,
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
            self.retire_navigation();
            self.navigation_context = context;
        }
    }

    pub fn set_navigation_visibility(&mut self, persistent: bool, visible: bool, menu_open: bool) {
        let visibility = (persistent, visible, !persistent && visible && menu_open);
        if self.navigation_visibility != visibility {
            self.retire_navigation();
            self.navigation_visibility = visibility;
        }
    }

    fn retire_navigation(&mut self) {
        self.navigation_revision = self
            .navigation_revision
            .checked_add(1)
            .expect("navigation revision exhausted");
    }

    pub fn set_unread_only(&mut self, enabled: bool) {
        if self.unread_only != enabled {
            self.retire_navigation();
        }
        self.unread_only = enabled;
        self.set_query(&self.query.clone());
    }

    pub fn set_show_empty(&mut self, enabled: bool) {
        if self.show_empty != enabled {
            self.retire_navigation();
        }
        self.show_empty = enabled;
        self.set_query(&self.query.clone());
    }

    pub fn show_empty(&self) -> bool {
        self.show_empty
    }

    /// Keeps a just-created room in the list while it has no message yet.
    pub fn keep_visible(&mut self, scope: RoomScope) {
        self.kept.insert(scope);
        self.set_query(&self.query.clone());
    }

    /// Rooms that are empty, whether or not they are currently shown.
    pub fn hidden_empty_count(&self) -> usize {
        self.rooms
            .iter()
            .filter(|room| room.is_empty() && !self.kept.contains(&room.scope))
            .count()
    }

    pub fn unread_only(&self) -> bool {
        self.unread_only
    }

    pub fn spaces(&self) -> impl Iterator<Item = &SidebarRoom> {
        self.rooms.iter().filter(|room| room.info.is_space)
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
            None => ("Tutte".into(), "▦".into(), None),
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
                .filter(|room| keep(room))
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
            selected: self.space.is_none(),
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

    /// «Tutte» plus one entry per tag in use (favourites, low priority, then `u.*` by name); empty
    /// when no room has a group tag. Unread counts every room of the group, whatever the other filters.
    pub fn group_entries(&self) -> Vec<GroupEntry> {
        let mut tags: Vec<&str> = self
            .rooms
            .iter()
            .filter(|room| !room.info.is_space)
            .flat_map(|room| room.info.tags.iter().map(String::as_str))
            .collect();
        let rank = |t: &str| match t {
            "m.favourite" => 0,
            "m.lowpriority" => 1,
            _ => 2,
        };
        tags.sort_by_key(|t| (rank(t), *t));
        tags.dedup();
        if tags.is_empty() {
            return Vec::new();
        }
        let mut entries = vec![GroupEntry {
            tag: None,
            name: "Tutte".into(),
            unread: 0,
            selected: self.group.is_none(),
        }];
        entries.extend(tags.into_iter().map(|tag| GroupEntry {
            tag: Some(tag.to_owned()),
            name: group_label(tag),
            unread: self
                .rooms
                .iter()
                .filter(|room| !room.info.is_space && room.info.tags.iter().any(|t| t == tag))
                .fold(0, |sum, room| sum.saturating_add(room.unread)),
            selected: self.group.as_deref() == Some(tag),
        }));
        entries
    }

    pub fn active_group(&self) -> Option<&str> {
        self.group.as_deref()
    }

    /// Filters the list by one group tag; None shows every room. Never changes the open conversation.
    pub fn set_group(&mut self, tag: Option<String>) {
        if self.group != tag {
            self.retire_navigation();
        }
        self.group = tag;
        self.set_query(&self.query.clone());
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
        if self.space != scope {
            self.retire_navigation();
        }
        self.space = scope;
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
        // A group nobody carries any more is dropped, like a vanished space.
        if self.group.as_ref().is_some_and(|g| !self.rooms.iter().any(|r| r.info.tags.contains(g))) {
            self.group = None;
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
                room.name.to_lowercase().contains(&needle)
                    && (!self.unread_only || room.unread > 0)
                    && (self.show_empty || !room.is_empty() || self.kept.contains(&room.scope))
                    && self.group.as_ref().is_none_or(|g| room.info.tags.contains(g))
                    && space.is_none_or(|s| {
                        room.scope == s.scope || s.info.children.contains(&room.scope.room_id)
                    })
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
    }

    /// Reject foreign account/room identities and rows outside the current filter.
    pub fn select(&mut self, scope: &RoomScope) -> bool {
        if let Some(&i) = self
            .visible
            .iter()
            .find(|&&i| self.rooms[i].scope == *scope)
        {
            if self.selected != Some(i) {
                self.retire_navigation();
            }
            self.selected = Some(i);
            true
        } else {
            false
        }
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
    pub fn selected(&self) -> Option<&SidebarRoom> {
        self.selected.map(|i| &self.rooms[i])
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
            sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
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
            }));
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

    fn scope(account: &AccountId, name: &str) -> RoomScope {
        RoomScope {
            account_id: *account,
            room_id: RoomId::new(format!("!{name}:t.invalid")),
        }
    }

    #[test]
    fn a_new_message_keeps_the_open_room_selected_in_every_filter() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(
            &mut sidebar,
            &account,
            &[("a", Some(3)), ("b", Some(2)), ("c", Some(1))],
        );
        let open = scope(&account, "c");
        assert!(sidebar.select(&open));
        sidebar.set_unread_only(false);
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
    fn a_new_message_reorders_inside_a_selected_space_and_the_unread_filter() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        fill(
            &mut sidebar,
            &account,
            &[("x", Some(1)), ("y", Some(2)), ("z", Some(3))],
        );
        sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
            scope: scope(&account, "space"),
            name: "space".into(),
            unread: 0,
            info: RoomInfo {
                is_space: true,
                children: vec![RoomId::new("!x:t.invalid"), RoomId::new("!y:t.invalid")],
                ..Default::default()
            },
        }));
        assert!(sidebar.set_space(Some(scope(&account, "space"))));
        assert!(sidebar.select(&scope(&account, "x")));
        fill(&mut sidebar, &account, &[("x", Some(9))]);
        assert_eq!(
            names(&sidebar),
            ["x", "y", "space"][..names(&sidebar).len()]
        );
        assert_eq!(sidebar.selected().map(|r| r.name.as_str()), Some("x"));
        sidebar.set_unread_only(true);
        sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
            scope: scope(&account, "y"),
            name: "y".into(),
            unread: 2,
            info: RoomInfo {
                preview: Some("x".into()),
                ..Default::default()
            },
        }));
        assert_eq!(names(&sidebar), ["y"]);
        assert_eq!(sidebar.selected().map(|r| r.name.as_str()), Some("x"));
    }

    #[test]
    fn list_header_names_the_selected_space_with_initials_or_tutte() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        assert_eq!(sidebar.list_header(), ("Tutte".into(), "▦".into(), None));
        assert_eq!(sidebar.list_header_avatar(), None);
        sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
            scope: scope(&account, "bots"),
            name: "Bots".into(),
            unread: 0,
            info: RoomInfo {
                is_space: true,
                avatar: Some("mxc://s/bots".into()),
                ..Default::default()
            },
        }));
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
            sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
                scope: scope(&account, "bots"),
                name: "Bots".into(),
                unread: 0,
                info: RoomInfo { is_space: true, ..Default::default() },
            }));
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
    fn optimistic_read_lowers_room_space_filter_and_mention_counters() {
        let other = AccountId::new(2);
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        let upsert = |sidebar: &mut Sidebar, id: &str, unread, mentions, children: &[&str]| {
            sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
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
            }));
        };
        upsert(&mut sidebar, "a", 3, 2, &[]);
        upsert(&mut sidebar, "b", 4, 1, &[]);
        upsert(&mut sidebar, "s", 0, 0, &["a"]);
        sidebar.set_unread_only(true);
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
        assert_eq!(names(&sidebar), ["b"]); // unread filter drops the room
        assert!(!sidebar.mark_read_optimistic(&scope(&account, "a"))); // idempotent

        // the next server projection wins
        upsert(&mut sidebar, "a", 1, 0, &[]);
        assert_eq!(counts(&sidebar), [(5, 1), (1, 0)]);
    }

    fn tagged(sidebar: &mut Sidebar, account: &AccountId, name: &str, unread: u64, tags: &[&str]) {
        sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
            scope: scope(account, name),
            name: name.into(),
            unread,
            info: RoomInfo {
                preview: Some("x".into()),
                tags: tags.iter().map(|t| (*t).into()).collect(),
                ..Default::default()
            },
        }));
    }

    #[test]
    fn groups_filter_the_list_and_count_unread_per_group() {
        let account = AccountId::new(1);
        let mut sidebar = Sidebar::connected(account);
        assert!(sidebar.group_entries().is_empty());
        tagged(&mut sidebar, &account, "a", 2, &["m.favourite", "u.work"]);
        tagged(&mut sidebar, &account, "b", 3, &["u.work"]);
        tagged(&mut sidebar, &account, "c", 5, &["m.lowpriority"]);
        tagged(&mut sidebar, &account, "d", 7, &[]);
        let summary = |s: &Sidebar| {
            s.group_entries().into_iter().map(|e| (e.name, e.unread, e.selected)).collect::<Vec<_>>()
        };
        assert_eq!(
            summary(&sidebar),
            [
                ("Tutte".into(), 0, true),
                ("Preferiti".into(), 2, false),
                ("Bassa priorità".into(), 5, false),
                ("work".into(), 5, false),
            ]
        );
        assert_eq!(names(&sidebar).len(), 4);
        sidebar.set_group(Some("u.work".into()));
        let mut got = names(&sidebar);
        got.sort();
        assert_eq!(got, ["a", "b"]);
        assert_eq!(sidebar.group_entries()[3].selected, true);
        // Counters ignore the group, search and unread filters; the unread filter still applies on top.
        sidebar.set_unread_only(true);
        sidebar.set_query("b");
        assert_eq!(names(&sidebar), ["b"]);
        assert_eq!(summary(&sidebar)[1], ("Preferiti".into(), 2, false));
        sidebar.set_query("");
        sidebar.set_unread_only(false);
        // A new message lifts the counter; read rooms drop out of it.
        tagged(&mut sidebar, &account, "b", 10, &["u.work"]);
        assert_eq!(sidebar.group_entries()[3].unread, 12);
        sidebar.mark_read_optimistic(&scope(&account, "b"));
        assert_eq!(sidebar.group_entries()[3].unread, 2);
        // The server drops the tag from the last room that had it: the filter falls back to «Tutte».
        tagged(&mut sidebar, &account, "a", 2, &["m.favourite"]);
        tagged(&mut sidebar, &account, "b", 0, &[]);
        assert_eq!(sidebar.active_group(), None);
        assert_eq!(names(&sidebar).len(), 4);
    }
}
