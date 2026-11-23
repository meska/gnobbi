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
        // As the engine does it: the index sits next to the store, encrypted with the store's passphrase.
        let index = path.with_file_name(format!("{}-search", path.file_name().unwrap().to_str().unwrap()));
        std::fs::create_dir_all(&index).unwrap();
        b = b.sqlite_store(path, Some(PASS)).search_index_store(matrix_engine::search_index::store_kind(&index, PASS));
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
        // The local index finds the start of a word (what is being typed), unlike the server.
        let part = search_until(&a, &enc_id, "cifra", |s| ids(s).len() == 2).await;
        report(&mut failed, "e2ee.prefix-matches-the-word-being-typed", ids(&part).len() == 2, String::new());

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

fn state_name(s: &MessageSearchState) -> &'static str {
    match s {
        MessageSearchState::Idle => "Idle",
        MessageSearchState::Searching { .. } => "Searching",
        MessageSearchState::Results { .. } => "Results",
        MessageSearchState::NoResults { .. } => "NoResults",
        MessageSearchState::Error { .. } => "Error",
    }
}

/// Edge cases on a real homeserver: unicode, special characters, empty and huge terms,
/// edited and deleted messages, paging in the encrypted (local) path. Positive controls prove the index is ready
/// before an absence is asserted. KNOWN FAILURES (open bugs, this test is the regression test for their fixes):
/// plain rooms on both servers: accent-insensitive («cafe» for «CAFÉ») and CJK terms miss; an edited message is
/// found by its OLD text and not by the new one; Synapse answers Error to terms with quotes/`;`/`--`/OR.
/// Encrypted rooms: results depend on what sync delivered (Tuwunel: a burst of 25 messages leaves only some in the
/// timeline: see e2ee.paged-fixtures-in-timeline), so the e2ee checks after it are only valid when that control passes.
#[test]
#[ignore = "requires the timeline_probe script"]
fn message_search_edge_scenarios() {
    use matrix_engine::sdk::ruma::events::room::message::ReplacementMetadata;
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let store = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("stores/alice-edge");
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
        let (ra, _rb) = (get(&a, &plain_id).await, get(&b, &plain_id).await);
        let (ea, _eb) = (get(&a, &enc_id).await, get(&b, &enc_id).await);
        let tl = ea.timeline().await.unwrap();

        // Same fixtures in both rooms; `tag` keeps terms unique per room kind.
        for (label, room, id) in [("plain", &ra, &plain_id), ("e2ee", &ea, &enc_id)] {
            let mut texts = Vec::new();
            for (k, body) in [
                ("accent", "Perché è già CAFÉ nel bar"),
                ("emoji", "🎉 festa grande 🎉"),
                ("cjk", "你好世界 ciao cjk"),
                ("special", "sconto 100% sicuro_ok a*b (c) \\ \"q\" 'x'; -- fine"),
                ("longline", "riga uno\nriga due lunga\tcon tab"),
            ] {
                texts.push((k, send(room, body).await.to_string()));
            }
            // A control sent last: once it is found, everything before it is indexed.
            let ctrl_body = format!("controllo{label}finale");
            let ctrl = send(room, &ctrl_body).await;
            if label == "e2ee" {
                until("e2ee fixtures decrypted", 60, async || {
                    let n = tl.items().await.iter().filter_map(|i| i.as_event()).filter(|e| e.content().as_message().is_some_and(|m| m.body().contains("controllo") || m.body().contains("grande") || m.body().contains("你好"))).count();
                    (n >= 3).then_some(())
                })
                .await;
            }
            let c = search_until(&a, id, &ctrl_body, |s| ids(s) == vec![ctrl.to_string()]).await;
            report(&mut failed, &format!("{label}.edge-control-indexed"), ids(&c) == vec![ctrl.to_string()], state_name(&c).into());
            let id_of = |k: &str| texts.iter().find(|(n, _)| *n == k).unwrap().1.clone();
            // One search each: the control above proved the index is ready, retries would only slow every real miss.
            let found = async |term: &str, want: &str| {
                let mut svc = MessageSearch::for_room(a.clone(), id.clone());
                let s = settled(&mut svc, term).await;
                (ids(&s).contains(&want.to_owned()), state_name(&s))
            };
            // Case and accents are ignored (the contract of message_match); the server may differ and that is a finding.
            for (name, term, key) in [
                ("upper-without-accent", "CAFE", "accent"),
                ("lower-with-accent", "perché", "accent"),
                ("unaccented-query-for-accented-text", "perche", "accent"),
                ("emoji-word", "festa", "emoji"),
                ("emoji-phrase", "festa grande", "emoji"),
                ("cjk-word", "你好", "cjk"),
                ("cjk-tail", "ciao cjk", "cjk"),
                ("percent", "100%", "special"),
                ("underscore", "sicuro_ok", "special"),
                ("star", "a*b", "special"),
                ("backslash-in-text", "c) \\", "special"),
                ("double-quote", "\"q\"", "special"),
                ("single-quote-sql", "'x'; --", "special"),
                ("multiline-word", "lunga", "longline"),
            ] {
                let (ok, st) = found(term, &id_of(key)).await;
                report(&mut failed, &format!("{label}.term-{name}"), ok, format!("term={term:?} state={st}"));
            }
            // Hostile terms must never break the service: any settled state but Error/Searching-forever.
            for (name, term) in [("sql-drop", "'; DROP TABLE events; --"), ("regex-dotstar", ".*"), ("percent-only", "%%"), ("only-spaces", "    "), ("empty", ""), ("one-emoji", "🎉"), ("huge", &"x".repeat(5000)), ("fts-operators", "a OR b -c \"d")] {
                let mut s = MessageSearch::for_room(a.clone(), id.clone());
                let st = settled(&mut s, term).await;
                let sane = !matches!(st, MessageSearchState::Error { .. });
                report(&mut failed, &format!("{label}.hostile-{name}"), sane, format!("state={}", state_name(&st)));
            }
            let mut s = MessageSearch::for_room(a.clone(), id.clone());
            let st = settled(&mut s, "    ").await;
            report(&mut failed, &format!("{label}.blank-is-idle"), st == MessageSearchState::Idle, format!("state={}", state_name(&st)));
            let st = settled(&mut s, "").await;
            report(&mut failed, &format!("{label}.empty-is-idle"), st == MessageSearchState::Idle, format!("state={}", state_name(&st)));

            // Edited: the new text is found (hit = the original event), the old text no longer is.
            let orig = send(room, &format!("originale{label}gamma")).await;
            let edit = plain(&format!("modificato{label}delta")).make_replacement(ReplacementMetadata::new(orig.clone(), None));
            room.send(edit).await.unwrap();
            let new_term = format!("modificato{label}delta");
            let st = search_until(&a, id, &new_term, |s| !hits(s).is_empty()).await;
            let h = hits(&st).first();
            report(&mut failed, &format!("{label}.edited-new-text-found-on-original"), h.is_some_and(|h| h.event_id == orig.as_str()), format!("state={} n={}", state_name(&st), hits(&st).len()));
            report(&mut failed, &format!("{label}.edited-excerpt-is-new-text"), h.is_some_and(|h| h.excerpt.contains(&new_term) && !h.excerpt.starts_with("* ")), format!("excerpt={:?}", h.map(|h| h.excerpt.clone())));
            let old = search_until(&a, id, &format!("originale{label}gamma"), |s| matches!(s, MessageSearchState::NoResults { .. })).await;
            report(&mut failed, &format!("{label}.edited-old-text-gone"), matches!(old, MessageSearchState::NoResults { .. }), format!("state={} n={}", state_name(&old), hits(&old).len()));
            // Deleted after being found.
            let doomed = send(room, &format!("effimero{label}omega")).await;
            let seen = search_until(&a, id, &format!("effimero{label}omega"), |s| ids(s) == vec![doomed.to_string()]).await;
            report(&mut failed, &format!("{label}.deleted-was-found-before"), ids(&seen) == vec![doomed.to_string()], state_name(&seen).into());
            room.redact(&doomed, None, None).await.unwrap();
            let gone = search_until(&a, id, &format!("effimero{label}omega"), |s| matches!(s, MessageSearchState::NoResults { .. })).await;
            report(&mut failed, &format!("{label}.deleted-gone"), matches!(gone, MessageSearchState::NoResults { .. }), format!("state={} n={}", state_name(&gone), hits(&gone).len()));

            // Paging: 25 more with a common word. Plain goes through the server, e2ee through the cache.
            let mut sent = Vec::new();
            for i in 1..=25 {
                sent.push(send(room, &format!("pagina{label} n{i}")).await.to_string());
            }
            sent.reverse();
            let term = format!("pagina{label}");
            let first = search_until(&a, id, &term, |s| hits(s).len() == PAGE).await;
            if label == "e2ee" {
                // The search itself must complete the cache (sync leaves a gap on Tuwunel): all 25 are in the timeline afterwards and the result is not marked partial.
                let seen = until("e2ee page fixtures", 60, async || {
                    let n = tl.items().await.iter().filter_map(|i| i.as_event()).filter(|e| e.content().as_message().is_some_and(|m| m.body().contains(&term))).count();
                    (n >= 25).then_some(n)
                })
                .await;
                report(&mut failed, "e2ee.paged-fixtures-in-timeline", seen == Some(25), format!("{seen:?}"));
                report(&mut failed, "e2ee.complete-cache-not-partial", matches!(first, MessageSearchState::Results { partial: false, .. }), String::new());
            }
            report(&mut failed, &format!("{label}.paged-first-page"), hits(&first).len() == PAGE && matches!(first, MessageSearchState::Results { more: MoreHits::Available, .. }), format!("n={}", hits(&first).len()));
            let mut paged = MessageSearch::for_room(a.clone(), id.clone());
            let mut rx = paged.subscribe();
            settled(&mut paged, &term).await;
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
            report(&mut failed, &format!("{label}.paged-rest-in-order-no-dupes"), all_ids == sent, format!("n={} unique={}", all_ids.len(), all_ids.iter().collect::<std::collections::HashSet<_>>().len()));
        }
        let _ = sa.stop().await;
        let _ = sb.stop().await;
    });
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}
