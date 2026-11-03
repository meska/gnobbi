//! Observer for the outage e2e (driven by the sync_outage_probe script): runs the sliding adapter against a
//! disposable homeserver reached through a controllable proxy and prints one line per status change,
//! `EV <ms since start> <Status>`. Nothing else is printed (no token, no content).
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListEvent, RoomSyncStatus, bounded,
    room_sync::RoomSync,
    sdk::{Client, reqwest::Certificate, sliding_sync::VersionBuilder},
    sliding,
};
use std::{io::Write, num::NonZeroUsize, time::Duration};

#[test]
#[ignore = "requires the sync_outage_probe script"]
fn observe_status_through_the_proxy() {
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
    }
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let secs: u64 = std::env::var("OBSERVE_SECS").unwrap().parse().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client = Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
            // Synapse's login well-known would send the client around the proxy.
            .respect_login_well_known(false)
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .unwrap();
        client
            .matrix_auth()
            .login_username(
                config["alice_mxid"].as_str().unwrap(),
                config["alice_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let mut sync = RoomSync::from_client(AccountId::new(1), client);
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(64).unwrap());
        let started = std::time::Instant::now();
        println!("START");
        let run = sliding::run(&mut sync, &mut tx, || {});
        let collect = async {
            while let Some(event) = rx.recv().await {
                if let RoomListEvent::Status(status) = event {
                    let name = match status {
                        RoomSyncStatus::Connecting => "Connecting",
                        RoomSyncStatus::Ready => "Ready",
                        RoomSyncStatus::AuthenticationFailed => "AuthenticationFailed",
                        RoomSyncStatus::SyncFailed => "SyncFailed",
                    };
                    println!("EV {} {name}", started.elapsed().as_millis());
                    std::io::stdout().flush().unwrap();
                }
            }
        };
        tokio::select! {
            _ = run => {},
            _ = collect => {},
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {},
        }
    });
}
