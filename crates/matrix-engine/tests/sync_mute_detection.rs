//! diagnostic: how long does the SDK ALONE take to notice a mute server
//! (TCP open, request read, never any answer)? `SyncService::with_offline_mode`, no whoami
//! probe of ours, real clock. Prints the state timeline + fake request log; the only pin is
//! that `Offline` is reached inside the window. Run:
//! the ct script 'cargo test --offline -p matrix-engine --features sdk --test sync_mute_detection -- --nocapture --test-threads=1'
//! Env: MUTE_WINDOW_SECS (default 260), MUTE_AT_SECS (default 10).
#[path = "support/fake_homeserver.rs"]
mod fake_homeserver;

use fake_homeserver::Fake;
use matrix_engine::sdk_ui::sync_service::SyncService;
use matrix_engine::sdk::{Client, sliding_sync::{Version, VersionBuilder}};
use std::time::{Duration, Instant};

fn secs(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[tokio::test(flavor = "current_thread")]
async fn sdk_only_detection_of_a_mute_server() {
    let (window, mute_at) = (
        secs("MUTE_WINDOW_SECS", 260),
        secs("MUTE_AT_SECS", 10),
    );
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
    client.set_sliding_sync_version(Version::Native);

    let service = SyncService::builder(client)
        .with_offline_mode()
        .build()
        .await
        .unwrap();
    let mut state = service.state();
    let t0 = Instant::now();
    service.start().await;

    let mut transitions: Vec<(f64, String)> = vec![(0.0, format!("{:?}", state.get()))];
    let mut muted = false;
    loop {
        let elapsed = t0.elapsed().as_secs_f64();
        if elapsed > window as f64 {
            break;
        }
        if !muted && elapsed >= mute_at as f64 {
            fake.set_mute(true);
            muted = true;
            println!("== mute server ON at {elapsed:.1}s");
        }
        tokio::select! {
            next = state.next() => {
                if let Some(next) = next {
                    transitions.push((t0.elapsed().as_secs_f64(), format!("{next:?}")));
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    println!("== SyncService state transitions (real seconds since start):");
    for (at, state) in &transitions {
        println!("   {at:8.1}s  {state}");
    }
    println!("== fake request log (last 30):");
    for (at, line) in fake.log().iter().rev().take(30).rev() {
        println!("   fake {:>6.1}s {line}", at.as_secs_f64());
    }
    let offline_at = transitions
        .iter()
        .find(|(_, s)| s.contains("Offline"))
        .map(|(at, _)| *at);
    let Some(offline_at) = offline_at else {
        panic!("SyncService never left for Offline within {window}s — mute server undetected");
    };
    println!(
        "== DETECTED: Offline at {offline_at:.1}s, i.e. {:.1}s after the server went mute",
        offline_at - mute_at as f64
    );
}
