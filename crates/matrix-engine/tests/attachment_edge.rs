//! Edge cases of the attachment send path (`sending::send_text`) against a
//! disposable homeserver, driven by the attachment_send script (PROBE_TARGET=attachment_edge):
//! empty and large files, odd names and MIME types, captions, voice-note shapes,
//! refused sends, repeated transaction ids, cancelled uploads and concurrent sends,
//! in a plain and in an encrypted room. Two synthetic accounts only.
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope, TransactionId};
use matrix_engine::{
    sdk::{
        Client,
        event_cache::RoomEventCache,
        media::{MediaFormat, MediaRequestParameters},
        reqwest::Certificate,
        ruma::{
            api::client::room::{
                Visibility,
                create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            },
            assign,
            events::room::MediaSource,
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
    sending::{Attachment, SendError, SendRequest, Voice, fresh_transaction_id, send_text},
};
use std::time::{Duration, Instant};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(90), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Raw JSON of the event with this id, polled from the accumulated room event cache.
async fn wait_event(events: &RoomEventCache, event_id: &str) -> serde_json::Value {
    let start = Instant::now();
    loop {
        if let Ok(list) = events.events().await
            && let Some(found) = list.iter().find_map(|event| {
                let value: serde_json::Value =
                    serde_json::from_str(event.raw().json().get()).ok()?;
                (value["event_id"] == event_id).then_some(value)
            })
        {
            return found;
        }
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "timeout: event {event_id}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn request(
    room: &matrix_engine::sdk::Room,
    tx: TransactionId,
    body: &str,
    name: &str,
    data: Vec<u8>,
    voice: Option<Voice>,
) -> SendRequest {
    SendRequest {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new(room.room_id().as_str()),
        },
        transaction_id: tx,
        body: body.to_owned(),
        attachment: Some(Attachment {
            filename: name.to_owned(),
            data,
            voice,
        }),
        mentions: Vec::new(),
    }
}

/// Send, retrying a few times (an encrypted room may refuse until the session reaches B).
async fn try_send(client: &Client, req: &SendRequest) -> Result<String, SendError> {
    let mut last = SendError::Transport;
    for _ in 0..5 {
        match send_text(client, AccountId::new(1), req).await {
            Ok(id) => return Ok(id.as_str().to_owned()),
            Err(error) => last = error,
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(last)
}

async fn send_ok(client: &Client, req: &SendRequest) -> String {
    try_send(client, req)
        .await
        .unwrap_or_else(|error| panic!("send failed: {error:?}"))
}

/// Pseudo-random bytes, so compression or dedup can't hide a truncation.
fn noise(len: usize) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

struct Case {
    name: &'static str,
    caption: String,
    file: String,
    data: Vec<u8>,
    voice: Option<Voice>,
    msgtype: &'static str,
    mime: &'static str,
}

#[test]
#[ignore = "requires the attachment_send script"]
fn attachment_edge_cases_plain_and_encrypted() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let image = std::fs::read(config["image_png"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let login = |user: &'static str| {
            let (url, ca, config) = (url.clone(), ca.clone(), config.clone());
            async move {
                let client = Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap();
                client
                    .matrix_auth()
                    .login_username(
                        config[format!("{user}_mxid")].as_str().unwrap(),
                        config[format!("{user}_password")].as_str().unwrap(),
                    )
                    .send()
                    .await
                    .unwrap();
                client
            }
        };
        let a = login("alice").await;
        let b = login("bob").await;
        let bob = b.user_id().unwrap().to_owned();
        let service_a = SyncService::builder(a.clone()).build().await.unwrap();
        service_a.start().await;
        let service_b = SyncService::builder(b.clone()).build().await.unwrap();
        service_b.start().await;

        let cases = vec![
            Case { name: "no extension", caption: String::new(), file: "LEGGIMI".into(), data: noise(300), voice: None, msgtype: "m.file", mime: "application/octet-stream" },
            Case { name: "unknown extension", caption: String::new(), file: "dati.xyz123".into(), data: noise(300), voice: None, msgtype: "m.file", mime: "application/octet-stream" },
            Case { name: "double extension", caption: String::new(), file: "archivio.tar.gz".into(), data: noise(300), voice: None, msgtype: "m.file", mime: "application/gzip" },
            Case { name: "upper-case image extension", caption: String::new(), file: "FOTO.PNG".into(), data: image.clone(), voice: None, msgtype: "m.image", mime: "image/png" },
            Case { name: "png name with text bytes", caption: String::new(), file: "finta.png".into(), data: b"non sono un'immagine".to_vec(), voice: None, msgtype: "m.image", mime: "image/png" },
            Case { name: "unicode and punctuation in name", caption: String::new(), file: "relazione è già ☃ 日本語 (1); a,b'c.txt".into(), data: noise(50), voice: None, msgtype: "m.file", mime: "text/plain" },
            Case { name: "caption of spaces only", caption: "  \n\t ".into(), file: "spazi.txt".into(), data: noise(10), voice: None, msgtype: "m.file", mime: "text/plain" },
            Case { name: "caption with markup, emoji, bidi, newline", caption: "<b>x</b> & \"q\" 'a' *m* _n_ `c` [l](http://x) 🙂👩‍👩‍👧 \u{202e}rtl\nsecond line\u{0301}".into(), file: "speciale.jpg".into(), data: noise(40), voice: None, msgtype: "m.image", mime: "image/jpeg" },
            Case { name: "20000-char caption", caption: "è".repeat(20_000), file: "lunga.pdf".into(), data: noise(40), voice: None, msgtype: "m.file", mime: "application/pdf" },
            Case { name: "voice, empty waveform, zero duration", caption: String::new(), file: "nota-vocale.wav".into(), data: noise(100), voice: Some(Voice { duration_ms: 0, waveform: vec![] }), msgtype: "m.audio", mime: "audio/wav" },
            Case { name: "voice, 500-point waveform", caption: String::new(), file: "nota-vocale.wav".into(), data: noise(100), voice: Some(Voice { duration_ms: 300_000, waveform: vec![0.25; 500] }), msgtype: "m.audio", mime: "audio/wav" },
            Case { name: "voice with caption", caption: "ascolta".into(), file: "nota-vocale.wav".into(), data: noise(100), voice: Some(Voice { duration_ms: 1000, waveform: vec![1.0; 10] }), msgtype: "m.audio", mime: "audio/wav" },
            Case { name: "15 MiB file", caption: String::new(), file: "grande.bin".into(), data: noise(15 << 20), voice: None, msgtype: "m.file", mime: "application/octet-stream" },
        ];

        let mut failures: Vec<String> = Vec::new();
        for encrypted in [false, true] {
            let mut creation = CreationContent::new();
            creation.federate = false;
            let room = a
                .create_room(assign!(CreateRoomRequest::new(), {
                    name: Some(format!("Gnobbi allegati limiti {}", if encrypted { "cifrata" } else { "chiara" })),
                    visibility: Visibility::Private,
                    preset: Some(RoomPreset::PrivateChat),
                    invite: vec![bob.clone()],
                    creation_content: Some(Raw::new(&creation).unwrap()),
                }))
                .await
                .unwrap();
            let id = room.room_id().to_owned();
            until("bob sees the invite", || b.invited_rooms().iter().any(|r| r.room_id() == id)).await;
            b.get_room(&id).unwrap().join().await.unwrap();
            let (room_a, room_b) = (a.get_room(&id).unwrap(), b.get_room(&id).unwrap());
            if encrypted {
                room_a.enable_encryption().await.unwrap();
                let start = Instant::now();
                loop {
                    let (x, y) = (room_a.latest_encryption_state().await, room_b.latest_encryption_state().await);
                    if matches!((&x, &y), (Ok(x), Ok(y)) if x.is_encrypted() && y.is_encrypted()) {
                        break;
                    }
                    assert!(start.elapsed() < Duration::from_secs(90), "timeout: room encryption state");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
            let (events, _keep) = room_b.event_cache().await.unwrap();
            let start = Instant::now();
            while events.events().await.map_or(true, |list| list.is_empty()) {
                assert!(start.elapsed() < Duration::from_secs(60), "timeout: room reaches bob");
                tokio::time::sleep(Duration::from_millis(300)).await;
            }

            // Zero-byte uploads: Synapse refuses them, Tuwunel accepts them; the app must keep them off the wire (see the composer test).
            for (file, mime_name) in [("vuoto.txt", ""), ("vuota.png", "vuota")] {
                let req = request(&room_a, fresh_transaction_id(), mime_name, file, vec![], None);
                println!("[edge] encrypted={encrypted} empty file {file}: {:?}", try_send(&a, &req).await);
            }
            for case in &cases {
                let req = request(&room_a, fresh_transaction_id(), &case.caption, &case.file, case.data.clone(), case.voice.clone());
                let event_id = match try_send(&a, &req).await {
                    Ok(id) => id,
                    Err(error) => {
                        println!("[edge] encrypted={encrypted} FAIL send {:?}: {error:?}", case.name);
                        failures.push(format!("encrypted={encrypted} send {:?}: {error:?}", case.name));
                        continue;
                    }
                };
                let event = wait_event(&events, &event_id).await;
                let content = &event["content"];
                let ctx = format!("encrypted={encrypted} case={:?}: {content}", case.name);
                assert_eq!(content["msgtype"], case.msgtype, "{ctx}");
                let caption = !case.caption.trim().is_empty();
                assert_eq!(content["body"], if caption { case.caption.as_str() } else { case.file.as_str() }, "{ctx}");
                assert_eq!(content["filename"].as_str().unwrap_or(&case.file), case.file, "{ctx}");
                assert_eq!(content["info"]["mimetype"], case.mime, "{ctx}");
                // Images that fail header parsing still carry size too.
                assert_eq!(content["info"]["size"].as_u64(), Some(case.data.len() as u64), "{ctx}");
                assert_eq!(content.get("file").is_some(), encrypted, "{ctx}");
                assert_eq!(content.get("org.matrix.msc3245.voice").is_some(), case.voice.is_some(), "{ctx}");
                let source: MediaSource = serde_json::from_value(content.clone()).unwrap();
                let bytes = b
                    .media()
                    .get_media_content(&MediaRequestParameters { source, format: MediaFormat::File }, false)
                    .await
                    .unwrap_or_else(|error| panic!("download failed ({error}) {ctx}"));
                assert!(bytes == case.data, "bytes differ ({} vs {}) {ctx}", bytes.len(), case.data.len());
                println!("[edge] encrypted={encrypted} ok: {} -> {} {:?}", case.name, content["msgtype"], content["info"]["mimetype"]);
            }

            // Refused sends never reach the server.
            let mut missing = request(&room_a, fresh_transaction_id(), "x", "a.txt", vec![1], None);
            missing.scope.room_id = RoomId::new("!nonexistent:matrix-desktop.test");
            assert_eq!(send_text(&a, AccountId::new(1), &missing).await.unwrap_err(), SendError::Unavailable);
            let mut wrong = request(&room_a, fresh_transaction_id(), "x", "a.txt", vec![1], None);
            wrong.scope.account_id = AccountId::new(2);
            assert_eq!(send_text(&a, AccountId::new(1), &wrong).await.unwrap_err(), SendError::WrongScope);
            let mut nothing = request(&room_a, fresh_transaction_id(), "  ", "a.txt", vec![1], None);
            nothing.attachment = None;
            assert_eq!(send_text(&a, AccountId::new(1), &nothing).await.unwrap_err(), SendError::InvalidInput);
            let mut no_tx = request(&room_a, TransactionId::new(""), "", "a.txt", vec![1], None);
            no_tx.transaction_id = TransactionId::new("");
            assert_eq!(send_text(&a, AccountId::new(1), &no_tx).await.unwrap_err(), SendError::InvalidInput);

            // Retry with the same transaction id (Riprova after an unconfirmed send) must not duplicate the event.
            let tx = fresh_transaction_id();
            let first = send_ok(&a, &request(&room_a, tx.clone(), "una volta", "r.txt", b"retry".to_vec(), None)).await;
            let second = send_ok(&a, &request(&room_a, tx.clone(), "una volta", "r.txt", b"retry".to_vec(), None)).await;
            println!("[edge] encrypted={encrypted} same txn twice: first={first} second={second}");
            println!("[edge] waiting same-txn event"); let _ = wait_event(&events, &first).await;
            let copies = events.events().await.unwrap().iter().filter(|e| {
                let v: serde_json::Value = serde_json::from_str(e.raw().json().get()).unwrap();
                v["content"]["body"] == "una volta"
            }).count();
            assert_eq!(copies, 1, "retrying a transaction id duplicated the message (first={first}, second={second})");

            // A cancelled upload (view dropped the future) leaves the room usable.
            let big = request(&room_a, fresh_transaction_id(), "", "annullato.bin", noise(30 << 20), None);
            let cancelled = tokio::time::timeout(Duration::from_millis(100), send_text(&a, AccountId::new(1), &big)).await;
            println!("[edge] encrypted={encrypted} cancelled upload finished early: {}", cancelled.is_ok());
            let after = send_ok(&a, &request(&room_a, fresh_transaction_id(), "dopo annullo", "dopo.txt", b"ok".to_vec(), None)).await;
            println!("[edge] encrypted={encrypted} sent after the failure, waiting for bob"); let _ = wait_event(&events, &after).await; println!("[edge] encrypted={encrypted} bob got it");

            // Three files dropped at once and sent together.
            let batch: Vec<_> = (0..3).map(|i| request(&room_a, fresh_transaction_id(), "", &format!("multi-{i}.bin"), noise(100_000 + i), None)).collect();
            println!("[edge] batch");
            let ids = futures_util_join(&a, &batch).await;
            // Bob's live cache keeps only the newest event of a burst (timeline limit of the room list), so read his history instead.
            let start = Instant::now();
            loop {
                let page = room_b.messages(matrix_engine::sdk::room::MessagesOptions::backward()).await.unwrap();
                let bodies: Vec<String> = page.chunk.iter().filter_map(|e| {
                    let v: serde_json::Value = serde_json::from_str(e.raw().json().get()).ok()?;
                    ids.contains(&v["event_id"].as_str()?.to_owned()).then(|| v["content"]["body"].as_str().unwrap_or("").to_owned())
                }).collect();
                if (0..3).all(|i| bodies.contains(&format!("multi-{i}.bin"))) {
                    break;
                }
                assert!(start.elapsed() < Duration::from_secs(60), "encrypted={encrypted}: batch not fully visible to bob: {bodies:?}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }

            // Over the server's upload limit: one clean error, room still usable.
            let huge = request(&room_a, fresh_transaction_id(), "", "enorme.bin", noise(60 << 20), None);
            let started = Instant::now();
            let refused = send_text(&a, AccountId::new(1), &huge).await;
            println!("[edge] encrypted={encrypted} 60 MiB upload -> {refused:?} after {:?}", started.elapsed());
            let after = send_ok(&a, &request(&room_a, fresh_transaction_id(), "dopo errore", "dopo2.txt", b"ok".to_vec(), None)).await;
            println!("[edge] waiting after refusal"); let _ = wait_event(&events, &after).await;
            assert!(matches!(refused, Err(SendError::TooLarge)), "encrypted={encrypted}: a 60 MiB upload must be refused as too large: {refused:?}");
        }
        service_a.stop().await;
        service_b.stop().await;
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
        assert!(failures.is_empty(), "edge cases failed:\n{}", failures.join("\n"));
    });
}

/// Starts all requests before awaiting any (the dropped files are sent together).
async fn futures_util_join(client: &Client, reqs: &[SendRequest]) -> Vec<String> {
    futures_util::future::join_all(reqs.iter().map(|req| send_ok(client, req))).await
}
