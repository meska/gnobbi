//! A room seen once opens from the SDK event cache after a restart, with the homeserver
//! unreachable. Driven by the cached_history_probe script against a disposable homeserver.
#![cfg(feature = "sdk")]
use super::AccountStore;
use crate::{ViewRequests, auth::AccountSession, discovery::HomeserverUrl, room_sync::RoomSync};
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_sdk::ruma::{
    api::client::room::create_room::v3::Request as CreateRoom,
    events::{reaction::ReactionEventContent, relation::Annotation, room::message::{ReplacementMetadata, RoomMessageEventContent}},
};
use std::time::Duration;

const PASSPHRASE: &str = "long disposable fixture passphrase";
const COUNT: usize = 30;
const OPEN: bool = true;

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

#[test]
#[ignore = "requires the cached_history_probe script"]
fn reopened_room_asks_the_server_only_for_the_messages_it_missed() {
    const MISSED: usize = 25;
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
        let store = AccountStore::create(&root, AccountId::new(293)).unwrap();
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
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, session));
        // Marks where the second run starts in the homeserver request log.
        http().get(format!("{url}_matrix/client/versions?mark=reopen")).send().await.unwrap();

        // Alice is away while more messages arrive.
        for n in COUNT..COUNT + MISSED {
            bob_room.send(RoomMessageEventContent::text_plain(format!("m{n}"))).await.unwrap();
        }

        // Restart online: opening the room brings in exactly the missed messages.
        let store = AccountStore::open(&root, AccountId::new(293)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let client = store.client(&HomeserverUrl::parse(&url).unwrap(), PASSPHRASE, http()).await.unwrap();
        client.restore_session(sdk_session).await.unwrap();
        let scope = RoomScope {
            account_id: AccountId::new(293),
            room_id: RoomId::new(room_id.as_str()),
        };
        // The same route the app uses: selecting the room sends an opening request.
        let (mut out, mut results) = crate::bounded(std::num::NonZeroUsize::new(4).unwrap());
        let mut views = ViewRequests::new();
        let tag = views.start(scope).unwrap();
        let request = crate::history::HistoryRequest { tag, before: None, open: OPEN };
        assert!(crate::room_sync::serve_history(&client, AccountId::new(293), request, &mut out).await);
        let saved = results.recv().await.unwrap().page.unwrap();
        assert_eq!(saved.events.len(), COUNT, "saved messages come first, before the gap");
        let page = tokio::time::timeout(Duration::from_secs(20), results.recv())
            .await
            .expect("the full list follows the saved one")
            .unwrap()
            .page
            .unwrap();
        drop((store, client));
        let bodies: Vec<&str> = page.events.iter().map(|e| e.body.as_str()).collect();
        let expect: Vec<String> = (0..COUNT + MISSED).map(|n| format!("m{n}")).collect();
        assert_eq!(bodies, expect.iter().map(String::as_str).collect::<Vec<_>>(), "cache plus gap, in order");
        let ids: std::collections::BTreeSet<_> =
            page.events.iter().map(|e| e.event_id.as_str().to_owned()).collect();
        assert_eq!(ids.len(), page.events.len(), "no duplicates");
    });
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn reopened_room_shows_edit_redaction_and_reaction_that_arrived_while_away() {
    const COUNT: usize = 10;
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
        let mut ids = Vec::new();
        for n in 0..COUNT {
            ids.push(bob_room.send(RoomMessageEventContent::text_plain(format!("m{n}"))).await.unwrap().response.event_id);
        }

        // First run: online, the room is seen and its history reaches the cache.
        let store = AccountStore::create(&root, AccountId::new(294)).unwrap();
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
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, session));

        // Alice is away: one message is edited, one redacted, one reacted to, one reacted to and unreacted.
        let sent = |text: &str| RoomMessageEventContent::text_plain(text.to_owned());
        let edit = sent("m1 edited").make_replacement(ReplacementMetadata::new(ids[1].clone(), None));
        bob_room.send(edit).await.unwrap();
        bob_room.redact(&ids[2], None, None).await.unwrap();
        bob_room
            .send(ReactionEventContent::new(Annotation::new(ids[3].clone(), "\u{1f44d}".into())))
            .await
            .unwrap();
        let undone = bob_room
            .send(ReactionEventContent::new(Annotation::new(ids[4].clone(), "\u{1f44e}".into())))
            .await
            .unwrap();
        bob_room.redact(&undone.response.event_id, None, None).await.unwrap();

        // Restart online: opening the room brings in exactly the missed messages.
        let store = AccountStore::open(&root, AccountId::new(294)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let client = store.client(&HomeserverUrl::parse(&url).unwrap(), PASSPHRASE, http()).await.unwrap();
        client.restore_session(sdk_session).await.unwrap();
        let sync = RoomSync::new(AccountSession {
            account_id: AccountId::new(294),
            client: Some(client),
            store: Some(store),
        });
        let scope = RoomScope {
            account_id: AccountId::new(294),
            room_id: RoomId::new(room_id.as_str()),
        };
        let (page, reactions) = sync.open_history(ViewRequests::new().start(scope).unwrap()).await.unwrap();
        let bodies: Vec<&str> = page.events.iter().map(|e| e.body.as_str()).collect();
        assert_eq!(bodies[1], "m1 edited", "edit applied: {bodies:?}");
        assert_eq!(bodies[2], "[Messaggio eliminato]", "redacted row: {bodies:?}");
        assert!(!bodies.contains(&"m2"), "redacted text does not reappear");
        assert_eq!(bodies.iter().filter(|b| b.starts_with("m")).count(), COUNT - 1, "no edit row, no duplicates");
        assert_eq!(reactions.len(), 1, "only the live reaction: {}", reactions.len());
        assert_eq!((reactions[0].key.as_str(), reactions[0].count), ("\u{1f44d}", 1));
        assert_eq!(reactions[0].target, page.events[3].event_id, "reaction sits on m3");
    });
}
