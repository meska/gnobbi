//! Export -> new device -> import round trip on a disposable homeserver (Synapse or Tuwunel).
//! Synthetic accounts only. Prints counts and outcomes (`KEYRT ...`), never passphrases, paths or bodies.
//! The exported file is also decrypted by an independent Python implementation of the Element
//! format (AES-CTR + HMAC-SHA256, PBKDF2-SHA512), standing in for a second standard client.
//! Driver (CT): PROBE_TARGET=key_export_roundtrip PROBE_TEST=key_export_roundtrip PROBE_FEATURES=sdk \
//!   PROBE_EVIDENCE=artifacts/key-export-rt PROBE_SERVER=synapse|tuwunel python3 the timeline_probe script
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_engine::{
    key_export::{KeyExportError, export_room_keys, import_room_keys},
    sdk::{
        Client, Room,
        encryption::{BackupDownloadStrategy, EncryptionSettings},
        reqwest::Certificate,
        ruma::{
            OwnedEventId, OwnedRoomId,
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{
                InitialStateEvent,
                room::{encryption::RoomEncryptionEventContent, message::RoomMessageEventContent},
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
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

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

async fn login(config: &serde_json::Value, who: &str, name: &str, dir: &std::path::Path) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let c = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .sqlite_store(dir, None)
        .with_encryption_settings(EncryptionSettings { backup_download_strategy: BackupDownloadStrategy::Manual, ..Default::default() })
        .build()
        .await
        .unwrap();
    c.matrix_auth()
        .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
        .initial_device_display_name(name)
        .send()
        .await
        .unwrap();
    c.event_cache().subscribe().unwrap();
    c
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

/// How many of the expected bodies the timeline shows decrypted, and how many events are UTD.
async fn readable(tl: &Timeline, bodies: &[String]) -> (usize, usize) {
    let items = tl.items().await;
    let ok = items
        .iter()
        .filter_map(|i| i.as_event())
        .filter(|e| e.content().as_message().is_some_and(|m| bodies.iter().any(|b| b == m.body())))
        .count();
    let utd = items.iter().filter_map(|i| i.as_event()).filter(|e| e.content().as_unable_to_decrypt().is_some()).count();
    (ok, utd)
}

async fn wait_readable(tl: &Timeline, bodies: &[String], secs: u64) -> (usize, usize) {
    let start = Instant::now();
    loop {
        let r = readable(tl, bodies).await;
        if r.0 == bodies.len() || start.elapsed() > Duration::from_secs(secs) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Independent decoder of the Element export format; prints `sessions,in_room` or `bad`.
fn python_decode(file: &std::path::Path, pass: &str, room: &str) -> String {
    const PY: &str = r#"
import sys, json, base64, hashlib, hmac
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
path, room = sys.argv[1], sys.argv[2]
pw = sys.stdin.read().encode()
t = open(path).read().strip().splitlines()
assert t[0] == "-----BEGIN MEGOLM SESSION DATA-----" and t[-1] == "-----END MEGOLM SESSION DATA-----"
b = "".join(t[1:-1]).rstrip("=")  # the SDK writes unpadded base64, as Element accepts
d = base64.b64decode(b + "=" * (-len(b) % 4))
assert d[0] == 1
salt, iv, rounds = d[1:17], d[17:33], int.from_bytes(d[33:37], "big")
ct, mac = d[37:-32], d[-32:]
k = hashlib.pbkdf2_hmac("sha512", pw, salt, rounds, 64)
if not hmac.compare_digest(hmac.new(k[32:], d[:-32], "sha256").digest(), mac):
    print("bad"); sys.exit(0)
iv = bytearray(iv); iv[8] &= 0x7f
dec = Cipher(algorithms.AES(k[:32]), modes.CTR(bytes(iv))).decryptor()
keys = json.loads(dec.update(ct) + dec.finalize())
ok = all(x.get("algorithm") == "m.megolm.v1.aes-sha2" and x.get("session_key") and x.get("sender_key") for x in keys)
print(f"{len(keys)},{sum(1 for x in keys if x['room_id'] == room)},{ok},{rounds}")
"#;
    let mut child = Command::new("artifacts/P015/venv/bin/python")
        .args(["-c", PY])
        .arg(file)
        .arg(room)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(pass.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    if out.status.success() { String::from_utf8_lossy(&out.stdout).trim().to_owned() } else { format!("python-error: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")) }
}

async fn run(config: &serde_json::Value) {
    let srv = config["kind"].as_str().unwrap().to_owned();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("stores-keyrt");
    let pass = format!("rt-{:x}-ünï", Instant::now().elapsed().as_nanos() ^ std::process::id() as u128 ^ 0x5a5a_1234_9876);
    let a = login(config, "alice", "A", &home.join("a")).await;
    let o = login(config, "bob", "O", &home.join("o")).await;

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
    until(40, async || (ro.joined_user_ids().await.map(|m| m.len()).unwrap_or(0) == 2).then_some(())).await.expect("2 members");

    // Two megolm sessions: one per sender, several messages each (bodies never printed).
    let mut bodies = Vec::new();
    let mut ids: Vec<OwnedEventId> = Vec::new();
    for (n, from) in [("o", &ro), ("o", &ro), ("o", &ro), ("a", &ra), ("a", &ra)] {
        let body = format!("keyrt-{n}-{}-ünï", bodies.len());
        ids.push(from.send(RoomMessageEventContent::text_plain(&body)).await.unwrap().response.event_id);
        bodies.push(body);
    }
    let ta = open(&ra).await;
    let (ok_a, utd_a) = wait_readable(&ta, &bodies, 40).await;
    eprintln!("KEYRT {srv} A readable={ok_a}/{} utd={utd_a}", bodies.len());
    assert_eq!(ok_a, bodies.len(), "device A must read all messages before the export");

    // 2. Export with a password.
    let out = home.join("export.txt");
    let scratch = home.join("export.pending");
    let count = export_room_keys(&a, &scratch, &out, &pass).await.expect("export");
    #[cfg(unix)]
    let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&out).unwrap().permissions()) & 0o777;
    #[cfg(not(unix))]
    let mode = 0;
    eprintln!("KEYRT {srv} export count={count} mode={mode:o} scratch_removed={} bytes>0={}", !scratch.exists(), std::fs::metadata(&out).unwrap().len() > 0);

    // 5. Independent decoder (standard format), right and wrong password.
    eprintln!("KEYRT {srv} independent-decoder right-password=<sessions,in_room,fields_ok,pbkdf2_rounds> {}", python_decode(&out, &pass, id.as_str()));
    eprintln!("KEYRT {srv} independent-decoder wrong-password {}", python_decode(&out, "not-the-password", id.as_str()));

    // 3. New device of the same user, no keys.
    let b = login(config, "alice", "B", &home.join("b")).await;
    let sb = SyncService::builder(b.clone()).build().await.unwrap();
    sb.start().await;
    let rb = get(&b).await;
    let tb = open(&rb).await;
    let (ok0, utd0) = wait_readable(&tb, &bodies, 10).await;
    eprintln!("KEYRT {srv} B before import readable={ok0}/{} utd={utd0}", bodies.len());

    // 4. Import: wrong password first, then the right one.
    let wrong = import_room_keys(&b, &out, "not-the-password").await;
    eprintln!("KEYRT {srv} B import wrong-password -> {wrong:?} (expect Err(WrongPassword))");
    assert_eq!(wrong, Err(KeyExportError::WrongPassword));
    let (imp, total) = import_room_keys(&b, &out, &pass).await.expect("import");
    eprintln!("KEYRT {srv} B import imported={imp} total={total}");
    let again = import_room_keys(&b, &out, &pass).await;
    eprintln!("KEYRT {srv} B import again -> {again:?}");

    // Same timeline (SDK retry), then a timeline rebuilt from a cleared cache.
    let (ok1, utd1) = wait_readable(&tb, &bodies, 20).await;
    eprintln!("KEYRT {srv} B after import, open timeline readable={ok1}/{} utd={utd1}", bodies.len());
    b.event_cache().clear_all_rooms().await.unwrap();
    let tb2 = open(&rb).await;
    let (ok2, utd2) = wait_readable(&tb2, &bodies, 20).await;
    eprintln!("KEYRT {srv} B after import, new timeline readable={ok2}/{} utd={utd2}", bodies.len());
    let mut fresh_ok = 0;
    for id in &ids {
        use matrix_engine::sdk::deserialized_responses::TimelineEventKind as K;
        if matches!(rb.event(id, None).await, Ok(e) if !matches!(e.kind, K::UnableToDecrypt { .. })) {
            fresh_ok += 1;
        }
    }
    eprintln!("KEYRT {srv} B after import, fresh fetch decrypted={fresh_ok}/{}", ids.len());
    for s in [sa, so, sb] {
        s.stop().await;
    }
    assert_eq!(ok2, bodies.len(), "all messages readable on the new device after import");
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn key_export_roundtrip() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(run(&config));
}
