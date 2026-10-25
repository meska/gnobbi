//! A room seen once opens from the SDK event cache after a restart, with the homeserver
//! unreachable. Driven by the cached_history_probe script against a disposable homeserver.
#![cfg(feature = "sdk")]
use super::AccountStore;
use crate::{ViewRequests, auth::AccountSession, discovery::HomeserverUrl, room_sync::RoomSync};
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_sdk::ruma::{
    api::client::room::create_room::v3::Request as CreateRoom,
    events::room::message::RoomMessageEventContent,
};
use std::time::Duration;

const PASSPHRASE: &str = "long disposable fixture passphrase";
const COUNT: usize = 30;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn eventually<T, F: std::future::Future<Output = Option<T>>>(what: &str, mut f: impl FnMut() -> F) -> T {
    for _ in 0..200 {
        if let Some(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timeout waiting for {what}");
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn seen_room_opens_from_the_cache_after_restart_with_the_homeserver_unreachable() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let root = std::path::PathBuf::from(env("CACHE_PROBE_ROOT"));
    let http = || {
        crate::auth::secure_http_builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&ca).unwrap())
            .build()
            .unwrap()
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let bob = matrix_sdk::Client::builder()
            .homeserver_url(&url)
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        bob.matrix_auth()
            .login_username("@bob:matrix-desktop.test", config["bob_password"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap_or_else(|| panic!("bob room"));
        for n in 0..COUNT {
            bob_room.send(RoomMessageEventContent::text_plain(format!("m{n}"))).await.unwrap();
        }

        // First run: online, the room is seen and its history reaches the cache.
        let store = AccountStore::create(&root, AccountId::new(292)).unwrap();
        let session = store
            .password_login_with_http(
                HomeserverUrl::parse(&url).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                PASSPHRASE,
                http(),
            )
            .await
            .unwrap();
        let alice = session.client.clone().unwrap();
        let invited = eventually("invite", || async {
            alice.sync_once(Default::default()).await.ok()?;
            alice.get_room(&room_id)
        })
        .await;
        invited.join().await.unwrap();
        alice.sync_once(Default::default()).await.unwrap();
        let room = alice.get_room(&room_id).unwrap();
        let (events, _drop) = room.event_cache().await.unwrap();
        eventually("history in cache", || async {
            events.pagination().run_backwards_until(COUNT as u16 + 5).await.ok()?;
            (events.events().await.ok()?.len() >= COUNT).then_some(())
        })
        .await;
        let scope = RoomScope {
            account_id: AccountId::new(292),
            room_id: RoomId::new(room_id.as_str()),
        };
        let mut views = ViewRequests::new();
        let sync = RoomSync::new(session);
        let before = sync.cached_history(views.start(scope.clone()).unwrap()).await.unwrap();
        assert!(before.events.len() >= COUNT, "online cache read has the messages");
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, sync));

        // Restart: same sealed store, homeserver address that nobody answers.
        let store = AccountStore::open(&root, AccountId::new(292)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession = serde_json::from_value(value["session"].clone()).unwrap();
        let offline = store
            .client(&HomeserverUrl::parse("https://localhost:1/").unwrap(), PASSPHRASE, http())
            .await
            .unwrap();
        offline.restore_session(sdk_session).await.unwrap();
        let sync = RoomSync::new(AccountSession {
            account_id: AccountId::new(292),
            client: Some(offline),
            store: Some(store),
        });
        let started = std::time::Instant::now();
        let page = sync.cached_history(views.start(scope).unwrap()).await.unwrap();
        println!("OFFLINE_OPEN_MS {}", started.elapsed().as_millis());
        let bodies: Vec<&str> = page.events.iter().map(|e| e.body.as_str()).collect();
        let expect: Vec<String> = (0..COUNT).map(|n| format!("m{n}")).collect();
        assert!(bodies.len() >= COUNT && bodies.ends_with(&expect.iter().map(String::as_str).collect::<Vec<_>>()),
            "cached messages in order, newest last: {bodies:?}");
        let ids: std::collections::BTreeSet<_> =
            page.events.iter().map(|e| e.event_id.as_str().to_owned()).collect();
        assert_eq!(ids.len(), page.events.len(), "no duplicates");
        assert_eq!(page.events.len(), before.events.len(), "same rows as before the restart");
    });
}
