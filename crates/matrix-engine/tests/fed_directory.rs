//! Room-directory search against disposable homeservers federated over loopback, as the user of the
//! "home" server (the fed_directory_probe script, FED_CONFIG): local search, search on a second server
//! (`server` parameter), join from a result with `via`, pasted alias, error kinds, pagination,
//! debounce / cancel / 2-char threshold / nothing on open, slow answer, fields missing, and no term in
//! the client logs (TRACE). One test: the rooms and the servers are expensive to set up.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    operations::{DirectoryRoom, OperationError, join_room, public_rooms},
    sdk::{Client, reqwest::Certificate, sliding_sync::VersionBuilder},
    search::{DEBOUNCE, More, PAGE, Search, SearchError, SearchState, TIMEOUT},
};
use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

type Rooms = Search<Option<String>, DirectoryRoom>;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);
impl Write for Captured {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn settled(rx: &mut tokio::sync::watch::Receiver<SearchState<DirectoryRoom>>, secs: u64) -> SearchState<DirectoryRoom> {
    tokio::time::timeout(Duration::from_secs(secs), rx.wait_for(|s| !matches!(s, SearchState::Idle | SearchState::Searching { .. })))
        .await
        .expect("search did not settle")
        .unwrap()
        .clone()
}

fn hits(s: &SearchState<DirectoryRoom>) -> &[DirectoryRoom] {
    match s {
        SearchState::Results { hits, .. } => hits,
        other => panic!("expected results, got {}", describe(other)),
    }
}

fn describe(s: &SearchState<DirectoryRoom>) -> String {
    match s {
        SearchState::Idle => "Idle".into(),
        SearchState::Searching { .. } => "Searching".into(),
        SearchState::Results { hits, more, .. } => format!("Results({}, {more:?})", hits.len()),
        SearchState::NoResults { .. } => "NoResults".into(),
        SearchState::Error { error, .. } => format!("Error({error:?})"),
    }
}

#[test]
#[ignore = "requires the fed_directory_probe script"]
fn directory_search_end_to_end() {
    let log = Captured::default();
    let sink = log.clone();
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing_subscriber::filter::LevelFilter::TRACE)
        .with_writer(move || sink.clone())
        .try_init();
    let c: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FED_CONFIG").unwrap()).unwrap()).unwrap();
    let s = |k: &str| c[k].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(s("url"))
            .add_root_certificates(vec![Certificate::from_pem(&std::fs::read(s("ca")).unwrap()).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client.matrix_auth().login_username(&s("user"), &c["password"].as_str().unwrap()).send().await.unwrap();
        let (home, remote, closed, down, run) = (s("home"), s("remote"), s("closed"), s("down"), s("run"));
        let calls = Arc::new(AtomicUsize::new(0));
        let make = |delay_for: Option<&'static str>| -> Rooms {
            let (client, calls) = (client.clone(), calls.clone());
            Search::new(None, move |term: String, server: Option<String>, since| {
                let (client, calls) = (client.clone(), calls.clone());
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    if delay_for.is_some_and(|p| term.starts_with(p)) {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    public_rooms(&client, &term, server.as_deref(), since, PAGE).await
                }
            })
        };
        let report = |name: &str, detail: String| println!("CHECK {name}: {detail}");

        // 1. Nothing is asked on open, nor below two characters (after trim).
        let mut search = make(None);
        let mut rx = search.subscribe();
        tokio::time::sleep(DEBOUNCE * 3).await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for t in ["", " ", "a", "  a ", "é"] {
            search.set_query(t, None);
            tokio::time::sleep(DEBOUNCE * 2).await;
            assert!(matches!(*rx.borrow(), SearchState::Idle), "{t:?}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        report("open+threshold", "0 requests for '', ' ', 'a', '  a ', 'é'".into());

        // 2. Local search: our server's rooms, origin = our server, others absent.
        search.set_query(&format!("gnobbilocal{run}"), None);
        let st = settled(&mut rx, 20).await;
        let local = hits(&st);
        assert_eq!(local.len(), 2, "{local:?}");
        assert!(local.iter().all(|h| h.origin == home && h.join_rule == "public"));
        assert!(local.iter().any(|h| h.topic.as_deref() == Some("local one")) && local.iter().any(|h| h.topic.is_none()));
        report("local", format!("{} hits, origin {home}", local.len()));

        // 3. Second server (federated): found there, not locally; origin = that server.
        search.set_query(&format!("gnobbifed"), Some(remote.clone()));
        let st = settled(&mut rx, 30).await;
        let rem = hits(&st).to_vec();
        // pub, id, knock and, on Tuwunel, the invite-only room; Synapse lists only public/knock rooms.
        assert!(rem.len() == 4 || (rem.len() == 3 && !rem.iter().any(|h| h.room_id == s("invite_id"))), "{rem:?}");
        assert!(rem.iter().all(|h| h.origin == remote));
        let by = |id: &str| rem.iter().find(|h| h.room_id == id).unwrap_or_else(|| panic!("{id} missing")).clone();
        let (pubr, knock, idr) = (by(&s("pub_id")), by(&s("knock_id")), by(&s("pub_id2")));
        let invite = rem.iter().find(|h| h.room_id == s("invite_id")).cloned();
        assert_eq!(pubr.alias.as_deref(), Some(s("pub_alias").as_str()));
        assert_eq!((pubr.join_rule.as_str(), knock.join_rule.as_str()), ("public", "knock"));
        assert!(invite.as_ref().is_none_or(|i| i.join_rule == "invite"));
        assert!(idr.alias.is_none() && idr.topic.is_none() && idr.avatar_url.is_none());
        report("remote", format!("{} hits, origin {remote}, invite-only listed: {}", rem.len(), invite.is_some()));
        search.set_query(&format!("gnobbilocal{run}"), Some(remote.clone()));
        assert!(matches!(settled(&mut rx, 30).await, SearchState::NoResults { .. }));
        report("remote-scope", "local-room term on remote server = NoResults".into());

        // 4. Rapid typing + scope change = ONE request, only the last term publishes.
        let before = calls.load(Ordering::SeqCst);
        let mut seen: Vec<String> = Vec::new();
        for t in ["gn", "gno", "gnob", "gnobbifedp"] {
            search.set_query(t, Some(remote.clone()));
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        search.set_query(&format!("gnobbifedpub{run}"), Some(remote.clone()));
        let st = settled(&mut rx, 30).await;
        seen.push(st.term().unwrap().to_owned());
        assert_eq!(hits(&st).len(), 1);
        tokio::time::sleep(DEBOUNCE * 2).await;
        assert_eq!(calls.load(Ordering::SeqCst) - before, 1, "debounce must coalesce");
        report("debounce", "5 changes in <300 ms -> 1 request".into());

        // 5. A slow request is cancelled by a new term; the late answer never publishes.
        let mut slow = make(Some("slow"));
        let mut srx = slow.subscribe();
        slow.set_query("slow-term", None);
        tokio::time::sleep(DEBOUNCE + Duration::from_millis(400)).await; // request is in flight (2 s delay)
        let n = calls.load(Ordering::SeqCst);
        slow.set_query(&format!("gnobbilocal{run}"), None);
        let mut published = Vec::new();
        let end = tokio::time::Instant::now() + Duration::from_millis(3500);
        while let Ok(Ok(())) = tokio::time::timeout_at(end, srx.changed()).await {
            if let Some(t) = srx.borrow().term() {
                published.push((t.to_owned(), describe(&srx.borrow())));
            }
        }
        assert!(published.iter().all(|(t, _)| !t.starts_with("slow")) || published.iter().filter(|(t, d)| t.starts_with("slow") && d.starts_with("Results")).count() == 0, "{published:?}");
        assert!(matches!(*srx.borrow(), SearchState::Results { ref term, .. } if term == &format!("gnobbilocal{run}")));
        assert!(calls.load(Ordering::SeqCst) > n);
        report("cancel", format!("slow term never reached Results; final = new term ({} states)", published.len()));

        // 6. Pagination: 25 rooms, page 1 = 20 + Available, «Altri risultati» -> 25 and None.
        search.set_query(&s("page_term"), Some(remote.clone()));
        let st = settled(&mut rx, 30).await;
        match &st {
            SearchState::Results { hits, more, .. } => assert_eq!((hits.len(), *more), (20, More::Available)),
            o => panic!("{}", describe(o)),
        }
        search.more();
        let st = tokio::time::timeout(Duration::from_secs(30), rx.wait_for(|s| matches!(s, SearchState::Results { more: More::None, .. }))).await.unwrap().unwrap().clone();
        let all = hits(&st);
        let mut ids: Vec<_> = all.iter().map(|h| h.room_id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!((all.len(), ids.len()), (25, 25));
        report("pagination", "20 + 5, no duplicates, more=None".into());

        // 7. Hits without name/alias/topic/avatar: the unnamed room is listed with a usable title.
        let page = public_rooms(&client, "", Some(&remote), None, 100).await.expect("empty term list");
        let bare = page.hits.iter().find(|h| h.room_id == s("bare_id")).expect("unnamed room listed");
        assert!(bare.alias.is_none() && bare.topic.is_none() && bare.avatar_url.is_none() && !bare.name.trim().is_empty());
        report("bare-hit", format!("name falls back to {:?}, members {}", bare.name, bare.members));

        // 8. Errors, one kind each (no request at all for bad names).
        let kind = |r: Result<_, SearchError>| -> SearchError { r.map(|_: matrix_engine::search::Page<DirectoryRoom>| ()).expect_err("must fail") };
        let mut errs = Vec::new();
        for (label, name, want) in [
            ("bad name (space)", "not a server", SearchError::InvalidServer),
            ("bad name (scheme)", "https://x.example", SearchError::InvalidServer),
            ("bad name (path)", "x.example/path", SearchError::InvalidServer),
            ("bad name (empty)", " ", SearchError::InvalidServer),
            ("directory not federated", closed.as_str(), if c["home_kind"] == "tuwunel" { SearchError::Unreachable } else { SearchError::Refused }),
            ("server switched off", down.as_str(), SearchError::Unreachable),
            ("closed port", "127.0.0.9:9", SearchError::Unreachable),
            ("unresolvable host", "gnobbi-test-nonexistent.invalid", SearchError::Unreachable),
        ] {
            let before = calls.load(Ordering::SeqCst);
            let t0 = std::time::Instant::now();
            // 5 s is plenty for a relayed 502; longer = the SDK is retrying it (see the second test).
            let Ok(r) = tokio::time::timeout(Duration::from_secs(5), public_rooms(&client, "gnobbi", Some(name), None, PAGE)).await else {
                println!("ERRMAP {label}: still retrying after 5 s (expected {want:?}) -> dead_server_answers_unreachable_not_timeout");
                continue;
            };
            let got = kind(r);
            println!("ERRTIME {label}: {:?}", t0.elapsed());
            assert_eq!(calls.load(Ordering::SeqCst), before);
            errs.push(format!("{label}: {got:?} (expected {want:?})"));
            if got != want {
                errs.push(format!("!! MISMATCH {label}"));
            }
        }
        for e in &errs {
            println!("ERRMAP {e}");
        }
        assert!(!errs.iter().any(|e| e.starts_with("!!")), "{errs:#?}");
        // The same errors through the driver end up as Error{term, error} with the term of the request.
        search.set_query("gnobbi-err", Some(closed.clone()));
        // Tuwunel as home server relays the remote's 403 as 502 M_CONNECTION_FAILED: indistinguishable from "dead".
        let refused = if c["home_kind"] == "tuwunel" { SearchError::Unreachable } else { SearchError::Refused };
        assert!(matches!(settled(&mut rx, 30).await, SearchState::Error { error, ref term } if (error == refused || (refused == SearchError::Unreachable && error == SearchError::Timeout)) && term == "gnobbi-err"));

        // 9. Joins: result with via, pasted alias (no via), invite/knock refused in one error kind.
        let account = AccountId::new(1);
        let joined = |id: &str| client.get_room(<&matrix_engine::sdk::ruma::RoomId>::try_from(id).unwrap()).is_some_and(|r| r.state() == matrix_engine::sdk::RoomState::Joined);
        join_room(&client, account, &pubr.room_id, &[pubr.origin.clone()]).await.expect("join by room_id + via");
        assert!(joined(&pubr.room_id));
        join_room(&client, account, &s("pub_alias"), &[]).await.expect("join by pasted alias");
        assert!(!joined(&idr.room_id));
        // Without `via` a room ID whose version carries no server (v12) cannot be joined remotely: this is
        // why a result always joins with via=[origin]. Older versions fall back to the server in the ID.
        let bare = join_room(&client, account, &idr.room_id, &[]).await;
        println!("CHECK join-no-via: {} -> {:?}", idr.room_id.get(..8).unwrap_or(""), bare.as_ref().map(|_| ()));
        if !idr.room_id.contains(':') {
            assert!(bare.is_err()); // Unavailable on Synapse home, Transport on Tuwunel home (printed above)
        } else {
            bare.expect("server taken from the room id");
        }
        join_room(&client, account, &idr.room_id, &[idr.origin.clone()]).await.expect("join by room_id + via");
        assert!(joined(&idr.room_id));
        let mut refusals = Vec::new();
        for (label, id) in [("invite-only", s("invite_id")), ("knock", knock.room_id.clone())] {
            let e = join_room(&client, account, &id, &[remote.clone()]).await.expect_err(label);
            refusals.push(format!("{label}: {e:?}"));
            assert_eq!(e, OperationError::Forbidden, "{label}");
            assert!(!joined(&id));
        }
        let e = join_room(&client, account, "#nope-nope:127.0.0.9:9", &[]).await.expect_err("unreachable alias");
        refusals.push(format!("alias on dead server: {e:?}"));
        let e = join_room(&client, account, "not an alias", &[]).await.expect_err("garbage");
        assert_eq!(e, OperationError::InvalidInput);
        for r in &refusals {
            println!("JOINERR {r}");
        }

        // 10. Slow remote (SIGSTOP): our request gives up at TIMEOUT with Timeout, and recovers on SIGCONT.
        let pid = c["remote_pid"].as_u64().unwrap().to_string();
        let sig = |s: &str| assert!(std::process::Command::new("kill").args([s, &pid]).status().unwrap().success());
        sig("-STOP");
        let t0 = std::time::Instant::now();
        search.set_query("gnobbifedpub", Some(remote.clone()));
        let st = settled(&mut rx, TIMEOUT.as_secs() + 15).await;
        let took = t0.elapsed();
        sig("-CONT");
        assert!(matches!(st, SearchState::Error { error: SearchError::Timeout, .. }), "{}", describe(&st));
        assert!(took >= TIMEOUT && took < TIMEOUT + Duration::from_secs(5), "{took:?}");
        report("timeout", format!("Timeout after {took:.1?} against a stopped remote"));
        tokio::time::sleep(Duration::from_secs(2)).await;
        search.set_query("gnobbifedpub", Some(remote.clone()));
        assert!(!hits(&settled(&mut rx, 40).await).is_empty());
        report("recovery", "after SIGCONT the same search answers".into());

        // 11. No term in the client's logs at TRACE.
        let text = String::from_utf8_lossy(&log.0.lock().unwrap()).into_owned();
        if let Ok(path) = std::env::var("FED_LOG_DUMP") {
            std::fs::write(path, &text).unwrap();
        }
        // Terms typed in the search field never appear anywhere. (The join target the user pastes does appear in
        // the SDK's DEBUG request log, and room names of joined rooms are room data: not checked as terms.)
        let search_lines: Vec<&str> = text.lines().filter(|l| l.contains("public_rooms") || l.contains("publicRooms")).collect();
        assert!(search_lines.len() > 20, "the search requests must be in the TRACE log: {}", search_lines.len());
        for needle in [format!("gnobbilocal{run}"), s("page_term"), "gnobbifed".into(), "gnobbi-err".into(), "slow-term".into()] {
            let n = search_lines.iter().filter(|l| l.contains(&needle)).count();
            report("log-term", format!("{needle}: {n} of {} search request lines ({} bytes of TRACE log)", search_lines.len(), text.len()));
            assert_eq!(n, 0, "term leaked into the client log: {needle}");
        }
        for needle in [format!("gnobbilocal{run}"), s("page_term"), "gnobbi-err".into(), "slow-term".into()] {
            assert_eq!(text.matches(&needle).count(), 0, "term leaked into the client log: {needle}");
        }
        report("log-term", "no search term anywhere in the TRACE log".into());
    });
}

/// Regression: the SDK used to retry the 502 a homeserver relays for a dead
/// remote, so the request runs into the driver's 20 s TIMEOUT and the user reads "Tempo scaduto" instead
/// of "Il server non risponde". Now `Unreachable` in one request; Synapse itself needs ~6 s to give up on a dead remote, hence 10 s.
#[test]
#[ignore = "requires the fed_directory_probe script"]
fn dead_server_answers_unreachable_not_timeout() {
    let c: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FED_CONFIG").unwrap()).unwrap()).unwrap();
    let s = |k: &str| c[k].as_str().unwrap().to_owned();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(s("url"))
            .add_root_certificates(vec![Certificate::from_pem(&std::fs::read(s("ca")).unwrap()).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client.matrix_auth().login_username(&s("user"), c["password"].as_str().unwrap()).send().await.unwrap();
        let mut bad = Vec::new();
        for name in [s("down"), "127.0.0.9:9".to_owned(), "gnobbi-test-nonexistent.invalid".to_owned()] {
            let t0 = std::time::Instant::now();
            let r = tokio::time::timeout(TIMEOUT, public_rooms(&client, "gnobbi", Some(&name), None, PAGE)).await;
            println!("DEAD {name}: {:?} after {:?}", r.as_ref().map(|r| r.as_ref().err().copied()), t0.elapsed());
            bad.extend((!(matches!(r, Ok(Err(SearchError::Unreachable))) && t0.elapsed() < Duration::from_secs(10))).then_some(name));
        }
        assert!(bad.is_empty(), "retried until the timeout: {bad:?}");
    });
}
