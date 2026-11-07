// Own-account avatar (drawer header) end to end on a disposable Synapse or Tuwunel, through the
// real worker: Profile event -> shared Avatars source -> 96 px picture. Run by the auth_smoke script
// with --server synapse|tuwunel --filter disposable_account_avatar.
use crate::avatar::{AvatarPicture, AvatarSlot};
use crate::verification::{VerificationCommand, VerificationEvent};

struct AvatarAccount {
    _worker: crate::room_sync::RoomListWorker,
    rooms: crate::Receiver<crate::RoomListEvent>,
    events: crate::Receiver<VerificationEvent>,
    _commands: crate::Sender<VerificationCommand>, // dropping it ends the verification service
    slot: AvatarSlot,
}

fn avatar_png(seed: u8) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_fn(300, 300, |x, y| image::Rgb([seed, (x / 2) as u8, (y / 2) as u8]))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn avatar_root(config: &serde_json::Value) -> std::path::PathBuf {
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap().join("avatars");
    let mut builder = std::fs::DirBuilder::new();
    // Windows has no mode bits: the fixture profile dir is already user-private.
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    let _ = builder.create(&root);
    root
}

/// The account's picture as the server has it, set over REST with the fixture token (no extra
/// login: Synapse rate-limits logins). `None` removes it.
async fn avatar_set(config: &serde_json::Value, name: &str, png: Option<Vec<u8>>) -> Option<String> {
    let (url, token) = (config["url"].as_str().unwrap(), config[format!("{name}_token")].as_str().unwrap());
    let client = http(config);
    let mxc = match png {
        Some(bytes) => {
            let up = client.post(format!("{url}_matrix/media/v3/upload?filename=a.png")).bearer_auth(token)
                .header("Content-Type", "image/png").body(bytes).send().await.unwrap();
            assert!(up.status().is_success(), "upload: {}", up.status());
            let json: serde_json::Value = serde_json::from_slice(&up.bytes().await.unwrap()).unwrap();
            Some(json["content_uri"].as_str().unwrap().to_owned())
        }
        None => None,
    };
    let body = mxc.as_ref().map_or(serde_json::json!({"avatar_url": ""}), |m| serde_json::json!({"avatar_url": m}));
    let put = client.put(format!("{url}_matrix/client/v3/profile/@{name}:matrix-desktop.test/avatar_url"))
        .bearer_auth(token).header("Content-Type", "application/json").body(body.to_string()).send().await.unwrap();
    let status = put.status();
    assert!(status.is_success(), "profile {body}: {status} {}", put.text().await.unwrap());
    mxc
}

fn avatar_start(config: &serde_json::Value, name: &'static str, id: u64) -> AvatarAccount {
    let (config, root) = (config.clone(), avatar_root(config));
    let (mut worker, rooms) = crate::room_sync::RoomListWorker::spawn(move || async move {
        Ok(AccountStore::create(&root, AccountId::new(id)).unwrap().password_login_with_http(
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            &format!("@{name}:matrix-desktop.test"),
            config[format!("{name}_password")].as_str().unwrap(),
            "disposable avatar store passphrase",
            http(&config),
        ).await.unwrap())
    })
    .unwrap();
    let (commands, events) = worker.take_verification_route().unwrap();
    AvatarAccount { slot: worker.avatar_slot(), _worker: worker, rooms, events, _commands: commands }
}

/// Next Profile event whose avatar satisfies `accept`; the room route is drained meanwhile.
async fn avatar_profile(a: &mut AvatarAccount, secs: u64, accept: impl Fn(&Option<String>) -> bool) -> Option<String> {
    let seen = std::cell::RefCell::new(Vec::new());
    tokio::time::timeout(Duration::from_secs(secs), async {
        loop {
            tokio::select! {
                e = a.rooms.recv() => { e.expect("room route open"); }
                e = a.events.recv() => {
                    if let VerificationEvent::Profile { avatar, .. } = e.expect("verification route open") {
                        seen.borrow_mut().push(avatar.clone());
                        if accept(&avatar) { return avatar; }
                    }
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no wanted Profile event in {secs} s; Profile avatars seen: {:?}", seen.borrow()))
}

/// What the window's Avatars source answers for `mxc`.
async fn avatar_picture(a: &AvatarAccount, mxc: &str) -> AvatarPicture {
    tokio::time::timeout(Duration::from_secs(40), async {
        let (tx, rx) = std::sync::mpsc::channel();
        while !a.slot.lock().unwrap().as_ref().is_some_and(|s| { s.request(mxc.to_owned(), tx.clone()); true }) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        loop {
            if let Ok(p) = rx.try_recv() { return p; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("avatar answer in time")
}

fn avatar_is_thumbnail(p: &AvatarPicture) -> bool {
    p.rgba.as_ref().is_some_and(|(w, h, px)| (*w, *h) == (96, 96) && px.len() == 96 * 96 * 4) && !p.retry
}

#[test]
#[ignore = "requires disposable homeserver"]
fn disposable_account_avatar_picture_when_set_initials_when_not() {
    runtime().block_on(async {
        let config = fixture();
        let mxc = avatar_set(&config, "alice", Some(avatar_png(10))).await.unwrap();
        let (mut alice, mut bob) = (avatar_start(&config, "alice", 91), avatar_start(&config, "bob", 92));
        // 1. picture on the server: the drawer gets the mxc and a decoded 96 px thumbnail.
        assert_eq!(avatar_profile(&mut alice, 30, |_| true).await.as_deref(), Some(mxc.as_str()));
        let picture = avatar_picture(&alice, &mxc).await;
        assert!(avatar_is_thumbnail(&picture), "alice: {} {:?}", picture.mxc, picture.rgba.as_ref().map(|p| (p.0, p.1)));
        // 2. no picture: no mxc is announced, so nothing is asked and the initials stay.
        assert_eq!(avatar_profile(&mut bob, 30, |_| true).await, None);
    });
}

#[test]
#[ignore = "requires disposable homeserver"]
fn disposable_account_avatar_set_changed_removed_after_login_without_restart() {
    runtime().block_on(async {
        let config = fixture();
        let first = avatar_set(&config, "alice", Some(avatar_png(20))).await.unwrap();
        avatar_set(&config, "carol", Some(avatar_png(30))).await;
        let mut alice = avatar_start(&config, "alice", 91);
        let mut bob = avatar_start(&config, "bob", 92);
        let mut carol = avatar_start(&config, "carol", 94);
        assert_eq!(avatar_profile(&mut alice, 30, |_| true).await.as_deref(), Some(first.as_str()));
        assert_eq!(avatar_profile(&mut bob, 30, |_| true).await, None);
        assert!(avatar_profile(&mut carol, 30, |_| true).await.is_some());
        // Same running sessions: bob sets one, alice changes hers, carol removes hers (the worker re-reads every 60 s).
        let second = avatar_set(&config, "alice", Some(avatar_png(200))).await.unwrap();
        let third = avatar_set(&config, "bob", Some(avatar_png(100))).await.unwrap();
        avatar_set(&config, "carol", None).await;
        let (a, b, c) = tokio::join!(
            avatar_profile(&mut alice, 90, |m| m.is_some()),
            avatar_profile(&mut bob, 90, |m| m.is_some()),
            avatar_profile(&mut carol, 90, |m| m.as_deref().is_none_or(str::is_empty)),
        );
        assert_eq!((a.as_deref(), b.as_deref()), (Some(second.as_str()), Some(third.as_str())));
        // Synapse reports a removed picture as None, Tuwunel as Some("") (own_profile does not normalise it).
        // Either way nothing is shown: the empty URL gives no picture and is not retried.
        assert!(c.as_deref().is_none_or(str::is_empty), "{c:?}");
        if c.is_some() {
            let p = avatar_picture(&carol, "").await;
            assert!(p.rgba.is_none() && !p.retry);
        }
        assert_ne!(first, second);
        for (account, mxc) in [(&alice, &second), (&bob, &third)] {
            assert!(avatar_is_thumbnail(&avatar_picture(account, mxc).await), "{mxc}");
        }
    });
}

#[test]
#[ignore = "requires disposable homeserver"]
fn disposable_account_avatar_transient_failure_then_picture_final_failure_initials() {
    runtime().block_on(async {
        let config = fixture();
        let mxc = avatar_set(&config, "alice", Some(avatar_png(40))).await.unwrap();
        let mut alice = avatar_start(&config, "alice", 91);
        avatar_profile(&mut alice, 30, |m| m.is_some()).await;
        // Server unreachable (stopped, not refusing): transient, "ask again", no picture.
        let paused = PausedServer::new(&config);
        let failed = avatar_picture(&alice, &mxc).await;
        assert!(failed.rgba.is_none() && failed.retry, "retry={}", failed.retry);
        drop(paused);
        // Back: the same request now gives the picture.
        assert!(avatar_is_thumbnail(&avatar_picture(&alice, &mxc).await));
        // Final: a media id the server does not have, and a non-mxc URL; both initials, never retried.
        let server = mxc.trim_start_matches("mxc://").split('/').next().unwrap().to_owned();
        for gone in [format!("mxc://{server}/doesnotexist"), "https://example.invalid/a.png".to_owned()] {
            let p = avatar_picture(&alice, &gone).await;
            assert!(p.rgba.is_none() && !p.retry, "{gone}: retry={}", p.retry);
        }
    });
}

/// Incompressible 300x300 picture: its 96 px thumbnail is far above a few KiB, so a small cap bites.
fn avatar_noise_png() -> Vec<u8> {
    let mut s = 0x2545F491u32;
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_fn(300, 300, |_, _| {
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        image::Rgb([(s >> 8) as u8, (s >> 16) as u8, (s >> 24) as u8])
    })
    .write_to(&mut out, image::ImageFormat::Png)
    .unwrap();
    out.into_inner()
}

#[test]
#[ignore = "requires disposable homeserver"]
fn disposable_account_avatar_download_cap_cache_hit_needs_no_network_bad_mxc_no_request() {
    use crate::avatar_cache::{AVATAR_MAX_BYTES, AvatarCache};
    runtime().block_on(async {
        let config = fixture();
        let mxc = avatar_set(&config, "alice", Some(avatar_noise_png())).await.unwrap();
        let session = login(&config, "alice", 91).await;
        let client = session.client.clone().expect("logged in");
        let dir = avatar_root(&config).join("cap-test");
        let (small, normal) = (dir.join("small"), dir.join("normal"));
        std::fs::create_dir_all(&dir).unwrap(); // the cache creates only its own last directory

        // 1. Over the cap: stopped while received, final failure (initials), nothing written to disk.
        let capped = AvatarCache::with_limits(small.clone(), 4 * 1024, 8, 2);
        assert_eq!(capped.try_get(&client, &mxc).await, Err(false), "over the cap must be final");
        assert_eq!(capped.downloads(), 1);
        assert!(!small.exists() || std::fs::read_dir(&small).unwrap().next().is_none(), "no file for a refused body");

        // 2. Under the real cap: bytes arrive, are within AVATAR_MAX_BYTES and are saved once.
        let cache = AvatarCache::new(normal.clone());
        let bytes = cache.try_get(&client, &mxc).await.expect("picture");
        assert!(bytes.len() > 4 * 1024 && bytes.len() <= AVATAR_MAX_BYTES, "{} bytes", bytes.len());
        assert_eq!(cache.downloads(), 1);

        // 3. Parked account with a saved profile: the server is away, the row is served from disk, no request starts.
        let paused = PausedServer::new(&config);
        assert_eq!(cache.try_get(&client, &mxc).await.as_deref(), Ok(bytes.as_slice()));
        assert_eq!(cache.downloads(), 1, "cache hit must not touch the network");
        drop(paused);

        // 4. Bad URLs never reach the network: not mxc, empty (Tuwunel's "no avatar"), no media id.
        for bad in ["https://example.invalid/a.png", "", "mxc://", "mxc://server-only"] {
            assert_eq!(cache.try_get(&client, bad).await, Err(false), "{bad:?}");
        }
        assert_eq!(cache.downloads(), 1, "invalid mxc must not start a download");
        let _ = std::fs::remove_dir_all(&dir);
    });
}
