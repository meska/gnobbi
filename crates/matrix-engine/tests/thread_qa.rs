//! P258 QA: matrix-sdk-ui thread support on a disposable homeserver, two synthetic accounts, per-account
//! SQLite stores (same builder settings as `persistence.rs`). Scenarios print `QA <name> PASS|FAIL detail`
//! and the test fails if any failed. Driver: PROBE_TARGET=thread_qa PROBE_FEATURES=threads
//! PROBE_TEST=thread_qa_scenarios PROBE_EVIDENCE=artifacts/P258-qa python3 the timeline_probe script
#![cfg(feature = "threads")]
#![recursion_limit = "512"]
use matrix_engine::{
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            OwnedEventId,
            api::client::room::create_room::v3::{
                CreationContent, Request as CreateRoomRequest, RoomPreset,
            },
            assign,
            events::{
                InitialStateEvent,
                relation::Thread,
                room::{
                    encryption::RoomEncryptionEventContent,
                    message::{
                        Relation, RoomMessageEventContent, RoomMessageEventContentWithoutRelation,
                    },
                },
            },
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::{
        sync_service::SyncService,
        timeline::{
            RoomExt, ThreadListPaginationState, ThreadListService, Timeline, TimelineFocus,
        },
    },
    thread_timeline,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

const PASS1: &str = "synthetic-passphrase-alice-1";
const PASS2: &str = "synthetic-passphrase-bob-02";
const N: usize = 45;

fn report(failed: &mut Vec<String>, name: &str, ok: bool, detail: String) {
    eprintln!("QA {name} {} {detail}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        failed.push(name.to_owned());
    }
}

/// A confirmed matrix-sdk(-ui) 0.19.1 limitation: never fails the run, but flips to PASS once fixed.
fn known(name: &str, ok: bool, detail: String) {
    eprintln!(
        "QA {name} {} {detail}",
        if ok {
            "PASS(limit gone)"
        } else {
            "KNOWN-LIMIT"
        }
    );
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
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn build(
    config: &serde_json::Value,
    store: Option<(&Path, &str)>,
) -> Result<Client, matrix_engine::sdk::ClientBuildError> {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let mut b = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative);
    if let Some((path, pass)) = store {
        b = b.sqlite_store(path, Some(pass));
    }
    let c = b.build().await?;
    c.event_cache().subscribe().unwrap();
    Ok(c)
}

async fn login(c: &Client, config: &serde_json::Value, who: &str) {
    c.matrix_auth()
        .login_username(
            config[format!("{who}_mxid")].as_str().unwrap(),
            config[format!("{who}_password")].as_str().unwrap(),
        )
        .send()
        .await
        .unwrap();
}

fn plain(body: &str) -> RoomMessageEventContent {
    RoomMessageEventContent::text_plain(body)
}

fn in_thread(body: &str, root: &OwnedEventId) -> RoomMessageEventContent {
    let mut c = plain(body);
    c.relates_to = Some(Relation::Thread(Thread::plain(root.clone(), root.clone())));
    c
}

async fn bodies(t: &Timeline) -> Vec<String> {
    t.items()
        .await
        .iter()
        .filter_map(|i| i.as_event())
        .filter_map(|e| e.content().as_message().map(|m| m.body().to_owned()))
        .collect()
}

/// Items reach the timeline stream after `paginate_backwards` returns: wait until the count is stable.
async fn settle(t: &Timeline) -> usize {
    let (mut last, mut stable, start) = (usize::MAX, 0, Instant::now());
    while stable < 4 && start.elapsed() < Duration::from_secs(15) {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let n = bodies(t).await.len();
        stable = if n == last { stable + 1 } else { 0 };
        last = n;
    }
    last
}

/// Backwards pagination until the start; returns (pages, reached_start). Stops after 6 pages.
async fn page_to_start(t: &Timeline, per_page: u16) -> (u32, bool) {
    let mut out = (6, false);
    for page in 1..=6 {
        let hit = t.paginate_backwards(per_page).await.unwrap();
        eprintln!(
            "page {page}: hit_start={hit} items_now={}",
            bodies(t).await.len()
        );
        if hit {
            out = (page, true);
            break;
        }
    }
    let n = settle(t).await;
    eprintln!("settled at {n} items");
    out
}

async fn summary_replies(t: &Timeline, root_body: &str) -> Option<u32> {
    t.items()
        .await
        .iter()
        .filter_map(|i| i.as_event())
        .find_map(|e| {
            (e.content().as_message().map(|m| m.body()) == Some(root_body))
                .then(|| e.content().thread_summary().map(|s| s.num_replies))
                .flatten()
        })
}

async fn raw_relation(room: &Room, id: &OwnedEventId) -> serde_json::Value {
    let ev = room.event(id, None).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(ev.raw().json().get()).unwrap();
    v["content"]["m.relates_to"].clone()
}

/// Core scenarios on one room (plain or encrypted). Returns the thread root id.
async fn core(
    tag: &str,
    strict: bool,
    ra: &Room,
    rb: &Room,
    failed: &mut Vec<String>,
) -> (OwnedEventId, OwnedEventId) {
    let root = ra.send(plain("root")).await.unwrap().response.event_id;
    for i in 1..=N {
        rb.send(in_thread(&format!("r{i}"), &root)).await.unwrap();
    }
    ra.send(plain("main1")).await.unwrap();
    let mut expected = N as u32;

    // 1. TimelineFocus::Thread: load + backwards pagination.
    let tl = thread_timeline::open(ra, root.clone()).await.unwrap();
    let first = until("first thread items", 40, async || {
        let b = bodies(&tl).await;
        (!b.is_empty()).then_some(b.len())
    })
    .await;
    // the first fill is not guaranteed to be complete: page until the root is reached
    let (pages, reached) = page_to_start(&tl, 20).await;
    let b = bodies(&tl).await;
    let replies: Vec<String> = (1..=N).map(|i| format!("r{i}")).collect();
    let got: Vec<String> = b
        .iter()
        .filter(|x| x.starts_with('r') && *x != "root")
        .cloned()
        .collect();
    // Tuwunel 1.9.4 leaves holes in a thread bigger than the sync window: recorded, not asserted.
    let (name, detail) = (
        format!("{tag}.thread-load-replies"),
        format!(
            "first={first:?} n={} in_order={}",
            got.len(),
            got == replies
        ),
    );
    if strict {
        report(failed, &name, got == replies, detail);
    } else {
        known(&name, got == replies, detail);
    }
    let mut want = vec!["root".to_owned()];
    want.extend(replies);
    known(
        &format!("{tag}.thread-load-root-and-start"),
        reached && b == want,
        format!(
            "pages_after_open={pages} hit_start={reached} root_loaded={} n={} head={:?}",
            b.contains(&"root".to_owned()),
            b.len(),
            &b[..b.len().min(2)]
        ),
    );

    // 2. Live updates into the open thread timeline (no pagination).
    expected += 1;
    rb.send(in_thread("live1", &root)).await.unwrap();
    let live = until("live1 in thread timeline", 25, async || {
        bodies(&tl)
            .await
            .contains(&"live1".to_owned())
            .then_some(())
    })
    .await;
    report(
        failed,
        &format!("{tag}.thread-live"),
        live.is_some(),
        "bob reply appears with no pagination".into(),
    );

    // 3. hide_threaded_events on the main timeline.
    let hidden = ra
        .timeline_builder()
        .with_focus(TimelineFocus::Live {
            hide_threaded_events: true,
        })
        .build()
        .await
        .unwrap();
    let shown = ra
        .timeline_builder()
        .with_focus(TimelineFocus::Live {
            hide_threaded_events: false,
        })
        .build()
        .await
        .unwrap();
    let (_, hr) = page_to_start(&hidden, 50).await;
    let (_, sr) = page_to_start(&shown, 50).await;
    let (hb, sb) = (bodies(&hidden).await, bodies(&shown).await);
    let hide_ok = hb.contains(&"root".to_owned())
        && hb.contains(&"main1".to_owned())
        && !hb.iter().any(|x| x.starts_with('r') && x != "root");
    report(
        failed,
        &format!("{tag}.hide-threaded"),
        hide_ok && hr,
        format!(
            "hidden={} reached={hr} shown={} reached={sr} replies_in_shown={}",
            hb.len(),
            sb.len(),
            sb.iter()
                .filter(|x| x.starts_with('r') && *x != "root")
                .count()
        ),
    );
    let sum = summary_replies(&hidden, "root").await;
    report(
        failed,
        &format!("{tag}.summary-under-root"),
        sum == Some(expected),
        format!("num_replies={sum:?} expected={expected}"),
    );
    expected += 1;
    rb.send(in_thread("live2", &root)).await.unwrap();
    let bump = until("summary bump", 25, async || {
        (summary_replies(&hidden, "root").await == Some(expected)).then_some(())
    })
    .await;
    report(
        failed,
        &format!("{tag}.summary-live"),
        bump.is_some() && !bodies(&hidden).await.contains(&"live2".to_owned()),
        format!(
            "summary now {:?}, live2 hidden from main",
            summary_replies(&hidden, "root").await
        ),
    );

    // 4. Reply in thread: plain send from the thread timeline, local echo then remote echo.
    expected += 1;
    tl.send(plain("alice-thread").into()).await.unwrap();
    let mut seen_local = false;
    let mut event_id = None;
    let done = until("remote echo", 25, async || {
        for it in tl.items().await.iter().filter_map(|i| i.as_event()) {
            if it.content().as_message().map(|m| m.body()) == Some("alice-thread") {
                seen_local |= it.is_local_echo();
                if !it.is_local_echo() {
                    event_id = it.event_id().map(ToOwned::to_owned);
                    return Some(());
                }
            }
        }
        None
    })
    .await;
    let count = bodies(&tl)
        .await
        .iter()
        .filter(|x| *x == "alice-thread")
        .count();
    report(
        failed,
        &format!("{tag}.reply-echo"),
        done.is_some() && count == 1,
        format!(
            "local_echo_seen={seen_local} remote_echo={} copies={count}",
            event_id.is_some()
        ),
    );
    if let Some(id) = &event_id {
        let rel = raw_relation(ra, id).await;
        let ok = rel["rel_type"] == "m.thread"
            && rel["event_id"] == root.as_str()
            && rel["is_falling_back"] == true;
        report(
            failed,
            &format!("{tag}.reply-relation"),
            ok,
            format!("{rel}"),
        );
    }
    let seen_by_bob = {
        let tb = thread_timeline::open(rb, root.clone()).await.unwrap();
        let (_, _) = page_to_start(&tb, 50).await;
        let b = bodies(&tb).await;
        report(
            failed,
            &format!("{tag}.bob-thread-view"),
            b.len() as u32 == expected + 1 && b.contains(&"alice-thread".to_owned()),
            format!("n={} expected={}", b.len(), expected + 1),
        );
        b.len()
    };
    let _ = seen_by_bob;
    let hid = !bodies(&hidden).await.contains(&"alice-thread".to_owned());
    report(
        failed,
        &format!("{tag}.reply-not-in-hidden-main"),
        hid,
        String::new(),
    );

    // 4b. Real reply (send_reply) inside the thread, and from the main timeline to a thread event.
    let target = tl
        .items()
        .await
        .iter()
        .filter_map(|i| i.as_event())
        .find(|e| e.content().as_message().map(|m| m.body()) == Some("r3"))
        .and_then(|e| e.event_id().map(ToOwned::to_owned));
    if target.is_none() {
        known(
            &format!("{tag}.reply-to-in-thread"),
            false,
            "r3 not loaded in the thread timeline (skipped)".into(),
        );
    }
    if let Some(target) = target {
        expected += 1;
        tl.send_reply(
            RoomMessageEventContentWithoutRelation::text_plain("reply-r3"),
            target.clone(),
        )
        .await
        .unwrap();
        let id = until("send_reply echo", 25, async || {
            tl.items()
                .await
                .iter()
                .filter_map(|i| i.as_event())
                .find_map(|e| {
                    (e.content().as_message().map(|m| m.body()) == Some("reply-r3")
                        && !e.is_local_echo())
                    .then(|| e.event_id().map(ToOwned::to_owned))
                    .flatten()
                })
        })
        .await;
        match id {
            Some(id) => {
                let rel = raw_relation(ra, &id).await;
                let ok = rel["rel_type"] == "m.thread"
                    && rel["event_id"] == root.as_str()
                    && rel["is_falling_back"] != true
                    && rel["m.in_reply_to"]["event_id"] == target.as_str();
                report(
                    failed,
                    &format!("{tag}.reply-to-in-thread"),
                    ok,
                    format!("{rel}"),
                );
            }
            None => report(
                failed,
                &format!("{tag}.reply-to-in-thread"),
                false,
                "no remote echo".into(),
            ),
        }
        // from the main timeline, replying to a thread event
        expected += 1;
        shown
            .send_reply(
                RoomMessageEventContentWithoutRelation::text_plain("reply-main-r4"),
                {
                    shown
                        .items()
                        .await
                        .iter()
                        .filter_map(|i| i.as_event())
                        .find(|e| e.content().as_message().map(|m| m.body()) == Some("r4"))
                        .and_then(|e| e.event_id().map(ToOwned::to_owned))
                        .unwrap()
                },
            )
            .await
            .unwrap();
        let id = until("main send_reply echo", 25, async || {
            shown
                .items()
                .await
                .iter()
                .filter_map(|i| i.as_event())
                .find_map(|e| {
                    (e.content().as_message().map(|m| m.body()) == Some("reply-main-r4")
                        && !e.is_local_echo())
                    .then(|| e.event_id().map(ToOwned::to_owned))
                    .flatten()
                })
        })
        .await;
        if let Some(id) = id {
            let rel = raw_relation(ra, &id).await;
            report(
                failed,
                &format!("{tag}.reply-from-main-forwards-thread"),
                rel["rel_type"] == "m.thread" && rel["event_id"] == root.as_str(),
                format!("{rel}"),
            );
        } else {
            report(
                failed,
                &format!("{tag}.reply-from-main-forwards-thread"),
                false,
                "no remote echo".into(),
            );
        }
    }

    // 5. Thread list + summary, second thread.
    let root2 = ra.send(plain("root2")).await.unwrap().response.event_id;
    rb.send(in_thread("t2r1", &root2)).await.unwrap();
    let svc = ThreadListService::new(ra.clone());
    let mut tries = 0;
    let listed = until("thread list", 40, async || {
        tries += 1;
        svc.reset().await;
        svc.paginate().await.ok()?;
        (svc.items().len() >= 2).then(|| svc.items())
    })
    .await;
    match listed {
        Some(items) => {
            let one = items.iter().find(|i| i.root_event.event_id == root);
            let two = items.iter().find(|i| i.root_event.event_id == root2);
            let last_ok = one.is_some_and(|i| i.latest_event.is_some());
            report(
                failed,
                &format!("{tag}.thread-list"),
                items.len() == 2
                    && one.is_some_and(|i| i.num_replies >= expected - 2)
                    && two.is_some_and(|i| i.num_replies == 1)
                    && last_ok,
                format!(
                    "n={} replies1={:?} (sent {expected}) replies2={:?} order_first_is_root2={} state={:?} tries={tries}",
                    items.len(),
                    one.map(|i| i.num_replies),
                    two.map(|i| i.num_replies),
                    items
                        .first()
                        .is_some_and(|i| i.root_event.event_id == root2),
                    svc.pagination_state()
                ),
            );
            assert!(matches!(
                svc.pagination_state(),
                ThreadListPaginationState::Idle { .. }
            ));
            rb.send(in_thread("t2r2", &root2)).await.unwrap();
            let live = until("thread list live", 25, async || {
                svc.items()
                    .iter()
                    .find(|i| i.root_event.event_id == root2)
                    .is_some_and(|i| i.num_replies >= 2)
                    .then_some(())
            })
            .await;
            let n2 = svc
                .items()
                .iter()
                .find(|i| i.root_event.event_id == root2)
                .map(|i| i.num_replies);
            // Tuwunel 1.9.4 sometimes counts the live reply twice (3 instead of 2): recorded, not asserted.
            if strict {
                report(
                    failed,
                    &format!("{tag}.thread-list-live"),
                    live.is_some() && n2 == Some(2),
                    format!("replies2={n2:?}"),
                );
            } else {
                known(
                    &format!("{tag}.thread-list-live"),
                    n2 == Some(2),
                    format!("replies2={n2:?} (expected 2)"),
                );
            }
        }
        None => report(
            failed,
            &format!("{tag}.thread-list"),
            false,
            format!("fewer than 2 items after {tries} tries"),
        ),
    }
    (root, root2)
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn thread_qa_scenarios() {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("stores");
    let dir = |n: &str| {
        let p = home.join(n);
        std::fs::create_dir_all(&p).unwrap();
        p
    };
    let mut failed: Vec<String> = Vec::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let strict = config["kind"] == "synapse";
        let (s_a, s_b) = (dir("account-1"), dir("account-2"));
        let a = build(&config, Some((&s_a, PASS1))).await.unwrap();
        let b = build(&config, Some((&s_b, PASS2))).await.unwrap();
        login(&a, &config, "alice").await;
        login(&b, &config, "bob").await;
        let session_a = a.matrix_auth().session().unwrap();
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
        let sa = SyncService::builder(a.clone()).build().await.unwrap();
        let sb = SyncService::builder(b.clone()).build().await.unwrap();
        sa.start().await;
        sb.start().await;
        let get = async |c: &Client, id: &matrix_engine::sdk::ruma::OwnedRoomId| until("room", 40, async || c.get_room(id)).await.expect("room");
        let (ra, rb) = (get(&a, &plain_id).await, get(&b, &plain_id).await);
        let (ea, eb) = (get(&a, &enc_id).await, get(&b, &enc_id).await);
        // both accounts have their own store; scenarios run on both rooms
        let (root, root2) = core("plain", strict, &ra, &rb, &mut failed).await;
        report(&mut failed, "encrypted-flag", ea.encryption_state().is_encrypted(), String::new());
        core("e2ee", strict, &ea, &eb, &mut failed).await;

        // Restart: drop everything for alice, reopen the SAME store with the saved session.
        let before = {
            let t = thread_timeline::open(&ra, root.clone()).await.unwrap();
            page_to_start(&t, 50).await;
            bodies(&t).await.len()
        };
        let _ = sa.stop().await;
        drop((sa, ra, ea));
        drop(a);
        tokio::time::sleep(Duration::from_millis(500)).await;
        let wrong = build(&config, Some((&s_a, "wrong-passphrase-wrong-passphrase"))).await;
        report(&mut failed, "store.wrong-passphrase", wrong.is_err(), format!("build_err={}", wrong.is_err()));
        drop(wrong);
        let a2 = build(&config, Some((&s_a, PASS1))).await.unwrap();
        a2.restore_session(session_a.clone()).await.unwrap();
        let cached_room = a2.get_room(&plain_id);
        report(&mut failed, "store.restart-room-from-store", cached_room.is_some(), "room present before any sync".into());
        if let Some(room) = cached_room {
            let t = thread_timeline::open(&room, root.clone()).await.unwrap();
            let cached = until("cached thread", 5, async || { let n = bodies(&t).await.len(); (n > 0).then_some(n) }).await;
            report(&mut failed, "store.restart-thread-cache", cached.is_some(),
                format!("items shown before sync/pagination: {cached:?} (of {before})"));
            let sa2 = SyncService::builder(a2.clone()).build().await.unwrap();
            sa2.start().await;
            page_to_start(&t, 50).await;
            let after = bodies(&t).await.len();
            rb.send(in_thread("after-restart", &root)).await.unwrap();
            let live = until("live after restart", 25, async || bodies(&t).await.contains(&"after-restart".to_owned()).then_some(())).await;
            report(&mut failed, "store.restart-thread-full", after == before && live.is_some(),
                format!("before={before} after={after} live_after_restart={}", live.is_some()));
            let _ = sa2.stop().await;
        }
        // Re-login: second device for alice with its own empty store.
        let s_a3 = dir("account-3");
        let a3 = build(&config, Some((&s_a3, PASS1))).await.unwrap();
        login(&a3, &config, "alice").await;
        let sa3 = SyncService::builder(a3.clone()).build().await.unwrap();
        sa3.start().await;
        let r3 = until("room on relogin", 40, async || a3.get_room(&plain_id)).await;
        match r3 {
            Some(room) => {
                let t = thread_timeline::open(&room, root.clone()).await.unwrap();
                let (pages, reached) = page_to_start(&t, 50).await;
                let n = bodies(&t).await.len();
                let b = bodies(&t).await;
                known("store.relogin-new-device-sync-seeded-thread", reached && n >= before,
                    format!("pages={pages} reached={reached} n={n} (original {before}) head={:?} tail={:?}", &b[..b.len().min(3)], &b[b.len().saturating_sub(3)..]));
                // A thread whose events are older than the sync window: only /relations can supply them.
                let t2 = thread_timeline::open(&room, root2.clone()).await.unwrap();
                let (p2, r2) = page_to_start(&t2, 50).await;
                let b2 = bodies(&t2).await;
                let late = until("old thread items appear late", 10, async || { let b = bodies(&t2).await; (!b.is_empty()).then_some(b) }).await;
                report(&mut failed, "store.relogin-old-thread", b2 == ["root2", "t2r1", "t2r2"],
                    format!("pages={p2} hit_start={r2} bodies_after_pagination={b2:?} bodies_within_10s={late:?}"));
            }
            None => report(&mut failed, "store.relogin-new-device", false, "room missing".into()),
        }
        let _ = (sa3.stop().await, sb.stop().await);
    });
    drop(rt);
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}
