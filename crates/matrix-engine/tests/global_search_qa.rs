#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
//! Message search of ALL rooms (`MessageSearch::everywhere`, the room list's «Messaggi» section) on a
//! disposable Synapse, two synthetic accounts, each with its own encrypted index directory next to its
//! store. Scenarios print `QA <name> PASS|FAIL detail`; `MEASURE` lines carry the numbers of the volume run.
//! Driver: PROBE_TARGET=global_search_qa PROBE_TEST=global_search_scenarios|global_search_volume
//! PROBE_SERVER=synapse PROBE_FEATURES=sdk python3 tools/timeline_probe.py
use matrix_engine::{
    message_search::{DEBOUNCE, MessageSearch},
    operations::{MessageHit, MessageSearchState, MessageSource},
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            OwnedEventId, OwnedRoomId,
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{
                InitialStateEvent,
                room::{
                    encryption::RoomEncryptionEventContent,
                    message::{ReplacementMetadata, RoomMessageEventContent},
                },
            },
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::{sync_service::SyncService, timeline::RoomExt},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

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

fn index_dir(store: &Path) -> PathBuf {
    store.with_file_name(format!("{}-search", store.file_name().unwrap().to_str().unwrap()))
}

/// A client as the engine builds it: sqlite store, and the index next to it, encrypted with the same passphrase.
async fn build(config: &serde_json::Value, store: &Path, url: Option<&str>) -> Client {
    let ca = Certificate::from_pem(&std::fs::read(config["ca"].as_str().unwrap()).unwrap()).unwrap();
    std::fs::create_dir_all(store).unwrap();
    std::fs::create_dir_all(index_dir(store)).unwrap();
    let mut b = Client::builder()
        .homeserver_url(url.unwrap_or(config["url"].as_str().unwrap()))
        .add_root_certificates(vec![ca])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .sqlite_store(store, Some(PASS))
        .search_index_store(matrix_engine::search_index::store_kind(&index_dir(store), PASS));
    if url.is_some() {
        b = b.server_versions([matrix_engine::sdk::ruma::api::MatrixVersion::V1_11]).sliding_sync_version_builder(VersionBuilder::None);
    }
    let c = b.build().await.unwrap();
    c.event_cache().subscribe().unwrap();
    c
}

async fn login(c: &Client, config: &serde_json::Value, who: &str) {
    c.matrix_auth().login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap()).send().await.unwrap();
}

fn plain(body: &str) -> RoomMessageEventContent {
    RoomMessageEventContent::text_plain(body)
}

async fn send(room: &Room, body: &str) -> OwnedEventId {
    room.send(plain(body)).await.unwrap().response.event_id
}

fn room_request(encrypted: bool, invite: Option<&Client>) -> CreateRoomRequest {
    let mut creation = CreationContent::new();
    creation.federate = false;
    let mut req = assign!(CreateRoomRequest::new(), {
        preset: Some(RoomPreset::PrivateChat),
        invite: invite.map(|c| vec![c.user_id().unwrap().to_owned()]).unwrap_or_default(),
        creation_content: Some(Raw::new(&creation).unwrap()),
    });
    if encrypted {
        req.initial_state = vec![InitialStateEvent::with_empty_state_key(RoomEncryptionEventContent::with_recommended_defaults()).to_raw_any()];
    }
    req
}

/// A fresh service per try, until `ok` (sync and indexing are asynchronous).
async fn global_until(client: &Client, term: &str, ok: impl Fn(&MessageSearchState) -> bool) -> MessageSearchState {
    let mut last = MessageSearchState::Idle;
    for _ in 0..60 {
        let mut s = MessageSearch::everywhere(client.clone());
        let mut rx = s.subscribe();
        s.set_query(term);
        tokio::time::sleep(DEBOUNCE).await;
        last = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let state = rx.borrow_and_update().clone();
                if !matches!(state, MessageSearchState::Searching { .. }) {
                    return state;
                }
                rx.changed().await.unwrap();
            }
        })
        .await
        .expect("search never settled");
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

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn bytes_under(dir: &Path) -> u64 {
    files_under(dir).iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn global_search_scenarios() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let (store_a, store_b) = (home.join("stores/alice-global"), home.join("stores/bob-global"));
    let mut failed = Vec::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let a = build(&config, &store_a, None).await;
        let b = build(&config, &store_b, None).await;
        login(&a, &config, "alice").await;
        login(&b, &config, "bob").await;
        let plain_id = a.create_room(room_request(false, Some(&b))).await.unwrap().room_id().to_owned();
        let enc_id = a.create_room(room_request(true, Some(&b))).await.unwrap().room_id().to_owned();
        // A room of bob alone: alice is not in it.
        let secret_id = b.create_room(room_request(false, None)).await.unwrap().room_id().to_owned();
        b.join_room_by_id(&plain_id).await.unwrap();
        b.join_room_by_id(&enc_id).await.unwrap();
        let (sa, sb) = (SyncService::builder(a.clone()).build().await.unwrap(), SyncService::builder(b.clone()).build().await.unwrap());
        sa.start().await;
        sb.start().await;
        let get = async |c: &Client, id: &OwnedRoomId| until("room", 60, async || c.get_room(id)).await.expect("room");
        let (pa, pb) = (get(&a, &plain_id).await, get(&b, &plain_id).await);
        let (ea, eb) = (get(&a, &enc_id).await, get(&b, &enc_id).await);
        let sb_room = get(&b, &secret_id).await;
        // Timelines keep the rooms' events decrypted in the cache, as the open app does.
        let (tl_pa, tl_ea) = (pa.timeline().await.unwrap(), ea.timeline().await.unwrap());
        let tl_eb = eb.timeline().await.unwrap();

        // ---- send, receive, find: unencrypted and encrypted room, from both rooms at once ----
        let p1 = send(&pa, "ciao pianeta uno").await;
        let e1 = send(&ea, "ciao cifrato due").await;
        let p2 = send(&pb, "ciao mondo tre").await;
        let e2 = send(&eb, "ciao cifrato quattro").await;
        until("encrypted bodies decrypted", 60, async || {
            let n = tl_ea.items().await.iter().filter_map(|i| i.as_event()).filter(|e| e.content().as_message().is_some_and(|m| m.body().contains("cifrato"))).count();
            (n == 2).then_some(())
        })
        .await;
        let want = [p1.to_string(), e1.to_string(), p2.to_string(), e2.to_string()];
        let all = global_until(&a, "ciao", |s| ids(s).len() == 4).await;
        let got: std::collections::HashSet<_> = ids(&all).into_iter().collect();
        report(&mut failed, "global.finds-both-kinds-of-room", got == want.iter().cloned().collect() && matches!(all, MessageSearchState::Results { source: MessageSource::LocalCache, .. }), format!("n={}", got.len()));
        let rooms: std::collections::HashSet<_> = hits(&all).iter().map(|h| h.room_id.clone()).collect();
        report(&mut failed, "global.hits-name-their-room", rooms == [plain_id.to_string(), enc_id.to_string()].into_iter().collect(), format!("{} rooms", rooms.len()));
        report(&mut failed, "global.newest-first", hits(&all).windows(2).all(|w| w[0].ts >= w[1].ts), String::new());
        report(&mut failed, "global.sender-and-excerpt", hits(&all).iter().all(|h| !h.sender_name.is_empty() && h.excerpt.contains("ciao") && !h.matches.is_empty()), String::new());
        report(&mut failed, "global.sender-name-not-raw-id", hits(&all).iter().any(|h| !h.sender_name.starts_with('@')), format!("{:?}", hits(&all).iter().map(|h| h.sender_name.clone()).collect::<Vec<_>>()));

        // ---- the word being typed, accents and case ----
        let city = send(&pa, "Una città bellissima").await;
        let by_prefix = global_until(&a, "bellis", |s| ids(s) == [city.to_string()]).await;
        report(&mut failed, "global.prefix-of-the-last-word", ids(&by_prefix) == [city.to_string()], String::new());
        let by_accent = global_until(&a, "CITTA bell", |s| ids(s) == [city.to_string()]).await;
        report(&mut failed, "global.accent-case-and-two-words", ids(&by_accent) == [city.to_string()], String::new());
        let odd = global_until(&a, "\"(a:b* -x", |s| !matches!(s, MessageSearchState::Searching { .. })).await;
        report(&mut failed, "global.query-syntax-is-just-text", matches!(odd, MessageSearchState::NoResults { .. }), String::new());
        let none = global_until(&a, "inesistente", |s| matches!(s, MessageSearchState::NoResults { .. })).await;
        report(&mut failed, "global.no-results", matches!(none, MessageSearchState::NoResults { .. }), String::new());
        let short = global_until(&a, "c", |_| true).await;
        report(&mut failed, "global.one-char-is-idle", short == MessageSearchState::Idle, String::new());

        // ---- edit: the latest version is what is found ----
        let edit = plain("salve rinnovato quattro").make_replacement(ReplacementMetadata::new(e2.clone(), None));
        eb.send(edit).await.unwrap();
        let renewed = global_until(&a, "rinnovato", |s| ids(s) == [e2.to_string()]).await;
        report(&mut failed, "global.edited-message-found-by-new-text", ids(&renewed) == [e2.to_string()] && hits(&renewed)[0].excerpt.contains("rinnovato") && !hits(&renewed)[0].excerpt.starts_with("* "), format!("{:?}", hits(&renewed).iter().map(|h| h.excerpt.clone()).collect::<Vec<_>>()));
        let old = global_until(&a, "cifrato", |s| ids(s) == [e1.to_string()]).await;
        report(&mut failed, "global.edited-message-not-found-by-old-text", ids(&old) == [e1.to_string()], format!("n={}", hits(&old).len()));

        // ---- redaction: the message leaves the results ----
        pb.redact(&p2, None, None).await.unwrap();
        let after = global_until(&a, "mondo", |s| matches!(s, MessageSearchState::NoResults { .. })).await;
        report(&mut failed, "global.redacted-message-disappears", matches!(after, MessageSearchState::NoResults { .. }), String::new());
        let still = global_until(&a, "pianeta", |s| ids(s) == [p1.to_string()]).await;
        report(&mut failed, "global.others-stay-after-a-redaction", ids(&still) == [p1.to_string()], String::new());

        // ---- the search of one room is the same index ----
        let mut one = MessageSearch::for_room(a.clone(), enc_id.clone());
        let mut rx = one.subscribe();
        one.set_query("rinnovato");
        tokio::time::sleep(DEBOUNCE).await;
        let state = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let s = rx.borrow_and_update().clone();
                if !matches!(s, MessageSearchState::Searching { .. }) {
                    return s;
                }
                rx.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        report(&mut failed, "room.encrypted-room-uses-the-index", ids(&state) == [e2.to_string()] && matches!(state, MessageSearchState::Results { source: MessageSource::LocalCache, .. }) && hits(&state)[0].room_id == enc_id.as_str(), String::new());

        // ---- encrypted at rest: no word of any message in any index file, a key file beside them ----
        let dir = index_dir(&store_a);
        let files = files_under(&dir);
        let leaked: Vec<_> = ["pianeta", "cifrato", "rinnovato", "bellissima", "città"].iter().filter(|w| files.iter().any(|f| contains(&std::fs::read(f).unwrap_or_default(), w.as_bytes()))).collect();
        let keyed = files.iter().filter(|f| f.file_name().is_some_and(|n| n == "seshat-index.key")).count();
        report(&mut failed, "index.no-message-word-in-clear", leaked.is_empty() && !files.is_empty(), format!("files={} leaked={leaked:?}", files.len()));
        report(&mut failed, "index.one-key-file-per-room", keyed >= 2, format!("keys={keyed}"));
        // Positive control: tantivy's meta.json is plain JSON when written in the clear; here it does not even parse.
        let metas: Vec<_> = files.iter().filter(|f| f.file_name().is_some_and(|n| n == "meta.json")).collect();
        report(&mut failed, "index.meta-json-is-not-readable-json", !metas.is_empty() && metas.iter().all(|f| serde_json::from_slice::<serde_json::Value>(&std::fs::read(f).unwrap()).is_err()), format!("meta files={}", metas.len()));

        // ---- two accounts: separate directories, separate results ----
        let bob_secret = send(&sb_room, "segretobob solo mio").await;
        let mine = global_until(&b, "segretobob", |s| ids(s) == [bob_secret.to_string()]).await;
        report(&mut failed, "accounts.bob-finds-his-private-room", ids(&mine) == [bob_secret.to_string()] && hits(&mine)[0].room_id == secret_id.as_str(), String::new());
        let theirs = global_until(&a, "segretobob", |_| true).await;
        report(&mut failed, "accounts.alice-does-not-see-it", matches!(theirs, MessageSearchState::NoResults { .. }), state_name(&theirs).into());
        let (da, db) = (index_dir(&store_a), index_dir(&store_b));
        let room_dirs = |d: &Path| -> std::collections::BTreeSet<String> { std::fs::read_dir(d).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect() };
        report(&mut failed, "accounts.own-index-directories", da != db && room_dirs(&da).contains(plain_id.as_str()) && room_dirs(&db).contains(secret_id.as_str()) && !room_dirs(&da).contains(secret_id.as_str()), format!("alice={} bob={}", room_dirs(&da).len(), room_dirs(&db).len()));

        // ---- after a restart and with no server at all the same results come from the saved index ----
        let session = a.matrix_auth().session().unwrap();
        let _ = sa.stop().await;
        drop((tl_pa, tl_ea, tl_eb, pa, ea, one));
        drop(a);
        tokio::time::sleep(Duration::from_millis(500)).await;
        let offline = build(&config, &store_a, Some("https://localhost:1/")).await;
        offline.restore_session(session).await.unwrap();
        let again = global_until(&offline, "pianeta", |s| ids(s) == [p1.to_string()]).await;
        report(&mut failed, "restart.found-with-no-server", ids(&again) == [p1.to_string()], state_name(&again).into());
        let again_enc = global_until(&offline, "rinnovato", |s| ids(s) == [e2.to_string()]).await;
        report(&mut failed, "restart.encrypted-found-with-no-server", ids(&again_enc) == [e2.to_string()], state_name(&again_enc).into());
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

/// History the app has not seen: old messages of a room the account joins later are found after the backfill pass.
#[test]
#[ignore = "requires the timeline_probe script"]
fn global_search_backfill() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let (store_a, store_b) = (home.join("stores/alice-backfill"), home.join("stores/bob-backfill"));
    let mut failed = Vec::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let a = build(&config, &store_a, None).await;
        let b = build(&config, &store_b, None).await;
        login(&a, &config, "alice").await;
        login(&b, &config, "bob").await;
        let room_id = b.create_room(room_request(false, None)).await.unwrap().room_id().to_owned();
        let sb = SyncService::builder(b.clone()).build().await.unwrap();
        sb.start().await;
        let room_b = until("room", 60, async || b.get_room(&room_id)).await.expect("room");
        let old = send(&room_b, "arcaico messaggio antico").await;
        for i in 0..60 {
            send(&room_b, &format!("riempitivo numero{i}")).await;
        }
        room_b.invite_user_by_id(a.user_id().unwrap()).await.unwrap();
        let sa = SyncService::builder(a.clone()).build().await.unwrap();
        sa.start().await;
        until("invited", 60, async || a.get_room(&room_id)).await.expect("invite reached alice");
        a.join_room_by_id(&room_id).await.unwrap();
        until("joined", 60, async || a.get_room(&room_id).filter(|r| r.state() == matrix_engine::sdk::RoomState::Joined)).await.expect("joined");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let before = global_until(&a, "arcaico", |_| true).await;
        report(&mut failed, "backfill.old-message-not-known-before", ids(&before).is_empty(), state_name(&before).into());
        let stop = AtomicBool::new(false);
        let dir = index_dir(&store_a);
        let started = Instant::now();
        let worked = matrix_engine::search_index::backfill(&a, Some(&dir), &stop).await;
        eprintln!("MEASURE backfill-65-messages-ms {}", started.elapsed().as_millis());
        let after = global_until(&a, "arcaico", |s| ids(s) == [old.to_string()]).await;
        report(&mut failed, "backfill.old-message-found-after", ids(&after) == [old.to_string()], format!("worked={worked}"));
        let again = matrix_engine::search_index::backfill(&a, Some(&dir), &stop).await;
        report(&mut failed, "backfill.a-finished-room-is-not-fetched-again", again == 0 && std::fs::read_to_string(dir.join("rooms.txt")).unwrap_or_default().contains(&format!("d {room_id}")), format!("again={again}"));
        // A stop request ends the pass at once.
        let halted = AtomicBool::new(true);
        report(&mut failed, "backfill.stop-ends-the-pass", matrix_engine::search_index::backfill(&a, None, &halted).await == 0, String::new());
        let _ = (sa.stop().await, sb.stop().await);
    });
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}

/// About 5000 synthetic messages in 5 unencrypted rooms: time to index, space of the index, time of a search.
#[test]
#[ignore = "requires the timeline_probe script"]
fn global_search_volume() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let (store_a, store_b) = (home.join("stores/alice-volume"), home.join("stores/bob-volume"));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    const ROOMS: usize = 5;
    const PER_ROOM: usize = 1000;
    let words = ["caffè", "treno", "domani", "riunione", "progetto", "cena", "weekend", "pagamento", "fattura", "ufficio", "viaggio", "telefono", "documento", "ordine", "consegna", "magazzino", "cliente", "preventivo", "scadenza", "ciao"];
    rt.block_on(async {
        let a = build(&config, &store_a, None).await;
        let b = build(&config, &store_b, None).await;
        login(&a, &config, "alice").await;
        login(&b, &config, "bob").await;
        let mut ids = Vec::new();
        for _ in 0..ROOMS {
            let id = b.create_room(room_request(false, Some(&a))).await.unwrap().room_id().to_owned();
            ids.push(id);
        }
        let sb = SyncService::builder(b.clone()).build().await.unwrap();
        sb.start().await;
        let sa = SyncService::builder(a.clone()).build().await.unwrap();
        sa.start().await;
        for id in &ids {
            until("invite", 60, async || a.get_room(id)).await.expect("invite");
            a.join_room_by_id(id).await.unwrap();
        }
        let t = Instant::now();
        let mut sent = 0;
        for (n, id) in ids.iter().enumerate() {
            let room = until("room", 60, async || b.get_room(id)).await.expect("room");
            // Twenty at a time: the server is the slow part, not the client.
            for chunk in (0..PER_ROOM).collect::<Vec<_>>().chunks(20) {
                let batch = chunk.iter().map(|i| {
                    let body = format!("{} {} {} numero{} stanza{} messaggio sintetico di prova {}", words[i % words.len()], words[(i * 7 + n) % words.len()], words[(i * 13 + 3) % words.len()], i, n, i * 31 % 977);
                    std::future::IntoFuture::into_future(room.send(plain(&body)))
                });
                for r in futures_util::future::join_all(batch).await {
                    r.unwrap();
                    sent += 1;
                }
            }
        }
        eprintln!("MEASURE sent-messages {sent} in-ms {}", t.elapsed().as_millis());
        // Alice pages all of it into her cache; each page reaches the index too.
        let t = Instant::now();
        let mut reached = 0;
        for id in &ids {
            if matrix_engine::search_index::fill(&a, id, 5000, Duration::from_secs(240)).await {
                reached += 1;
            }
        }
        eprintln!("MEASURE page-and-index-all-rooms-ms {} rooms-at-start {reached}/{ROOMS}", t.elapsed().as_millis());
        // The SDK indexes the same events on its own; wait until the index holds the last message of every room.
        let probe = global_until(&a, "numero999", |s| hits(s).len() >= ROOMS).await;
        eprintln!("MEASURE last-messages-indexed rooms={}", hits(&probe).len());
        let dir = index_dir(&store_a);
        eprintln!("MEASURE index-bytes {} files {}", bytes_under(&dir), files_under(&dir).len());
        eprintln!("MEASURE store-bytes {}", bytes_under(&store_a));
        for (label, term) in [("one-word", "riunione"), ("prefix", "riun"), ("two-words", "riunione treno"), ("rare-number", "numero777"), ("accent", "caffe"), ("no-match", "zzzzqq")] {
            let mut ms = Vec::new();
            let mut n = 0;
            for _ in 0..7 {
                let mut s = MessageSearch::everywhere(a.clone());
                let mut rx = s.subscribe();
                s.set_query(term);
                tokio::time::sleep(DEBOUNCE).await;
                let t = Instant::now();
                let state = loop {
                    let state = rx.borrow_and_update().clone();
                    if !matches!(state, MessageSearchState::Searching { .. }) {
                        break state;
                    }
                    rx.changed().await.unwrap();
                };
                ms.push(t.elapsed().as_millis());
                n = hits(&state).len();
            }
            ms.sort();
            eprintln!("MEASURE search-all-rooms {label} term-len {} hits-first-page {n} median-ms {} max-ms {}", term.len(), ms[3], ms[6]);
        }
        let _ = (sa.stop().await, sb.stop().await);
        let held = a.joined_rooms().len();
        drop((a, b));
        tokio::time::sleep(Duration::from_millis(500)).await;
        // Cold start: a new client on the saved store, the first search after it.
        let t = Instant::now();
        let again = build(&config, &store_a, Some("https://localhost:1/")).await;
        eprintln!("MEASURE reopen-client-ms {}", t.elapsed().as_millis());
        let _ = held;
        let t = Instant::now();
        let mut s = MessageSearch::everywhere(again.clone());
        let mut rx = s.subscribe();
        s.set_query("riunione");
        tokio::time::sleep(DEBOUNCE).await;
        let state = loop {
            let state = rx.borrow_and_update().clone();
            if !matches!(state, MessageSearchState::Searching { .. }) {
                break state;
            }
            rx.changed().await.unwrap();
        };
        eprintln!("MEASURE first-search-after-restart-ms {} hits {}", t.elapsed().as_millis(), hits(&state).len());
    });
}
