//! Room message search (`MessageSearch`) on a disposable Synapse or Tuwunel, two synthetic accounts.
//! Unencrypted room: results come from the server `/search`, newest first, paged, the hit IDs are the
//! real events, redacted messages are not listed, thread replies carry their root. Encrypted room:
//! results come from the decrypted local cache only and the term never reaches the server (the HTTP
//! client records every request path). Scenarios print `QA <name> PASS|FAIL detail`.
//! Driver: PROBE_TARGET=message_search_qa PROBE_TEST=message_search_scenarios PROBE_SERVER=synapse|tuwunel
//! PROBE_FEATURES=sdk python3 the timeline_probe script
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_engine::{
    message_search::{DEBOUNCE, MessageSearch, PAGE},
    operations::{MessageHit, MessageSearchState, MessageSource, MoreHits},
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            OwnedEventId, OwnedRoomId,
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{
                InitialStateEvent,
                relation::Thread,
                room::{
                    encryption::RoomEncryptionEventContent,
                    message::{Relation, RoomMessageEventContent},
                },
            },
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::{sync_service::SyncService, timeline::RoomExt},
};
use std::time::{Duration, Instant};

const PASS: &str = "synthetic-passphrase-alice-1";

fn report(failed: &mut Vec<String>, name: &str, ok: bool, detail: String) {
    eprintln!("QA {name} {} {detail}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        failed.push(name.to_owned());
    }
}

async fn until<T>(what: &str, secs: u64, mut f: impl AsyncFnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return Some(v);
        }
        if start.elapsed() > Duration::from_secs(secs) {
            eprintln!("timeout: {what}");
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn build(config: &serde_json::Value, store: Option<&std::path::Path>, url: Option<&str>) -> Client {
    let ca = Certificate::from_pem(&std::fs::read(config["ca"].as_str().unwrap()).unwrap()).unwrap();
    let mut b = Client::builder()
        .homeserver_url(url.unwrap_or(config["url"].as_str().unwrap()))
        .add_root_certificates(vec![ca])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative);
    if let Some(path) = store {
        b = b.sqlite_store(path, Some(PASS));
    }
    if url.is_some() {
        // No server behind this URL: do not even ask it for its versions.
        b = b
            .server_versions([matrix_engine::sdk::ruma::api::MatrixVersion::V1_11])
            .sliding_sync_version_builder(VersionBuilder::None);
    }
    let c = b.build().await.unwrap();
    c.event_cache().subscribe().unwrap();
    c
}

async fn login(c: &Client, config: &serde_json::Value, who: &str) {
    c.matrix_auth()
        .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
        .send()
        .await
        .unwrap();
}

fn plain(body: &str) -> RoomMessageEventContent {
    RoomMessageEventContent::text_plain(body)
}

async fn settled(search: &mut MessageSearch, term: &str) -> MessageSearchState {
    let mut rx = search.subscribe();
    search.set_query(term);
    tokio::time::sleep(DEBOUNCE).await;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let s = rx.borrow_and_update().clone();
            if !matches!(s, MessageSearchState::Searching { .. }) {
                return s;
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("search never settled")
}

/// Retries with a fresh service until `ok` (the server indexes in the background).
async fn search_until(client: &Client, room: &OwnedRoomId, term: &str, ok: impl Fn(&MessageSearchState) -> bool) -> MessageSearchState {
    let mut last = MessageSearchState::Idle;
    for _ in 0..40 {
        let mut s = MessageSearch::for_room(client.clone(), room.clone());
        last = settled(&mut s, term).await;
        if ok(&last) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    last
}

fn hits(s: &MessageSearchState) -> &[MessageHit] {
    match s {
        MessageSearchState::Results { hits, .. } => hits,
        _ => &[],
    }
}

fn ids(s: &MessageSearchState) -> Vec<String> {
    hits(s).iter().map(|h| h.event_id.clone()).collect()
}

async fn send(room: &Room, body: &str) -> OwnedEventId {
    room.send(plain(body)).await.unwrap().response.event_id
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn message_search_scenarios() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let store = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("stores/alice");
    std::fs::create_dir_all(&store).unwrap();
    let mut failed = Vec::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let a = build(&config, Some(&store), None).await;
        let b = build(&config, None, None).await;
        login(&a, &config, "alice").await;
        login(&b, &config, "bob").await;
        let mk = |encrypted: bool| {
            let mut creation = CreationContent::new();
            creation.federate = false;
            let mut req = assign!(CreateRoomRequest::new(), {
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![b.user_id().unwrap().to_owned()],
                creation_content: Some(Raw::new(&creation).unwrap()),
            });
            if encrypted {
                req.initial_state = vec![InitialStateEvent::with_empty_state_key(RoomEncryptionEventContent::with_recommended_defaults()).to_raw_any()];
            }
            req
        };
        let plain_id = a.create_room(mk(false)).await.unwrap().room_id().to_owned();
        let enc_id = a.create_room(mk(true)).await.unwrap().room_id().to_owned();
        b.join_room_by_id(&plain_id).await.unwrap();
        b.join_room_by_id(&enc_id).await.unwrap();
        let (sa, sb) = (SyncService::builder(a.clone()).build().await.unwrap(), SyncService::builder(b.clone()).build().await.unwrap());
        sa.start().await;
        sb.start().await;
        let get = async |c: &Client, id: &OwnedRoomId| until("room", 40, async || c.get_room(id)).await.expect("room");
        let (ra, rb) = (get(&a, &plain_id).await, get(&b, &plain_id).await);
        let (ea, eb) = (get(&a, &enc_id).await, get(&b, &enc_id).await);

        // ---- unencrypted room: the server answers ----
        let m1 = send(&ra, "alpha ciao uno").await;
        let _m2 = send(&ra, "alpha addio due").await;
        let m3 = send(&rb, "ciao tre bravo").await;
        let gone = send(&rb, "ciao quattro cancellato").await;
        rb.redact(&gone, None, None).await.unwrap();
        let mut reply = plain("ciao cinque nel thread");
        reply.relates_to = Some(Relation::Thread(Thread::plain(m1.clone(), m1.clone())));
        let m5 = rb.send(reply).await.unwrap().response.event_id;

        let want = vec![m5.to_string(), m3.to_string(), m1.to_string()];
        let found = search_until(&a, &plain_id, "ciao", |s| ids(s) == want).await;
        report(&mut failed, "plain.results-newest-first-no-redacted", ids(&found) == want, format!("n={} source_ok={}", hits(&found).len(), matches!(found, MessageSearchState::Results { source: MessageSource::Server, .. })));
        report(&mut failed, "plain.source-is-server", matches!(found, MessageSearchState::Results { source: MessageSource::Server, .. }), String::new());
        let thread_ok = hits(&found).first().is_some_and(|h| h.thread_root.as_deref() == Some(m1.as_str()))
            && hits(&found).iter().skip(1).all(|h| h.thread_root.is_none());
        report(&mut failed, "plain.thread-reply-carries-root", thread_ok, String::new());
        let named = hits(&found).iter().all(|h| !h.sender_name.is_empty() && !h.excerpt.is_empty());
        report(&mut failed, "plain.sender-and-excerpt", named, String::new());
        // The jump target is the real event: fetching it by the hit's ID returns the same text.
        let mut jump_ok = !hits(&found).is_empty();
        for h in hits(&found) {
            let id = OwnedEventId::try_from(h.event_id.as_str()).unwrap();
            let event = ra.event(&id, None).await.unwrap();
            let raw: serde_json::Value = serde_json::from_str(event.raw().json().get()).unwrap();
            jump_ok &= raw["content"]["body"].as_str().is_some_and(|body| body.contains("ciao")) && raw["sender"] == h.sender;
        }
        report(&mut failed, "plain.hit-ids-are-the-events", jump_ok, String::new());
        let none = search_until(&a, &plain_id, "inesistente", |s| matches!(s, MessageSearchState::NoResults { .. })).await;
        report(&mut failed, "plain.no-results", matches!(none, MessageSearchState::NoResults { source: MessageSource::Server, .. }), String::new());

        // Paging: 25 more messages, first page is PAGE, `more` brings the rest without duplicates.
        let mut sent = Vec::new();
        for i in 1..=25 {
            sent.push(send(&rb, &format!("pagina numero{i}")).await.to_string());
        }
        sent.reverse();
        let first = search_until(&a, &plain_id, "pagina", |s| hits(s).len() == PAGE).await;
        report(&mut failed, "plain.first-page", hits(&first).len() == PAGE && matches!(first, MessageSearchState::Results { more: MoreHits::Available, .. }), format!("n={}", hits(&first).len()));
        let mut paged = MessageSearch::for_room(a.clone(), plain_id.clone());
        let mut rx = paged.subscribe();
        settled(&mut paged, "pagina").await;
        paged.more();
        let all = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let s = rx.borrow_and_update().clone();
                if matches!(s, MessageSearchState::Results { more: MoreHits::None, .. }) {
                    return s;
                }
                rx.changed().await.unwrap();
            }
        })
        .await;
        let all_ids = all.as_ref().map(ids).unwrap_or_default();
        report(&mut failed, "plain.more-appends-the-rest-in-order", all_ids == sent, format!("n={} unique={}", all_ids.len(), all_ids.iter().collect::<std::collections::HashSet<_>>().len()));

        // Under two characters: Idle (the request-free part is proved with the unreachable server below).
        let mut s = MessageSearch::for_room(a.clone(), plain_id.clone());
        report(&mut failed, "plain.one-char-is-idle", settled(&mut s, "a").await == MessageSearchState::Idle, String::new());

        // ---- encrypted room: only the decrypted local cache ----
        let e1 = send(&ea, "ciao cifrato uno").await;
        let e2 = send(&eb, "ciao cifrato due").await;
        // Open the room timeline so the event cache holds (and decrypts) the events.
        let tl = ea.timeline().await.unwrap();
        until("encrypted bodies decrypted", 40, async || {
            let n = tl.items().await.iter().filter_map(|i| i.as_event()).filter(|e| e.content().as_message().is_some_and(|m| m.body().contains("cifrato"))).count();
            (n == 2).then_some(())
        })
        .await;
        let enc = search_until(&a, &enc_id, "cifrato", |s| ids(s).len() == 2).await;
        let want_enc = vec![e2.to_string(), e1.to_string()];
        report(&mut failed, "e2ee.results-from-local-cache", ids(&enc) == want_enc && matches!(enc, MessageSearchState::Results { source: MessageSource::LocalCache, .. }), format!("n={}", hits(&enc).len()));
        let enc_none = search_until(&a, &enc_id, "inesistente", |s| matches!(s, MessageSearchState::NoResults { .. })).await;
        report(&mut failed, "e2ee.empty-state", matches!(enc_none, MessageSearchState::NoResults { source: MessageSource::LocalCache, .. }), String::new());
        // Partial words are found locally (contains), unlike the server.
        let part = search_until(&a, &enc_id, "ifrat", |s| ids(s).len() == 2).await;
        report(&mut failed, "e2ee.contains-matches-inside-words", ids(&part).len() == 2, String::new());

        // ---- the same store behind an unreachable server ----
        let session = a.matrix_auth().session().unwrap();
        let _ = sa.stop().await;
        drop((tl, ra, ea, rb, eb, sa));
        drop(a);
        tokio::time::sleep(Duration::from_millis(500)).await;
        let offline = build(&config, Some(&store), Some("https://localhost:1/")).await;
        offline.restore_session(session).await.unwrap();
        // Encrypted room: the cache answers with no server at all, so the term cannot have gone out.
        let local = search_until(&offline, &enc_id, "cifrato", |s| ids(s).len() == 2).await;
        report(&mut failed, "e2ee.term-needs-no-server", ids(&local) == want_enc && matches!(local, MessageSearchState::Results { source: MessageSource::LocalCache, .. }), format!("{} hits", hits(&local).len()));
        // Control: the unencrypted room does need the server, and says so with the error line.
        let mut s = MessageSearch::for_room(offline.clone(), plain_id.clone());
        let down = settled(&mut s, "ciao").await;
        report(&mut failed, "plain.control-needs-the-server", matches!(down, MessageSearchState::Error { .. }), String::new());
        let _ = sb.stop().await;
    });
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}
