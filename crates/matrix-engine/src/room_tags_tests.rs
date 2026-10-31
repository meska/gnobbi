#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_room_groups_follow_tags_changed_by_another_device() {
    use crate::room_tags::RoomGroups;
    use matrix_sdk::{ruma::{RoomId, events::tag::{TagInfo, TagName}}};
    let order = |o: f64| { let mut i = TagInfo::new(); i.order = Some(o); i };
    runtime().block_on(async {
        let config = fixture();
        // Two synthetic devices of one disposable account: A only reads, B plays "the other client".
        let a = login(&config, "alice", 200).await.client.unwrap();
        let b = login(&config, "alice", 201).await.client.unwrap();
        let mut ids = Vec::new();
        for name in ["gnobbi-test-one", "gnobbi-test-two"] {
            let made = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"name": name, "visibility": "private"})).await;
            ids.push(made["room_id"].as_str().unwrap().to_owned());
        }
        for c in [&a, &b] { crate::room_sync::RoomSync::sliding_catch_up(c).await.unwrap(); }
        let on_b = |i: usize| b.get_room(<&RoomId>::try_from(ids[i].as_str()).unwrap()).expect("B joined");
        let (one, two) = (ids[0].clone(), ids[1].clone());

        assert_eq!(RoomGroups::read(&a).await.unwrap(), RoomGroups::default());

        on_b(0).set_tag(TagName::Favorite, order(0.5)).await.unwrap();
        on_b(0).set_tag(TagName::User("u.work".parse().unwrap()), order(0.8)).await.unwrap();
        on_b(1).set_tag(TagName::User("u.work".parse().unwrap()), order(0.2)).await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&a).await.unwrap();
        let g = RoomGroups::read(&a).await.unwrap();
        assert_eq!(g.favourites, [one.clone()]);
        assert!(g.low_priority.is_empty());
        assert_eq!(g.user["u.work"], [two.clone(), one.clone()]);

        // The other client moves things around: A's model follows after its next sync.
        on_b(0).remove_tag(TagName::Favorite).await.unwrap();
        on_b(0).set_tag(TagName::LowPriority, TagInfo::new()).await.unwrap();
        on_b(1).set_tag(TagName::User("u.work".parse().unwrap()), order(0.9)).await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&a).await.unwrap();
        let g = RoomGroups::read(&a).await.unwrap();
        assert!(g.favourites.is_empty());
        assert_eq!(g.low_priority, [one.clone()]);
        assert_eq!(g.user["u.work"], [one, two]);
        eprintln!("ROOM_TAGS ok: {g:?}");
    });
}

#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_explicit_group_edits_by_one_client_reach_the_other() {
    use crate::operations::{OperationError, TagEdit};
    use crate::room_tags::{RoomGroups, edit};
    runtime().block_on(async {
        let config = fixture();
        // Client A writes (the user's explicit actions), client B only syncs and reads.
        let a = login(&config, "alice", 210).await.client.unwrap();
        let b = login(&config, "alice", 211).await.client.unwrap();
        let mut ids = Vec::new();
        for name in ["gnobbi-test-one", "gnobbi-test-two"] {
            let made = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"name": name, "visibility": "private"})).await;
            ids.push(made["room_id"].as_str().unwrap().to_owned());
        }
        for c in [&a, &b] { crate::room_sync::RoomSync::sliding_catch_up(c).await.unwrap(); }
        let assign = |i: usize, tag: &str, on: bool| TagEdit::Assign { room_id: ids[i].clone(), tag: tag.into(), on };
        let seen = || async { crate::room_sync::RoomSync::sliding_catch_up(&b).await.unwrap(); RoomGroups::read(&b).await.unwrap() };
        assert_eq!(seen().await, RoomGroups::default());

        // Create (first room gives the group its existence, name with a space), assign, favourite.
        edit(&a, &assign(0, "u.gnobbi lavoro", true)).await.unwrap();
        edit(&a, &assign(1, "u.gnobbi lavoro", true)).await.unwrap();
        edit(&a, &assign(0, "m.favourite", true)).await.unwrap();
        let g = seen().await;
        assert_eq!(g.user["u.gnobbi lavoro"].len(), 2);
        assert_eq!(g.favourites, [ids[0].clone()]);

        // A's store follows the server through its own sync, as the app's sync does between clicks.
        crate::room_sync::RoomSync::sliding_catch_up(&a).await.unwrap();
        // Favourite then low priority swap, like the SDK does.
        edit(&a, &assign(0, "m.lowpriority", true)).await.unwrap();
        let g = seen().await;
        assert!(g.favourites.is_empty());
        assert_eq!(g.low_priority, [ids[0].clone()]);

        crate::room_sync::RoomSync::sliding_catch_up(&a).await.unwrap();
        // Rename moves every room; the old name is gone.
        edit(&a, &TagEdit::Rename { from: "u.gnobbi lavoro".into(), to: "u.casa".into() }).await.unwrap();
        let g = seen().await;
        assert_eq!(g.user.keys().collect::<Vec<_>>(), ["u.casa"]);
        assert_eq!(g.user["u.casa"].len(), 2);

        crate::room_sync::RoomSync::sliding_catch_up(&a).await.unwrap();
        // Unassign one room, then delete the group.
        edit(&a, &assign(1, "u.casa", false)).await.unwrap();
        assert_eq!(seen().await.user["u.casa"], [ids[0].clone()]);
        crate::room_sync::RoomSync::sliding_catch_up(&a).await.unwrap();
        edit(&a, &TagEdit::Delete { tag: "u.casa".into() }).await.unwrap();
        let g = seen().await;
        assert!(g.user.is_empty());
        assert_eq!(g.low_priority, [ids[0].clone()]);

        // Not groups, empty names, same-name rename and unknown rooms are refused before any request.
        for bad in [
            assign(0, "m.server_notice", true), assign(0, "u.", true), assign(0, "u.   ", true),
            TagEdit::Rename { from: "u.a".into(), to: "u.a".into() },
            TagEdit::Rename { from: "m.favourite".into(), to: "u.a".into() },
            TagEdit::Delete { tag: "m.favourite".into() },
        ] {
            assert_eq!(edit(&a, &bad).await, Err(OperationError::InvalidInput), "{bad:?}");
        }
        let unknown = TagEdit::Assign { room_id: "!nope:matrix-desktop.test".into(), tag: "u.x".into(), on: true };
        assert_eq!(edit(&a, &unknown).await, Err(OperationError::Unavailable));
        eprintln!("TAG_EDIT ok");
    });
}
