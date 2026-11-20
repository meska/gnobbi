//! Fixture for the account_cli_e2e script, not a test of its own: gives a disposable account an
//! identity, secret storage and key backup (what a user's first device has), and writes the
//! recovery key to SEED_OUT (0600). The CLI then logs in as a NEW, unverified device.
#![cfg(feature = "sdk")]
use matrix_engine::sdk::{Client, reqwest::Certificate};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

#[tokio::test]
#[ignore = "fixture for the account_cli_e2e script; needs a disposable homeserver"]
async fn seed_recovery() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let who = std::env::var("SEED_USER").unwrap();
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(format!("seed-{who}"));
    std::fs::create_dir_all(&dir).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sqlite_store(&dir, None)
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(&who, config[format!("{who}_password")].as_str().unwrap())
        .initial_device_display_name("seed")
        .send()
        .await
        .unwrap();
    client.encryption().bootstrap_cross_signing_if_needed(None).await.expect("bootstrap cross-signing");
    let key = client.encryption().recovery().enable().wait_for_backups_to_upload().await.expect("recovery enable");
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(std::env::var("SEED_OUT").unwrap()).unwrap();
    out.write_all(key.as_bytes()).unwrap();
}

/// Evidence for the `keys` bugs of `gnobbi account` (prints `PROBE ...` lines): what the SDK says about a
/// fresh, never-synced device, and about recovering twice with the same key.
#[tokio::test]
#[ignore = "fixture for the account_cli_e2e script; needs a disposable homeserver and SEED_KEY_FILE"]
async fn probe_recovery_state() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let who = std::env::var("SEED_USER").unwrap();
    let key = std::fs::read_to_string(std::env::var("SEED_KEY_FILE").unwrap()).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(format!("probe-{who}"));
    std::fs::create_dir_all(&dir).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sqlite_store(&dir, None)
        .build()
        .await
        .unwrap();
    client.matrix_auth().login_username(&who, config[format!("{who}_password")].as_str().unwrap()).initial_device_display_name("probe").send().await.unwrap();
    let rec = client.encryption().recovery();
    eprintln!("PROBE state before any sync/recover: {:?}", rec.state());
    eprintln!("PROBE recover #1: {:?}", rec.recover(key.trim()).await.map_err(|e| format!("{e:?}")));
    eprintln!("PROBE state after recover #1 (no sync): {:?}", rec.state());
    eprintln!("PROBE recover #2 (same client): {:?}", rec.recover(key.trim()).await.map_err(|e| format!("{e:?}")));
    let _ = client.sync_once(Default::default()).await;
    eprintln!("PROBE state after one sync: {:?}", rec.state());
}
