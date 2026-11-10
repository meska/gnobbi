// Real SDK and persistent store behind a selective loopback TLS relay.
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_sliding_cache_requires_live_response_after_restart() {
    use std::io::BufRead;
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let gate = root.join("sliding-blocked");
    let observed = root.join("sliding-observed");
    let room = runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"cache fixture","visibility":"private"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let mut proxy = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/sliding_gate.py"
        ))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    struct Cleanup(std::process::Child);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut reader = std::io::BufReader::new(proxy.stdout.take().unwrap());
    let mut url = String::new();
    reader.read_line(&mut url).unwrap();
    assert!(url.starts_with("https://localhost:"));
    let _proxy = Cleanup(proxy);
    let mut behind = config.clone();
    behind["url"] = url.trim().into();
    let phrase = "disposable sliding cache phrase";
    let session = runtime().block_on(async {
        AccountStore::create(root, AccountId::new(121))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(behind["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                behind["alice_password"].as_str().unwrap(),
                phrase,
                http(&behind),
            )
            .await
            .unwrap()
    });
    let (worker, mut events) =
        RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
    runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut ready = false;
            let mut found = false;
            while !(ready && found) {
                match events.recv().await.expect("warm events") {
                    RoomListEvent::Status(RoomSyncStatus::Ready) => ready = true,
                    RoomListEvent::Diff(crate::RoomListDiff::Upsert(summary))
                        if summary.scope.room_id.as_str() == room =>
                    {
                        found = true
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("warm store and room list");
    });
    worker.shutdown().unwrap();
    for _ in 0..2 {
        let _ = std::fs::remove_file(&observed);
        std::fs::write(&gate, b"blocked").unwrap();
        let session = runtime().block_on(async {
            AccountStore::open(root, AccountId::new(121))
                .unwrap()
                .restore_with_http(phrase, http(&behind))
                .await
                .unwrap()
        });
        let (worker, mut events) =
            RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
        runtime().block_on(async {
            let versions = http(&behind)
                .get(format!("{}_matrix/client/versions", url.trim()))
                .send()
                .await
                .unwrap();
            assert!(
                versions.status().is_success(),
                "versions reachable while sliding blocked"
            );
            tokio::time::timeout(Duration::from_secs(15), async {
                while !observed.exists() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("sliding request reached gate");
            let mut cached = false;
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    match events.recv().await.expect("events while blocked") {
                        RoomListEvent::Status(RoomSyncStatus::Ready) => {
                            panic!("Ready without sliding response")
                        }
                        RoomListEvent::Diff(crate::RoomListDiff::Upsert(summary))
                            if summary.scope.room_id.as_str() == room =>
                        {
                            cached = true
                        }
                        _ => {}
                    }
                }
            })
            .await;
            assert!(result.is_err());
            assert!(
                cached,
                "warm cached room must be observed while sliding is blocked"
            );
            std::fs::remove_file(&gate).unwrap();
            tokio::time::timeout(Duration::from_secs(30), async {
                while !matches!(
                    events.recv().await,
                    Some(RoomListEvent::Status(RoomSyncStatus::Ready))
                ) {}
            })
            .await
            .expect("Ready after successful sliding response");
        });
        worker.shutdown().unwrap();
    }
    let failing = root.join("sliding-failing");
    std::fs::write(&failing, b"503").unwrap();
    let session = runtime().block_on(async {
        AccountStore::open(root, AccountId::new(121))
            .unwrap()
            .restore_with_http(phrase, http(&behind))
            .await
            .unwrap()
    });
    let (worker, mut events) =
        RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
    runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                match events.recv().await.expect("events during error") {
                    RoomListEvent::Status(RoomSyncStatus::SyncFailed) => break,
                    RoomListEvent::Status(RoomSyncStatus::Ready) => {
                        panic!("Ready while sync returns 503")
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("sliding error reported");
        let _ = std::fs::remove_file(&observed);
        std::fs::write(&gate, b"blocked").unwrap();
        std::fs::remove_file(&failing).unwrap();
        tokio::time::timeout(Duration::from_secs(60), async {
            while !observed.exists() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("restart reached sliding gate");
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(
                    events.recv().await,
                    Some(RoomListEvent::Status(RoomSyncStatus::Ready))
                ) {
                    panic!("Ready on restart before live response");
                }
            }
        })
        .await;
        assert!(result.is_err());
        std::fs::remove_file(&gate).unwrap();
        tokio::time::timeout(Duration::from_secs(45), async {
            while !matches!(
                events.recv().await,
                Some(RoomListEvent::Status(RoomSyncStatus::Ready))
            ) {}
        })
        .await
        .expect("Ready after recovery response");
    });
    worker.shutdown().unwrap();
}
