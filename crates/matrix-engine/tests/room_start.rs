//! Start a direct chat and create rooms through the engine operations, against a disposable
//! homeserver (driven by the room_mgmt_probe script with PROBE_TEST=room_start). A creates, B sees.
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    operations::{OperationError, create_room, edit_room, start_dm},
    sdk::{Client, reqwest::Certificate, sliding_sync::VersionBuilder},
    sdk_ui::sync_service::SyncService,
};
use std::time::{Duration, Instant};

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn a_starts_a_dm_and_creates_rooms_and_b_sees_them() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let login = |who: &'static str| {
            let (url, ca) = (config["url"].as_str().unwrap().to_owned(), ca.clone());
            let (user, pass) = (
                config[format!("{who}_mxid")].as_str().unwrap().to_owned(),
                config[format!("{who}_password")]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
            async move {
                let client = Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap();
                client
                    .matrix_auth()
                    .login_username(&user, &pass)
                    .send()
                    .await
                    .unwrap();
                let service = SyncService::builder(client.clone()).build().await.unwrap();
                service.start().await;
                (client, service)
            }
        };
        let (a, _sa) = login("alice").await;
        let (b, _sb) = login("bob").await;
        let account = AccountId::new(1);
        let bob = b.user_id().unwrap().to_string();

        assert!(matches!(
            start_dm(&a, account, "not a user id").await,
            Err(OperationError::InvalidInput)
        ));
        assert!(matches!(
            create_room(&a, account, "  ", None, false).await,
            Err(OperationError::InvalidInput)
        ));

        let dm = start_dm(&a, account, &bob).await.unwrap();
        let private = create_room(
            &a,
            account,
            "gnobbi-test-private",
            Some("gnobbi-topic"),
            false,
        )
        .await
        .unwrap();
        let public = create_room(&a, account, "gnobbi-test-public", None, true)
            .await
            .unwrap();
        assert!(dm.room_id != private.room_id && private.room_id != public.room_id);

        // The SDK recorded m.direct: asking again reuses the chat instead of making a second one.
        until("A sees the DM in m.direct", || {
            a.get_dm_room(b.user_id().unwrap()).is_some()
        })
        .await;
        assert_eq!(start_dm(&a, account, &bob).await.unwrap(), dm);

        // B is invited to the DM; the DM is flagged direct for B. B is not invited to the rooms.
        until("B sees the DM invite", || {
            b.invited_rooms()
                .iter()
                .any(|r| r.room_id().as_str() == dm.room_id.as_str())
        })
        .await;
        let invited = b
            .invited_rooms()
            .into_iter()
            .find(|r| r.room_id().as_str() == dm.room_id.as_str())
            .unwrap();
        assert!(invited.is_direct().await.unwrap());
        for room in [&private, &public] {
            let id = matrix_engine::sdk::ruma::RoomId::parse(room.room_id.as_str()).unwrap();
            let own = a.get_room(&id).expect("A is in its new room");
            assert_eq!(own.state(), matrix_engine::sdk::RoomState::Joined);
        }
        // Visibility: the public room is joinable by B without invitation; the private one is not.
        let public_id = matrix_engine::sdk::ruma::RoomId::parse(public.room_id.as_str()).unwrap();
        let private_id = matrix_engine::sdk::ruma::RoomId::parse(private.room_id.as_str()).unwrap();
        assert!(
            b.join_room_by_id(&public_id).await.is_ok(),
            "B joins the public room"
        );
        assert!(
            b.join_room_by_id(&private_id).await.is_err(),
            "B cannot join the private room"
        );
        // Encryption: only the private room is born encrypted.
        let own_private = a.get_room(&private_id).unwrap();
        let own_public = a.get_room(&public_id).unwrap();
        assert!(
            own_private
                .latest_encryption_state()
                .await
                .unwrap()
                .is_encrypted()
        );
        assert!(
            !own_public
                .latest_encryption_state()
                .await
                .unwrap()
                .is_encrypted()
        );
        assert_eq!(own_private.topic().as_deref(), Some("gnobbi-topic"));

        // A user the server does not know: no new room.
        let joined = a.joined_rooms().len();
        let ghost = format!(
            "@gnobbi-test-ghost-nobody:{}",
            b.user_id().unwrap().server_name()
        );
        let r = start_dm(&a, account, &ghost).await;
        assert!(matches!(r, Err(OperationError::Unavailable)));
        assert_eq!(a.joined_rooms().len(), joined);

        // Over-long name: both servers accept 300 chars today; the contract is only "no panic".
        let r = create_room(&a, account, &"n".repeat(300), None, false).await;
        println!("[start] long-name result {r:?}");
        // Same name twice is allowed (names are not unique).
        let dup = create_room(&a, account, "gnobbi-test-private", None, false)
            .await
            .unwrap();
        assert!(dup.room_id != private.room_id);

        // «Modifica stanza»: B (plain member of the public room) is refused, A (creator) changes
        // name, topic and avatar through the SDK setters.
        let r = edit_room(&b, AccountId::new(1), &public, Some("hijack"), None, None).await;
        assert!(matches!(r, Err(OperationError::Forbidden)), "member edit: {r:?}");
        assert!(matches!(
            edit_room(&a, account, &public, Some("  "), None, None).await,
            Err(OperationError::InvalidInput)
        ));
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff,
            0xff, 0x3f, 0, 5, 0xfe, 2, 0xfe, 0xa7, 0x35, 0x81, 0x84, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
            0x60, 0x82,
        ];
        let file = std::env::temp_dir().join("gnobbi-test-avatar.png");
        std::fs::write(&file, PNG).unwrap();
        edit_room(&a, account, &public, Some("gnobbi-test-renamed"), Some("new topic"), Some(&file))
            .await
            .unwrap();
        let own = a.get_room(&public_id).unwrap();
        until("A sees the new name, topic and avatar", || {
            own.name().as_deref() == Some("gnobbi-test-renamed")
                && own.topic().as_deref() == Some("new topic")
                && own.avatar_url().is_some()
        })
        .await;

        // B declines the DM invite; A asks for the chat again.
        invited.leave().await.unwrap();
        until("B no longer has the invite", || {
            !b.invited_rooms()
                .iter()
                .any(|r| r.room_id().as_str() == dm.room_id.as_str())
        })
        .await;
        // A learns about the decline through its own sync before the retry.
        let old = a.get_room(&matrix_engine::sdk::ruma::RoomId::parse(dm.room_id.as_str()).unwrap());
        let old = old.expect("A still has the old DM");
        let bob_id = b.user_id().unwrap().to_owned();
        let start = Instant::now();
        while matches!(
            old.get_member_no_sync(&bob_id).await,
            Ok(Some(m)) if matches!(m.membership(), matrix_engine::sdk::ruma::events::room::member::MembershipState::Invite)
        ) {
            assert!(start.elapsed() < Duration::from_secs(40), "timeout: A sees B leave");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let again = start_dm(&a, account, &bob).await;
        let again = again.expect("start_dm after a declined invite must not fail");
        assert!(again.room_id != dm.room_id, "a declined DM must not be reused");
        until("B sees the new invite", || {
            b.invited_rooms()
                .iter()
                .any(|r| r.room_id().as_str() == again.room_id.as_str())
        })
        .await;
        // Two direct rooms with B now (abandoned + pending): a further ask must reuse the pending one, whatever order the SDK lists them.
        until("A lists both DMs with B", || a.get_dm_rooms(b.user_id().unwrap()).count() == 2).await;
        assert_eq!(start_dm(&a, account, &bob).await.unwrap(), again);
        println!(
            "[start] dm {} public {} private {} ok",
            dm.room_id.as_str(),
            public.room_id.as_str(),
            private.room_id.as_str()
        );
    });
}
