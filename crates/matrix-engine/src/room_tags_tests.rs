#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_room_groups_follow_tags_changed_by_another_device() {
    use crate::room_tags::RoomGroups;
    use matrix_sdk::{config::SyncSettings, ruma::{RoomId, events::tag::{TagInfo, TagName}}};
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
        for c in [&a, &b] { c.sync_once(SyncSettings::default()).await.unwrap(); }
        let on_b = |i: usize| b.get_room(<&RoomId>::try_from(ids[i].as_str()).unwrap()).expect("B joined");
        let (one, two) = (ids[0].clone(), ids[1].clone());

        assert_eq!(RoomGroups::read(&a).await.unwrap(), RoomGroups::default());

        on_b(0).set_tag(TagName::Favorite, order(0.5)).await.unwrap();
        on_b(0).set_tag(TagName::User("u.work".parse().unwrap()), order(0.8)).await.unwrap();
        on_b(1).set_tag(TagName::User("u.work".parse().unwrap()), order(0.2)).await.unwrap();
        a.sync_once(SyncSettings::default()).await.unwrap();
        let g = RoomGroups::read(&a).await.unwrap();
        assert_eq!(g.favourites, [one.clone()]);
        assert!(g.low_priority.is_empty());
        assert_eq!(g.user["u.work"], [two.clone(), one.clone()]);

        // The other client moves things around: A's model follows after its next sync.
        on_b(0).remove_tag(TagName::Favorite).await.unwrap();
        on_b(0).set_tag(TagName::LowPriority, TagInfo::new()).await.unwrap();
        on_b(1).set_tag(TagName::User("u.work".parse().unwrap()), order(0.9)).await.unwrap();
        a.sync_once(SyncSettings::default()).await.unwrap();
        let g = RoomGroups::read(&a).await.unwrap();
        assert!(g.favourites.is_empty());
        assert_eq!(g.low_priority, [one.clone()]);
        assert_eq!(g.user["u.work"], [one, two]);
        eprintln!("ROOM_TAGS ok: {g:?}");
    });
}
