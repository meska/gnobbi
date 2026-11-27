//! The receiving side of attachments on a disposable homeserver (Synapse or Tuwunel), plain and encrypted room:
//! an album of 3 pictures with a caption reads back as 3 rows carrying the album mark and the declared size, a 5 MB PDF reads
//! back as a file row that saves and opens with receive-time progress, and an upload in progress (or waiting for its turn)
//! can be cancelled and never reaches the room. alice sends, bob reads through `RoomSync`. Synthetic accounts only.
//! Driver: PROBE_TARGET=attachment_view PROBE_SERVER=synapse|tuwunel python3 tools/attachment_send.py (via ct.sh).
#![cfg(feature = "sdk")]
#![recursion_limit = "256"]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    TestOutbox, ViewRequests,
    history::{HistoryEvent, MediaKind},
    media::MediaError,
    receipts::ReadVisibility,
    room_sync::RoomSync,
    sdk::{
        Client, Room,
        reqwest::Certificate,
        ruma::{
            OwnedRoomId, assign,
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
    sending::{Album, Attachment, Delivery, SendError, SendEvent, SendRequest, fresh_transaction_id, send_text},
    video_file::{download_video, prefetch_attachment, save_attachment_with_progress},
};
use serde_json::Value;
use std::time::{Duration, Instant};

fn acc() -> AccountId {
    AccountId::new(1)
}
const PDF_BYTES: usize = 5 * 1024 * 1024;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required"))
}

async fn login(config: &Value, who: &str) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let c = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .build()
        .await
        .unwrap();
    c.matrix_auth().login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap()).send().await.unwrap();
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

async fn read(sync: &RoomSync, scope: &RoomScope, want: &[String]) -> Vec<HistoryEvent> {
    let hidden = ReadVisibility { window_focused: false, room_foreground: false, bottom_gap_px: 0.0, public_receipts: true };
    let mut views = ViewRequests::default();
    let start = Instant::now();
    loop {
        let (page, _) = sync.open_history(views.start(scope.clone()).unwrap(), hidden).await.unwrap();
        let seen = page.events.iter().filter(|e| want.iter().any(|w| w == e.event_id.as_str())).count();
        if seen == want.len() {
            return page.events;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "events missing: {seen} of {}", want.len());
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn request(scope: &RoomScope, body: &str, name: &str, data: Vec<u8>, album: Option<Album>) -> SendRequest {
    SendRequest {
        scope: scope.clone(),
        transaction_id: fresh_transaction_id(),
        body: body.into(),
        attachment: Some(Attachment { filename: name.into(), data, album, ..Default::default() }),
        poll: None,
        mentions: Vec::new(),
    }
}

async fn send(client: &Client, request: &SendRequest) -> String {
    let start = Instant::now();
    loop {
        match send_text(client, acc(), request).await {
            Ok(id) => return id.as_str().to_owned(),
            Err(e) => {
                // An encrypted room may refuse the first attempts until the Megolm session reaches bob.
                assert!(start.elapsed() < Duration::from_secs(120), "send kept failing: {e:?}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

#[test]
#[ignore = "requires the attachment_send script"]
fn attachment_view_plain_and_encrypted() {
    let config: Value = serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let image = std::fs::read(config["image_png"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (a, b) = (login(&config, "alice").await, login(&config, "bob").await);
        let mut services = Vec::new();
        for c in [&a, &b] {
            let s = SyncService::builder(c.clone()).build().await.unwrap();
            s.start().await;
            services.push(s);
        }
        let out = std::env::temp_dir().join("attachment-view");
        std::fs::create_dir_all(&out).unwrap();
        for encrypted in [false, true] {
            let (_ra, id) = room_pair(&a, &b, encrypted).await;
            let scope = RoomScope { account_id: acc(), room_id: RoomId::new(id.as_str()) };
            let sync = RoomSync::from_client(acc(), b.clone());

            // The album of 3 with its caption, then a 5 MB PDF.
            let mut ids = Vec::new();
            for index in 0..3u8 {
                let caption = if index == 2 { "didascalia album" } else { "" };
                let album = Some(Album { id: "album-view".into(), index, count: 3 });
                ids.push(send(&a, &request(&scope, caption, &format!("album-{index}.png"), image.clone(), album)).await);
            }
            let mut pdf = b"%PDF-1.4\n".to_vec();
            pdf.extend((0..PDF_BYTES - pdf.len()).map(|i| (i.wrapping_mul(31) % 251) as u8));
            let pdf_id = send(&a, &request(&scope, "", "relazione.pdf", pdf.clone(), None)).await;
            let mut all = ids.clone();
            all.push(pdf_id.clone());
            // The sender's own view, read at once (the echo of the send queue, before sync brings the event back), has the marker too.
            let own = RoomSync::from_client(acc(), a.clone());
            let mine = read(&own, &scope, &all).await;
            for id in &ids {
                assert!(mine.iter().find(|e| e.event_id.as_str() == id).unwrap().media.as_ref().is_some_and(|m| m.album.is_some()), "encrypted={encrypted}: the sender's own row lost the album mark");
            }
            let rows = read(&sync, &scope, &all).await;
            let row = |id: &str| rows.iter().find(|e| e.event_id.as_str() == id).unwrap();

            for (index, id) in ids.iter().enumerate() {
                let media = row(id).media.as_ref().expect("a picture row");
                assert_eq!(media.kind, MediaKind::Image);
                assert_eq!(media.album, Some(Album { id: "album-view".into(), index: index as u8, count: 3 }), "encrypted={encrypted}");
                assert_eq!(media.dims, Some((32, 24)), "the declared size lays the picture out before it loads");
                // The caption is the last item's; the others have none.
                assert_eq!(row(id).body, if index == 2 { "didascalia album" } else { "" }, "encrypted={encrypted} item {index}");
            }
            let file = row(&pdf_id);
            assert_eq!(file.media.as_ref().map(|m| m.kind), Some(MediaKind::File));
            assert!(file.media.as_ref().unwrap().album.is_none());
            assert_eq!((file.attachment_name.as_deref(), file.attachment_size), (Some("relazione.pdf"), Some(PDF_BYTES as u64)));

            // Save: bytes arrive with progress, the file is the original.
            let mut views = ViewRequests::default();
            let tag = views.start(scope.clone()).unwrap();
            let pdf_event = matrix_domain::EventId::new(&pdf_id);
            let dest = out.join(format!("{encrypted}-relazione.pdf"));
            let mut seen = Vec::new();
            save_attachment_with_progress(&b, acc(), &tag, &pdf_event, &dest, 256 << 20, |got, total| seen.push((got, total))).await.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), pdf, "encrypted={encrypted}: saved bytes differ");
            assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "progress only grows: {seen:?}");
            assert_eq!(seen.last().copied(), Some((PDF_BYTES as u64, Some(PDF_BYTES as u64))), "{seen:?}");
            // The cap holds while the body is received: a lower cap refuses it, nothing is written.
            let refused = out.join(format!("{encrypted}-refused.pdf"));
            let result = save_attachment_with_progress(&b, acc(), &tag, &pdf_event, &refused, 1 << 20, |_, _| ()).await;
            assert!(matches!(result, Err(MediaError::TooLarge)), "{result:?}");
            assert!(!refused.exists());

            // Open: the same download into the private temp file, with the extension of the type.
            let mut got = Vec::new();
            let file = download_video(&b, acc(), &tag, &pdf_event, &out, 256 << 20, |n| got.push(n)).await.unwrap();
            assert_eq!(file.path().extension().and_then(|e| e.to_str()), Some("pdf"));
            assert_eq!(std::fs::read(file.path()).unwrap(), pdf);
            assert_eq!(got.last().copied(), Some(PDF_BYTES), "{got:?}");
            drop(file);

            // Auto download: a cap below the size refuses it while received and keeps nothing; a fetch keeps the original in memory
            // only, and Save and Open then use it: no byte is received again (their progress never fires), the files are the same.
            let lane = views.start(scope.clone()).unwrap();
            let refused = prefetch_attachment(&b, acc(), &lane, &pdf_event, 1 << 20, |_, _| ()).await;
            assert!(matches!(refused, Err(MediaError::TooLarge)), "{refused:?}");
            let mut fetched = Vec::new();
            prefetch_attachment(&b, acc(), &lane, &pdf_event, 256 << 20, |got, total| fetched.push((got, total))).await.unwrap();
            assert_eq!(fetched.last().copied(), Some((PDF_BYTES as u64, Some(PDF_BYTES as u64))), "{fetched:?}");
            let (mut seen, mut got) = (Vec::new(), Vec::new());
            let again = out.join(format!("{encrypted}-again.pdf"));
            save_attachment_with_progress(&b, acc(), &lane, &pdf_event, &again, 256 << 20, |g, t| seen.push((g, t))).await.unwrap();
            assert_eq!(std::fs::read(&again).unwrap(), pdf);
            let file = download_video(&b, acc(), &lane, &pdf_event, &out, 256 << 20, |n| got.push(n)).await.unwrap();
            assert_eq!(std::fs::read(file.path()).unwrap(), pdf);
            assert!(seen.is_empty() && got.is_empty(), "encrypted={encrypted}: the bytes were fetched again: {seen:?} {got:?}");
            drop(file);
            matrix_engine::media::auto_clear_for_test(acc());

            // Upload progress: the bytes sent are reported while it goes out, never more than the total, and it ends well.
            let outbox = TestOutbox::new(a.clone(), acc());
            let sample = request(&scope, "", "progresso.bin", vec![9u8; 12 << 20], None);
            let id = sample.transaction_id.clone();
            let (result, samples) = tokio::join!(outbox.send(&sample), async {
                let mut samples: Vec<(u64, u64)> = Vec::new();
                for _ in 0..4000 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    match outbox.control().uploads().into_iter().find(|(i, ..)| *i == id) {
                        Some((_, got, all)) => samples.push((got, all)),
                        None if !samples.is_empty() => break,
                        None => {}
                    }
                }
                samples
            });
            assert!(matches!(result, Some(SendEvent::Result(ref r)) if r.result.is_ok()), "the upload ends well");
            assert!(samples.iter().all(|(got, all)| got <= all) && samples.iter().any(|(_, all)| *all >= 12 << 20), "encrypted={encrypted}: {samples:?}");
            assert!(samples.windows(2).all(|w| w[0].0 <= w[1].0), "progress only grows: {samples:?}");
            assert!(outbox.control().uploads().is_empty());

            // Upload cancel: the first (big) one is stopped while it uploads, the second never starts.
            let outbox = TestOutbox::new(a.clone(), acc());
            let big = request(&scope, "", "grande.bin", vec![7u8; 23 << 20], None);
            let later = request(&scope, "", "dopo.bin", vec![8u8; 1 << 10], None);
            let (first, second) = (big.transaction_id.clone(), later.transaction_id.clone());
            let (results, ()) = tokio::join!(
                async { (outbox.send(&big).await, outbox.send(&later).await) },
                async {
                    until("the upload starts", || outbox.control().uploads().iter().any(|(id, ..)| *id == first)).await;
                    assert!(outbox.control().cancel_upload(&second), "a waiting upload can be withdrawn");
                    assert!(outbox.control().cancel_upload(&first));
                }
            );
            let outcome = |event: Option<SendEvent>| match event {
                Some(SendEvent::Result(r)) => r.result.map(|_| ()),
                other => panic!("unexpected {}", if other.is_some() { "notice" } else { "silence" }),
            };
            assert_eq!(outcome(results.0), Err(SendError::Cancelled), "encrypted={encrypted}");
            assert_eq!(outcome(results.1), Err(SendError::Cancelled), "encrypted={encrypted}");
            assert!(outbox.control().uploads().is_empty(), "the card's progress goes with it");
            // Nothing of them in the room: only the 4 events above (and room state) are there.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let rows = read(&sync, &scope, &all).await;
            assert!(!rows.iter().any(|e| matches!(e.attachment_name.as_deref(), Some("grande.bin" | "dopo.bin")) && !e.redacted), "a cancelled upload reached the room");
            // Cancel while the picture is still being compressed (before any echo exists): it must not go out afterwards.
            let outbox = TestOutbox::new(a.clone(), acc());
            let mut big_picture = request(&scope, "", "enorme.png", Vec::new(), None);
            let noisy = image::RgbImage::from_fn(4000, 4000, |x, y| image::Rgb([(x * 7 + y) as u8, (x ^ y) as u8, (x * y >> 3) as u8]));
            let mut png = Vec::new();
            noisy.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            if let Some(file) = big_picture.attachment.as_mut() {
                (file.data, file.delivery) = (png, Delivery::Compressed);
            }
            let id = big_picture.transaction_id.clone();
            let (result, ()) = tokio::join!(outbox.send(&big_picture), async {
                until("the picture is in preparation", || outbox.control().uploads().iter().any(|(i, ..)| *i == id)).await;
                assert!(outbox.control().cancel_upload(&id));
            });
            assert!(matches!(result, Some(SendEvent::Result(ref r)) if r.result == Err(SendError::Cancelled)), "encrypted={encrypted}: cancelled while preparing");
            assert!(outbox.control().uploads().is_empty());
            tokio::time::sleep(Duration::from_secs(3)).await;
            let rows = read(&sync, &scope, &all).await;
            assert!(!rows.iter().any(|e| e.attachment_name.as_deref() == Some("enorme.png")), "encrypted={encrypted}: a picture cancelled while preparing reached the room");
            println!("[view] encrypted={encrypted}: album of 3, 5 MB PDF saved+opened with progress, upload cancelled");
        }
        for s in services {
            s.stop().await;
        }
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
