//! «Non lette» and «Tutte le chat» across accounts: the rooms of every running account in one list, newest message first.
use crate::{Sidebar, SidebarRoom};

/// The merged rows as `(slot, room index)`: `slot` is the position of the account in the sidebars handed in, the index is into
/// that sidebar's `rooms()`. Each sidebar comes with a key of its account that does not change when the accounts swap places
/// (the user id): it settles ties, so a switch of account never reorders the list. Rebuilt only when a sidebar changed (its `revision`), the view or the search term did.
#[derive(Default)]
pub struct UnifiedList {
    revisions: Vec<u64>,
    unread_view: bool,
    query: String,
    rows: Vec<(usize, usize)>,
    /// How many times the rows were rebuilt (the cache test reads it).
    builds: u32,
}

/// Whether the list holds the rooms of every account: the setting is on, more than one account runs, and neither an account
/// (picked in the menu) nor a space was picked. Off or not, the single-account list is the one the shown account always had.
pub fn unified_applies(enabled: bool, one_account: bool, other_accounts: usize, in_space: bool) -> bool {
    enabled && !one_account && other_accounts > 0 && !in_space
}

impl UnifiedList {
    /// Invitations first, then pinned, the rest and low priority last (as in a single account); inside each, the last message
    /// (received or sent) first. The SDK order of one account cannot be compared with another's, so the stamp decides.
    pub fn rows<'a>(&mut self, sidebars: impl Iterator<Item = (&'a Sidebar, &'a str)> + Clone, unread_view: bool, query: &str) -> &[(usize, usize)] {
        let fresh = self.unread_view == unread_view
            && self.query == query
            && self.revisions.len() == sidebars.clone().count()
            && self.revisions.iter().zip(sidebars.clone()).all(|(r, (s, _))| *r == s.revision());
        if !fresh {
            let needle = query.trim().to_lowercase();
            let mut rows: Vec<_> = sidebars
                .clone()
                .enumerate()
                .flat_map(|(slot, (s, _))| s.unified_matches(unread_view, &needle).map(move |i| (slot, i)))
                .collect();
            let all: Vec<_> = sidebars.clone().collect();
            rows.sort_by(|&(sa, ia), &(sb, ib)| order((&all[sa].0.rooms()[ia], all[sa].0, all[sa].1), (&all[sb].0.rooms()[ib], all[sb].0, all[sb].1)));
            self.builds += 1;
            (self.revisions, self.unread_view, self.query, self.rows) = (sidebars.map(|(s, _)| s.revision()).collect(), unread_view, query.to_owned(), rows);
        }
        &self.rows
    }
}

/// The merged order of two rows (room, its sidebar, account key). Rooms without a stamp (nothing decryptable yet) follow the SDK order of their account. No allocation: it runs over every room of every account whenever one changes.
fn order(a: (&SidebarRoom, &Sidebar, &str), b: (&SidebarRoom, &Sidebar, &str)) -> std::cmp::Ordering {
    let rank = |r: &SidebarRoom| (!r.info.invited, if r.pinned() { 0 } else if r.low_priority() { 2 } else { 1 }, std::cmp::Reverse(r.info.last_event_ts.unwrap_or(0)));
    fn name(r: &SidebarRoom) -> impl Iterator<Item = char> + '_ {
        r.name.chars().flat_map(char::to_lowercase)
    }
    rank(a.0).cmp(&rank(b.0)).then_with(|| a.1.rank(a.0).cmp(&b.1.rank(b.0))).then_with(|| name(a.0).cmp(name(b.0))).then_with(|| a.0.scope.room_id.as_str().cmp(b.0.scope.room_id.as_str())).then_with(|| a.2.cmp(b.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId, RoomScope};
    use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};

    /// Both accounts share the engine account id on purpose: the rows must be told apart by slot, not by scope.
    fn upsert(sidebar: &mut Sidebar, id: &str, unread: u64, ts: Option<u64>) {
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(format!("!{id}:t.invalid")) },
            name: id.into(),
            unread,
            info: RoomInfo { last_event_ts: ts, ..Default::default() },
        })));
    }

    #[test]
    fn setting_off_keeps_the_list_to_the_shown_account() {
        let (mut a, mut b) = (Sidebar::connected(AccountId::new(1)), Sidebar::connected(AccountId::new(1)));
        upsert(&mut a, "mine", 0, Some(10));
        upsert(&mut b, "theirs", 0, Some(20));
        let sidebars = [a, b];
        let mut list = UnifiedList::default();
        // On: both accounts. Off: `unified_applies` is false, the app draws the shown sidebar's own list.
        assert_eq!(names(&mut list, &sidebars, false, "").len(), 2);
        assert!(unified_applies(true, false, 1, false));
        assert!(!unified_applies(false, false, 1, false));
        assert_eq!(sidebars[0].visible_indices().len(), 1);
        assert!(!unified_applies(true, true, 1, false) && !unified_applies(true, false, 0, false) && !unified_applies(true, false, 1, true));
    }

    fn names(list: &mut UnifiedList, sidebars: &[Sidebar], unread: bool, query: &str) -> Vec<(usize, String)> {
        let rows = list.rows(sidebars.iter().zip(["@a:t", "@b:t"]), unread, query).to_vec();
        rows.into_iter().map(|(slot, i)| (slot, sidebars[slot].rooms()[i].name.clone())).collect()
    }

    #[test]
    fn search_across_accounts_pins_first_newest_next_low_priority_last() {
        let tagged = |sidebar: &mut Sidebar, id: &str, tag: &str, ts: u64| {
            upsert(sidebar, id, 0, Some(ts));
            let room = sidebar.rooms().iter().find(|r| r.name == id).unwrap();
            sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary { scope: room.scope.clone(), name: id.into(), unread: 0, info: RoomInfo { last_event_ts: Some(ts), tags: vec![tag.into()], ..Default::default() } })));
        };
        let (mut a, mut b) = (Sidebar::connected(AccountId::new(1)), Sidebar::connected(AccountId::new(1)));
        tagged(&mut a, "x_low", "m.lowpriority", 100);
        upsert(&mut a, "x_old", 0, Some(10));
        tagged(&mut a, "x_pin", "m.favourite", 1);
        upsert(&mut b, "x_new", 0, Some(90));
        upsert(&mut b, "x_mid", 0, Some(50));
        upsert(&mut b, "other", 0, Some(99));
        let sidebars = [a, b];
        let got = names(&mut UnifiedList::default(), &sidebars, false, "x_");
        assert_eq!(got, [(0, "x_pin".into()), (1, "x_new".into()), (1, "x_mid".into()), (0, "x_old".into()), (0, "x_low".into())]);
    }

    #[test]
    fn rooms_of_every_account_by_last_message_with_a_shared_room_once_per_account() {
        let mut a = Sidebar::connected(AccountId::new(1));
        let mut b = Sidebar::connected(AccountId::new(1));
        upsert(&mut a, "old", 0, Some(10));
        upsert(&mut a, "shared", 2, Some(30));
        upsert(&mut b, "new", 1, Some(50));
        upsert(&mut b, "shared", 0, Some(20));
        upsert(&mut b, "idle", 0, None);
        let mut sidebars = [a, b];
        let mut list = UnifiedList::default();
        let all = names(&mut list, &sidebars, false, "");
        let order: Vec<_> = all.iter().map(|(slot, n)| (*slot, n.as_str())).collect();
        assert_eq!(order, [(1, "new"), (0, "shared"), (1, "shared"), (0, "old"), (1, "idle")]);
        // «Non lette»: only rooms with unread messages, whichever the account; the search runs over both.
        assert_eq!(names(&mut list, &sidebars, true, ""), [(1, "new".into()), (0, "shared".into())]);
        assert_eq!(names(&mut list, &sidebars, false, " SHA"), [(0, "shared".into()), (1, "shared".into())]);
        // A message on the older account moves its room to the top; the cache notices (the new revision), an unchanged call does not rebuild.
        upsert(&mut sidebars[0], "old", 1, Some(99));
        assert_eq!(names(&mut list, &sidebars, false, "")[0], (0, "old".into()));
        let builds = list.builds;
        // Rooms without a stamp keep the SDK order of their account, interleaved by position.
        let (mut c, mut d) = (Sidebar::connected(AccountId::new(1)), Sidebar::connected(AccountId::new(1)));
        for id in ["c0", "c1"] { upsert(&mut c, id, 0, None); }
        for id in ["d0", "d1"] { upsert(&mut d, id, 0, None); }
        let scope = |id: &str| RoomScope { account_id: AccountId::new(1), room_id: RoomId::new(format!("!{id}:t.invalid")) };
        c.set_order(vec![scope("c1"), scope("c0")]);
        d.set_order(vec![scope("d0"), scope("d1")]);
        let both = [c, d];
        assert_eq!(names(&mut UnifiedList::default(), &both, false, ""), [(0, "c1".to_owned()), (1, "d0".to_owned()), (0, "c0".to_owned()), (1, "d1".to_owned())]);
        names(&mut list, &sidebars, false, "");
        assert_eq!(list.builds, builds);
    }
}
