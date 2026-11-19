//! Join of a large federated room that takes long, and the failure causes, against disposable homeservers
//! federated over loopback (the fed_join_probe script, FED_CONFIG). Real `join_tracked`, real sliding sync.
//! Slow join = the remote server frozen with SIGSTOP (the home server's make_join hangs), thawed later.
//! Prints `CHECK <name>: <phases with elapsed> final=<..> sync=<status>`; asserts at the end so every line is logged.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListEvent, RoomSyncStatus, bounded,
    join::{JOIN_CONFIRM_WINDOW, JoinPhase, join_tracked},
    operations::OperationError,
    room_sync::RoomSync,
    sdk::{Client, reqwest::Certificate, sliding_sync::VersionBuilder},
    sliding,
};
use std::{
    cell::RefCell,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

fn name(s: Option<RoomSyncStatus>) -> &'static str {
    match s {
        None => "none",
        Some(RoomSyncStatus::SyncFailed) => "SyncFailed",
        Some(_) => "up",
    }
}

#[test]
#[ignore = "requires the fed_join_probe script"]
fn join_slow_and_failures_end_to_end() {
    if std::env::var("FED_TRACE").is_ok() {
        let _ = tracing_subscriber::fmt().with_max_level(tracing_subscriber::filter::LevelFilter::DEBUG).with_writer(std::io::stdout).try_init();
    }
    let c: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FED_CONFIG").unwrap()).unwrap()).unwrap();
    let s = |k: &str| c[k].as_str().unwrap().to_owned();
    let pid = |k: &str| c[k].as_u64().unwrap().to_string();
    let kill = |sig: &str, k: &str| assert!(std::process::Command::new("kill").args([sig, &pid(k)]).status().unwrap().success());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(s("url"))
            .add_root_certificates(vec![Certificate::from_pem(&std::fs::read(s("ca")).unwrap()).unwrap()])
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client.matrix_auth().login_username(s("user"), c["password"].as_str().unwrap()).send().await.unwrap();
        let (remote, down, home) = (s("remote"), s("down"), s("home"));
        let account = AccountId::new(1);
        let status: RefCell<Option<RoomSyncStatus>> = RefCell::new(None);
        let failures: RefCell<Vec<String>> = RefCell::new(vec![]);

        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(account, client.clone());
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let drain = async {
            while let Some(e) = rx.recv().await {
                if let RoomListEvent::Status(st) = e {
                    *status.borrow_mut() = Some(st);
                }
            }
        };
        // One join as the engine runs it; returns (phases with elapsed secs, final phase, sync status at the end).
        let attempt = |label: &'static str, target: String, via: Vec<String>| {
            let (client, status, failures) = (client.clone(), &status, &failures);
            async move {
                let t0 = Instant::now();
                let phases = RefCell::new(vec![]);
                // A join that outlives its own window by a minute never ends for the UI (spinner forever): that is a bug.
                let end = match tokio::time::timeout(JOIN_CONFIRM_WINDOW + Duration::from_secs(60), join_tracked(&client, account, &target, &via, |p| phases.borrow_mut().push((t0.elapsed().as_secs_f32(), format!("{p:?}"))))).await {
                    Ok(end) => end,
                    Err(_) => {
                        println!("HUNG {label}: no final phase after {:.0?}; home log tail:\n{}", t0.elapsed(), std::fs::read_to_string(s("home_log")).map(|l| l[l.len().saturating_sub(3000)..].to_owned()).unwrap_or_default());
                        failures.borrow_mut().push(format!("{label}: HUNG, join_tracked never returned"));
                        JoinPhase::Refused(OperationError::Transport)
                    }
                };
                let took = t0.elapsed();
                let seq: Vec<String> = phases.borrow().iter().map(|(t, p)| format!("{p}@{t:.1}s")).collect();
                println!("CHECK {label}: [{}] final={end:?} after {took:.1?} sync={}", seq.join(", "), name(*status.borrow()));
                (phases.into_inner(), end, took)
            }
        };
        let expect = |ok: bool, what: String| {
            if !ok {
                println!("FAIL {what}");
                failures.borrow_mut().push(what);
            }
        };
        let steps = async {
            // Let the first sync settle.
            tokio::time::sleep(Duration::from_secs(5)).await;
            if std::env::var("FED_ONLY").as_deref() == Ok("none") {
                return;
            }
            // 0. First federated contact of a cold pair of servers (key fetch): reported, not asserted.
            attempt("cold-first-contact", s("pub_id5"), vec![remote.clone()]).await;
            // 1. Control: quick joins, by room ID with via and by alias. No Slow.
            for (label, target) in [("fast-id", s("pub_id")), ("fast-alias", s("pub_alias"))] {
                let (ph, end, _) = attempt(label, target, vec![remote.clone()]).await;
                expect(matches!(end, JoinPhase::Joined(_)) && ph.is_empty(), format!("{label}: expected Joined without Slow, got {end:?} {ph:?}"));
            }
            // 2. Missing: alias on the remote and on the home server, room ID that no server knows.
            for (label, target, via) in [
                ("missing-alias-remote", format!("#gnobbi-nope{}:{remote}", s("run")), vec![]),
                ("missing-alias-home", format!("#gnobbi-nope{}:{home}", s("run")), vec![]),
            ] {
                let (_, end, took) = attempt(label, target, via).await;
                expect(end == JoinPhase::Refused(OperationError::Unavailable) && took < Duration::from_secs(20), format!("{label}: expected Refused(Unavailable) fast, got {end:?} after {took:?}"));
            }
            let only = std::env::var("FED_ONLY").unwrap_or_default();
            if only == "missing" {
                let (_, end, took) = attempt("missing-room-id", format!("!gnobbi-nope{}:{remote}", s("run")), vec![remote.clone()]).await;
                println!("ONLY missing-room-id: {end:?} after {took:.1?}");
                return;
            }
            let (_, end, took) = attempt("missing-room-id", format!("!gnobbi-nope{}:{remote}", s("run")), vec![remote.clone()]).await;
            expect(!matches!(end, JoinPhase::Joined(_)), format!("missing-room-id joined?! {end:?}"));
            println!("NOTE missing-room-id took {took:.1?}");

            // 3. Slow join: the remote is frozen, the client's 10 s request gives up, the join goes on in the server.
            kill("-STOP", "remote_pid");
            let thaw = async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                kill("-CONT", "remote_pid");
                println!("NOTE remote thawed at 30 s");
            };
            let ((ph, end, took), _) = tokio::join!(attempt("slow-then-ok", s("pub_id2"), vec![remote.clone()]), thaw);
            expect(ph.len() == 1 && ph[0].1 == "Slow" && ph[0].0 >= 5.0, format!("slow-then-ok: expected one Slow after the client timeout, got {ph:?}"));
            expect(matches!(end, JoinPhase::Joined(_)), format!("slow-then-ok: final should be Joined after the thaw, got {end:?} after {took:?}"));

            // 4. Never answers + dead server, together (each waits the whole window): unreachable.
            kill("-STOP", "remote_pid");
            let frozen = attempt("remote-frozen-forever", s("pub_id3"), vec![remote.clone()]);
            let dead = attempt("remote-down", format!("!gnobbi-x{}:{down}", s("run")), vec![down.clone()]);
            let ((fp, fend, ftook), (dp, dend, dtook)) = tokio::join!(frozen, dead);
            kill("-CONT", "remote_pid");
            expect(fp.iter().any(|p| p.1 == "Slow") && fend == JoinPhase::Unconfirmed && ftook >= JOIN_CONFIRM_WINDOW, format!("frozen: expected Slow then Unconfirmed at the window end, got {fp:?} {fend:?} {ftook:?}"));
            expect(dp.iter().any(|p| p.1 == "Slow") && dend == JoinPhase::Unconfirmed, format!("down: expected Slow then Unconfirmed, got {dp:?} {dend:?} {dtook:?}"));
            println!("NOTE remote-down phases {dp:?} final {dend:?} after {dtook:.1?}");

            // 5. Client offline: our own homeserver goes away (connection refused), the join target is fine.
            kill("-KILL", "home_pid");
            let (ph, end, took) = attempt("client-offline", s("pub_id4"), vec![remote.clone()]).await;
            expect(ph.iter().any(|p| p.1 == "Slow") && end == JoinPhase::Unconfirmed, format!("offline: expected Slow then Unconfirmed, got {ph:?} {end:?} {took:?}"));
            expect(*status.borrow() == Some(RoomSyncStatus::SyncFailed), format!("offline: own sync should read SyncFailed (the UI's `online=false`), is {}", name(*status.borrow())));
        };
        tokio::select! {
            _ = &mut run => panic!("engine stopped"),
            _ = drain => panic!("event channel closed"),
            r = tokio::time::timeout(Duration::from_secs(1500), steps) => r.expect("steps over time"),
        }
        let f = failures.borrow();
        assert!(f.is_empty(), "{} failed checks:\n{}", f.len(), f.join("\n"));
    });
}
