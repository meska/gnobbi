//! Start a direct chat and create rooms through the engine operations, against a disposable
//! homeserver (driven by the room_mgmt_probe script with PROBE_TEST=room_start). A creates, B sees.
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    operations::{OperationError, create_room, start_dm},
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
            create_room(&a, account, "  ", false).await,
            Err(OperationError::InvalidInput)
        ));

        let dm = start_dm(&a, account, &bob).await.unwrap();
        let private = create_room(&a, account, "gnobbi-test-private", false)
            .await
            .unwrap();
        let public = create_room(&a, account, "gnobbi-test-public", true)
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
        println!(
            "[start] dm {} public {} private {} ok",
            dm.room_id.as_str(),
            public.room_id.as_str(),
            private.room_id.as_str()
        );
    });
}
