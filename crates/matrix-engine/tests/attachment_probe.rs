//! Attachment probe against a disposable homeserver (read-only verification).
//! Driven by the attachment_probe script inside a private namespace. Exercises
//! the official matrix-sdk 0.19.1 attachment path for: image with caption and
//! thumbnail, generic file with caption, voice note (MSC3245/MSC1767 blocks),
//! the queued send (RoomSendQueue) and the encrypted-room behaviour (encrypted
//! media + encrypted thumbnail, metadata inside the Megolm event). Two
//! synthetic accounts only; no secrets here (the fixture config path comes
//! from the environment).
#![cfg(feature = "sdk")]
use matrix_engine::{
    sdk::{
        Client, Room,
        attachment::{AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseImageInfo, Thumbnail},
        event_cache::RoomEventCache,
        media::{MediaFormat, MediaRequestParameters},
        reqwest::Certificate,
        ruma::{
            api::{
                Direction,
                client::{
                    message::get_message_events::v3::Request as MessagesRequest,
                    room::Visibility,
                    room::create_room::v3::{
                        CreationContent, Request as CreateRoomRequest, RoomPreset,
                    },
                },
            },
            assign,
            events::room::{MediaSource, message::TextMessageEventContent},
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::sync_service::SyncService,
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

/// Raw JSON of a decrypted (or plain) room message sent by `sender` whose
/// msgtype is `msgtype`, polled from the accumulated room event cache.
async fn wait_message(events: &RoomEventCache, sender: &str, msgtype: &str) -> serde_json::Value {
    let start = Instant::now();
    loop {
        if let Ok(list) = events.events().await {
            if let Some(found) = list.iter().find_map(|event| {
                let value: serde_json::Value =
                    serde_json::from_str(event.raw().json().get()).ok()?;
                let content = &value["content"];
                (value["sender"] == sender
                    && value["type"] == "m.room.message"
                    && content["msgtype"] == msgtype)
                    .then_some(value)
            }) {
                return found;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "timeout: m.{msgtype} from {sender}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The `MediaSource` of the first cached room message matching `msgtype`.
async fn cached_source(events: &RoomEventCache, sender: &str, msgtype: &str) -> MediaSource {
    let event = wait_message(events, sender, msgtype).await;
    serde_json::from_value(event["content"].clone()).unwrap()
}

#[test]
#[ignore = "requires the attachment_probe script"]
fn attachment_probe_covers_image_file_voice_captions_encrypted_room() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env("FIXTURE_AUTH_CONFIG")).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let image = std::fs::read(config["image_png"].as_str().unwrap()).unwrap();
    let thumb = std::fs::read(config["thumb_png"].as_str().unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let build = |_user: &str| {
            let (url, ca) = (url.clone(), ca.clone());
            async move {
                Client::builder()
                    .homeserver_url(url)
                    .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
                    .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
                    .build()
                    .await
                    .unwrap()
            }
        };
        let a = build(config["alice_mxid"].as_str().unwrap()).await;
        let b = build(config["bob_mxid"].as_str().unwrap()).await;
        a.matrix_auth()
            .login_username(
                config["alice_mxid"].as_str().unwrap(),
                config["alice_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        b.matrix_auth()
            .login_username(
                config["bob_mxid"].as_str().unwrap(),
                config["bob_password"].as_str().unwrap(),
            )
            .send()
            .await
            .unwrap();
        let alice = a.user_id().unwrap().to_owned();
        let bob = b.user_id().unwrap().to_owned();
        let service_a = SyncService::builder(a.clone()).build().await.unwrap();
        service_a.start().await;
        let service_b = SyncService::builder(b.clone()).build().await.unwrap();
        service_b.start().await;
        println!("[probe] logged in {alice} and {bob}, sync services running");

        // Plain (non-encrypted) private room: create_dm would turn on
        // encryption with the e2e feature, and the plaintext assertions need
        // url/thumbnail_url in the clear.
        let mut plain_creation = CreationContent::new();
        plain_creation.federate = false;
        let dm = a
            .create_room(assign!(CreateRoomRequest::new(), {
                name: Some("Gnobbi probe chiara".to_owned()),
                visibility: Visibility::Private,
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![bob.clone()],
                creation_content: Some(Raw::new(&plain_creation).unwrap()),
            }))
            .await
            .unwrap();
        let dm_id = dm.room_id().to_owned();
        until("bob sees the room invite", || {
            b.invited_rooms().iter().any(|r| r.room_id() == &dm_id)
        })
        .await;
        b.get_room(&dm_id).unwrap().join().await.unwrap();
        let dm_b = b.get_room(&dm_id).unwrap();
        let (dm_events, _dm_drop) = dm_b.event_cache().await.unwrap();
        println!("[probe] dm {dm_id} joined by bob");

        // Image with caption (MSC2530: the caption becomes the body, the name
        // moves to `filename`) and a client-provided thumbnail.
        let image_cfg = AttachmentConfig::new()
            .info(AttachmentInfo::Image(BaseImageInfo {
                width: Some(32u32.into()),
                height: Some(24u32.into()),
                size: Some((image.len() as u32).into()),
                ..Default::default()
            }))
            .thumbnail(Some(Thumbnail {
                data: thumb.clone(),
                content_type: "image/png".parse().unwrap(),
                width: 16u32.into(),
                height: 12u32.into(),
                size: (thumb.len() as u32).into(),
            }))
            .caption(Some(TextMessageEventContent::plain("didascalia immagine")));
        dm.send_attachment("foto.png", &"image/png".parse().unwrap(), image.clone(), image_cfg)
            .await
            .unwrap();

        // Generic file with caption.
        let file_bytes = b"contenuto del file di prova\n".to_vec();
        let file_cfg =
            AttachmentConfig::new()
                .caption(Some(TextMessageEventContent::plain("il file di prova")));
        dm.send_attachment(
            "nota.txt",
            &"text/plain".parse().unwrap(),
            file_bytes,
            file_cfg,
        )
        .await
        .unwrap();

        // Voice note: AttachmentInfo::Voice emits the org.matrix.msc3245.voice
        // and org.matrix.msc1767.audio blocks (ruma unstable-msc3245-v1-compat
        // is enabled by matrix-sdk's own ruma dependency).
        let ogg = vec![0x4f_u8, 0x67, 0x67, 0x53, 1, 2, 3, 4];
        let voice_cfg = AttachmentConfig::new().info(AttachmentInfo::Voice(BaseAudioInfo {
            duration: Some(Duration::from_millis(1250)),
            waveform: Some(vec![0.1, 0.5, 0.9]),
            size: Some((ogg.len() as u32).into()),
        }));
        dm.send_attachment("nota.ogg", &"audio/ogg".parse().unwrap(), ogg, voice_cfg)
            .await
            .unwrap();
        println!("[probe] image+caption+thumbnail, file+caption, voice sent in the DM");

        // Bob receives and verifies all three.
        let img_event = wait_message(&dm_events, alice.as_str(), "m.image").await;
        let img_content = &img_event["content"];
        assert_eq!(img_content["body"], "didascalia immagine");
        assert_eq!(img_content["filename"], "foto.png");
        assert!(img_content["url"].as_str().unwrap().starts_with("mxc://"));
        assert!(img_content["info"]["thumbnail_url"].as_str().unwrap_or("").starts_with("mxc://"));
        assert_eq!(img_content["info"]["thumbnail_info"]["w"], 16);
        assert_eq!(img_content["info"]["mimetype"], "image/png");
        assert_eq!(img_content["info"]["w"], 32);

        let file_event = wait_message(&dm_events, alice.as_str(), "m.file").await;
        let file_content = &file_event["content"];
        assert_eq!(file_content["body"], "il file di prova");
        assert_eq!(file_content["filename"], "nota.txt");
        assert!(file_content["url"].as_str().unwrap().starts_with("mxc://"));

        let voice_event = wait_message(&dm_events, alice.as_str(), "m.audio").await;
        let voice_content = &voice_event["content"];
        assert_eq!(voice_content["body"], "nota.ogg");
        assert!(voice_content.get("filename").is_none());
        assert!(voice_content.get("org.matrix.msc3245.voice").is_some());
        assert_eq!(voice_content["org.matrix.msc1767.audio"]["duration"], 1250);
        assert_eq!(
            voice_content["org.matrix.msc1767.audio"]["waveform"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        println!("[probe] bob verified caption/filename/voice blocks in the DM");

        // Download round-trip through the official media API (plain source).
        let source = cached_source(&dm_events, alice.as_str(), "m.image").await;
        let params = MediaRequestParameters { source, format: MediaFormat::File };
        let downloaded = b.media().get_media_content(&params, false).await.unwrap();
        assert_eq!(downloaded, image);
        println!("[probe] image bytes round-trip via Media::get_media_content");

        // Encrypted room: queued send (RoomSendQueue) for the image, direct
        // send for the voice note.
        let mut creation = CreationContent::new();
        creation.federate = false;
        let enc = a
            .create_room(assign!(CreateRoomRequest::new(), {
                name: Some("Gnobbi probe cifrata".to_owned()),
                visibility: Visibility::Private,
                preset: Some(RoomPreset::PrivateChat),
                invite: vec![bob.clone()],
                creation_content: Some(Raw::new(&creation).unwrap()),
            }))
            .await
            .unwrap();
        let enc_id = enc.room_id().to_owned();
        until("bob sees the encrypted-room invite", || {
            b.invited_rooms().iter().any(|r| r.room_id() == &enc_id)
        })
        .await;
        b.get_room(&enc_id).unwrap().join().await.unwrap();
        a.get_room(&enc_id).unwrap().enable_encryption().await.unwrap();
        let enc_a = a.get_room(&enc_id).unwrap();
        let enc_b = b.get_room(&enc_id).unwrap();
        let start = Instant::now();
        loop {
            let state_a = enc_a.latest_encryption_state().await;
            let state_b = enc_b.latest_encryption_state().await;
            if matches!((&state_a, &state_b), (Ok(a), Ok(b)) if a.is_encrypted() && b.is_encrypted())
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(90), "timeout: room encryption state");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let (enc_events, _enc_drop) = enc_b.event_cache().await.unwrap();
        println!("[probe] encrypted room {enc_id} ready");

        /// Direct send with bounded retry: the encrypted room may reject the
        /// first attempts until the megolm session is shared with bob.
        async fn send_with_retry(
            room: Room,
            name: &'static str,
            filename: &str,
            mime_str: &'static str,
            data: Vec<u8>,
            make_cfg: impl Fn() -> AttachmentConfig,
        ) {
            let filename = filename.to_owned();
            let start = Instant::now();
            loop {
                let mime = mime_str.parse().unwrap();
                match room
                    .send_attachment(filename.clone(), &mime, data.clone(), make_cfg())
                    .await
                {
                    Ok(_response) => return,
                    Err(err) => {
                        assert!(
                            start.elapsed() < Duration::from_secs(120),
                            "send {name} kept failing: {err}"
                        );
                        println!("[probe] retrying {name} after: {err}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        }

        let enc_image_cfg = AttachmentConfig::new()
            .info(AttachmentInfo::Image(BaseImageInfo {
                width: Some(32u32.into()),
                height: Some(24u32.into()),
                size: Some((image.len() as u32).into()),
                ..Default::default()
            }))
            .thumbnail(Some(Thumbnail {
                data: thumb.clone(),
                content_type: "image/png".parse().unwrap(),
                width: 16u32.into(),
                height: 12u32.into(),
                size: (thumb.len() as u32).into(),
            }))
            .caption(Some(TextMessageEventContent::plain("didascalia cifrata")));
        enc_a
            .send_queue()
            .send_attachment(
                "foto-cifrata.png",
                "image/png".parse().unwrap(),
                image.clone(),
                enc_image_cfg,
            )
            .await
            .unwrap();
        println!("[probe] encrypted image queued via RoomSendQueue");

        let ogg2 = vec![0x4f_u8, 0x67, 0x67, 0x53, 9, 8, 7, 6];
        send_with_retry(
            enc_a.clone(),
            "encrypted voice",
            "nota-cifrata.ogg",
            "audio/ogg",
            ogg2.clone(),
            || {
                AttachmentConfig::new().info(AttachmentInfo::Voice(BaseAudioInfo {
                    duration: Some(Duration::from_millis(900)),
                    waveform: Some(vec![0.2, 0.4]),
                    size: Some((ogg2.len() as u32).into()),
                }))
            },
        )
        .await;
        println!("[probe] encrypted voice sent");

        // Bob receives decrypted events with encrypted media sources.
        let enc_img = wait_message(&enc_events, alice.as_str(), "m.image").await;
        let enc_img_content = &enc_img["content"];
        assert_eq!(enc_img_content["body"], "didascalia cifrata");
        assert_eq!(enc_img_content["filename"], "foto-cifrata.png");
        assert!(enc_img_content.get("url").is_none());
        for key in ["key", "iv", "hashes", "v"] {
            assert!(enc_img_content["file"].get(key).is_some(), "missing file.{key}");
        }
        for key in ["key", "iv", "hashes", "v"] {
            assert!(
                enc_img_content["info"]["thumbnail_file"].get(key).is_some(),
                "missing info.thumbnail_file.{key}"
            );
        }

        let enc_voice = wait_message(&enc_events, alice.as_str(), "m.audio").await;
        let enc_voice_content = &enc_voice["content"];
        assert!(enc_voice_content.get("org.matrix.msc3245.voice").is_some());
        assert_eq!(enc_voice_content["org.matrix.msc1767.audio"]["duration"], 900);
        assert!(enc_voice_content["file"].get("key").is_some());
        println!("[probe] bob decrypted both events; media and thumbnail sources encrypted");

        // The server only ever saw Megolm ciphertext for these events.
        let messages = b
            .send(assign!(MessagesRequest::new(enc_id.clone(), Direction::Backward), {
                limit: 10u32.into(),
            }))
            .await
            .unwrap();
        let encrypted_count = messages
            .chunk
            .iter()
            .filter(|raw| {
                serde_json::from_str::<serde_json::Value>(raw.json().get())
                    .map(|v| v["type"] == "m.room.encrypted")
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(encrypted_count, 2, "expected exactly 2 megolm events on the server");
        println!("[probe] server stores 2 m.room.encrypted events (ciphertext only)");

        // Encrypted media round-trip: download + decrypt equals the original.
        let enc_source = cached_source(&enc_events, alice.as_str(), "m.image").await;
        let params = MediaRequestParameters { source: enc_source, format: MediaFormat::File };
        let decrypted_bytes = b.media().get_media_content(&params, false).await.unwrap();
        assert_eq!(decrypted_bytes, image);
        println!("[probe] encrypted media download+decrypt round-trip ok");

        service_a.stop().await;
        service_b.stop().await;
        let _ = a.matrix_auth().logout().await;
        let _ = b.matrix_auth().logout().await;
    });
}
