//! Member list and another user's profile through the engine operations, against a disposable
//! homeserver (the room_mgmt_probe script with PROBE_TEST=room_profile). Two synthetic accounts:
//! A has a name and an avatar and makes a room, B joins and reads the members and A's profile.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    bounded,
    room_sync::RoomSync,
    sliding,
    operations::{OperationError, join_room, members, user_profile},
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{api::client::room::create_room::v3::Request as CreateRoomRequest, assign},
        sliding_sync::VersionBuilder,
    },
};
use std::{num::NonZeroUsize, time::Duration};

// 1x1 transparent PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0, 5,
    0xfe, 2, 0xfe, 0xa7, 0x35, 0x81, 0x84, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn b_lists_members_and_reads_the_profile_of_a() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let login = |who: &'static str| {
            let (url, ca) = (config["url"].as_str().unwrap().to_owned(), ca.clone());
            let user = config[format!("{who}_mxid")].as_str().unwrap().to_owned();
            let pass = config[format!("{who}_password")].as_str().unwrap().to_owned();
            async move {
                let client = Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap();
                client.matrix_auth().login_username(&user, &pass).send().await.unwrap();
                client
            }
        };
        let (a, b) = (login("alice").await, login("bob").await);
        let account = AccountId::new(2);
        let a_id = a.user_id().unwrap().to_string();
        let b_id = b.user_id().unwrap().to_string();
        a.account().set_display_name(Some("Alice Gnobbi")).await.unwrap();
        a.account().upload_avatar(&"image/png".parse().unwrap(), PNG.to_vec()).await.unwrap();
        let mut request = assign!(CreateRoomRequest::new(), { name: Some("Gnobbi members".to_owned()) });
        request.visibility = matrix_engine::sdk::ruma::api::client::room::Visibility::Public;
        let room = a.create_room(request).await.unwrap();
        let scope = RoomScope { account_id: account, room_id: RoomId::new(room.room_id().as_str()) };
        join_room(&b, account, room.room_id().as_str(), &[]).await.unwrap();

        // Both members, with the id and the name of each.
        let mut list = Vec::new();
        for _ in 0..20 {
            list = members(&b, account, &scope).await.unwrap();
            if list.len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let mut ids: Vec<_> = list.iter().map(|m| m.user_id.clone()).collect();
        ids.sort();
        let mut want = vec![a_id.clone(), b_id.clone()];
        want.sort();
        assert_eq!(ids, want);
        assert!(list.iter().all(|m| !m.name.is_empty()));

        // Profile of the other user: id, name and avatar; a missing user is refused.
        let profile = user_profile(&b, &a_id).await.unwrap();
        assert_eq!((profile.user_id.as_str(), profile.name.as_str()), (a_id.as_str(), "Alice Gnobbi"));
        assert!(profile.avatar.as_deref().is_some_and(|u| u.starts_with("mxc://")));
        let server = a.user_id().unwrap().server_name().to_owned();
        assert_eq!(user_profile(&b, &format!("@gnobbi-test-missing:{server}")).await, Err(OperationError::Unavailable));
        assert_eq!(user_profile(&b, "not an id").await, Err(OperationError::InvalidInput));
        println!("[profile] members {ids:?} profile {profile:?} ok");
    });
}

///: B already read the members, then A leaves while B syncs. The next read must list only B
/// (it came back with A and B, while the counter said 1).
#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn members_after_a_leaves_while_b_syncs_lists_only_b() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let login = |who: &'static str| {
            let (url, ca) = (config["url"].as_str().unwrap().to_owned(), ca.clone());
            let user = config[format!("{who}_mxid")].as_str().unwrap().to_owned();
            let pass = config[format!("{who}_password")].as_str().unwrap().to_owned();
            async move {
                let client = Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap();
                client.matrix_auth().login_username(&user, &pass).send().await.unwrap();
                client
            }
        };
        let (a, b) = (login("alice").await, login("bob").await);
        let account = AccountId::new(2);
        let b_id = b.user_id().unwrap().to_string();
        let mut request = assign!(CreateRoomRequest::new(), { name: Some("Gnobbi leave".to_owned()) });
        request.visibility = matrix_engine::sdk::ruma::api::client::room::Visibility::Public;
        let room = a.create_room(request).await.unwrap();
        let scope = RoomScope { account_id: account, room_id: RoomId::new(room.room_id().as_str()) };
        join_room(&b, account, room.room_id().as_str(), &[]).await.unwrap();

        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(account, b.clone());
        // Phase 1: B's sync runs until B has read both members.
        {
            let run = sliding::run(&mut sync, &mut tx, || {});
            tokio::pin!(run);
            let first = async {
                let mut list = Vec::new();
                for _ in 0..20 {
                    list = members(&b, account, &scope).await.unwrap();
                    if list.len() == 2 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                assert_eq!(list.len(), 2, "precondition: both members listed");
            };
            tokio::select! {
                _ = &mut run => panic!("engine stopped"),
                r = tokio::time::timeout(Duration::from_secs(60), first) => r.expect("timed out"),
            }
        }
        // A leaves and B writes more than one sync window's worth of events while its sync is stopped:
        // the next sync delivers only the tail of the timeline, so the leave reaches B as a count alone.
        room.leave().await.unwrap();
        let bob_room = b.get_room(room.room_id()).unwrap();
        for n in 0..5 {
            bob_room
                .send(matrix_engine::sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(format!("m{n}")))
                .await
                .unwrap();
        }
        // Phase 2: sync resumes; wait for the counter, then read.
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let second = async {
            let end = tokio::time::Instant::now() + Duration::from_secs(30);
            while bob_room.joined_members_count() != 1 {
                tokio::select! {
                    _ = rx.recv() => {}
                    _ = tokio::time::sleep_until(end) => panic!("the counter never went to 1"),
                }
            }
            let ids: Vec<_> = members(&b, account, &scope).await.unwrap().into_iter().map(|m| m.user_id).collect();
            assert_eq!(ids, vec![b_id], "counter says 1, the list must be only B");
        };
        tokio::select! {
            _ = &mut run => panic!("engine stopped"),
            r = tokio::time::timeout(Duration::from_secs(120), second) => r.expect("timed out"),
        }
    });
}
