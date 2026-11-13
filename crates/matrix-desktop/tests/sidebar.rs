use matrix_desktop::{Navigation, Sidebar};
use matrix_domain::{AccountId, RoomId, RoomScope};

#[test]
fn each_room_can_be_filtered_and_selected_by_its_stable_scope() {
    let mut sidebar = Sidebar::demo(AccountId::new(42));
    for i in 0..300 {
        let name = sidebar.rooms()[i].name.clone();
        let scope = sidebar.rooms()[i].scope.clone();
        sidebar.set_query(&name);
        assert_eq!(sidebar.visible_indices(), &[i]);
        assert!(sidebar.select(&scope));
        assert_eq!(sidebar.selected().unwrap().scope, scope);
    }
    sidebar.set_query("");
    let unknown = RoomScope {
        account_id: AccountId::new(42),
        room_id: RoomId::new("!not-in-fixture:fixture.invalid"),
    };
    assert!(!sidebar.select(&unknown));
    assert_eq!(
        sidebar.selected().unwrap().scope,
        sidebar.rooms()[299].scope
    );
}

#[test]
fn navigation_clamps_to_filtered_endpoints_and_empty_results_are_noop() {
    let mut s = Sidebar::demo(AccountId::new(7));
    assert_eq!(s.navigate(Navigation::Previous), Some(0));
    assert_eq!(s.navigate(Navigation::Next), Some(1));
    assert_eq!(s.navigate(Navigation::Last), Some(299));
    assert_eq!(s.navigate(Navigation::Next), Some(299));
    assert_eq!(s.navigate(Navigation::First), Some(0));
    s.set_query("demo 29");
    assert_eq!(s.navigate(Navigation::Next), Some(0));
    assert_eq!(s.selected().unwrap().name, "Stanza demo 290");
    assert_eq!(s.navigate(Navigation::Next), Some(1));
    assert_eq!(s.selected().unwrap().name, "Stanza demo 291");
    assert_eq!(s.navigate(Navigation::Last), Some(9));
    assert_eq!(s.selected().unwrap().name, "Stanza demo 299");
    s.set_query("nothing here");
    assert_eq!(s.navigate(Navigation::Next), None);
    assert_eq!(s.selected().unwrap().name, "Stanza demo 299");
}

#[test]
fn selection_requires_visible_room_in_current_account() {
    let mut a = Sidebar::demo(AccountId::new(7));
    let b = Sidebar::demo(AccountId::new(8));
    let target = a.rooms()[299].scope.clone();
    assert!(!a.select(&b.rooms()[299].scope));
    assert_eq!(a.selected().unwrap().scope, a.rooms()[0].scope);
    assert!(a.select(&target));
    assert_eq!(a.selected().unwrap().scope, target);
    a.set_query("Design");
    assert!(!a.select(&target));
    assert_eq!(a.selected().unwrap().scope, target);
    let visible = a.rooms()[1].scope.clone();
    assert!(a.select(&visible));
    assert_eq!(a.selected().unwrap().scope, visible);
}

#[test]
fn filter_is_case_insensitive_trimmed_name_search_without_changing_selection() {
    let mut sidebar = Sidebar::demo(AccountId::new(7));
    let selected = sidebar.selected().unwrap().scope.clone();
    sidebar.set_query("  ACCESSIBILITÀ  ");
    assert_eq!(sidebar.query(), "  ACCESSIBILITÀ  ");
    assert_eq!(sidebar.visible_indices(), &[5]);
    assert_eq!(sidebar.selected().unwrap().scope, selected);
    sidebar.set_query("demo 30");
    assert_eq!(sidebar.visible_indices(), &[299]);
    sidebar.set_query("no such room");
    assert!(sidebar.visible_indices().is_empty());
    assert_eq!(sidebar.selected().unwrap().scope, selected);
    sidebar.set_query("  ");
    assert_eq!(sidebar.visible_indices().len(), 300);
}

#[test]
fn navigation_revision_retires_actions_after_conversation_away_back() {
    let mut sidebar = Sidebar::demo(AccountId::new(7));
    let original = sidebar.rooms()[0].scope.clone();
    let other = sidebar.rooms()[1].scope.clone();
    let initial = sidebar.navigation_revision();
    sidebar.select(&other);
    sidebar.select(&original);
    assert_ne!(sidebar.navigation_revision(), initial);
    let current = sidebar.navigation_revision();
    sidebar.select(&original);
    sidebar.navigate(Navigation::First);
    assert_eq!(sidebar.navigation_revision(), current);
    sidebar.navigate(Navigation::Next);
    sidebar.navigate(Navigation::Previous);
    assert_ne!(sidebar.navigation_revision(), current);
}

#[test]
fn navigation_revision_retires_actions_after_search_away_back() {
    let mut sidebar = Sidebar::demo(AccountId::new(7));
    let initial = sidebar.navigation_revision();
    sidebar.set_query("missing");
    sidebar.set_query("");
    assert_ne!(sidebar.navigation_revision(), initial);
    let current = sidebar.navigation_revision();
    sidebar.set_query("");
    assert_eq!(sidebar.navigation_revision(), current);
    sidebar.set_query("missing");
    sidebar.set_query("");
    assert_ne!(sidebar.navigation_revision(), current);
}

#[test]
fn sidebar_fixture_has_300_stable_account_scoped_rooms() {
    let a = Sidebar::demo(AccountId::new(7));
    let again = Sidebar::demo(AccountId::new(7));
    let b = Sidebar::demo(AccountId::new(8));
    assert_eq!(a.rooms().len(), 300);
    assert_eq!(a.visible_indices().len(), 300);
    for (i, room) in a.rooms().iter().enumerate() {
        assert_eq!(room.scope.account_id, AccountId::new(7));
        assert_eq!(room.scope, again.rooms()[i].scope);
        assert_eq!(room.name, again.rooms()[i].name);
        assert_ne!(room.scope, b.rooms()[i].scope);
    }
    let ids: std::collections::HashSet<_> = a.rooms().iter().map(|r| &r.scope).collect();
    assert_eq!(ids.len(), 300);
    assert_eq!(a.rooms()[0].name, "Laboratorio UI");
    assert_eq!(a.rooms()[299].name, "Stanza demo 300");
    assert_eq!(a.selected().unwrap().scope, a.rooms()[0].scope);
}

#[test]
fn pinned_and_low_priority_rows_are_named_and_ordered_for_screen_readers() {
    use matrix_desktop::accessibility::RoomNodes;
    use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};
    let account = AccountId::new(5);
    let mut sidebar = Sidebar::connected(account);
    for (id, tag, order) in [("low", Some("m.lowpriority"), None), ("plain", None, None), ("pin", Some("m.favourite"), Some(0.5f64))] {
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
            scope: RoomScope { account_id: account, room_id: RoomId::new(format!("!{id}:t")) },
            name: id.into(),
            unread: 0,
            info: RoomInfo {
                tags: tag.map(|t| vec![t.to_owned()]).unwrap_or_default(),
                favourite_order: order.map(f64::to_bits),
                ..Default::default()
            },
        })));
    }
    let rooms = RoomNodes::default().project(&sidebar);
    let seen: Vec<_> = rooms.iter().map(|r| (r.name.as_str(), r.mark)).collect();
    assert_eq!(seen, [("pin", Some("Fissata")), ("plain", None), ("low", Some("Bassa priorità"))]);
}
