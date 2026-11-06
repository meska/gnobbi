// Sender names on a real Synapse/Tuwunel (the member_names_probe script; alice, bob, carol are the disposable accounts).
// The route the app uses: serve_history → history_sender → disambiguated_name. Included by auth_tests (uses md_login/md_until).

/// Newest page as the app opens a room, on a fresh Timelines so no row keeps an older name.
async fn mn_senders(client: &matrix_sdk::Client, rid: &ruma::OwnedRoomId) -> Vec<(String, crate::history::HistorySender)> {
    let timelines = crate::room_sync::Timelines::default();
    let (mut out, mut results) = crate::bounded(std::num::NonZeroUsize::new(4).unwrap());
    let scope = RoomScope { account_id: AccountId::new(7), room_id: matrix_domain::RoomId::new(rid.as_str()) };
    let mut views = crate::ViewRequests::new(); // dropping it cancels the tag
    let tag = views.start(scope).unwrap();
    let request = crate::history::HistoryRequest { tag, before: None, open: false };
    assert!(crate::room_sync::serve_history(client, &timelines, AccountId::new(7), request, &mut out).await);
    let page = tokio::time::timeout(Duration::from_secs(20), results.recv()).await.expect("history page in 20 s").unwrap().page.unwrap();
    page.events.into_iter().filter(|e| !e.status && !e.redacted).map(|e| (e.body, e.sender)).collect()
}

/// Wait until `body` is in the page, then return its sender as shown (name may be None: the UI then shows the id).
async fn mn_sender_of(client: &matrix_sdk::Client, rid: &ruma::OwnedRoomId, body: &str, want: Option<&str>) -> crate::history::HistorySender {
    let start = Instant::now();
    loop {
        let found = mn_senders(client, rid).await.into_iter().find(|(b, _)| b == body).map(|(_, s)| s);
        if let Some(s) = found.filter(|s| want.is_none_or(|w| s.name.as_deref() == Some(w))) {
            return s;
        }
        assert!(start.elapsed() < Duration::from_secs(45), "timeout: sender of {body:?} never became {want:?}; seen {:?}", mn_senders(client, rid).await);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[test]
#[ignore = "requires the member_names_probe script"]
fn live_sender_names_are_disambiguated_on_a_real_homeserver() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    runtime().block_on(async {
        let (alice, bob, carol) = (md_login(&config, "alice").await, md_login(&config, "bob").await, md_login(&config, "carol").await);
        let (a_id, b_id, c_id) = (alice.user_id().unwrap().to_string(), bob.user_id().unwrap().to_string(), carol.user_id().unwrap().to_string());
        let mut services = Vec::new();
        for c in [&alice, &bob, &carol] {
            let s = matrix_sdk_ui::sync_service::SyncService::builder(c.clone()).build().await.unwrap();
            s.start().await;
            services.push(s);
        }
        let join_all = async |room: &matrix_sdk::Room, who: &[&matrix_sdk::Client]| {
            for c in who {
                room.invite_user_by_id(c.user_id().unwrap()).await.unwrap();
                c.join_room_by_id(room.room_id()).await.unwrap();
            }
        };
        let known = async |rid: &ruma::OwnedRoomId, n: usize| {
            for c in [&alice, &bob, &carol] {
                md_until("room known with all members", async || match c.get_room(rid) {
                    Some(r) => r.members(matrix_sdk::RoomMemberships::JOIN).await.map(|m| m.len() == n).unwrap_or(false),
                    None => false,
                })
                .await;
            }
        };
        let say = async |c: &matrix_sdk::Client, rid: &ruma::OwnedRoomId, text: &str| {
            c.get_room(rid).unwrap().send(RoomMessageEventContent::text_plain(text)).await.unwrap();
        };

        md_step("a. two members with the same display name: both carry their id");
        alice.account().set_display_name(Some("Sam")).await.unwrap();
        bob.account().set_display_name(Some("Sam")).await.unwrap();
        carol.account().set_display_name(Some("Carla")).await.unwrap();
        let room = alice.create_room(ruma::api::client::room::create_room::v3::Request::new()).await.unwrap();
        let rid = room.room_id().to_owned();
        join_all(&room, &[&bob, &carol]).await;
        known(&rid, 3).await;
        say(&alice, &rid, "da alice").await;
        say(&bob, &rid, "da bob").await;
        say(&carol, &rid, "da carol").await;
        let want_a = format!("Sam ({a_id})");
        let want_b = format!("Sam ({b_id})");
        // Seen from the third member and from each of the two namesakes.
        for viewer in [&carol, &alice, &bob] {
            let a = mn_sender_of(viewer, &rid, "da alice", Some(&want_a)).await;
            let b = mn_sender_of(viewer, &rid, "da bob", Some(&want_b)).await;
            assert_eq!((a.id.as_str(), b.id.as_str()), (a_id.as_str(), b_id.as_str()));
            // c. the unique name stays bare.
            assert_eq!(mn_sender_of(viewer, &rid, "da carol", Some("Carla")).await.name.as_deref(), Some("Carla"));
        }
        println!("A ok: {want_a} | {want_b} | Carla");

        md_step("b. one of the two renames: no more parentheses, on rows already in the timeline too");
        bob.account().set_display_name(Some("Roberto")).await.unwrap();
        say(&bob, &rid, "da bob dopo").await;
        for viewer in [&carol, &alice] {
            assert_eq!(mn_sender_of(viewer, &rid, "da bob dopo", Some("Roberto")).await.name.as_deref(), Some("Roberto"));
            assert_eq!(mn_sender_of(viewer, &rid, "da alice", Some("Sam")).await.name.as_deref(), Some("Sam"), "alice is the only Sam now");
        }
        println!("B ok: Sam | Roberto without ids");

        md_step("c. member without a display name: no name, the UI shows the id");
        // Clear the name BEFORE joining, so the member event of the new room carries none.
        let bare = carol.account().set_display_name(None).await;
        println!("C clear display name: {bare:?}");
        let profile = carol.account().get_display_name().await.unwrap();
        println!("C profile displayname after clear: {profile:?}");
        let room_c = alice.create_room(ruma::api::client::room::create_room::v3::Request::new()).await.unwrap();
        let rid_c = room_c.room_id().to_owned();
        join_all(&room_c, &[&carol, &bob]).await;
        known_two(&alice, &carol, &rid_c).await;
        say(&carol, &rid_c, "senza nome").await;
        let s = mn_sender_of(&alice, &rid_c, "senza nome", None).await;
        println!("C sender as shown: id={} name={:?}", s.id, s.name);
        assert_eq!(s.id, c_id);
        assert_eq!(s.name, None, "no display name → name None → UI falls back to the id");

        md_step("d. edge: long name, bridge-style ghosts, name equal to another member's id");
        let long = "Maddalena ".repeat(20).trim_end().to_owned();
        alice.account().set_display_name(Some(&long)).await.unwrap();
        bob.account().set_display_name(Some("Mario (Telegram)")).await.unwrap();
        carol.account().set_display_name(Some(&a_id)).await.unwrap();
        let room_d = alice.create_room(ruma::api::client::room::create_room::v3::Request::new()).await.unwrap();
        let rid_d = room_d.room_id().to_owned();
        join_all(&room_d, &[&bob, &carol]).await;
        known(&rid_d, 3).await;
        say(&alice, &rid_d, "lungo").await;
        say(&bob, &rid_d, "bridge").await;
        say(&carol, &rid_d, "id come nome").await;
        let l = mn_sender_of(&bob, &rid_d, "lungo", Some(&long)).await;
        assert_eq!(l.name.as_deref(), Some(long.as_str()), "a long unique name is kept whole (the UI truncates)");
        let br = mn_sender_of(&alice, &rid_d, "bridge", Some("Mario (Telegram)")).await;
        assert_eq!(br.id, b_id);
        // carol's name IS alice's id: it must not make alice ambiguous, and carol keeps her own id as the way to tell them apart.
        let imp = mn_sender_of(&bob, &rid_d, "id come nome", None).await;
        println!("D name equal to another id: shown name={:?} id={}", imp.name, imp.id);
        assert_eq!(imp.id, c_id);
        let al = mn_sender_of(&bob, &rid_d, "lungo", None).await;
        assert_eq!(al.id, a_id);
        println!("D ok");

        for s in services {
            s.stop().await;
        }
    });
}

async fn known_two(a: &matrix_sdk::Client, b: &matrix_sdk::Client, rid: &ruma::OwnedRoomId) {
    for c in [a, b] {
        md_until("room known with members", async || match c.get_room(rid) {
            Some(r) => r.members(matrix_sdk::RoomMemberships::JOIN).await.map(|m| m.len() == 3).unwrap_or(false),
            None => false,
        })
        .await;
    }
}
