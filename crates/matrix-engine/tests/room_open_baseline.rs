//! Baseline timing of the history request an opening room makes (backward page of 25 through
//! the SDK, decryption included), on synthetic rooms behind a delayed proxy.
//! Driven by the room_open_baseline script inside a private namespace.
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_engine::sdk::{
    Client,
    reqwest::Certificate,
    room::MessagesOptions,
    ruma::{
        api::client::room::create_room::v3::Request as CreateRoom,
        events::{
            InitialStateEvent, room::encryption::RoomEncryptionEventContent,
            room::message::RoomMessageEventContent,
        },
    },
    sliding_sync::VersionBuilder,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn login(url: &str, ca: &[u8], user: &str, password: &str) -> Client {
    let client = Client::builder()
        .homeserver_url(url)
        .add_root_certificates(vec![Certificate::from_pem(ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::None)
        // Synapse advertises its direct address; keep the delayed route.
        .respect_login_well_known(false)
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(user, password)
        .send()
        .await
        .unwrap();
    client
}

fn keep_syncing(client: &Client) {
    let client = client.clone();
    tokio::spawn(async move {
        let _ = client.sync(Default::default()).await;
    });
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
#[ignore = "requires the room_open_baseline script"]
fn first_history_page_time_behind_simulated_latency() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(PathBuf::from(env("FIXTURE_AUTH_CONFIG"))).unwrap())
            .unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let direct = config["url"].as_str().unwrap().to_owned();
    let delayed = config["proxy_url"].as_str().unwrap().to_owned();
    let runs: usize = env("BASELINE_RUNS").parse().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let bob = login(&direct, &ca, "@bob:matrix-desktop.test", config["bob_password"].as_str().unwrap()).await;
        // The measured client talks only through the delayed proxy.
        let alice = login(&delayed, &ca, "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap()).await;
        keep_syncing(&bob);
        keep_syncing(&alice);
        // Calibration: a bare authenticated round trip through the delayed proxy.
        let mut rtt = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            alice.send(matrix_engine::sdk::ruma::api::client::discovery::get_supported_versions::Request::new()).await.unwrap();
            rtt.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        println!("CALIBRATION versions_ms {rtt:?} homeserver {} direct {}", alice.homeserver(), delayed);
        for (encrypted, count) in [(false, 50usize), (false, 500), (true, 50), (true, 500)] {
            let mut request = CreateRoom::new();
            request.invite = vec!["@alice:matrix-desktop.test".try_into().unwrap()];
            if encrypted {
                let state = InitialStateEvent::with_empty_state_key(
                    RoomEncryptionEventContent::with_recommended_defaults(),
                );
                request.initial_state = vec![state.to_raw_any()];
            }
            let room_id = bob.create_room(request).await.unwrap().room_id().to_owned();
            let alice_room = eventually("alice invite", || async {
                alice.get_room(&room_id)
            })
            .await;
            alice_room.join().await.unwrap();
            let bob_room = bob.get_room(&room_id).unwrap();
            // Bob must know alice's device before the first encrypted send.
            eventually("bob sees alice joined", || async {
                let members = bob_room.members(matrix_engine::sdk::RoomMemberships::JOIN).await.ok()?;
                (members.len() == 2).then_some(())
            })
            .await;
            let last = format!("last-{count}-{encrypted}");
            for n in 0..count {
                let body = if n + 1 == count { last.clone() } else { format!("m{n}") };
                bob_room.send(RoomMessageEventContent::text_plain(body)).await.unwrap();
            }
            let alice_room = alice.get_room(&room_id).unwrap();
            let mut samples = Vec::new();
            for run in 0..runs + 3 {
                let started = Instant::now();
                let mut options = MessagesOptions::backward();
                options.limit = 25u32.into();
                let page = alice_room.messages(options).await.unwrap();
                let first = page
                    .chunk
                    .iter()
                    .find_map(|e| e.raw().get_field::<serde_json::Value>("content").ok().flatten())
                    .and_then(|c| c["body"].as_str().map(str::to_owned));
                let elapsed = started.elapsed();
                // The newest message must be readable, or the timing measures nothing.
                if first.as_deref() != Some(last.as_str()) {
                    // Keys can lag behind the first sends: retry, never record.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    assert!(run < runs + 2, "newest message never decrypted");
                    continue;
                }
                samples.push(elapsed.as_secs_f64() * 1000.0);
            }
            assert!(samples.len() >= runs, "too few valid samples");
            let cold = samples[0];
            let mut warm: Vec<f64> = samples[1..].iter().copied().take(runs).collect();
            warm.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "RESULT {}",
                serde_json::json!({
                    "encrypted": encrypted, "messages": count, "samples": warm.len(),
                    "first_ms": cold, "median_ms": warm[warm.len() / 2],
                    "min_ms": warm[0], "max_ms": warm[warm.len() - 1],
                })
            );
        }
    });
}
