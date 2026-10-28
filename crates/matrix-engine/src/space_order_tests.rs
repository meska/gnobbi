// Probe: standard mechanisms for a personal, synchronized space order.
// A is the app's real path (SyncService, simplified sliding sync); B is a
// second client of the same account that reorders the spaces, like another
// device would. Verifies the unmerged MSC3230 room account data
// (org.matrix.msc3230.space_order, the Element Web format) and the spec's
// shared m.space.child order, to keep the two mechanisms apart.
#[test]
#[ignore = "requires disposable Synapse; run via the space_order_probe script"]
fn disposable_space_order_room_account_data_reaches_the_sliding_client() {
    use crate::sdk::{
        self,
        reqwest,
        sliding_sync::VersionBuilder,
        ruma::{
            RoomId,
            events::{AnyRoomAccountDataEventContent, RoomAccountDataEventType, StateEventType},
            serde::Raw,
        },
    };
    use crate::sdk_ui::sync_service::SyncService;
    use std::time::{Duration, Instant};

    const TYPE: &str = "org.matrix.msc3230.space_order";
    let config = fixture();
    runtime().block_on(async {
        // A: app-like client, simplified sliding sync only.
        let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
        let a = sdk::Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        a.matrix_auth()
            .login_username(
                config["alice_mxid"].as_str().unwrap(),
                config["alice_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let service = SyncService::builder(a.clone()).build().await.unwrap();
        service.start().await;

        // Three spaces plus one plain child room.
        let mut ids: Vec<String> = Vec::new();
        for name in ["gnobbi-test-space-one", "gnobbi-test-space-two", "gnobbi-test-space-three"] {
            let made = room_request(
                &config,
                "alice",
                reqwest::Method::POST,
                "createRoom",
                serde_json::json!({"name": name, "visibility": "private",
                                    "creation_content": {"type": "m.space"}}),
            )
            .await;
            ids.push(made["room_id"].as_str().unwrap().to_owned());
        }
        let child = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name": "gnobbi-test-child", "visibility": "private"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let server = config["alice_mxid"].as_str().unwrap().rsplit(':').next().unwrap().to_owned();

        // B: another client of the same account sets a personal order on each
        // space (room account data, the unmerged MSC3230 / Element Web format)
        // and the spec's shared child order inside space one.
        let b = login(&config, "alice", 201).await.client.unwrap();
        b.sync_once(sdk::config::SyncSettings::default()).await.unwrap();
        let ty = RoomAccountDataEventType::from(TYPE);
        for (room, order) in ids.iter().zip(["m", "z", "a"]) {
            let room = b.get_room(&RoomId::parse(room.as_str()).unwrap()).expect("B joined the space");
            let raw = serde_json::from_value::<Raw<AnyRoomAccountDataEventContent>>(
                serde_json::json!({"order": order}),
            )
            .unwrap();
            room.set_account_data_raw(ty.clone(), raw).await.unwrap();
        }
        room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{}/state/m.space.child/{}", ids[0], child),
            serde_json::json!({"via": [server], "order": "b", "suggested": false}),
        )
        .await;

        // Controls: the server stored the account data, and a classic-sync
        // client of the same account sees it.
        room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!(
                "user/{}/rooms/{}/account_data/{}",
                config["alice_mxid"].as_str().unwrap(),
                ids[0],
                TYPE
            ),
            serde_json::json!({}),
        )
        .await;
        let c = login(&config, "alice", 202).await.client.unwrap();
        c.sync_once(sdk::config::SyncSettings::default()).await.unwrap();
        let c_room = c.get_room(&RoomId::parse(ids[0].as_str()).unwrap()).expect("C joined");
        let stored = c_room
            .account_data(ty.clone())
            .await
            .unwrap()
            .expect("classic client sees the space_order account data");
        assert!(
            stored
                .deserialize()
                .map(|e: sdk::ruma::events::AnyRoomAccountDataEvent| {
                    e.event_type().to_string()
                })
                .unwrap()
                == TYPE
        );

        // A must receive the personal orders through sliding sync alone.
        let start = Instant::now();
        let mut seen: Vec<(String, String)> = Vec::new();
        loop {
            seen.clear();
            for room in ids.iter() {
                let Some(r) = a.get_room(&RoomId::parse(room.as_str()).unwrap()) else { continue };
                if let Ok(Some(raw)) = r.account_data(ty.clone()).await {
                    let v: serde_json::Value = serde_json::from_str(raw.json().get()).unwrap();
                    if let Some(order) = v["content"]["order"].as_str() {
                        seen.push((order.to_owned(), room.clone()));
                    }
                }
            }
            if seen.len() == 3 {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "timeout: space_order not delivered to the sliding client ({}/3); A knows {} rooms",
                seen.len(),
                a.rooms().len()
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        seen.sort();
        // Lexicographic order first, room id on ties (Element Web's rule).
        assert_eq!(seen[0], ("a".to_owned(), ids[2].clone()), "space three comes first");
        assert_eq!(seen[1], ("m".to_owned(), ids[0].clone()));
        assert_eq!(seen[2], ("z".to_owned(), ids[1].clone()));

        // The shared m.space.child order also reaches A's state store.
        let start = Instant::now();
        loop {
            let state = a
                .get_room(&RoomId::parse(ids[0].as_str()).unwrap())
                .unwrap()
                .get_state_event(StateEventType::SpaceChild, &child)
                .await;
            if let Ok(Some(raw)) = state {
                let v: serde_json::Value = serde_json::to_value(&raw).unwrap();
                assert_eq!(v["content"]["order"].as_str(), Some("b"));
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "timeout: m.space.child not synced");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        eprintln!(
            "SPACE_ORDER ok: rail {:?}, child order 'b' on {}",
            seen.iter().map(|(o, r)| format!("{o}:{r}")).collect::<Vec<_>>(),
            ids[0]
        );
    });
}
