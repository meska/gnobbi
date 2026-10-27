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
    pub selected: bool,
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
    visible: Vec<usize>,
    query: String,
    unread_only: bool,
    show_empty: bool,
    space: Option<RoomScope>,
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
            visible: Vec::new(),
            query: String::new(),
            unread_only: false,
            show_empty: false,
            selected: None,
            space: None,
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
        if summary.scope.account_id != self.account_id {
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
            visible: (0..300).collect(),
            query: String::new(),
            unread_only: false,
            show_empty: false,
            selected: Some(0),
            space: None,
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

    /// Rooms that are empty, whether or not they are currently shown.
    pub fn hidden_empty_count(&self) -> usize {
        self.rooms.iter().filter(|room| room.is_empty()).count()
    }

    pub fn unread_only(&self) -> bool {
        self.unread_only
    }

    pub fn spaces(&self) -> impl Iterator<Item = &SidebarRoom> {
        self.rooms.iter().filter(|room| room.info.is_space)
    }

    /// Counts each joined row once; independent of transient search/unread filters.
    pub fn navigation_entries(&self) -> Vec<NavigationEntry> {
        let mut entries = vec![NavigationEntry {
            scope: None,
            name: "Tutte".into(),
            unread: self
                .rooms
                .iter()
                .fold(0u64, |n, room| n.saturating_add(room.unread)),
            selected: self.space.is_none(),
        }];
        entries.extend(self.spaces().map(|space| {
            NavigationEntry {
                scope: Some(space.scope.clone()),
                name: space.name.clone(),
                unread: self
                    .rooms
                    .iter()
                    .filter(|room| {
                        room.scope == space.scope
                            || space.info.children.contains(&room.scope.room_id)
                    })
                    .fold(0u64, |n, room| n.saturating_add(room.unread)),
                selected: self.space.as_ref() == Some(&space.scope),
            }
        }));
        entries
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
                    && (self.show_empty || !room.is_empty())
                    && space.is_none_or(|s| {
                        room.scope == s.scope || s.info.children.contains(&room.scope.room_id)
                    })
            })
            .map(|(i, _)| i)
            .collect();
        // Search keeps match order; the plain list is newest activity first, ties by name, idle rooms last.
        if needle.is_empty() {
            let rooms = &self.rooms;
            self.visible.sort_by_cached_key(|&i| {
                (
                    std::cmp::Reverse(rooms[i].info.last_activity),
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

    fn fill(sidebar: &mut Sidebar, account: &AccountId, rooms: &[(&str, Option<u64>)]) {
        for (name, last_activity) in rooms {
            sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
                scope: RoomScope {
                    account_id: *account,
                    room_id: RoomId::new(format!("!{name}:t.invalid")),
                },
                name: (*name).into(),
                unread: 0,
                info: RoomInfo {
                    last_activity: *last_activity,
                    preview: Some("x".into()),
                    ..Default::default()
                },
            }));
        }
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
                last_activity: Some(70),
                preview: Some("x".into()),
                ..Default::default()
            },
        }));
        assert_eq!(names(&sidebar), ["y"]);
        assert_eq!(sidebar.selected().map(|r| r.name.as_str()), Some("x"));
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
}
