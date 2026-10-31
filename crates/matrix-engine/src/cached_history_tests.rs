//! A room seen once opens from the SDK event cache after a restart, with the homeserver
//! unreachable. Driven by the cached_history_probe script against a disposable homeserver.
#![cfg(feature = "sdk")]
use super::AccountStore;
use crate::{ViewRequests, auth::AccountSession, discovery::HomeserverUrl, room_sync::RoomSync};
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_sdk::ruma::{
    api::client::room::create_room::v3::Request as CreateRoom,
    events::{
        reaction::ReactionEventContent,
        relation::Annotation,
        room::{
            join_rules::{JoinRule, RoomJoinRulesEventContent},
            message::{ReplacementMetadata, RoomMessageEventContent},
        },
    },
};
use std::time::Duration;

const PASSPHRASE: &str = "long disposable fixture passphrase";
const COUNT: usize = 30;
const OPEN: bool = true;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn eventually<T, F: std::future::Future<Output = Option<T>>>(
    what: &str,
    mut f: impl FnMut() -> F,
) -> T {
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let bob = matrix_sdk::Client::builder()
            .homeserver_url(&url)
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        bob.matrix_auth()
            .login_username(
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap_or_else(|| panic!("bob room"));
        for n in 0..COUNT {
            bob_room
                .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                .await
                .unwrap();
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
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.ok()?;
            alice.get_room(&room_id)
        })
        .await;
        invited.join().await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        let room = alice.get_room(&room_id).unwrap();
        let (events, _drop) = room.event_cache().await.unwrap();
        eventually("history in cache", || async {
            events
                .pagination()
                .run_backwards_until(COUNT as u16 + 5)
                .await
                .ok()?;
            (events.events().await.ok()?.len() >= COUNT).then_some(())
        })
        .await;
        let scope = RoomScope {
            account_id: AccountId::new(292),
            room_id: RoomId::new(room_id.as_str()),
        };
        let mut views = ViewRequests::new();
        let sync = RoomSync::new(session);
        let (before, _) = sync
            .open_history(
                views.start(scope.clone()).unwrap(),
                crate::receipts::ReadVisibility {
                    window_focused: false,
                    room_foreground: false,
                    bottom_gap_px: 0.0,
                    public_receipts: true,
                },
            )
            .await
            .unwrap();
        assert!(
            before.events.len() >= COUNT,
            "online cache read has the messages"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, sync));

        // Restart: same sealed store, homeserver address that nobody answers.
        let store = AccountStore::open(&root, AccountId::new(292)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let offline = store
            .client(
                &HomeserverUrl::parse("https://localhost:1/").unwrap(),
                PASSPHRASE,
                http(),
            )
            .await
            .unwrap();
        offline.restore_session(sdk_session).await.unwrap();
        let sync = RoomSync::new(AccountSession {
            account_id: AccountId::new(292),
            client: Some(offline),
            store: Some(store),
        });
        let started = std::time::Instant::now();
        let (page, _) = sync
            .open_history(
                views.start(scope).unwrap(),
                crate::receipts::ReadVisibility {
                    window_focused: false,
                    room_foreground: false,
                    bottom_gap_px: 0.0,
                    public_receipts: true,
                },
            )
            .await
            .unwrap();
        println!("OFFLINE_OPEN_MS {}", started.elapsed().as_millis());
        let bodies: Vec<&str> = page.events.iter().map(|e| e.body.as_str()).collect();
        let expect: Vec<String> = (0..COUNT).map(|n| format!("m{n}")).collect();
        assert!(
            bodies.len() >= COUNT
                && bodies.ends_with(&expect.iter().map(String::as_str).collect::<Vec<_>>()),
            "cached messages in order, newest last: {bodies:?}"
        );
        let ids: std::collections::BTreeSet<_> = page
            .events
            .iter()
            .map(|e| e.event_id.as_str().to_owned())
            .collect();
        assert_eq!(ids.len(), page.events.len(), "no duplicates");
        assert_eq!(
            page.events.len(),
            before.events.len(),
            "same rows as before the restart"
        );
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let bob = matrix_sdk::Client::builder()
            .homeserver_url(&url)
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        bob.matrix_auth()
            .login_username(
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap_or_else(|| panic!("bob room"));
        for n in 0..COUNT {
            bob_room
                .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                .await
                .unwrap();
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
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.ok()?;
            alice.get_room(&room_id)
        })
        .await;
        invited.join().await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        let room = alice.get_room(&room_id).unwrap();
        let (events, _drop) = room.event_cache().await.unwrap();
        eventually("history in cache", || async {
            events
                .pagination()
                .run_backwards_until(COUNT as u16 + 5)
                .await
                .ok()?;
            (events.events().await.ok()?.len() >= COUNT).then_some(())
        })
        .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, session));
        // Marks where the second run starts in the homeserver request log.
        http()
            .get(format!("{url}_matrix/client/versions?mark=reopen"))
            .send()
            .await
            .unwrap();

        // Alice is away while more messages arrive.
        let mut newest = None;
        for n in COUNT..COUNT + MISSED {
            newest = Some(
                bob_room
                    .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                    .await
                    .unwrap()
                    .response
                    .event_id,
            );
        }

        // Restart online: opening the room brings in exactly the missed messages.
        let store = AccountStore::open(&root, AccountId::new(293)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let client = store
            .client(&HomeserverUrl::parse(&url).unwrap(), PASSPHRASE, http())
            .await
            .unwrap();
        client.restore_session(sdk_session).await.unwrap();
        // The app's running sync service delivers what was missed. A sliding round answers with
        // whatever window the server has ready (Tuwunel under load stopped at m44), so keep
        // syncing, as the service does, until the newest sent message is stored.
        eventually("the missed messages are synced", || async {
            crate::room_sync::RoomSync::sliding_catch_up(&client).await.ok()?;
            let (stored, _, _) = crate::room_sync::stored_events(&client, &room_id).await.ok()?;
            (stored.last()?.event_id() == newest.as_deref()).then_some(())
        })
        .await;
        let scope = RoomScope {
            account_id: AccountId::new(293),
            room_id: RoomId::new(room_id.as_str()),
        };
        // The same route the app uses: selecting the room sends an opening request.
        let timelines = crate::room_sync::Timelines::default();
        let (mut out, mut results) = crate::bounded(std::num::NonZeroUsize::new(4).unwrap());
        let mut views = ViewRequests::new();
        let tag = views.start(scope).unwrap();
        let request = crate::history::HistoryRequest {
            tag,
            before: None,
            open: OPEN,
        };
        assert!(
            crate::room_sync::serve_history(
                &client,
                &timelines,
                AccountId::new(293),
                request,
                &mut out
            )
            .await
        );
        let saved = results.recv().await.unwrap().page.unwrap();
        assert!(
            !saved.events.is_empty(),
            "saved messages come first, before the gap"
        );
        let page = tokio::time::timeout(Duration::from_secs(20), results.recv())
            .await
            .expect("the full list follows the saved one")
            .unwrap()
            .page
            .unwrap();
        drop((store, client));
        // Status rows (alice's own join) sit in the timeline too; the claim is about messages.
        let bodies: Vec<&str> = page
            .events
            .iter()
            .filter(|e| !e.status)
            .map(|e| e.body.as_str())
            .collect();
        let expect: Vec<String> = (0..COUNT + MISSED).map(|n| format!("m{n}")).collect();
        // The Timeline shows the newest part and pages the rest on scroll: a contiguous tail.
        assert!(
            bodies.len() >= MISSED,
            "every missed message is shown: {bodies:?}"
        );
        assert_eq!(
            bodies,
            expect[expect.len() - bodies.len()..]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "cache plus gap, in order"
        );
        let ids: std::collections::BTreeSet<_> = page
            .events
            .iter()
            .map(|e| e.event_id.as_str().to_owned())
            .collect();
        assert_eq!(ids.len(), page.events.len(), "no duplicates");
    });
}

/// Restart while the homeserver is down, then it comes back.
///
/// Expected semantics:
/// 1. Down: the saved room opens from the cache. `open_history` returns `Ok` with exactly the
///    messages seen before, in order, no duplicates, no blocking error and no network wait.
///    Messages that arrived on the server meanwhile are (rightly) not shown yet.
/// 2. Back online: one sync plus the opening request yields saved + missed messages, in send order,
///    each exactly once (same event ids as before the outage keep their place), and the server is
///    asked for at most one `/messages` page (the gap), never the whole room (checked by the probe).
#[test]
#[ignore = "requires the cached_history_probe script"]
fn restart_with_the_server_down_then_online_shows_cache_then_only_the_missing_messages() {
    const MISSED: usize = 15;
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let bob = matrix_sdk::Client::builder()
            .homeserver_url(&url)
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        bob.matrix_auth()
            .login_username(
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap_or_else(|| panic!("bob room"));
        for n in 0..COUNT {
            bob_room
                .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                .await
                .unwrap();
        }
        let scope = RoomScope {
            account_id: AccountId::new(297),
            room_id: RoomId::new(room_id.as_str()),
        };
        let visibility = || crate::receipts::ReadVisibility {
            window_focused: false,
            room_foreground: false,
            bottom_gap_px: 0.0,
            public_receipts: true,
        };

        // First run: online, the room is seen and its history reaches the cache.
        let store = AccountStore::create(&root, AccountId::new(297)).unwrap();
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
            events
                .pagination()
                .run_backwards_until(COUNT as u16 + 5)
                .await
                .ok()?;
            (events.events().await.ok()?.len() >= COUNT).then_some(())
        })
        .await;
        let mut views = ViewRequests::new();
        let sync = RoomSync::new(session);
        let (before, _) = sync
            .open_history(views.start(scope.clone()).unwrap(), visibility())
            .await
            .unwrap();
        assert!(before.events.len() >= COUNT, "online read has the messages");
        // Lets the SDK flush the event cache to disk before the "app" quits.
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, sync));

        // Messages arrive while Alice's app is closed.
        for n in COUNT..COUNT + MISSED {
            bob_room
                .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                .await
                .unwrap();
        }

        // Restart 1: the homeserver is down (address nobody answers). The cache opens, no error.
        let store = AccountStore::open(&root, AccountId::new(297)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let down = store
            .client(
                &HomeserverUrl::parse("https://localhost:1/").unwrap(),
                PASSPHRASE,
                http(),
            )
            .await
            .unwrap();
        down.restore_session(sdk_session.clone()).await.unwrap();
        let sync = RoomSync::new(AccountSession {
            account_id: AccountId::new(297),
            client: Some(down.clone()),
            store: Some(store),
        });
        let (offline, _) = sync
            .open_history(views.start(scope.clone()).unwrap(), visibility())
            .await
            .expect("offline open is not a blocking error");
        let offline_ids: Vec<String> = offline
            .events
            .iter()
            .map(|e| e.event_id.as_str().to_owned())
            .collect();
        assert_eq!(
            offline_ids,
            before
                .events
                .iter()
                .map(|e| e.event_id.as_str().to_owned())
                .collect::<Vec<_>>(),
            "offline shows exactly the saved rows, same order, no duplicates"
        );
        assert!(
            down.sync_once(Default::default()).await.is_err(),
            "the server is really unreachable"
        );
        drop((sync, down));

        // Marks where the online run starts in the homeserver request log.
        http()
            .get(format!("{url}_matrix/client/versions?mark=reopen"))
            .send()
            .await
            .unwrap();

        // Restart 2: the server is back. Same sealed store, real address.
        let store = AccountStore::open(&root, AccountId::new(297)).unwrap();
        let client = store
            .client(&HomeserverUrl::parse(&url).unwrap(), PASSPHRASE, http())
            .await
            .unwrap();
        client.restore_session(sdk_session).await.unwrap();
        client.sync_once(Default::default()).await.unwrap();
        let timelines = crate::room_sync::Timelines::default();
        let (mut out, mut results) = crate::bounded(std::num::NonZeroUsize::new(4).unwrap());
        let tag = views.start(scope).unwrap();
        let request = crate::history::HistoryRequest {
            tag,
            before: None,
            open: OPEN,
        };
        assert!(
            crate::room_sync::serve_history(
                &client,
                &timelines,
                AccountId::new(297),
                request,
                &mut out
            )
            .await
        );
        let saved = results.recv().await.unwrap().page.unwrap();
        assert!(!saved.events.is_empty(), "saved messages come first");
        let page = tokio::time::timeout(Duration::from_secs(20), results.recv())
            .await
            .expect("the full list follows the saved one")
            .unwrap()
            .page
            .unwrap();
        drop((store, client));
        // Status rows (alice's own join) sit in the timeline too; the claim is about messages.
        let bodies: Vec<&str> = page
            .events
            .iter()
            .filter(|e| !e.status)
            .map(|e| e.body.as_str())
            .collect();
        let expect: Vec<String> = (0..COUNT + MISSED).map(|n| format!("m{n}")).collect();
        assert!(bodies.len() >= MISSED, "every missed message is shown: {bodies:?}");
        assert_eq!(
            bodies,
            expect[expect.len() - bodies.len()..]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "cache plus gap, in send order, none lost"
        );
        let ids: std::collections::BTreeSet<_> = page
            .events
            .iter()
            .map(|e| e.event_id.as_str().to_owned())
            .collect();
        assert_eq!(ids.len(), page.events.len(), "no duplicates");
        let tail = &offline_ids[offline_ids.len().saturating_sub(page.events.len() - MISSED)..];
        assert!(
            tail.iter().all(|id| ids.contains(id)),
            "rows seen offline are the same events after the resync"
        );
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let bob = matrix_sdk::Client::builder()
            .homeserver_url(&url)
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        bob.matrix_auth()
            .login_username(
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap_or_else(|| panic!("bob room"));
        let mut ids = Vec::new();
        for n in 0..COUNT {
            ids.push(
                bob_room
                    .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                    .await
                    .unwrap()
                    .response
                    .event_id,
            );
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
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.ok()?;
            alice.get_room(&room_id)
        })
        .await;
        invited.join().await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        let room = alice.get_room(&room_id).unwrap();
        let (events, _drop) = room.event_cache().await.unwrap();
        eventually("history in cache", || async {
            events
                .pagination()
                .run_backwards_until(COUNT as u16 + 5)
                .await
                .ok()?;
            (events.events().await.ok()?.len() >= COUNT).then_some(())
        })
        .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, session));

        // Alice is away: one message is edited, one redacted, one reacted to, one reacted to and unreacted.
        let sent = |text: &str| RoomMessageEventContent::text_plain(text.to_owned());
        let edit =
            sent("m1 edited").make_replacement(ReplacementMetadata::new(ids[1].clone(), None));
        bob_room.send(edit).await.unwrap();
        bob_room.redact(&ids[2], None, None).await.unwrap();
        bob_room
            .send(ReactionEventContent::new(Annotation::new(
                ids[3].clone(),
                "\u{1f44d}".into(),
            )))
            .await
            .unwrap();
        let undone = bob_room
            .send(ReactionEventContent::new(Annotation::new(
                ids[4].clone(),
                "\u{1f44e}".into(),
            )))
            .await
            .unwrap();
        bob_room
            .redact(&undone.response.event_id, None, None)
            .await
            .unwrap();

        // Restart online: opening the room brings in exactly the missed messages.
        let store = AccountStore::open(&root, AccountId::new(294)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let client = store
            .client(&HomeserverUrl::parse(&url).unwrap(), PASSPHRASE, http())
            .await
            .unwrap();
        client.restore_session(sdk_session).await.unwrap();
        // The app's running sync service delivers what was missed; here one sync does.
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let sync = RoomSync::new(AccountSession {
            account_id: AccountId::new(294),
            client: Some(client),
            store: Some(store),
        });
        let scope = RoomScope {
            account_id: AccountId::new(294),
            room_id: RoomId::new(room_id.as_str()),
        };
        let (page, reactions) = sync
            .open_history(
                ViewRequests::new().start(scope).unwrap(),
                crate::receipts::ReadVisibility {
                    window_focused: true,
                    room_foreground: true,
                    bottom_gap_px: 0.0,
                    public_receipts: true,
                },
            )
            .await
            .unwrap();
        let bodies: Vec<&str> = page.events.iter().map(|e| e.body.as_str()).collect();
        assert_eq!(bodies[1], "m1 edited", "edit applied: {bodies:?}");
        assert_eq!(
            bodies[2], "[Messaggio eliminato]",
            "redacted row: {bodies:?}"
        );
        assert!(!bodies.contains(&"m2"), "redacted text does not reappear");
        assert_eq!(
            bodies.iter().filter(|b| b.starts_with("m")).count(),
            COUNT - 1,
            "no edit row, no duplicates"
        );
        assert_eq!(
            reactions.len(),
            1,
            "only the live reaction: {}",
            reactions.len()
        );
        assert_eq!(
            (reactions[0].key.as_str(), reactions[0].count),
            ("\u{1f44d}", 1)
        );
        assert_eq!(
            reactions[0].target, page.events[3].event_id,
            "reaction sits on m3"
        );
    });
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn older_messages_page_in_after_a_room_opened_from_the_saved_ones() {
    const TOTAL: usize = 90;
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let bob = matrix_sdk::Client::builder()
            .homeserver_url(&url)
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        bob.matrix_auth()
            .login_username(
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap_or_else(|| panic!("bob room"));
        for n in 0..TOTAL {
            bob_room
                .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                .await
                .unwrap();
        }

        // First run: Alice sees only the newest part of a long room.
        let store = AccountStore::create(&root, AccountId::new(296)).unwrap();
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
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.ok()?;
            alice.get_room(&room_id)
        })
        .await;
        invited.join().await.unwrap();
        // A sliding round saves only the room's newest event: make it a visible message, not Alice's own join.
        bob_room.send(RoomMessageEventContent::text_plain(format!("m{TOTAL}"))).await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        let room = alice.get_room(&room_id).unwrap();
        let (events, _drop) = room.event_cache().await.unwrap();
        eventually("some history in cache", || async {
            events.pagination().run_backwards_until(20).await.ok()?;
            (events.events().await.ok()?.len() >= 20).then_some(())
        })
        .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, session));

        // Restart online and open the room the way the app does.
        let store = AccountStore::open(&root, AccountId::new(296)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let client = store
            .client(&HomeserverUrl::parse(&url).unwrap(), PASSPHRASE, http())
            .await
            .unwrap();
        client.restore_session(sdk_session).await.unwrap();
        let scope = RoomScope {
            account_id: AccountId::new(296),
            room_id: RoomId::new(room_id.as_str()),
        };
        let timelines = crate::room_sync::Timelines::default();
        let (mut out, mut results) = crate::bounded(std::num::NonZeroUsize::new(4).unwrap());
        let mut views = ViewRequests::new();
        let request = crate::history::HistoryRequest {
            tag: views.start(scope.clone()).unwrap(),
            before: None,
            open: OPEN,
        };
        assert!(
            crate::room_sync::serve_history(
                &client,
                &timelines,
                AccountId::new(296),
                request,
                &mut out
            )
            .await
        );
        let saved = results.recv().await.unwrap().page.unwrap();
        let opened = tokio::time::timeout(Duration::from_secs(20), results.recv())
            .await
            .unwrap()
            .unwrap()
            .page
            .unwrap();
        let first_saved: usize = opened.events[0].body[1..].parse().unwrap();
        assert!(
            first_saved > 0,
            "the room was only partly saved: first row m{first_saved}"
        );
        assert!(
            saved.next.is_some() && opened.next.is_some(),
            "a cursor to older messages is kept"
        );

        // Scrolling up: a plain page before the cursor, through the same route as load_older.
        let request = crate::history::HistoryRequest {
            tag: views.start(scope).unwrap(),
            before: opened.next.clone(),
            open: false,
        };
        assert!(
            crate::room_sync::serve_history(
                &client,
                &timelines,
                AccountId::new(296),
                request,
                &mut out
            )
            .await
        );
        let older = results.recv().await.unwrap().page.unwrap();
        let last = older.events.last().expect("older rows").body.clone();
        assert_eq!(
            last,
            format!("m{}", first_saved - 1),
            "older page ends right before the saved list"
        );
        let ids: std::collections::BTreeSet<_> = older
            .events
            .iter()
            .chain(opened.events.iter())
            .map(|e| e.event_id.as_str().to_owned())
            .collect();
        assert_eq!(
            ids.len(),
            older.events.len() + opened.events.len(),
            "no duplicates across pages"
        );

        // Scrolled-back messages are saved: after a restart the first page still ends right at the
        // newest row and the cursor continues, with no duplicate, into what was scrolled back.
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((store, client));
        let store = AccountStore::open(&root, AccountId::new(296)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let client = store
            .client(&HomeserverUrl::parse(&url).unwrap(), PASSPHRASE, http())
            .await
            .unwrap();
        client.restore_session(sdk_session).await.unwrap();
        let scope = RoomScope {
            account_id: AccountId::new(296),
            room_id: RoomId::new(room_id.as_str()),
        };
        let timelines = crate::room_sync::Timelines::default();
        let request = crate::history::HistoryRequest {
            tag: views.start(scope.clone()).unwrap(),
            before: None,
            open: OPEN,
        };
        assert!(
            crate::room_sync::serve_history(
                &client,
                &timelines,
                AccountId::new(296),
                request,
                &mut out
            )
            .await
        );
        let _saved = results.recv().await.unwrap().page.unwrap();
        let reopened = tokio::time::timeout(Duration::from_secs(20), results.recv())
            .await
            .unwrap()
            .unwrap()
            .page
            .unwrap();
        assert_eq!(
            reopened.events.last().map(|e| e.body.clone()),
            opened.events.last().map(|e| e.body.clone()),
            "same newest row after the restart"
        );
        let request = crate::history::HistoryRequest {
            tag: views.start(scope).unwrap(),
            before: reopened.next.clone(),
            open: false,
        };
        assert!(
            crate::room_sync::serve_history(
                &client,
                &timelines,
                AccountId::new(296),
                request,
                &mut out
            )
            .await
        );
        let again = results.recv().await.unwrap().page.unwrap();
        let n = |body: &str| body[1..].parse::<usize>().unwrap();
        assert_eq!(
            n(&again.events.last().expect("older rows").body) + 1,
            n(&reopened.events[0].body),
            "the older page ends right before the reopened list"
        );
        drop((store, client));
    });
}

/// Room history as (visible messages, hidden join-rule changes) segments, oldest first; Alice reopens it offline.
/// Returns the bodies of the first page and the ms it took.
fn offline_first_page(id: u64, plan: &[(usize, usize)]) -> (Vec<String>, u128) {
    let visible: usize = plan.iter().map(|p| p.0).sum();
    let hidden: usize = plan.iter().map(|p| p.1).sum();
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let bob = matrix_sdk::Client::builder()
            .homeserver_url(&url)
            .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
            .build()
            .await
            .unwrap();
        bob.matrix_auth()
            .login_username(
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap_or_else(|| panic!("bob room"));
        let mut sent = 0;
        for (messages, topics) in plan {
            for _ in 0..*messages {
                bob_room
                    .send(RoomMessageEventContent::text_plain(format!("m{sent}")))
                    .await
                    .unwrap();
                sent += 1;
            }
            // join_rules never becomes a visible row (not a message, not a status row).
            for n in 0..*topics {
                let rule = if n % 2 == 0 { JoinRule::Public } else { JoinRule::Invite };
                bob_room
                    .send_state_event(RoomJoinRulesEventContent::new(rule))
                    .await
                    .unwrap();
            }
        }

        // First run: Alice joins, catches up, and the whole room reaches the cache on disk.
        let store = AccountStore::create(&root, AccountId::new(id)).unwrap();
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
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.ok()?;
            alice.get_room(&room_id)
        })
        .await;
        invited.join().await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        let room = alice.get_room(&room_id).unwrap();
        let (events, _drop) = room.event_cache().await.unwrap();
        eventually("visible messages in cache", || async {
            events.pagination().run_backwards_until(100).await.ok()?;
            let all = events.events().await.ok()?;
            let texts = all
                .iter()
                .filter(|e| {
                    e.raw()
                        .get_field::<String>("type")
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some("m.room.message")
                })
                .count();
            (texts >= visible && all.len() >= visible + hidden).then_some(())
        })
        .await;
        let cached = events.events().await.unwrap().len();
        println!("CACHED_EVENTS {cached}");
        assert!(cached >= visible + hidden, "chunks hold the hidden tail: {cached}");
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop((events, _drop, room, alice, invited, session));

        // Restart: same sealed store, homeserver nobody answers; open the room as the app does.
        let store = AccountStore::open(&root, AccountId::new(id)).unwrap();
        let value = store.unseal(PASSPHRASE).unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        let offline = store
            .client(
                &HomeserverUrl::parse("https://localhost:1/").unwrap(),
                PASSPHRASE,
                http(),
            )
            .await
            .unwrap();
        offline.restore_session(sdk_session).await.unwrap();
        let scope = RoomScope {
            account_id: AccountId::new(id),
            room_id: RoomId::new(room_id.as_str()),
        };
        let timelines = crate::room_sync::Timelines::default();
        let (mut out, mut results) = crate::bounded(std::num::NonZeroUsize::new(4).unwrap());
        let mut views = ViewRequests::new();
        let request = crate::history::HistoryRequest {
            tag: views.start(scope).unwrap(),
            before: None,
            open: OPEN,
        };
        let started = std::time::Instant::now();
        // The saved page must arrive without waiting for the unreachable server.
        let (_, saved) = tokio::join!(
            crate::room_sync::serve_history(&offline, &timelines, AccountId::new(id), request, &mut out),
            async {
                let first = tokio::time::timeout(Duration::from_secs(3), results.recv())
                    .await
                    .expect("no saved page within 3 s offline");
                println!("OFFLINE_FIRST_PAGE_MS {}", started.elapsed().as_millis());
                first.expect("a result").page.expect("saved page")
            }
        );
        let ids: std::collections::BTreeSet<_> =
            saved.events.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids.len(), saved.events.len(), "no duplicates");
        (
            // The join row may trail the messages; only the messages are asserted.
            saved.events.iter().filter(|e| !e.status).map(|e| e.body.clone()).collect(),
            started.elapsed().as_millis(),
        )
    })
}

fn assert_newest_page(plan: &[(usize, usize)], id: u64) {
    let visible: usize = plan.iter().map(|p| p.0).sum();
    let (bodies, ms) = offline_first_page(id, plan);
    println!("EDGE id={id} plan={plan:?} first_page_ms={ms} got={}", bodies.len());
    let want: Vec<String> = (visible.saturating_sub(crate::room_sync::PAGE)..visible)
        .map(|n| format!("m{n}"))
        .collect();
    assert!(
        bodies.ends_with(&want),
        "newest visible messages, in order, newest last: {bodies:?}"
    );
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn offline_room_whose_newest_chunk_holds_only_hidden_events_still_shows_the_last_page() {
    // 40 messages (more than one PAGE), 300 hidden (more than two event-cache chunks)
    assert_newest_page(&[(40, 300)], 297);
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn edge_offline_fewer_than_a_page_of_messages_under_hidden_events() {
    assert_newest_page(&[(5, 300)], 311);
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn edge_offline_messages_spread_over_several_chunks() {
    // Declared limit: stepping back stops at the first chunk with a message, so the first page
    // holds that chunk's 20 messages (fewer than a PAGE); older ones come by scrolling back.
    let (bodies, ms) = offline_first_page(312, &[(20, 150), (20, 150), (20, 150)]);
    println!("EDGE spread first_page_ms={ms} got={}", bodies.len());
    let want: Vec<String> = (40..60).map(|n| format!("m{n}")).collect();
    assert_eq!(bodies, want, "the newest chunk's messages, in order");
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn edge_offline_no_hidden_events_control() {
    assert_newest_page(&[(40, 0)], 313);
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn edge_offline_only_hidden_events_gives_an_empty_page_without_hanging() {
    let (bodies, ms) = offline_first_page(314, &[(0, 300)]);
    println!("EDGE only-hidden first_page_ms={ms} got={}", bodies.len());
    assert!(bodies.is_empty());
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn edge_offline_room_never_cached_is_empty_without_hanging() {
    let (bodies, ms) = offline_first_page(315, &[(0, 0)]);
    println!("EDGE empty first_page_ms={ms} got={}", bodies.len());
    assert!(bodies.is_empty());
}

/// Alice signed in with a sealed store and joined to three rooms of 30 messages, one sync done.
async fn seeded(id: u64) -> (AccountSession, Vec<matrix_sdk::ruma::OwnedRoomId>) {
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
    let bob = matrix_sdk::Client::builder()
        .homeserver_url(&url)
        .add_root_certificates(vec![reqwest::Certificate::from_pem(&ca).unwrap()])
        .build()
        .await
        .unwrap();
    bob.matrix_auth()
        .login_username(
            "@bob:matrix-desktop.test",
            config["bob_password"].as_str().unwrap(),
        )
        .send()
        .await
        .unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let mut request = CreateRoom::new();
        request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
        let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
        let bob_room = bob.get_room(&room_id).unwrap();
        for n in 0..30 {
            bob_room
                .send(RoomMessageEventContent::text_plain(format!("m{n}")))
                .await
                .unwrap();
        }
        ids.push(room_id);
    }
    let store = AccountStore::create(&root, AccountId::new(id)).unwrap();
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
    for room_id in &ids {
        let invited = eventually("invite", || async {
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.ok()?;
            alice.get_room(room_id)
        })
        .await;
        invited.join().await.unwrap();
    }
    crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
    (session, ids)
}

async fn warm_fixture(id: u64, interrupt: bool) -> (usize, Vec<usize>) {
    let (session, ids) = seeded(id).await;
    let alice = session.client.clone().unwrap();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let stop_now = std::sync::atomic::AtomicBool::new(interrupt);
    let warmed = crate::warm::warm(&alice, &stop, &stop_now).await;
    let mut saved = Vec::new();
    for room_id in &ids {
        saved.push(
            crate::room_sync::stored_events(&alice, room_id)
                .await
                .unwrap()
                .0
                .len(),
        );
    }
    drop(alice);
    (warmed, saved)
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn warm_up_saves_the_history_of_the_first_rooms() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (warmed, saved) = rt.block_on(warm_fixture(273, false));
    println!("WARM warmed {warmed} saved {saved:?}");
    assert_eq!(warmed, 3);
    assert!(
        saved.iter().all(|n| *n >= 20),
        "every room has saved messages: {saved:?}"
    );
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn warm_up_does_nothing_once_the_user_opened_another_room() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (warmed, saved) = rt.block_on(warm_fixture(274, true));
    println!("WARM warmed {warmed} saved {saved:?}");
    assert_eq!(warmed, 0);
    assert!(
        saved.iter().all(|n| *n <= 10),
        "only the sync window is saved: {saved:?}"
    );
}

fn probe_root(id: u64) -> std::path::PathBuf {
    std::path::PathBuf::from(env("CACHE_PROBE_ROOT")).join(format!("account-{id}"))
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn logout_removes_the_account_cache_and_keeps_the_profile_private() {
    use std::os::unix::fs::PermissionsExt;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let (mut session, ids) = seeded(277).await;
        let alice = session.client.clone().unwrap();
        crate::warm::warm(
            &alice,
            &std::sync::atomic::AtomicBool::new(false),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .await;
        assert!(
            crate::room_sync::stored_events(&alice, &ids[0])
                .await
                .unwrap()
                .0
                .len()
                >= 25
        );
        drop(alice);
        let dir = probe_root(277);
        let names = |d: &std::path::Path| {
            let mut n: Vec<String> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            n.sort();
            n
        };
        println!("BEFORE {:?}", names(&dir));
        assert!(names(&dir).len() > 2, "the cache files exist before logout");
        session.logout().await.unwrap();
        println!("AFTER {:?}", names(&dir));
        assert_eq!(
            names(&dir),
            vec![".lock".to_owned()],
            "nothing of the account cache is left"
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    });
}

#[test]
#[ignore = "requires the cached_history_probe script"]
fn cache_over_the_size_limit_is_pruned_and_a_small_one_is_kept() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let (session, ids) = seeded(278).await;
        let alice = session.client.clone().unwrap();
        crate::warm::warm(
            &alice,
            &std::sync::atomic::AtomicBool::new(false),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .await;
        let saved = crate::room_sync::stored_events(&alice, &ids[0])
            .await
            .unwrap()
            .0
            .len();
        assert!(saved >= 25);
        assert!(
            !crate::warm::prune(&alice, crate::warm::CACHE_LIMIT).await,
            "a small cache is kept"
        );
        assert_eq!(
            crate::room_sync::stored_events(&alice, &ids[0])
                .await
                .unwrap()
                .0
                .len(),
            saved
        );
        assert!(
            crate::warm::prune(&alice, 1).await,
            "a cache over the limit is cleared"
        );
        assert_eq!(
            crate::room_sync::stored_events(&alice, &ids[0])
                .await
                .map_or(0, |s| s.0.len()),
            0
        );
    });
}
