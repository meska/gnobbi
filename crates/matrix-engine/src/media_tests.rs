const MEDIA_PNG: &[u8] = include_bytes!("../tests/fixtures/checker.png");

#[test]
fn video_thumbnail_receive_cap_precedes_mime_and_decode() {
    runtime().block_on(async {
        use std::{cell::Cell, io::Read, sync::{Arc, Mutex}};
        use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};
        use matrix_sdk::ruma::{api::MatrixVersion, events::room::EncryptedFile, owned_mxc_uri};
        const CAP: usize = 1_048_576;
        for case in ["plain-over", "encrypted-over", "plain-exact", "encrypted-exact", "plain", "encrypted", "bad-hash", "no-source", "no-info"] {
            let oversized = case.ends_with("over");
            let encrypted = case.starts_with("encrypted") || case == "bad-hash";
            let mut body = MEDIA_PNG.to_vec();
            if oversized { body.resize(CAP + 1, 0); }
            if case.ends_with("exact") { body.resize(CAP, 0); }
            let expected_bytes = body.clone();
            let mut content = serde_json::json!({"msgtype":"m.video", "body":"synthetic.mp4", "url":"mxc://fixture.invalid/full-video", "info":{}});
            if encrypted {
                let mut input = std::io::Cursor::new(&body);
                let mut encryptor = matrix_sdk_crypto::AttachmentEncryptor::new(&mut input);
                let mut ciphertext = Vec::new();
                encryptor.read_to_end(&mut ciphertext).unwrap();
                let info = encryptor.finish();
                let mut file = EncryptedFile::new(owned_mxc_uri!("mxc://fixture.invalid/thumbnail"), info.encryption_info, info.hashes);
                if case == "bad-hash" { file.hashes = matrix_sdk::ruma::events::room::EncryptedFileHashes::with_sha256([0; 32]); }
                content["info"]["thumbnail_file"] = serde_json::to_value(file).unwrap();
                body = ciphertext;
            } else if case != "no-source" && case != "no-info" {
                content["info"]["thumbnail_url"] = serde_json::json!("mxc://fixture.invalid/thumbnail");
            }
            if case == "no-info" { content.as_object_mut().unwrap().remove("info"); }
            let event = serde_json::json!({"type":"m.room.message", "event_id":"$video", "room_id":"!video:fixture.invalid", "sender":"@alice:fixture.invalid", "origin_server_ts":1, "content":content});
            let sync = serde_json::json!({"pos":"1", "lists":{"all_rooms":{"count":1}}, "extensions":{}, "rooms":{"!video:fixture.invalid":{"initial":true, "joined_count":1, "required_state":[{"type":"m.room.member", "state_key":"@alice:fixture.invalid", "sender":"@alice:fixture.invalid", "event_id":"$member", "origin_server_ts":1, "content":{"membership":"join"}}], "timeline":[]}}});
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let server = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") { headers.push(stream.read_u8().await.unwrap()); }
                    let headers = String::from_utf8(headers).unwrap();
                    // Windows resets the connection (RST) when we close with an unread request
                    // body, which the SDK reports as a transport failure; drain the body first.
                    if let Some(len) = headers
                        .split("\r\n")
                        .find_map(|l| {
                            l.split_once(':').and_then(|(k, v)| {
                                k.eq_ignore_ascii_case("content-length").then(|| v.trim().to_owned())
                            })
                        })
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        let mut request_body = vec![0u8; len as usize];
                        let _ = stream.read_exact(&mut request_body).await;
                    }
                    let path = headers.split_whitespace().nth(1).unwrap().to_owned();
                    recorded.lock().unwrap().push(path.clone());
                    let response = if path.contains("/sync?") { serde_json::to_vec(&sync).unwrap() }
                        else if path.contains("/event/") { serde_json::to_vec(&event).unwrap() }
                        else if path.contains("/thumbnail/") || path.contains("/download/") { body.clone() }
                        else { b"{}".to_vec() };
                    // Chunked exercises receive-time counting rather than a header-only rejection.
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", response.len());
                    if stream.write_all(header.as_bytes()).await.is_ok() {
                        let _ = stream.write_all(&response).await;
                        let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
                    }
                }
            });
            let client = matrix_sdk::Client::builder().homeserver_url(format!("http://{address}"))
                .sliding_sync_version_builder(matrix_sdk::sliding_sync::VersionBuilder::None)
                .request_config(matrix_sdk::config::RequestConfig::new().retry_limit(3))
                .server_versions([MatrixVersion::V1_0]).build().await.unwrap();
            let session: matrix_sdk::authentication::matrix::MatrixSession = serde_json::from_value(serde_json::json!({"user_id":"@alice:fixture.invalid", "device_id":"TEST", "access_token":"synthetic"})).unwrap();
            client.restore_session(session).await.unwrap();
            crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
            let session = AccountSession { account_id: AccountId::new(26), client: Some(client), store: None };
            let mut views = crate::ViewRequests::new();
            let tag = views.start(matrix_domain::RoomScope { account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new("!video:fixture.invalid") }).unwrap();
            crate::media_sdk::VIDEO_DECODE_ENTRIES.scope(Cell::new((0, 0)), async {
                let result = session.download_video_thumbnail(&tag, &matrix_domain::EventId::new("$video")).await;
                let entries = crate::media_sdk::VIDEO_DECODE_ENTRIES.with(Cell::get);
                if oversized {
                    assert!(matches!(result, Err(crate::media::MediaError::TooLarge)), "{case}: expected TooLarge, error={:?}, decoder entries={entries:?}", result.as_ref().err());
                    assert_eq!(entries, (0, 0), "{case}: no MIME sniff or decode");
                } else if case == "bad-hash" {
                    assert!(matches!(result, Err(crate::media::MediaError::Failed)));
                    assert_eq!(entries, (0, 0));
                } else if case.starts_with("no-") {
                    assert!(matches!(result, Ok(None)));
                    assert_eq!(entries, (0, 0));
                } else {
                    let downloaded = result.unwrap().unwrap();
                    assert_eq!(downloaded.bytes, expected_bytes);
                    assert_eq!((downloaded.preview.width, downloaded.preview.height), (2, 2));
                    assert_eq!(downloaded.preview.rgba, image::load_from_memory(MEDIA_PNG).unwrap().into_rgba8().into_raw());
                    assert_eq!(entries, (1, 1), "positive control for both probes");
                }
            }).await;
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
            let requests = requests.lock().unwrap();
            assert!(!requests.iter().any(|p| p.contains("full-video")), "no MP4 fallback");
            let media: Vec<_> = requests.iter().filter(|p| p.contains("/thumbnail/") || p.contains("/download/")).collect();
            assert_eq!(media.len(), usize::from(!case.starts_with("no-")), "{case}: {requests:?}");
            if case.starts_with("plain") { assert!(media[0].contains("width=320") && media[0].contains("height=240")); }
        }
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse and a synthetic MP4 fixture"]
fn disposable_video_thumbnail_decodes_without_persistent_cache() {
    runtime().block_on(async {
        use matrix_sdk::attachment::{AttachmentConfig, Thumbnail};
        use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
        use matrix_sdk::ruma::events::room::{MediaSource, message::{MessageType, RoomMessageEventContent}};
        let config = fixture();
        let mp4 = std::fs::read(std::env::var("FIXTURE_VIDEO_MP4").unwrap()).unwrap();
        for encrypted in [false, true] {
            let mut create = serde_json::json!({"visibility":"private", "name":"video thumbnail", "creation_content":{"m.federate":false}});
            if encrypted {
                create["initial_state"] = serde_json::json!([{"type":"m.room.encryption", "state_key":"", "content":{"algorithm":"m.megolm.v1.aes-sha2"}}]);
            }
            let room_id = room_request(&config, "alice", reqwest::Method::POST, "createRoom", create).await["room_id"].as_str().unwrap().to_owned();
            // A persistent store makes the no-cache assertion meaningful for disk-backed clients too.
            let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap().join(format!("video-{encrypted}"));
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            let session = AccountStore::create(&root, AccountId::new(26)).unwrap().password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap(),
                "disposable video store passphrase", http(&config),
            ).await.unwrap();
            let client = session.client.as_ref().unwrap();
            client.encryption().wait_for_e2ee_initialization_tasks().await;
            crate::room_sync::RoomSync::sliding_catch_up(client).await.unwrap();
            let room = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room_id).unwrap()).unwrap();
            let scope = matrix_domain::RoomScope { account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new(&room_id) };
            let mut requests = crate::ViewRequests::new();
            let tag = requests.start(scope).unwrap();
            for with_thumbnail in [true, false] {
                let thumbnail = with_thumbnail.then(|| Thumbnail {
                    data: MEDIA_PNG.to_vec(), content_type: "image/png".parse().unwrap(),
                    width: 2u32.into(), height: 2u32.into(), size: (MEDIA_PNG.len() as u32).into(),
                });
                let event = room.send_attachment("synthetic.mp4", &"video/mp4".parse().unwrap(), mp4.clone(), AttachmentConfig::new().thumbnail(thumbnail)).await.unwrap().event_id;
                let wire = room_request(&config, "alice", reqwest::Method::GET, &format!("rooms/{room_id}/event/{event}"), serde_json::json!({})).await;
                assert_eq!(wire["type"], if encrypted { "m.room.encrypted" } else { "m.room.message" });
                let sdk_event = room.event(&event, None).await.unwrap();
                let content = sdk_event.raw().get_field::<RoomMessageEventContent>("content").unwrap().unwrap();
                let MessageType::Video(video) = content.msgtype else { panic!("video required") };
                let source = video.info.as_ref().and_then(|i| i.thumbnail_source.clone());
                assert_eq!(source.is_some(), with_thumbnail);
                if let Some(source) = &source {
                    assert_eq!(matches!(source, MediaSource::Encrypted(_)), encrypted);
                    if let MediaSource::Encrypted(file) = source {
                        let (server, id) = file.url.parts().unwrap();
                        let bytes = http(&config).get(format!("{}_matrix/client/v1/media/download/{server}/{id}", config["url"].as_str().unwrap())).bearer_auth(config["alice_token"].as_str().unwrap()).send().await.unwrap().bytes().await.unwrap();
                        assert!(bytes.as_ref() != MEDIA_PNG, "server stores ciphertext, not the thumbnail PNG");
                    }
                    // Upload helpers may cache: remove that entry before testing the download.
                    client.media().remove_media_content_for_uri(match source { MediaSource::Plain(uri) => uri, MediaSource::Encrypted(file) => &file.url }).await.unwrap();
                }
                let downloaded = session.download_video_thumbnail(&tag, &matrix_domain::EventId::new(event.as_str())).await.unwrap();
                assert_eq!(downloaded.is_some(), with_thumbnail);
                if let Some(downloaded) = downloaded {
                    assert_eq!((downloaded.preview.width, downloaded.preview.height), (2, 2));
                    assert_eq!(downloaded.preview.rgba[0..4], [255, 0, 0, 255]);
                    let expected = image::load_from_memory(MEDIA_PNG).unwrap().into_rgba8();
                    assert_eq!(downloaded.preview.rgba, expected.into_raw());
                    assert_eq!(image::load_from_memory(&downloaded.bytes).unwrap().into_rgba8().into_raw(), downloaded.preview.rgba);
                    if encrypted {
                        assert_eq!(downloaded.bytes, MEDIA_PNG, "SDK decrypts the original encrypted thumbnail");
                    }
                }
                if let Some(source) = source {
                    let request = MediaRequestParameters { source, format: MediaFormat::Thumbnail(MediaThumbnailSettings::new(320u32.into(), 240u32.into())) };
                    assert!(client.media_store().lock().await.unwrap().get_media_content(&request).await.unwrap().is_none(), "download must not persist decrypted media");
                }
            }
            let wrong = requests.start(matrix_domain::RoomScope { account_id: AccountId::new(999), room_id: matrix_domain::RoomId::new(&room_id) }).unwrap();
            assert!(matches!(session.download_video_thumbnail(&wrong, &matrix_domain::EventId::new("$not-fetched")).await, Err(crate::media::MediaError::InvalidScope)));
            requests.cancel();
            assert!(matches!(session.download_video_thumbnail(&tag, &matrix_domain::EventId::new("$not-fetched")).await, Err(crate::media::MediaError::Cancelled)));
        }
    });
}

#[test]
fn media_preview_rejects_unsupported_types_before_decode() {
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        for mime in [
            "image/svg+xml",
            "image/tiff",
            "text/html",
            "image/png; charset=utf-8",
            "",
        ] {
            assert!(
                crate::media_sdk::decode_png(MEDIA_PNG.to_vec(), mime, &tag)
                    .await
                    .is_err(),
                "only exact image MIME types are supported"
            );
        }
    });
}

#[test]
fn media_ceiling_is_25_mib_and_a_five_mib_png_is_accepted() {
    assert_eq!(crate::media::MAX_MEDIA_BYTES, 25 * 1024 * 1024);
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        let mut data = MEDIA_PNG.to_vec();
        data.resize(5 * 1024 * 1024, 0);
        assert!(
            crate::media_sdk::decode_png(data, "image/png", &tag)
                .await
                .is_ok(),
            "a 5 MiB file is below the ceiling"
        );
        let mut data = MEDIA_PNG.to_vec();
        data.resize(25 * 1024 * 1024 + 1, 0);
        assert!(matches!(
            crate::media_sdk::decode_png(data, "image/png", &tag).await,
            Err(crate::media::MediaError::TooLarge)
        ));
    });
}

fn media_generated(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(width, height)
        .write_to(&mut out, format)
        .unwrap();
    out.into_inner()
}

#[test]
fn media_preview_decodes_png_jpeg_webp_and_gif() {
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        for (format, mime) in [
            (image::ImageFormat::Png, "image/png"),
            (image::ImageFormat::Jpeg, "image/jpeg"),
            (image::ImageFormat::WebP, "image/webp"),
            (image::ImageFormat::Gif, "image/gif"),
        ] {
            let preview =
                crate::media_sdk::decode_png(media_generated(format, 640, 480), mime, &tag)
                    .await
                    .unwrap_or_else(|e| panic!("{mime}: {e:?}"));
            assert_eq!((preview.width, preview.height), (320, 240), "{mime}");
        }
    });
}

#[test]
fn media_preview_rejects_bytes_that_do_not_match_the_declared_format() {
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        assert!(matches!(
            crate::media_sdk::decode_png(MEDIA_PNG.to_vec(), "image/jpeg", &tag).await,
            Err(crate::media::MediaError::InvalidImage)
        ));
    });
}

fn media_generated_png(width: u32, height: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(width, height)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[test]
fn media_preview_resizes_wide_images_and_refuses_only_above_the_pixel_cap() {
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        let wide = crate::media_sdk::decode_png(media_generated_png(4096, 16), "image/png", &tag)
            .await
            .expect("wide image under the pixel cap is resized, not refused");
        assert!(wide.width <= 320 && wide.height <= 240);
        assert_eq!(crate::media::MAX_IMAGE_PIXELS, 40_000_000);
        assert!(matches!(
            crate::media_sdk::decode_png(media_generated_png(8001, 5000), "image/png", &tag).await,
            Err(crate::media::MediaError::TooLarge)
        ));
    });
}

#[test]
fn large_view_decode_keeps_both_caps_stays_off_the_ui_thread_and_stops_on_close() {
    let scope = || matrix_domain::RoomScope {
        account_id: AccountId::new(26),
        room_id: matrix_domain::RoomId::new("!media:test"),
    };
    let large = (crate::media_sdk::LARGE_VIEW_SIDE, crate::media_sdk::LARGE_VIEW_SIDE);
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests.start(scope()).unwrap();
        let mut data = MEDIA_PNG.to_vec();
        data.resize(25 * 1024 * 1024 + 1, 0);
        assert!(matches!(
            crate::media_sdk::decode_within(data, "image/png", &tag, large).await,
            Err(crate::media::MediaError::TooLarge)
        ));
        assert!(matches!(
            crate::media_sdk::decode_within(media_generated_png(8001, 5000), "image/png", &tag, large).await,
            Err(crate::media::MediaError::TooLarge)
        ));
        let ok = crate::media_sdk::decode_within(media_generated_png(5000, 100), "image/png", &tag, large)
            .await
            .unwrap();
        assert!(ok.width <= large.0 && ok.width > 320, "kept above bubble size");
        assert_ne!(ok.decoded_on, std::thread::current().id());
        // Closing while the decode is queued: no pixels come back.
        let pending = crate::media_sdk::decode_within(media_generated_png(4000, 4000), "image/png", &tag, large);
        requests.cancel();
        assert!(matches!(pending.await, Err(crate::media::MediaError::Cancelled)));
    });
}

#[test]
fn media_preview_scales_to_bounded_320_by_240_without_upscaling() {
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        let preview =
            crate::media_sdk::decode_png(media_generated_png(640, 480), "image/png", &tag)
                .await
                .unwrap();
        assert_eq!((preview.width, preview.height), (320, 240));
        assert_eq!(preview.rgba.len(), 320 * 240 * 4);
    });
}

#[test]
fn media_preview_cancelled_view_does_not_return_decoded_pixels() {
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        requests.cancel();
        assert!(
            crate::media_sdk::decode_png(MEDIA_PNG.to_vec(), "image/png", &tag)
                .await
                .is_err(),
            "cancelled work must not publish pixels"
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_media_upload_png_has_actual_server_event_and_bytes() {
    runtime().block_on(async {
        let config = fixture();
        let room = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"visibility":"private", "name":"media"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = login(&config, "alice", 26).await;
        crate::room_sync::RoomSync::sliding_catch_up(session.client.as_ref().unwrap()).await
            .unwrap();
        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(26),
            room_id: matrix_domain::RoomId::new(&room),
        };
        let event_id = session
            .send_png(
                &scope,
                &matrix_domain::TransactionId::new("p026-upload"),
                MEDIA_PNG.to_vec(),
            )
            .await
            .expect("attachment accepted");
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", event_id.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(event["content"]["msgtype"] == "m.image");
        assert!(event["content"]["info"]["mimetype"] == "image/png");
        let uri = event["content"]["url"]
            .as_str()
            .expect("plain media reference");
        let uri = <&matrix_sdk::ruma::MxcUri>::from(uri);
        let (server, media_id) = uri.parts().unwrap();
        let response = http(&config)
            .get(format!(
                "{}_matrix/client/v1/media/download/{server}/{media_id}",
                config["url"].as_str().unwrap()
            ))
            .bearer_auth(config["alice_token"].as_str().unwrap())
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert!(
            response.bytes().await.unwrap().as_ref() == MEDIA_PNG,
            "actual uploaded bytes independently read back"
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_media_download_png_returns_exact_bytes_and_preview() {
    runtime().block_on(async {
        let config = fixture();
        let room = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"visibility":"private", "name":"download"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = login(&config, "alice", 26).await;
        crate::room_sync::RoomSync::sliding_catch_up(session.client.as_ref().unwrap()).await
            .unwrap();
        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(26),
            room_id: matrix_domain::RoomId::new(&room),
        };
        let event = session
            .send_png(
                &scope,
                &matrix_domain::TransactionId::new("p026-download"),
                MEDIA_PNG.to_vec(),
            )
            .await
            .unwrap();
        let mut requests = crate::ViewRequests::new();
        let tag = requests.start(scope).unwrap();
        let downloaded = session
            .download_png(&tag, &event, false)
            .await
            .expect("download succeeds");
        assert!(downloaded.bytes == MEDIA_PNG);
        assert_eq!(
            (downloaded.preview.width, downloaded.preview.height),
            (2, 2)
        );
        assert_eq!(downloaded.preview.rgba[0..4], [255, 0, 0, 255]);
        let sync = crate::room_sync::RoomSync::new(session);
        let page = sync.history_page(tag, None).await.unwrap();
        assert!(
            page.events
                .iter()
                .any(|e| e.event_id == event && crate::history::is_image_body(&e.body))
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_media_encrypted_png_is_ciphertext_and_preview_decrypts() {
    runtime().block_on(async {
        let config = fixture();
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"visibility":"private", "name":"encrypted media", "initial_state":[{"type":"m.room.encryption", "state_key":"", "content":{"algorithm":"m.megolm.v1.aes-sha2"}}]})).await["room_id"].as_str().unwrap().to_owned();
        let session = login(&config, "alice", 26).await;
        let client = session.client.as_ref().unwrap();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        crate::room_sync::RoomSync::sliding_catch_up(client).await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new(&room) };
        let event = session.send_png(&scope, &matrix_domain::TransactionId::new("p026-encrypted"), MEDIA_PNG.to_vec()).await.unwrap();
        let wire = room_request(&config, "alice", reqwest::Method::GET, &format!("rooms/{room}/event/{}", event.as_str()), serde_json::json!({})).await;
        assert!(wire["type"] == "m.room.encrypted");
        assert!(wire["content"].get("url").is_none());
        let sdk_room = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap();
        let decrypted = sdk_room.event(&matrix_sdk::ruma::EventId::parse(event.as_str()).unwrap(), None).await.unwrap();
        let content = decrypted.raw().get_field::<serde_json::Value>("content").unwrap().unwrap();
        assert!(content.get("url").is_none());
        let uri = <&matrix_sdk::ruma::MxcUri>::from(content["file"]["url"].as_str().unwrap());
        let (server, id) = uri.parts().unwrap();
        let wire_bytes = http(&config).get(format!("{}_matrix/client/v1/media/download/{server}/{id}", config["url"].as_str().unwrap())).bearer_auth(config["alice_token"].as_str().unwrap()).send().await.unwrap().bytes().await.unwrap();
        assert!(wire_bytes.as_ref() != MEDIA_PNG, "media blob must also be ciphertext");
        let mut requests = crate::ViewRequests::new();
        let tag = requests.start(scope).unwrap();
        let downloaded = session.download_png(&tag, &event, false).await.expect("SDK attachment decryption succeeds");
        assert!(downloaded.bytes == MEDIA_PNG);
        assert_eq!(downloaded.preview.rgba[0..4], [255,0,0,255]);
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_media_cancel_interrupts_blocked_network_read() {
    runtime().block_on(async {
        let config = fixture();
        let room = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"visibility":"private", "name":"cancel"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = login(&config, "alice", 26).await;
        crate::room_sync::RoomSync::sliding_catch_up(session.client.as_ref().unwrap()).await
            .unwrap();
        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(26),
            room_id: matrix_domain::RoomId::new(&room),
        };
        let event = session
            .send_png(
                &scope,
                &matrix_domain::TransactionId::new("p026-cancel"),
                MEDIA_PNG.to_vec(),
            )
            .await
            .unwrap();
        let mut requests = crate::ViewRequests::new();
        let tag = requests.start(scope).unwrap();
        let paused = PausedServer::new(&config);
        let cancel = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            requests.cancel();
        });
        let start = std::time::Instant::now();
        assert!(session.download_png(&tag, &event, false).await.is_err());
        assert!(
            start.elapsed() < std::time::Duration::from_millis(750),
            "view cancellation must interrupt network I/O promptly"
        );
        cancel.await.unwrap();
        drop(paused);
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_media_worker_routes_upload_download_and_clean_shutdown() {
    let config = fixture();
    let room = runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"visibility":"private", "name":"worker"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let (mut worker, mut rooms) = crate::room_sync::RoomListWorker::spawn(move || async move {
        Ok(login(&config, "alice", 26).await)
    })
    .unwrap();
    let (mut commands, mut results) = worker.take_media_route().expect("single media route");
    assert!(worker.take_media_route().is_none());
    runtime().block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !matches!(
                rooms.recv().await,
                Some(crate::RoomListEvent::Status(crate::RoomSyncStatus::Ready))
            ) {}
        })
        .await
        .unwrap();
        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(26),
            room_id: matrix_domain::RoomId::new(&room),
        };
        let transaction = matrix_domain::TransactionId::new("p026-worker");
        assert!(
            commands
                .send(crate::media::MediaCommand::Send {
                    scope: scope.clone(),
                    transaction: transaction.clone(),
                    bytes: MEDIA_PNG.to_vec()
                })
                .await
                .is_ok()
        );
        let Some(crate::media::MediaEvent::Sent {
            scope: returned_scope,
            transaction: returned_transaction,
            result,
        }) = results.recv().await
        else {
            panic!("send result required");
        };
        assert!(returned_scope == scope && returned_transaction == transaction);
        let event = result.unwrap();
        let mut requests = crate::ViewRequests::new();
        let tag = requests.start(scope).unwrap();
        assert!(
            commands
                .send(crate::media::MediaCommand::Download {
                    tag: tag.clone(),
                    event: event.clone(),
                    original: false
                })
                .await
                .is_ok()
        );
        let Some(crate::media::MediaEvent::Downloaded {
            tag: returned,
            event: returned_event,
            result,
        }) = results.recv().await
        else {
            panic!("preview result required");
        };
        assert!(requests.accepts(&returned) && event == returned_event);
        assert!(result.unwrap().bytes == MEDIA_PNG);
        let paused = PausedServer::new(&fixture());
        assert!(
            commands
                .send(crate::media::MediaCommand::Download { tag, event, original: false })
                .await
                .is_ok()
        );
        let cached = tokio::time::timeout(std::time::Duration::from_millis(750), results.recv())
            .await
            .expect("second read must hit bounded worker cache without network");
        let Some(crate::media::MediaEvent::Downloaded { result, .. }) = cached else {
            panic!("cached preview required");
        };
        assert!(result.unwrap().bytes == MEDIA_PNG);
        drop(paused);
    });
    worker.shutdown().unwrap();
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_media_rejects_untrusted_payloads_and_wrong_account() {
    runtime().block_on(async {
        use crate::media::MediaError;
        let config = fixture();
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"visibility":"private", "name":"limits"})).await["room_id"].as_str().unwrap().to_owned();
        let session = login(&config, "alice", 26).await;
        crate::room_sync::RoomSync::sliding_catch_up(session.client.as_ref().unwrap()).await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new(&room) };
        let mut requests = crate::ViewRequests::new();
        let tag = requests.start(scope.clone()).unwrap();
        for (index, (data, mime, expected)) in [
            (vec![0; crate::media::MAX_MEDIA_BYTES + 1], "image/png", MediaError::TooLarge),
            (b"not an image".to_vec(), "image/png", MediaError::InvalidImage),
            (MEDIA_PNG.to_vec(), "image/svg+xml", MediaError::UnsupportedType),
        ].into_iter().enumerate() {
            let response = http(&config).post(format!("{}_matrix/media/v3/upload", config["url"].as_str().unwrap()))
                .bearer_auth(config["alice_token"].as_str().unwrap()).header("Content-Type", mime).body(data).send().await.unwrap();
            assert!(response.status().is_success());
            let response: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            let uri = response["content_uri"].as_str().unwrap().to_owned();
            let event = room_request(&config,"alice",reqwest::Method::PUT,&format!("rooms/{room}/send/m.room.message/negative{index}"),serde_json::json!({"msgtype":"m.image","body":"image","url":uri,"info":{"mimetype":mime,"size":1}})).await["event_id"].as_str().unwrap().to_owned();
            assert!(matches!(session.download_png(&tag, &matrix_domain::EventId::new(event), false).await, Err(error) if error == expected));
        }
        let wrong = requests.start(matrix_domain::RoomScope { account_id: AccountId::new(999), ..scope.clone() }).unwrap();
        assert!(matches!(session.download_png(&wrong, &matrix_domain::EventId::new("$no-network"), false).await, Err(crate::media::MediaError::InvalidScope)));
        assert!(matches!(session.send_png(&scope, &matrix_domain::TransactionId::new("invalid"), b"not PNG".to_vec()).await, Err(crate::media::MediaError::InvalidImage)));
        let messages = room_request(&config,"alice",reqwest::Method::GET,&format!("rooms/{room}/messages?dir=b&limit=100"),serde_json::json!({})).await;
        assert_eq!(messages["chunk"].as_array().unwrap().iter().filter(|event| event["type"] == "m.room.message").count(), 3, "invalid upload must not create a message");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_media_shutdown_deadline_never_claims_pending_upload_succeeded() {
    let config = fixture();
    let room = runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"visibility":"private", "name":"drain"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let (mut worker, mut rooms) = crate::room_sync::RoomListWorker::spawn(move || async move {
        Ok(login(&config, "alice", 26).await)
    })
    .unwrap();
    let (mut commands, _results) = worker.take_media_route().unwrap();
    runtime().block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !matches!(
                rooms.recv().await,
                Some(crate::RoomListEvent::Status(crate::RoomSyncStatus::Ready))
            ) {}
        })
        .await
        .unwrap();
    });
    let paused = PausedServer::new(&fixture());
    assert!(
        commands
            .try_send(crate::media::MediaCommand::Send {
                scope: matrix_domain::RoomScope {
                    account_id: AccountId::new(26),
                    room_id: matrix_domain::RoomId::new(room)
                },
                transaction: matrix_domain::TransactionId::new("p026-drain"),
                bytes: MEDIA_PNG.to_vec(),
            })
            .is_ok()
    );
    let start = std::time::Instant::now();
    assert_eq!(
        worker.shutdown(),
        Err(crate::room_sync::ShutdownError::DrainDeadline)
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(9));
    drop(paused);
}

#[test]
fn media_preview_decodes_original_rgba_off_runtime_thread() {
    runtime().block_on(async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(26),
                room_id: matrix_domain::RoomId::new("!media:test"),
            })
            .unwrap();
        let preview = crate::media_sdk::decode_png(MEDIA_PNG.to_vec(), "image/png", &tag)
            .await
            .unwrap();
        assert_eq!((preview.width, preview.height), (2, 2));
        assert_eq!(
            preview.rgba,
            [
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255
            ]
        );
        assert_ne!(preview.decoded_on, std::thread::current().id());
    });
}

#[test]
fn media_cached_bytes_are_decoded_with_their_own_format() {
    for (format, mime) in [
        (image::ImageFormat::Png, "image/png"),
        (image::ImageFormat::Jpeg, "image/jpeg"),
        (image::ImageFormat::WebP, "image/webp"),
        (image::ImageFormat::Gif, "image/gif"),
    ] {
        assert_eq!(
            crate::media_sdk::mime_of(&media_generated(format, 8, 8)),
            Some(mime)
        );
    }
    assert_eq!(crate::media_sdk::mime_of(b"not an image"), None);
}

#[test]
fn image_bubble_uses_thumbnail_and_keeps_original_a_separate_resource() {
    runtime().block_on(async {
        use crate::media::MediaVariant::{Original, Thumbnail};
        use matrix_sdk::ruma::{api::MatrixVersion, events::room::EncryptedFile, owned_mxc_uri};
        use std::{io::Read, sync::{Arc, Mutex}};
        use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};
        // (case, encrypted, thumbnail reply, original size, expected variant, expected size, thumbnail requests, download requests)
        let over_cap = { let mut b = media_generated_png(320, 240); b.resize(1_048_577, 0); b };
        let table = vec![
            ("small", false, Some(media_generated_png(100, 80)), (100, 80), Thumbnail, (100, 80), 1, 0),
            ("large", false, Some(media_generated_png(320, 240)), (1000, 800), Thumbnail, (320, 240), 1, 0),
            ("over-cap", false, Some(over_cap), (640, 480), Original, (320, 240), 1, 1),
            ("encrypted", true, None, (640, 480), Original, (320, 240), 0, 1),
            ("no-thumbnail", false, None, (640, 480), Original, (320, 240), 1, 1),
        ];
        for (case, encrypted, thumb, (ow, oh), variant, (w, h), thumb_requests, downloads) in table {
            let mut original = media_generated_png(ow, oh);
            let mut content = serde_json::json!({"msgtype":"m.image", "body":"a.png", "info":{"mimetype":"image/png"}});
            if encrypted {
                let mut input = std::io::Cursor::new(&original);
                let mut encryptor = matrix_sdk_crypto::AttachmentEncryptor::new(&mut input);
                let mut ciphertext = Vec::new();
                encryptor.read_to_end(&mut ciphertext).unwrap();
                let info = encryptor.finish();
                content["file"] = serde_json::to_value(EncryptedFile::new(owned_mxc_uri!("mxc://fixture.invalid/original"), info.encryption_info, info.hashes)).unwrap();
                original = ciphertext;
            } else {
                content["url"] = serde_json::json!("mxc://fixture.invalid/original");
            }
            let event = serde_json::json!({"type":"m.room.message", "event_id":"$image", "room_id":"!image:fixture.invalid", "sender":"@alice:fixture.invalid", "origin_server_ts":1, "content":content});
            let sync = serde_json::json!({"pos":"1", "lists":{"all_rooms":{"count":1}}, "extensions":{}, "rooms":{"!image:fixture.invalid":{"initial":true, "joined_count":1, "required_state":[{"type":"m.room.member", "state_key":"@alice:fixture.invalid", "sender":"@alice:fixture.invalid", "event_id":"$member", "origin_server_ts":1, "content":{"membership":"join"}}], "timeline":[]}}});
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::<String>::new()));
            let recorded = requests.clone();
            let server = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") { headers.push(stream.read_u8().await.unwrap()); }
                    let headers = String::from_utf8(headers).unwrap();
                    // Windows resets the connection (RST) when we close with an unread request
                    // body, which the SDK reports as a transport failure; drain the body first.
                    if let Some(len) = headers
                        .split("\r\n")
                        .find_map(|l| {
                            l.split_once(':').and_then(|(k, v)| {
                                k.eq_ignore_ascii_case("content-length").then(|| v.trim().to_owned())
                            })
                        })
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        let mut request_body = vec![0u8; len as usize];
                        let _ = stream.read_exact(&mut request_body).await;
                    }
                    let path = headers.split_whitespace().nth(1).unwrap().to_owned();
                    recorded.lock().unwrap().push(path.clone());
                    let (status, response) = if path.contains("/sync?") { ("200 OK", serde_json::to_vec(&sync).unwrap()) }
                        else if path.contains("/event/") { ("200 OK", serde_json::to_vec(&event).unwrap()) }
                        else if path.contains("/thumbnail/") { match &thumb { Some(b) => ("200 OK", b.clone()), None => ("404 Not Found", br#"{"errcode":"M_NOT_FOUND","error":"no thumbnail"}"#.to_vec()) } }
                        else if path.contains("/download/") { ("200 OK", original.clone()) }
                        else { ("200 OK", b"{}".to_vec()) };
                    let header = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", response.len());
                    if stream.write_all(header.as_bytes()).await.is_ok() {
                        let _ = stream.write_all(&response).await;
                        let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
                    }
                }
            });
            let client = matrix_sdk::Client::builder().homeserver_url(format!("http://{address}"))
                .sliding_sync_version_builder(matrix_sdk::sliding_sync::VersionBuilder::None)
                .request_config(matrix_sdk::config::RequestConfig::new().retry_limit(3))
                .server_versions([MatrixVersion::V1_0]).build().await.unwrap();
            let session: matrix_sdk::authentication::matrix::MatrixSession = serde_json::from_value(serde_json::json!({"user_id":"@alice:fixture.invalid", "device_id":"TEST", "access_token":"synthetic"})).unwrap();
            client.restore_session(session).await.unwrap();
            crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
            let mut views = crate::ViewRequests::new();
            let tag = views.start(matrix_domain::RoomScope { account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new("!image:fixture.invalid") }).unwrap();
            let got = crate::media_sdk::download_png(&client, AccountId::new(26), &tag, &matrix_domain::EventId::new("$image"), false).await
                .unwrap_or_else(|e| panic!("{case}: {e:?}"));
            assert_eq!(got.variant, variant, "{case}");
            assert_eq!((got.preview.width, got.preview.height), (w, h), "{case}");
            {
                let requests = requests.lock().unwrap();
                let count = |needle: &str| requests.iter().filter(|p| p.contains(needle)).count();
                assert_eq!((count("/thumbnail/"), count("/download/")), (thumb_requests, downloads), "{case}: {requests:?}");
            }
            // The large view asks for the original (File format, decrypted by the SDK) and gets more pixels than the bubble.
            let large = crate::media_sdk::download_png(&client, AccountId::new(26), &tag, &matrix_domain::EventId::new("$image"), true).await
                .unwrap_or_else(|e| panic!("{case} original: {e:?}"));
            server.abort();
            assert_eq!(large.variant, Original, "{case}");
            assert_eq!((large.preview.width, large.preview.height), (ow, oh), "{case}");
            assert!(u64::from(large.preview.width) * u64::from(large.preview.height) >= u64::from(w) * u64::from(h), "{case}");
            if case == "large" { assert!(large.preview.width > w && large.preview.height > h); }
            let requests = requests.lock().unwrap();
            assert_eq!(requests.iter().filter(|p| p.contains("/thumbnail/")).count(), thumb_requests, "{case}: original skips the thumbnail");
        }
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_avatar_thumbnails_download_cache_cap_and_logout() {
    use crate::avatar_cache::{AVATAR_MAX_BYTES, AVATAR_PX, AvatarCache};
    use image::GenericImageView;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let png = |w: u32, h: u32, f: &dyn Fn(u32, u32) -> [u8; 3]| {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_fn(w, h, |x, y| image::Rgb(f(x, y))).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let colour = |c: u8| png(400, 400, &move |x, y| [c, (x / 2) as u8, (y / 2) as u8]);
    let noise = png(400, 400, &|x, y| { let n = x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ (x * y); [n as u8, (n >> 8) as u8, (n >> 16) as u8] });
    runtime().block_on(async {
        let config = fixture();
        let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap().join("avatars");
        std::fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let mut session = AccountStore::create(&root, AccountId::new(195)).unwrap().password_login_with_http(
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap(),
            "disposable avatar store passphrase", http(&config),
        ).await.unwrap();
        let client = session.client.clone().unwrap();
        let mime: mime_guess::mime::Mime = "image/png".parse().unwrap();
        let mut mxcs = vec![client.account().upload_avatar(&mime, colour(10)).await.unwrap().to_string()];
        for (name, kind) in [("gnobbi-test-room", None), ("gnobbi-test-space", Some("m.space"))] {
            let mut create = serde_json::json!({"visibility":"private", "name":name});
            if let Some(kind) = kind { create["creation_content"] = serde_json::json!({"type":kind}); }
            let id = room_request(&config, "alice", reqwest::Method::POST, "createRoom", create).await["room_id"].as_str().unwrap().to_owned();
            crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
            let room = client.get_room(&matrix_sdk::ruma::RoomId::parse(&id).unwrap()).unwrap();
            room.upload_avatar(&mime, colour(if kind.is_some() { 200 } else { 100 }), None).await.unwrap();
            crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
            mxcs.push(room.avatar_url().unwrap().to_string());
        }
        let upload = |data: Vec<u8>| { let client = client.clone(); let mime = mime.clone(); async move { client.media().upload(&mime, data, None).await.unwrap().content_uri.to_string() } };
        let cache = session.avatar_cache().unwrap();
        let dir = cache.dir().to_owned();

        // Download: user, room and space avatars arrive as 96x96 thumbnails, not the 400x400 originals.
        for mxc in &mxcs {
            let bytes = session.avatar_thumbnail(mxc).await.unwrap().expect("thumbnail");
            assert_eq!(image::load_from_memory(&bytes).unwrap().dimensions(), (AVATAR_PX, AVATAR_PX));
            assert!(bytes.len() <= AVATAR_MAX_BYTES);
        }
        assert_eq!(cache.downloads(), 3);
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|f| mode(f) == 0o600));

        // Cache: a second round does no network request; broken or missing avatars give the initials.
        for mxc in &mxcs { assert!(session.avatar_thumbnail(mxc).await.unwrap().is_some()); }
        assert_eq!(cache.downloads(), 3);
        let server = mxcs[0].trim_start_matches("mxc://").split('/').next().unwrap().to_owned();
        assert!(session.avatar_thumbnail(&format!("mxc://{server}/doesnotexist")).await.unwrap().is_none());
        assert!(session.avatar_thumbnail("https://example.invalid/a.png").await.unwrap().is_none());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3, "failures leave no file");

        // Cap: a thumbnail over the byte cap is cut while received and not stored.
        let capped = AvatarCache::with_limits(root.join("cap"), 2048, 8, 4);
        assert!(capped.get(&client, &upload(noise).await).await.is_none());
        assert_eq!((capped.downloads(), capped.dir().exists()), (1, false));
        // Cap on entries: the oldest file goes.
        let few = AvatarCache::with_limits(root.join("few"), AVATAR_MAX_BYTES, 2, 4);
        for mxc in &mxcs { assert!(few.get(&client, mxc).await.is_some()); }
        assert_eq!(std::fs::read_dir(few.dir()).unwrap().count(), 2);
        // Cap on concurrency and no duplicate requests.
        let mut many = Vec::new();
        for c in 0..12 { many.push(upload(colour(c)).await); }
        let narrow = AvatarCache::with_limits(root.join("narrow"), AVATAR_MAX_BYTES, 512, 2);
        let got = futures_util::future::join_all(many.iter().map(|m| narrow.get(&client, m))).await;
        assert!(got.iter().all(Option::is_some));
        assert_eq!(narrow.downloads(), 12);
        assert!(narrow.peak_concurrent() <= 2, "peak {}", narrow.peak_concurrent());
        let same = AvatarCache::with_limits(root.join("same"), AVATAR_MAX_BYTES, 512, 4);
        let got = futures_util::future::join_all((0..8).map(|_| same.get(&client, &many[0]))).await;
        assert!(got.iter().all(Option::is_some));
        assert_eq!(same.downloads(), 1);

        // Logout deletes the whole cache; nothing is served afterwards.
        session.logout().await.unwrap();
        assert!(!dir.exists());
        assert!(matches!(session.avatar_thumbnail(&mxcs[0]).await, Err(crate::media::MediaError::LoggedOut)));
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_avatar_edge_cases_corrupt_huge_unreachable_changed() {
    use crate::avatar_cache::{AVATAR_MAX_BYTES, AVATAR_PX};
    use image::GenericImageView;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let png = |w: u32, h: u32, c: u8| {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(w, h, image::Rgb([c, 90, 200])).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    runtime().block_on(async {
        let config = fixture();
        let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap().join("avatars-edge");
        std::fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let session = AccountStore::create(&root, AccountId::new(196)).unwrap().password_login_with_http(
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap(),
            "disposable avatar edge passphrase", http(&config),
        ).await.unwrap();
        let client = session.client.clone().unwrap();
        let mime: mime_guess::mime::Mime = "image/png".parse().unwrap();
        let cache = session.avatar_cache().unwrap();
        let dir = cache.dir().to_owned();
        let files = || std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);

        // Corrupt: PNG signature then junk. Never a panic; whatever is returned must not be a usable picture.
        let mut junk = b"\x89PNG\r\n\x1a\n".to_vec();
        junk.extend((0..2000u32).map(|i| (i * 31 % 251) as u8));
        let corrupt = client.media().upload(&mime, junk, None).await.unwrap().content_uri.to_string();
        let got = session.avatar_thumbnail(&corrupt).await.unwrap();
        eprintln!("EDGE corrupt: returned={:?} files={}", got.as_ref().map(Vec::len), files());
        assert!(got.as_ref().is_none_or(|b| image::load_from_memory(b).is_err()));

        // Enormous: 5000x5000 (under Synapse's pixel limit) is cut to a thumbnail; 9000x9000 is refused.
        let big = client.media().upload(&mime, png(5000, 5000, 1), None).await.unwrap().content_uri.to_string();
        let thumb = session.avatar_thumbnail(&big).await.unwrap();
        eprintln!("EDGE big5000: {:?}", thumb.as_ref().map(|b| image::load_from_memory(b).map(|i| i.dimensions())));
        if let Some(b) = &thumb { assert!(b.len() <= AVATAR_MAX_BYTES); assert_eq!(image::load_from_memory(b).unwrap().dimensions(), (AVATAR_PX, AVATAR_PX)); }
        let huge = client.media().upload(&mime, png(9000, 9000, 2), None).await.unwrap().content_uri.to_string();
        let thumb = session.avatar_thumbnail(&huge).await.unwrap();
        eprintln!("EDGE huge9000: {:?}", thumb.as_ref().map(|b| (b.len(), image::load_from_memory(b).map(|i| i.dimensions()))));
        if let Some(b) = &thumb { assert!(b.len() <= AVATAR_MAX_BYTES); }

        // Unreachable: a remote server that does not exist gives the initials, within the request timeout.
        let before = files();
        let started = std::time::Instant::now();
        let none = session.avatar_thumbnail("mxc://unreachable.invalid/abcdefghijklmnop").await.unwrap();
        eprintln!("EDGE unreachable: {:?} in {:?}", none.as_ref().map(Vec::len), started.elapsed());
        assert!(none.is_none() && started.elapsed() < std::time::Duration::from_secs(40));
        assert_eq!(files(), before, "failure leaves no file");

        // Final vs transient: a missing media id is final; a homeserver nobody listens on is worth a retry.
        let missing = "mxc://matrix-desktop.test/doesnotexist";
        assert_eq!(cache.try_get(&client, missing).await.err(), Some(false));
        let dead = matrix_sdk::Client::builder().homeserver_url("http://127.0.0.1:1").build().await.unwrap();
        assert_eq!(cache.try_get(&dead, "mxc://gone.invalid/abcdefghijklmnop").await.err(), Some(true));

        // Changed after the cache: a new mxc is a new download and shows the new picture.
        let first = client.account().upload_avatar(&mime, png(400, 400, 10)).await.unwrap().to_string();
        let a = session.avatar_thumbnail(&first).await.unwrap().unwrap();
        let second = client.account().upload_avatar(&mime, png(400, 400, 240)).await.unwrap().to_string();
        assert_ne!(first, second);
        let downloads = cache.downloads();
        let b = session.avatar_thumbnail(&second).await.unwrap().unwrap();
        assert_ne!(a, b);
        assert_eq!(cache.downloads(), downloads + 1);
        let again = session.avatar_thumbnail(&first).await.unwrap().unwrap();
        assert_eq!(again, a, "the old picture is still served from the cache for the old mxc");
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| mode(&e.unwrap().path()) == 0o600));
        eprintln!("EDGE changed: ok; files={}", files());
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_avatar_logout_with_requests_in_flight_leaves_nothing() {
    use crate::avatar::AvatarSource;
    use std::os::unix::fs::DirBuilderExt;
    let png = |c: u8| {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(400, 400, image::Rgb([c, 90, 200])).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let rt = runtime();
    rt.block_on(async {
        let config = fixture();
        let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap().join("avatars-race");
        std::fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let mut session = AccountStore::create(&root, AccountId::new(197)).unwrap().password_login_with_http(
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap(),
            "disposable avatar race passphrase", http(&config),
        ).await.unwrap();
        let client = session.client.clone().unwrap();
        let mime: mime_guess::mime::Mime = "image/png".parse().unwrap();
        let mut mxcs = Vec::new();
        for c in 0..40 { mxcs.push(client.media().upload(&mime, png(c), None).await.unwrap().content_uri.to_string()); }
        let dir = session.avatar_cache().unwrap().dir().to_owned();
        let fetcher = crate::avatar_cache::AvatarFetcher::new(&session, tokio::runtime::Handle::current()).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        for m in &mxcs { fetcher.request(m.clone(), tx.clone()); }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        session.logout().await.unwrap();
        let after_logout = dir.exists();
        let mut answered = 0;
        for _ in 0..40 { if rx.recv_timeout(std::time::Duration::from_secs(20)).is_ok() { answered += 1; } }
        let left = std::fs::read_dir(&dir).map(|d| d.count()).ok();
        eprintln!("RACE answered={answered} dir_after_logout={after_logout} dir_after_settle={} files={left:?}", dir.exists());
        assert!(!dir.exists(), "avatar cache recreated after logout: {left:?} files");
    });
}
