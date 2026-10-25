const MEDIA_PNG: &[u8] = include_bytes!("../tests/fixtures/checker.png");

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
            crate::media_sdk::decode_png(data, "image/png", &tag).await.is_ok(),
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
            let preview = crate::media_sdk::decode_png(media_generated(format, 640, 480), mime, &tag)
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
        session
            .client
            .as_ref()
            .unwrap()
            .sync_once(matrix_sdk::config::SyncSettings::default())
            .await
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
        session
            .client
            .as_ref()
            .unwrap()
            .sync_once(matrix_sdk::config::SyncSettings::default())
            .await
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
            .download_png(&tag, &event)
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
        client.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
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
        let downloaded = session.download_png(&tag, &event).await.expect("SDK attachment decryption succeeds");
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
        session
            .client
            .as_ref()
            .unwrap()
            .sync_once(matrix_sdk::config::SyncSettings::default())
            .await
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
        assert!(session.download_png(&tag, &event).await.is_err());
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
                    event: event.clone()
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
                .send(crate::media::MediaCommand::Download { tag, event })
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
        session.client.as_ref().unwrap().sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
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
            assert!(matches!(session.download_png(&tag, &matrix_domain::EventId::new(event)).await, Err(error) if error == expected));
        }
        let wrong = requests.start(matrix_domain::RoomScope { account_id: AccountId::new(999), ..scope.clone() }).unwrap();
        assert!(matches!(session.download_png(&wrong, &matrix_domain::EventId::new("$no-network")).await, Err(crate::media::MediaError::InvalidScope)));
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
        assert_eq!(crate::media_sdk::mime_of(&media_generated(format, 8, 8)), Some(mime));
    }
    assert_eq!(crate::media_sdk::mime_of(b"not an image"), None);
}
