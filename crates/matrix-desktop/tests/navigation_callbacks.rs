use matrix_desktop::Sidebar;
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};

#[test]
fn painted_navigation_callback_cannot_rebind_to_a_returned_visit() {
    let mut sidebar = Sidebar::demo(AccountId::new(17));
    sidebar.set_navigation_visibility(false, true, true);
    let captured = sidebar.navigation_callback();
    assert!(captured(&sidebar));
    sidebar.set_navigation_visibility(false, true, false);
    sidebar.set_navigation_visibility(false, true, true);
    assert!(!captured(&sidebar));
    let current = sidebar.navigation_callback();
    assert!(current(&sidebar));
    let mut foreign = Sidebar::demo(AccountId::new(18));
    foreign.set_navigation_visibility(false, true, true);
    assert!(!current(&foreign));
}

#[test]
fn hidden_navigation_cannot_issue_a_live_painted_callback() {
    let mut sidebar = Sidebar::demo(AccountId::new(17));
    sidebar.set_navigation_visibility(false, false, false);
    let hidden = sidebar.navigation_callback();
    assert!(!hidden(&sidebar));
}

#[test]
fn painted_callbacks_retire_on_unobserved_presentation_roundtrips() {
    for (source, away) in [
        ((false, true, true), (false, true, false)),
        ((false, true, false), (false, false, false)),
        ((true, true, false), (false, true, false)),
        ((true, true, false), (true, false, false)),
    ] {
        let mut sidebar = Sidebar::demo(AccountId::new(17));
        sidebar.set_navigation_visibility(source.0, source.1, source.2);
        let queued = sidebar.navigation_callback();
        sidebar.set_navigation_visibility(away.0, away.1, away.2);
        assert!(!queued(&sidebar));
        sidebar.set_navigation_visibility(source.0, source.1, source.2);
        assert!(!queued(&sidebar));
        assert!(sidebar.navigation_callback()(&sidebar));
    }
}

fn room() -> RoomSummary {
    RoomSummary {
        scope: RoomScope {
            account_id: AccountId::new(17),
            room_id: RoomId::new("!room:test"),
        },
        name: "Room".into(),
        unread: 1,
        info: RoomInfo::default(),
    }
}

#[test]
fn painted_callback_rejects_eligibility_replacement_and_same_scope_return() {
    for case in ["remove", "reclassify", "unread", "search", "direct-child"] {
        let mut sidebar = Sidebar::connected(AccountId::new(17));
        let original = room();
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(original.clone())));
        sidebar.set_filter(sidebar.filter().with_unread(true));
        let mut space = original.clone();
        space.scope.room_id = RoomId::new("!space:test");
        space.info.is_space = true;
        space.info.children = vec![original.scope.room_id.clone()];
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(space.clone())));
        if case == "direct-child" {
            sidebar.set_space(Some(space.scope.clone()));
        }
        let captured = sidebar.navigation_callback();
        let mut changed = original.clone();
        match case {
            "remove" => {
                sidebar.apply_diff(RoomListDiff::Remove(original.scope.clone()));
            }
            "reclassify" => {
                changed.info.is_space = true;
                sidebar.apply_diff(RoomListDiff::Upsert(Box::new(changed)));
            }
            "unread" => {
                changed.unread = 0;
                sidebar.apply_diff(RoomListDiff::Upsert(Box::new(changed)));
            }
            "search" => sidebar.set_query("absent"),
            "direct-child" => {
                let mut changed = space.clone();
                changed.info.children.clear();
                sidebar.apply_diff(RoomListDiff::Upsert(Box::new(changed)));
            }
            _ => unreachable!(),
        }
        assert!(!captured(&sidebar), "{case}");
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(original)));
        sidebar.apply_diff(RoomListDiff::Upsert(Box::new(space)));
        sidebar.set_query("");
        assert!(!captured(&sidebar), "returned {case}");
        assert!(sidebar.navigation_callback()(&sidebar));
    }
}

#[test]
fn live_callback_survives_labels_counts_and_irrelevant_hidden_menu_flags() {
    let mut sidebar = Sidebar::connected(AccountId::new(17));
    let mut row = room();
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(row.clone())));
    sidebar.set_navigation_visibility(true, true, false);
    let captured = sidebar.navigation_callback();
    row.name = "New live label".into();
    row.unread = 5;
    row.info.topic = Some("New topic".into());
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(row)));
    sidebar.set_navigation_visibility(true, true, true);
    assert!(captured(&sidebar));
}
