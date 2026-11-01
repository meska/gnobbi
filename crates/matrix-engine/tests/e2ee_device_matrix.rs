//! New-device decryption matrix on a disposable homeserver (Synapse or Tuwunel), synthetic accounts only.
//! Not pass/fail on the crypto outcome: it RECORDS what the official SDK does and prints `E2EE ...` lines
//! (state of both devices + which messages are readable) so the result can be compared across servers.
//! The test fails only when the fixture itself breaks. Driver:
//! PROBE_TARGET=e2ee_device_matrix PROBE_TEST=e2ee_with_backup|e2ee_with_backup_oneshot|e2ee_with_backup_after_failure|e2ee_no_backup PROBE_FEATURES=sdk \
//!   PROBE_EVIDENCE=artifacts/e2ee-matrix PROBE_SERVER=synapse|tuwunel python3 the timeline_probe script
//! Per case: receiver R has device A (cross-signed, optional backup) and two fresh devices B1/B2 created after
//! the "pre" messages and before the "post" ones. Another account O is the sender (stands in for a bridge ghost).
//!   B1: unverified -> verified by A (device cross-signing, no recovery key)
//!   B2: unverified -> recovery key (secret storage import + backup download)
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_engine::{
    sdk::{
        Client, Room,
        encryption::{BackupDownloadStrategy, EncryptionSettings},
        event_cache::DecryptionRetryRequest,
        reqwest::Certificate,
        ruma::{
            OwnedEventId, OwnedRoomId,
            api::client::room::create_room::v3::{
                CreationContent, Request as CreateRoomRequest, RoomPreset,
            },
            assign,
            events::{
                InitialStateEvent,
                room::{
                    encryption::RoomEncryptionEventContent, message::RoomMessageEventContent,
                },
            },
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::{
        sync_service::SyncService,
        timeline::{RoomExt, Timeline},
    },
};
use std::time::{Duration, Instant};

async fn until<T>(secs: u64, mut f: impl AsyncFnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return Some(v);
        }
        if start.elapsed() > Duration::from_secs(secs) {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// E2EE_APP=old reproduces the client of aa6a596 (event cache subscribed BEFORE login, per-case strategy);
/// anything else is the client of main >= 624baa4 (AfterDecryptionFailure, event cache subscribed AFTER login).
fn app_old() -> bool {
    std::env::var("E2EE_APP").is_ok_and(|v| v == "old")
}

async fn login(config: &serde_json::Value, who: &str, name: &str, dir: &std::path::Path, strategy: BackupDownloadStrategy) -> Client {
    let strategy = if app_old() || name.len() == 1 { strategy } else { BackupDownloadStrategy::AfterDecryptionFailure };
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let c = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .sqlite_store(dir, None)
        .with_encryption_settings(EncryptionSettings { backup_download_strategy: strategy, ..Default::default() })
        .build()
        .await
        .unwrap();
    if app_old() {
        c.event_cache().subscribe().unwrap();
    }
    c.matrix_auth()
        .login_username(
            config[format!("{who}_mxid")].as_str().unwrap(),
            config[format!("{who}_password")].as_str().unwrap(),
        )
        .initial_device_display_name(name)
        .send()
        .await
        .unwrap();
    if !app_old() {
        c.event_cache().subscribe().unwrap();
    }
    c
}

/// Device state as the SDK reports it on that device itself.
async fn state(c: &Client) -> String {
    let e = c.encryption();
    let cs = e.cross_signing_status().await;
    let own = e.get_own_device().await.ok().flatten();
    let me = c.user_id().unwrap();
    let identity_verified = e.get_user_identity(me).await.ok().flatten().map(|i| i.is_verified());
    format!(
        "identity_trusted_here={identity_verified:?} xsign_private={:?} own_signed_by_owner={:?} own_verified={:?} backup_enabled={} backup_state={:?} recovery={:?}",
        cs.map(|s| s.is_complete()),
        own.as_ref().map(|d| d.is_cross_signed_by_owner()),
        own.as_ref().map(|d| d.is_verified()),
        e.backups().are_enabled().await,
        e.backups().state(),
        e.recovery().state(),
    )
}

/// `label=ok|UTD(cause)|absent` for every message we sent, in send order.
async fn view(tl: &Timeline, sent: &[(OwnedEventId, &str)]) -> (String, bool) {
    let items = tl.items().await;
    let mut all = true;
    let parts: Vec<String> = sent
        .iter()
        .map(|(id, label)| {
            let ev = items.iter().filter_map(|i| i.as_event()).find(|e| e.event_id() == Some(id));
            let v = match ev {
                None => "absent".to_owned(),
                Some(e) => match e.content().as_unable_to_decrypt() {
                    Some(m) => utd_short(m),
                    None => "ok".to_owned(),
                },
            };
            all &= v == "ok";
            format!("{label}={v}")
        })
        .collect();
    (parts.join(" "), all)
}

/// `readable/total` of the sent messages: the number the matrix document reports.
fn count(v: &str) -> String {
    let n = v.split(' ').count();
    format!("{}/{n}", v.split(' ').filter(|p| p.ends_with("=ok")).count())
}

fn utd_short(m: &matrix_engine::sdk_ui::timeline::EncryptedMessage) -> String {
    use matrix_engine::sdk_ui::timeline::EncryptedMessage as M;
    match m {
        M::MegolmV1AesSha2 { session_id, cause, .. } => format!("UTD({cause:?},s={})", &session_id[..6]),
        _ => "UTD(other)".into(),
    }
}

async fn open(room: &Room) -> Timeline {
    let tl = room.timeline_builder().build().await.unwrap();
    for _ in 0..10 {
        if tl.paginate_backwards(50).await.unwrap() {
            break;
        }
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    tl
}

/// Poll until every message is readable or `secs` pass. Prints the view and the time it took.
async fn watch(srv: &str, case: &str, step: &str, tl: &Timeline, sent: &[(OwnedEventId, &str)], secs: u64) {
    let start = Instant::now();
    let (mut v, mut done) = view(tl, sent).await;
    while !done && start.elapsed() < Duration::from_secs(secs) {
        tokio::time::sleep(Duration::from_millis(500)).await;
        (v, done) = view(tl, sent).await;
    }
    eprintln!("E2EE {srv} {case} {step} all_readable={done} readable={} after={:.1}s | {v}", count(&v), start.elapsed().as_secs_f32());
}

/// Ground truth independent of the live timeline: re-fetch each event and let the SDK decrypt it now.
async fn fresh(srv: &str, case: &str, step: &str, room: &Room, sent: &[(OwnedEventId, &str)]) {
    use matrix_engine::sdk::deserialized_responses::TimelineEventKind as K;
    let mut parts = Vec::new();
    for (id, label) in sent {
        let v = match room.event(id, None).await {
            Ok(e) if matches!(e.kind, K::UnableToDecrypt { .. }) => "UTD",
            Ok(_) => "ok",
            Err(_) => "err",
        };
        parts.push(format!("{label}={v}"));
    }
    eprintln!("E2EE {srv} {case} {step} FRESH-FETCH readable={} | {}", count(&parts.join(" ")), parts.join(" "));
}

async fn run_case(config: &serde_json::Value, srv: &str, case: &str, recv: &str, other: &str, backup: bool, strategy: BackupDownloadStrategy) {
    let strategy = if app_old() { strategy } else { BackupDownloadStrategy::AfterDecryptionFailure };
    eprintln!("E2EE {srv} {case} app={} B devices use backup_download_strategy={strategy:?}", if app_old() { "old" } else { "new" });
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(format!("stores-{case}"));
    let a = login(config, recv, "A", &home.join("a"), BackupDownloadStrategy::Manual).await;
    let o = login(config, other, "O", &home.join("o"), BackupDownloadStrategy::Manual).await;
    let mut recovery_key = None;
    for c in [&a, &o] {
        c.encryption().bootstrap_cross_signing_if_needed(None).await.expect("bootstrap cross-signing");
        if backup && std::ptr::eq(c, &a) {
            recovery_key = Some(
                c.encryption().recovery().enable().wait_for_backups_to_upload().await.expect("recovery enable"),
            );
        }
    }
    eprintln!("E2EE {srv} {case} setup A: {}", state(&a).await);

    let mut creation = CreationContent::new();
    creation.federate = false;
    let req = assign!(CreateRoomRequest::new(), {
        preset: Some(RoomPreset::PrivateChat),
        invite: vec![a.user_id().unwrap().to_owned()],
        creation_content: Some(Raw::new(&creation).unwrap()),
        initial_state: vec![InitialStateEvent::with_empty_state_key(RoomEncryptionEventContent::with_recommended_defaults()).to_raw_any()],
    });
    let room_o = o.create_room(req).await.unwrap();
    let id: OwnedRoomId = room_o.room_id().to_owned();
    a.join_room_by_id(&id).await.unwrap();
    let (sa, so) = (SyncService::builder(a.clone()).build().await.unwrap(), SyncService::builder(o.clone()).build().await.unwrap());
    sa.start().await;
    so.start().await;
    let get = async |c: &Client| until(40, async || c.get_room(&id)).await.expect("room");
    let (ra, ro) = (get(&a).await, get(&o).await);
    // Both members must be known before sending, or the sender shares the key with nobody.
    until(40, async || (ro.joined_user_ids().await.map(|m| m.len()).unwrap_or(0) == 2).then_some(())).await.expect("2 members");

    let mut sent: Vec<(OwnedEventId, &str)> = Vec::new();
    for (label, from) in [("pre-O1", &ro), ("pre-O2", &ro), ("pre-A1", &ra)] {
        sent.push((from.send(RoomMessageEventContent::text_plain(label)).await.unwrap().response.event_id, label));
    }
    if backup {
        a.encryption().backups().wait_for_steady_state().await.unwrap();
        eprintln!("E2EE {srv} {case} pre-messages uploaded to backup by A");
    }

    // Two fresh devices of R, created after the pre messages.
    let b1 = login(config, recv, "B1", &home.join("b1"), strategy).await;
    let b2 = login(config, recv, "B2", &home.join("b2"), strategy).await;
    let (sb1, sb2) = (SyncService::builder(b1.clone()).build().await.unwrap(), SyncService::builder(b2.clone()).build().await.unwrap());
    sb1.start().await;
    sb2.start().await;
    let (rb1, rb2) = (get(&b1).await, get(&b2).await);
    // Let the senders learn about the new devices (device-list update through sync) before sending "post".
    tokio::time::sleep(Duration::from_secs(5)).await;
    for (label, from) in [("post-O1", &ro), ("post-A1", &ra)] {
        sent.push((from.send(RoomMessageEventContent::text_plain(label)).await.unwrap().response.event_id, label));
    }
    let (t1, t2) = (open(&rb1).await, open(&rb2).await);
    for (n, c, t) in [("B1", &b1, &t1), ("B2", &b2, &t2)] {
        eprintln!("E2EE {srv} {case} {n} unverified-state: {}", state(c).await);
        watch(srv, case, &format!("{n} unverified (A/C-deferred)"), t, &sent, 20).await;
    }

    fresh(srv, case, "B1 unverified", &rb1, &sent).await;
    // B1: device cross-signing by A only (no recovery key, no backup access).
    let dev = until(30, async || a.encryption().get_device(b1.user_id().unwrap(), b1.device_id().unwrap()).await.ok().flatten()).await.expect("A sees B1");
    dev.verify().await.expect("A cross-signs B1");
    tokio::time::sleep(Duration::from_secs(3)).await;
    eprintln!("E2EE {srv} {case} B1 after-A-verifies-it: {}", state(&b1).await);
    watch(srv, case, "B1 verified-by-A, nothing else", &t1, &sent, 30).await;
    fresh(srv, case, "B1 verified-by-A", &rb1, &sent).await;

    // B2: recovery key (cross-signing secrets + backup key), then maybe explicit per-room download.
    match &recovery_key {
        Some(k) => {
            let r = b2.encryption().recovery().recover(k).await;
            eprintln!("E2EE {srv} {case} B2 recover()={r:?}: {}", state(&b2).await);
            watch(srv, case, "B2 after recover(), no explicit download", &t2, &sent, 30).await;
            fresh(srv, case, "B2 after recover()", &rb2, &sent).await;
            let d = b2.encryption().backups().download_room_keys_for_room(&id).await;
            eprintln!("E2EE {srv} {case} B2 download_room_keys_for_room={d:?}");
            watch(srv, case, "B2 after explicit backup download (old timeline)", &t2, &sent, 30).await;
            fresh(srv, case, "B2 after explicit download", &rb2, &sent).await;
            // Ask the event cache to retry exactly the UTD sessions (the SDK's own API for "keys changed").
            let utd_sessions = t2.items().await.iter().filter_map(|i| i.as_event()).filter_map(|e| match e.content().as_unable_to_decrypt() {
                Some(matrix_engine::sdk_ui::timeline::EncryptedMessage::MegolmV1AesSha2 { session_id, .. }) => Some(session_id.as_str().into()),
                _ => None,
            }).collect();
            b2.event_cache().request_decryption(DecryptionRetryRequest { room_id: id.clone(), utd_session_ids: utd_sessions, refresh_info_session_ids: Default::default() });
            watch(srv, case, "B2 old timeline after request_decryption", &t2, &sent, 15).await;
            // Workaround probe: dropping the cached (still undecrypted) events forces a re-fetch + decrypt.
            b2.event_cache().clear_all_rooms().await.unwrap();
            let t2c = open(&rb2).await;
            watch(srv, case, "B2 NEW timeline after event_cache.clear_all_rooms()", &t2c, &sent, 10).await;
            // Does a timeline opened now (cache already holds the keys) show the messages?
            let t2b = open(&rb2).await;
            watch(srv, case, "B2 NEW timeline after explicit download", &t2b, &sent, 10).await;
            // (d) app restart: B2 from its store only, session restored, event cache subscribed after restore, cache NOT cleared.
            let session = b2.matrix_auth().session().expect("B2 session");
            sb2.stop().await;
            drop((t2, t2b, t2c, rb2));
            drop(b2);
            let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
            let b2r = Client::builder()
                .homeserver_url(config["url"].as_str().unwrap())
                .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                .sqlite_store(home.join("b2"), None)
                .with_encryption_settings(EncryptionSettings { backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure, ..Default::default() })
                .build()
                .await
                .unwrap();
            b2r.matrix_auth().restore_session(session, matrix_engine::sdk::store::RoomLoadSettings::default()).await.unwrap();
            b2r.event_cache().subscribe().unwrap();
            let sbr = SyncService::builder(b2r.clone()).build().await.unwrap();
            sbr.start().await;
            let rbr = until(40, async || b2r.get_room(&id)).await.expect("room after restart");
            let tr = open(&rbr).await;
            watch(srv, case, "(d) B2 after app restart, timeline opened from the store", &tr, &sent, 20).await;
            sbr.stop().await;
        }
        None => eprintln!("E2EE {srv} {case} B2 no recovery key exists (no secret storage, no backup): n/a"),
    }
    for s in [sa, so, sb1, sb2] {
        s.stop().await;
    }
}

/// One case per test (= per fresh homeserver): an identity left on the server by a previous case would
/// exist without private keys on the new login.
fn case(name: &str, backup: bool, strategy: BackupDownloadStrategy) {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let srv = config["kind"].as_str().unwrap().to_owned();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(run_case(&config, &srv, name, "alice", "bob", backup, strategy));
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn e2ee_with_backup() {
    case("with-backup", true, BackupDownloadStrategy::Manual);
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn e2ee_with_backup_oneshot() {
    case("with-backup-oneshot", true, BackupDownloadStrategy::OneShot);
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn e2ee_with_backup_after_failure() {
    case("with-backup-afterfail", true, BackupDownloadStrategy::AfterDecryptionFailure);
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn e2ee_no_backup() {
    case("no-backup", false, BackupDownloadStrategy::Manual);
}
