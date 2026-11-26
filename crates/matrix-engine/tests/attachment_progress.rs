//! The progress of a real upload through a rate-limited link, plain and encrypted room, on a disposable homeserver:
//! logs time and bytes of every update the SDK send queue emits and of every value the upload card would read
//! (`uploads()`), and checks the card sees the bytes go out while the file is in flight. Synthetic accounts only.
//! Driver: PROBE_TARGET=attachment_progress PROBE_SERVER=synapse|tuwunel UP_RATE=<bytes/s> python3 tools/attachment_send.py (via ct.sh).
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    TestOutbox,
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            OwnedRoomId, assign,
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            serde::Raw,
        },
        send_queue::RoomSendQueueUpdate,
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
    sending::{Attachment, SendEvent, SendRequest, fresh_transaction_id},
};
use serde_json::Value;
use std::{sync::{Arc, atomic::{AtomicU64, Ordering::Relaxed}}, time::{Duration, Instant}};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{TcpSocket, TcpStream}};

const FILE_BYTES: usize = 30 << 20;

fn acc() -> AccountId {
    AccountId::new(1)
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

/// A TCP forwarder that caps the client→server rate; TLS passes through untouched. Returns its port.
/// `through` counts the bytes the server side has actually been given. The receive buffer is small, so the kernel
/// does not swallow the file ahead of the link.
async fn slow_link(target_port: u16, rate: u64, through: Arc<AtomicU64>) -> u16 {
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(32 * 1024).unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = socket.listen(128).unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else { return };
            let Ok(server) = TcpStream::connect(("localhost", target_port)).await else { continue };
            let ((mut cr, mut cw), (mut sr, mut sw)) = (client.into_split(), server.into_split());
            let through = through.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 16384];
                while let Ok(n) = cr.read(&mut buf).await {
                    if n == 0 || sw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    through.fetch_add(n as u64, Relaxed);
                    tokio::time::sleep(Duration::from_secs_f64(n as f64 / rate as f64)).await;
                }
            });
            tokio::spawn(async move {
                let _ = tokio::io::copy(&mut sr, &mut cw).await;
            });
        }
    });
    port
}

async fn login(url: &str, ca: &[u8], mxid: &str, password: &str) -> Client {
    let c = Client::builder()
        .homeserver_url(url)
        .add_root_certificates(vec![Certificate::from_pem(ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .build()
        .await
        .unwrap();
    c.matrix_auth().login_username(mxid, password).send().await.unwrap();
    c
}

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(90), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn room_pair(a: &Client, b: &Client, encrypted: bool) -> (Room, OwnedRoomId) {
    let mut creation = CreationContent::new();
    creation.federate = false;
    let id = a
        .create_room(assign!(CreateRoomRequest::new(), {
            preset: Some(RoomPreset::PrivateChat),
            invite: vec![b.user_id().unwrap().to_owned()],
            creation_content: Some(Raw::new(&creation).unwrap()),
        }))
        .await
        .unwrap()
        .room_id()
        .to_owned();
    until("bob sees the invite", || b.invited_rooms().iter().any(|r| r.room_id() == id)).await;
    b.get_room(&id).unwrap().join().await.unwrap();
    until("alice has the room", || a.get_room(&id).is_some()).await;
    let (ra, rb) = (a.get_room(&id).unwrap(), b.get_room(&id).unwrap());
    if encrypted {
        ra.enable_encryption().await.unwrap();
        let start = Instant::now();
        loop {
            let both = ra.latest_encryption_state().await.is_ok_and(|s| s.is_encrypted()) && rb.latest_encryption_state().await.is_ok_and(|s| s.is_encrypted());
            if both {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(60), "timeout: room encryption state");
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    (ra, id)
}

#[test]
#[ignore = "requires the attachment_send script"]
fn upload_progress_plain_and_encrypted() {
    let config: Value = serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap();
    let server_port: u16 = url.trim_end_matches('/').rsplit(':').next().unwrap().parse().unwrap();
    let rate: u64 = std::env::var("UP_RATE").ok().and_then(|v| v.parse().ok()).unwrap_or(2_000_000);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        // Alice reaches the server through the slow link; bob directly.
        let through = Arc::new(AtomicU64::new(0));
        let slow = format!("https://localhost:{}/", slow_link(server_port, rate, through.clone()).await);
        // The login response carries the server's own address and the SDK adopts it, which would bypass the link:
        // log in directly, then restore the same session (same encrypted sqlite store, like the app's) on a client
        // that talks to the slow link.
        let store = std::env::temp_dir().join("attachment-progress-store");
        let build = |homeserver: &str| {
            let (store, ca) = (store.clone(), ca.clone());
            let homeserver = homeserver.to_owned();
            async move {
                Client::builder()
                    .homeserver_url(homeserver)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .sqlite_store(store, Some("probe-store-passphrase"))
                    .build()
                    .await
                    .unwrap()
            }
        };
        let direct = build(url).await;
        direct.matrix_auth().login_username(config["alice_mxid"].as_str().unwrap(), config["alice_password"].as_str().unwrap()).send().await.unwrap();
        let session = direct.matrix_auth().session().unwrap();
        drop(direct);
        let a = build(&slow).await;
        a.matrix_auth().restore_session(session, matrix_engine::sdk::store::RoomLoadSettings::default()).await.unwrap();
        let b = login(url, &ca, config["bob_mxid"].as_str().unwrap(), config["bob_password"].as_str().unwrap()).await;
        let mut services = Vec::new();
        for c in [&a, &b] {
            let s = SyncService::builder(c.clone()).build().await.unwrap();
            s.start().await;
            services.push(s);
        }
        for encrypted in [false, true] {
            let (room, id) = room_pair(&a, &b, encrypted).await;
            let scope = RoomScope { account_id: acc(), room_id: RoomId::new(id.as_str()) };
            let request = SendRequest {
                scope,
                transaction_id: fresh_transaction_id(),
                body: String::new(),
                attachment: Some(Attachment { filename: "grande.bin".into(), data: (0..FILE_BYTES).map(|i| (i.wrapping_mul(31) % 251) as u8).collect(), ..Default::default() }),
                poll: None,
                mentions: Vec::new(),
            };
            let txn = request.transaction_id.clone();
            let outbox = TestOutbox::new(a.clone(), acc());
            // Subscribe before the send, like the app does, to see what the queue says and when.
            let (_, mut raw) = room.send_queue().subscribe().await.unwrap();
            let t0 = Instant::now();
            let ms = || t0.elapsed().as_millis();
            let base = through.load(Relaxed);
            let (result, raw_log, ui_log, _) = tokio::join!(
                outbox.send(&request),
                async {
                    let mut log: Vec<(u128, usize, usize)> = Vec::new();
                    loop {
                        match tokio::time::timeout(Duration::from_secs(120), raw.recv()).await {
                            Ok(Ok(RoomSendQueueUpdate::MediaUpload { progress, .. })) => log.push((ms(), progress.current, progress.total)),
                            Ok(Ok(RoomSendQueueUpdate::NewLocalEvent(_))) => println!("PROBE enc={encrypted} t={}ms queue: local echo", ms()),
                            Ok(Ok(RoomSendQueueUpdate::SentEvent { .. })) => {
                                println!("PROBE enc={encrypted} t={}ms queue: event sent", ms());
                                break log;
                            }
                            Ok(Ok(_)) => {}
                            _ => break log,
                        }
                    }
                },
                async {
                    let mut log: Vec<(u128, u64, u64)> = Vec::new();
                    for _ in 0..40000 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        match outbox.control().uploads().into_iter().find(|(i, ..)| *i == txn) {
                            Some((_, got, all)) => log.push((ms(), got, all)),
                            None if !log.is_empty() => break,
                            None => {}
                        }
                    }
                    log
                },
                async {
                    // What the server side was really given, once a second, until the send is over.
                    for _ in 0..90 {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        println!("PROBE enc={encrypted} LINK t={}ms server_got={}", ms(), through.load(Relaxed) - base);
                        if through.load(Relaxed) - base > FILE_BYTES as u64 && outbox.control().uploads().is_empty() { break; }
                    }
                }
            );
            println!("PROBE enc={encrypted} t={}ms outcome ok={}", ms(), matches!(result, Some(SendEvent::Result(ref r)) if r.result.is_ok()));
            for (t, cur, tot) in &raw_log {
                println!("PROBE enc={encrypted} SDK t={t}ms current={cur} total={tot}");
            }
            for (t, cur, tot) in &ui_log {
                println!("PROBE enc={encrypted} UI  t={t}ms sent={cur} total={tot}");
            }
            let sdk: Vec<(u128, u64, u64)> = raw_log.iter().map(|(t, c, n)| (*t, *c as u64, *n as u64)).collect();
            let partial = |v: &[(u128, u64, u64)]| v.iter().filter(|(_, c, n)| *c > 0 && c < n).count();
            let (sdk_mid, ui_mid) = (partial(&sdk), partial(&ui_log));
            // Where the time goes before the first byte: «preparing» is what the card must not show as «0 B».
            let first_byte = |v: &[(u128, u64, u64)]| v.iter().find(|(_, c, _)| *c > 0).map(|(t, ..)| *t);
            println!("PROBE enc={encrypted} summary: first byte SDK={:?}ms UI={:?}ms; sdk updates={} partial={sdk_mid}; ui samples={} partial={ui_mid}", first_byte(&sdk), first_byte(&ui_log), sdk.len(), ui_log.len());

            assert!(matches!(result, Some(SendEvent::Result(ref r)) if r.result.is_ok()), "encrypted={encrypted}: the upload ends well");
            for (name, log) in [("SDK", &sdk), ("UI", &ui_log)] {
                assert!(log.windows(2).all(|w| w[0].1 <= w[1].1), "encrypted={encrypted} {name}: progress only grows");
                assert!(log.iter().all(|(_, c, n)| c <= n) && log.iter().any(|(_, c, n)| c == n && *n >= FILE_BYTES as u64), "encrypted={encrypted} {name}: it ends at the total");
            }
            // The bytes go out at 2 MB/s for ~15 s: the SDK reports many steps and the card's reader sees them too (it polls every 100 ms).
            assert!(sdk_mid >= 10, "encrypted={encrypted}: the SDK reports progress while the file goes out: {sdk_mid}");
            assert!(ui_mid >= 10, "encrypted={encrypted}: the card's reader sees it: {ui_mid}");
            // Once bytes flow, the value moves at least every 3 s until the end (the end itself may wait for the server).
            let (mut value, mut since) = (0, 0);
            for (t, c, n) in ui_log.iter().filter(|(_, c, n)| *c > 0 && c < n) {
                if *c != value {
                    (value, since) = (*c, *t);
                }
                assert!(t - since < 3000, "encrypted={encrypted}: the card's value stood still for 3 s at {c} of {n}");
            }
        }
    });
}
