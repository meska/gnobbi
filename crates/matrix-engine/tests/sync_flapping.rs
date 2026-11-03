//! Flapping of the sync status (Ready -> SyncFailed -> Ready) against a scriptable fake homeserver,
//! with a paused clock (outages: the real clock, see `observe`). Each test records the
//! `RoomSyncStatus` sequence that `sliding::run` emits (with seconds) and pins the hysteresis:
//! no «sync failed» for a short outage or a sync that keeps answering; a mute server is caught by
//! the SDK alone (no probe of ours).
//! Run: the ct script 'cargo test --locked -p matrix-engine --features sdk --test sync_flapping -- --nocapture'
#![recursion_limit = "512"]
#[path = "support/fake_homeserver.rs"]
mod fake_homeserver;

use fake_homeserver::{Fake, SyncMode};
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListEvent, RoomSyncStatus, bounded,
    room_sync::RoomSync,
    sdk::{Client, sliding_sync::VersionBuilder},
    sliding,
};
use std::{future::Future, num::NonZeroUsize, time::Duration};

const S: Duration = Duration::from_secs(1);

fn name(status: RoomSyncStatus) -> &'static str {
    match status {
        RoomSyncStatus::Connecting => "Connecting",
        RoomSyncStatus::Ready => "Ready",
        RoomSyncStatus::AuthenticationFailed => "AuthenticationFailed",
        RoomSyncStatus::SyncFailed => "SyncFailed",
    }
}

/// Runs the adapter against `fake` for `window` virtual time while `script` drives the faults;
/// returns the statuses as (whole seconds since start, status).
fn observe<F, Fut>(window: u64, real: bool, script: F) -> (Vec<(u64, &'static str)>, Fake)
where
    F: FnOnce(Fake) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let fake = Fake::start().await;
        let client = Client::builder()
            .homeserver_url(&fake.url)
            .sliding_sync_version_builder(VersionBuilder::None)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username("@a:fake.test", "x")
            .send()
            .await
            .unwrap();
        // After the real-time setup: the clock only auto-advances once the adapter runs.
        // Real clock for outages: with a paused one the SDK's offline check times out virtually.
        if !real {
            tokio::time::pause();
        }
        let mut sync = RoomSync::from_client(AccountId::new(1), client);
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(64).unwrap());
        let started = tokio::time::Instant::now();
        tokio::spawn(script(fake.clone()));
        let mut seen = Vec::new();
        let run = sliding::run(&mut sync, &mut tx, || {});
        let collect = async {
            while let Some(event) = rx.recv().await {
                if let RoomListEvent::Status(status) = event {
                    seen.push((started.elapsed().as_secs(), name(status)));
                }
            }
        };
        tokio::select! {
            _ = run => {},
            _ = collect => {},
            _ = tokio::time::sleep(Duration::from_secs(window)) => {},
        }
        // `run` may end (auth failed) with its last event still queued.
        while let Ok(event) = rx.try_recv() {
            if let RoomListEvent::Status(status) = event {
                seen.push((started.elapsed().as_secs(), name(status)));
            }
        }
        (seen, fake)
    })
}

fn show(name: &str, seen: &[(u64, &str)], fake: &Fake) {
    println!("== {name}: {seen:?}");
    if std::env::var_os("FAKE_LOG").is_some() {
        for (at, line) in fake.log() {
            println!("   fake {:>4}s {line}", at.as_secs());
        }
    }
}

fn names<'a>(seen: &[(u64, &'a str)]) -> Vec<&'a str> {
    seen.iter().map(|(_, n)| *n).collect()
}

fn times(seen: &[(u64, &str)], status: &str) -> Vec<u64> {
    seen.iter()
        .filter(|(_, n)| *n == status)
        .map(|(t, _)| *t)
        .collect()
}

/// H2: a proxy that cuts the sliding long-poll after 60 s or 100 s, so no answer ever arrives:
/// the SDK gives up (Offline) after ~181 s, «sync failed» shows 20 s later and stays (no flapping).
#[test]
fn h2_long_poll_closed_by_a_proxy_shows_one_stable_failure() {
    for cut in [60, 100] {
        let (seen, fake) = observe(5 * cut, false, move |f| async move {
            f.set_sync(SyncMode::DropAfter(Duration::from_secs(cut)));
        });
        show(&format!("H2 cut after {cut}s"), &seen, &fake);
        assert_eq!(names(&seen), ["Ready", "SyncFailed"], "{seen:?}");
        assert!(
            (200..=205).contains(&times(&seen, "SyncFailed")[0]),
            "{seen:?}"
        );
    }
}

/// H3: the server answers 500 for 30 s: one «sync failed» after the third failed episode (not one
/// per retry), Ready once the server answers again. The SDK's restart backoff is random: the third
/// episode lands at ~15 s, or not before the 20 s rule fires (~31 s); both are one failure.
#[test]
fn h3_server_error_in_sliding_sync_shows_one_failure() {
    let (seen, fake) = observe(120, false, |f| async move {
        tokio::time::sleep(10 * S).await;
        f.set_sync(SyncMode::Status(500));
        tokio::time::sleep(30 * S).await;
        f.set_sync(SyncMode::Normal);
    });
    show("H3", &seen, &fake);
    assert_eq!(names(&seen), ["Ready", "SyncFailed", "Ready"], "{seen:?}");
    assert!(
        (12..=32).contains(&times(&seen, "SyncFailed")[0]),
        "{seen:?}"
    );
    assert!(times(&seen, "Ready")[1] >= 40, "{seen:?}");
}

/// H4: a mute server (connection open, nothing ever answered): no probe of ours, the SDK gives up
/// after ~181 s of held long-polls, «sync failed» shows 20 s later and stays.
#[test]
fn h4_mute_server_is_detected_by_the_sdk_alone() {
    let (seen, fake) = observe(300, false, |f| async move {
        tokio::time::sleep(10 * S).await;
        f.set_mute(true);
    });
    show("H4", &seen, &fake);
    assert_eq!(names(&seen), ["Ready", "SyncFailed"], "{seen:?}");
    let at = times(&seen, "SyncFailed")[0];
    println!(
        "== mute server: SyncFailed shown {} s after it went mute",
        at - 10
    );
    assert!((200..=235).contains(&at), "{seen:?}");
    assert_eq!(fake.count("whoami"), 0, "no whoami probe");
}

/// H5: an expired token (401 M_UNKNOWN_TOKEN on sliding sync) ends in AuthenticationFailed at
/// once, with no retry and no whoami.
#[test]
fn h5_expired_token_reports_authentication_failed() {
    let (seen, fake) = observe(60, false, |f| async move {
        tokio::time::sleep(10 * S).await;
        f.set_sync(SyncMode::Status(401));
    });
    show("H5", &seen, &fake);
    assert_eq!(names(&seen), ["Ready", "AuthenticationFailed"], "{seen:?}");
    assert_eq!(fake.count("whoami"), 0);
}

/// Network outage (connections closed unanswered), real clock: `secs` down from second 10.
fn outage(secs: u64) -> (Vec<(u64, &'static str)>, Fake) {
    observe(secs + 45, true, move |f| async move {
        tokio::time::sleep(10 * S).await;
        f.set_outage(true);
        tokio::time::sleep(Duration::from_secs(secs)).await;
        f.set_outage(false);
    })
}

#[test]
fn network_outage_3s_shows_nothing() {
    let (seen, fake) = outage(3);
    show("outage 3s", &seen, &fake);
    assert_eq!(names(&seen), ["Ready"]);
}

/// One failure after 20 s of outage (second ~30), Ready once the network is back.
#[test]
fn network_outage_40s_shows_one_failure() {
    let (seen, fake) = outage(40);
    show("outage 40s", &seen, &fake);
    assert_eq!(names(&seen), ["Ready", "SyncFailed", "Ready"], "{seen:?}");
    assert!(
        (29..=33).contains(&times(&seen, "SyncFailed")[0]),
        "{seen:?}"
    );
}

/// Offline from the start: one failure after 20 s, Ready once the network is back at second 30.
#[test]
fn offline_start_shows_one_failure_then_ready() {
    let (seen, fake) = observe(80, true, |f| async move {
        f.set_outage(true);
        tokio::time::sleep(Duration::from_secs(30)).await;
        f.set_outage(false);
    });
    show("offline start", &seen, &fake);
    assert_eq!(names(&seen), ["SyncFailed", "Ready"], "{seen:?}");
    assert!((18..=23).contains(&times(&seen, "SyncFailed")[0]), "{seen:?}");
    assert!((28..=34).contains(&times(&seen, "Ready")[0]), "{seen:?}");
}
