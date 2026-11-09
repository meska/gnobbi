//! Local TCP server, real fixtures, no internet.
use super::super::verify::LINUX_APPIMAGE;
use super::*;
use crate::update::startup;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// 2026-12-05T18:40:00Z minus 10 days: inside the fixtures' validity.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(now_secs())
}
fn now_secs() -> u64 {
    1_796_496_000 - 10 * 86_400
}
fn fx(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/update")
            .join(name),
    )
    .unwrap()
}
fn keys() -> UpdateKeys {
    UpdateKeys::parse(&String::from_utf8(fx("keys.pub")).unwrap()).unwrap()
}

/// Fresh private store per test; `prefs` is written first so the check and the other writers
/// share the same settings.json and lock file.
fn store(name: &str, prefs: &Preferences) -> AppDirs {
    let root = std::env::temp_dir().join(format!("gnobbi-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dirs = AppDirs {
        config: root.join("cfg"),
        data: root.join("data"),
        cache: root.join("cache"),
    };
    prefs.save(&dirs).unwrap();
    dirs
}
fn base_prefs(base: &str) -> Preferences {
    Preferences {
        update_base: Some(base.into()),
        ..Preferences::default()
    }
}

/// Serves `reply(path)` (a full raw HTTP response) to every connection; returns the base URL
/// and the raw requests seen.
fn serve(reply: impl Fn(&str) -> Vec<u8> + Send + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/dl", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = req.split_whitespace().nth(1).unwrap_or("").to_owned();
            log.lock().unwrap().push(req);
            let _ = s.write_all(&reply(&path));
        }
    });
    (base, seen)
}
fn ok(body: &[u8]) -> Vec<u8> {
    let mut r = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    r.extend_from_slice(body);
    r
}
fn serve_fixture(minisig: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    serve(move |path| match path {
        "/dl/latest.json" => ok(&fx("valid.json")),
        "/dl/latest.json.minisig" => ok(&fx(minisig)),
        _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
    })
}

/// Like [`serve_fixture`], but holds the `latest.json` reply until the returned sender fires and
/// reports on the returned receiver once that request has arrived: the caller can act while the
/// fetch is suspended (barrier handshake, no sleep).
fn serve_gated(
    minisig: &'static str,
) -> (String, Arc<Mutex<Vec<String>>>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/dl", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = req.split_whitespace().nth(1).unwrap_or("").to_owned();
            log.lock().unwrap().push(req);
            if path == "/dl/latest.json" {
                arrived_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            }
            let reply = match path.as_str() {
                "/dl/latest.json" => ok(&fx("valid.json")),
                "/dl/latest.json.minisig" => ok(&fx(minisig)),
                _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            };
            let _ = s.write_all(&reply);
        }
    });
    (base, seen, arrived_rx, release_tx)
}

/// The production path on its own runtime, build and clock injected.
async fn go(dirs: &AppDirs, p: &Preferences, running: u64) -> Result<Outcome, CheckError> {
    run(dirs, p, &keys(), running, LINUX_APPIMAGE, UPDATE_HOST, now()).await
}

/// [`go`] on a thread with its own runtime, so the test can act while the fetch is suspended.
fn go_blocking(
    dirs: AppDirs,
    p: Preferences,
    running: u64,
) -> std::thread::JoinHandle<Result<Outcome, CheckError>> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(run(
            &dirs,
            &p,
            &keys(),
            running,
            LINUX_APPIMAGE,
            UPDATE_HOST,
            now(),
        ))
    })
}

#[tokio::test]
async fn valid_manifest_gives_candidate_resolved_against_effective_base() {
    let (base, seen) = serve_fixture("valid.minisig");
    let d = store("valid", &base_prefs(&base));
    let p = Preferences::load(&d);
    let Ok(Outcome::Candidate(o)) = go(&d, &p, 41).await else {
        panic!("no candidate")
    };
    assert_eq!(o.url, format!("{base}/Gnobbi-x86_64-42.AppImage"));
    assert_eq!((o.build, o.size, o.version.as_str()), (42, 1000, "0.1.0"));
    let after = Preferences::load(&d);
    assert_eq!(
        (after.max_build_seen, after.last_check),
        (42, Some(now_secs()))
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].starts_with("GET /dl/latest.json HTTP/1.1"));
    assert!(
        seen.iter()
            .all(|r| r.to_lowercase().contains("user-agent: gnobbi\r\n"))
    );
}

#[tokio::test]
async fn signed_schema_one_resolves_on_two_bases() {
    let (a, _) = serve_fixture("valid.minisig");
    let (b, _) = serve_fixture("valid.minisig");
    for base in [a, b] {
        let d = store("twobases", &base_prefs(&base));
        let p = Preferences::load(&d);
        let Ok(Outcome::Candidate(offer)) = go(&d, &p, 41).await else {
            panic!("no candidate")
        };
        assert_eq!(offer.url, format!("{base}/Gnobbi-x86_64-42.AppImage"));
    }
}

#[tokio::test]
async fn redirect_is_not_followed() {
    let (base, seen) = serve(|_| {
        b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\r\n".to_vec()
    });
    let d = store("redirect", &base_prefs(&base));
    let p = Preferences::load(&d);
    assert_eq!(go(&d, &p, 41).await, Err(CheckError::Status(302)));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(Preferences::load(&d).last_check, None);
}

#[tokio::test]
async fn body_over_cap_without_content_length_is_aborted() {
    let (base, _) = serve(|_| {
        let mut r = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        r.extend(std::iter::repeat_n(b'x', MAX_MANIFEST_BYTES + 1));
        r
    });
    let d = store("overcap", &base_prefs(&base));
    let p = Preferences::load(&d);
    assert_eq!(go(&d, &p, 41).await, Err(CheckError::TooLarge));
}

#[tokio::test]
async fn wrong_signature_is_an_error_and_changes_nothing() {
    let (base, _) = serve_fixture("valid.other.minisig");
    let d = store("wrongsig", &base_prefs(&base));
    let p = Preferences::load(&d);
    assert_eq!(
        go(&d, &p, 41).await,
        Err(CheckError::Verify(UpdateError::UnknownKey))
    );
    let after = Preferences::load(&d);
    assert_eq!((after.max_build_seen, after.last_check), (0, None));
}

#[tokio::test]
async fn not_newer_is_no_update_but_records_build() {
    let (base, _) = serve_fixture("valid.minisig");
    let d = store("notnewer", &base_prefs(&base));
    let p = Preferences::load(&d);
    assert_eq!(go(&d, &p, 42).await, Ok(Outcome::NoUpdate));
    assert_eq!(Preferences::load(&d).max_build_seen, 42);
}

#[tokio::test]
async fn older_than_max_build_seen_is_discarded_and_max_kept() {
    let (base, _) = serve_fixture("valid.minisig");
    let d = store(
        "rollback",
        &Preferences {
            max_build_seen: 50,
            ..base_prefs(&base)
        },
    );
    let p = Preferences::load(&d);
    assert_eq!(go(&d, &p, 41).await, Ok(Outcome::NoUpdate));
    assert_eq!(Preferences::load(&d).max_build_seen, 50);
}

#[tokio::test]
async fn signed_manifest_without_local_artifact_records_rollback_floor() {
    let floor_keys = UpdateKeys::parse(&String::from_utf8(fx("floor.pub")).unwrap()).unwrap();
    let serve_pair = |json: &'static str, sig: &'static str| {
        serve(move |path| match path {
            "/dl/latest.json" => ok(&fx(json)),
            "/dl/latest.json.minisig" => ok(&fx(sig)),
            _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
        })
        .0
    };
    let run_with = |d: AppDirs| {
        let p = Preferences::load(&d);
        let keys = &floor_keys;
        async move { run(&d, &p, keys, 40, LINUX_APPIMAGE, UPDATE_HOST, now()).await }
    };
    let d = store("floor", &base_prefs(&serve_pair("floor-high.json", "floor-high.minisig")));
    assert_eq!(run_with(d.clone()).await, Ok(Outcome::NoUpdate));
    assert_eq!(Preferences::load(&d).max_build_seen, 42);
    // Same store, endpoint now serving an older signed manifest: discarded, floor kept.
    let lower = serve_pair("lower.json", "lower.minisig");
    Preferences::update(&d, |p| p.update_base = Some(lower.clone())).unwrap();
    assert_eq!(run_with(d.clone()).await, Ok(Outcome::NoUpdate));
    assert_eq!(Preferences::load(&d).max_build_seen, 42);
    // Control: the same older manifest on a fresh store is a real candidate.
    let fresh = store("floor-fresh", &base_prefs(&lower));
    assert!(matches!(run_with(fresh).await, Ok(Outcome::Candidate(_))));
}

#[tokio::test]
async fn persistence_failure_never_offers_candidate() {
    let (base, _) = serve_fixture("valid.minisig");
    let path = std::env::temp_dir().join(format!("gnobbi-notdir-{}", std::process::id()));
    std::fs::write(&path, b"not a directory").unwrap();
    let dirs = AppDirs { config: path.clone(), data: path.clone(), cache: path.clone() };
    let p = base_prefs(&base);
    assert!(matches!(go(&dirs, &p, 41).await, Err(CheckError::Persist(_))));
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn bad_build_is_discarded() {
    let (base, _) = serve_fixture("valid.minisig");
    let d = store(
        "badbuild",
        &Preferences {
            bad_builds: [42].into(),
            ..base_prefs(&base)
        },
    );
    let p = Preferences::load(&d);
    assert_eq!(go(&d, &p, 41).await, Ok(Outcome::NoUpdate));
    assert_eq!(Preferences::load(&d).max_build_seen, 42);
}

#[tokio::test]
async fn a_disabled_switch_opens_no_connection() {
    let (base, seen) = serve_fixture("valid.minisig");
    let d = store(
        "disabled",
        &Preferences {
            update_check_enabled: false,
            ..base_prefs(&base)
        },
    );
    let p = Preferences::load(&d);
    assert_eq!(go(&d, &p, 41).await, Ok(Outcome::NoUpdate));
    assert!(
        seen.lock().unwrap().is_empty(),
        "no request while the switch is off"
    );
}

/// The card's core: the switch can change while the fetch is in flight, so the decision must be
/// taken on the values current then, not on the snapshot read before the network.
#[test]
fn a_switch_turned_off_during_the_fetch_keeps_off_and_offers_nothing() {
    let (base, seen, arrived, release) = serve_gated("valid.minisig");
    let d = store("off-in-flight", &base_prefs(&base));
    let p = Preferences::load(&d);
    let check = go_blocking(d.clone(), p, 41);
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    // Exactly the write the settings handler performs: one field, on the current value.
    Preferences::update(&d, |p| p.update_check_enabled = false).unwrap();
    release.send(()).unwrap();
    assert_eq!(
        check.join().unwrap().unwrap(),
        Outcome::NoUpdate,
        "OFF at completion never offers a download"
    );
    let after = Preferences::load(&d);
    assert!(
        !after.update_check_enabled,
        "the concurrent OFF survives the check"
    );
    assert_eq!(
        (after.max_build_seen, after.last_check),
        (42, Some(now_secs())),
        "metadata kept"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|r| !r.contains("Gnobbi-x86_64")),
        "no artifact request"
    );
}

#[test]
fn malformed_switch_during_fetch_fails_closed_without_rewriting() {
    let (base, _, arrived, release) = serve_gated("valid.minisig");
    let d = store("malformed-switch", &base_prefs(&base));
    let p = Preferences::load(&d);
    let check = go_blocking(d.clone(), p, 41);
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    let path = d.config.join("settings.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw["update_check_enabled"] = serde_json::json!("false");
    let bytes = raw.to_string();
    std::fs::write(&path, &bytes).unwrap();
    release.send(()).unwrap();
    assert_eq!(check.join().unwrap(), Err(CheckError::Persist(std::io::ErrorKind::InvalidData)));
    assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
}

/// Two real writers (a self-test failure and a newer max seen elsewhere) and the shared stores
/// land while the fetch is suspended: the commit must not clobber them, and a banned or
/// rolled-back build is never offered.
#[test]
fn a_banned_or_rolled_back_build_is_never_offered_over_concurrent_writes() {
    let (base, _, arrived, release) = serve_gated("valid.minisig");
    let d = store(
        "in-flight-writers",
        &Preferences {
            bad_builds: [7].into(),
            ..base_prefs(&base)
        },
    );
    // Other stores share settings.json; an outcome and an unknown key must survive too.
    let path = d.config.join("settings.json");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw["update_outcome"] = serde_json::json!({ "result": "updated", "build": 31 });
    raw["future"] = serde_json::json!({ "x": 1 });
    std::fs::write(&path, raw.to_string()).unwrap();
    let mut servers = crate::directory_servers::SavedServers::default();
    servers.add(1, "matrix.example");
    servers.save(&d).unwrap();

    let p = Preferences::load(&d);
    let check = go_blocking(d.clone(), p, 41);
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    startup::add_bad_build(&d, 42).unwrap();
    Preferences::update(&d, |p| {
        p.max_build_seen = 100;
        p.last_check = Some(now_secs() + 60);
    }).unwrap();
    release.send(()).unwrap();
    assert_eq!(
        check.join().unwrap().unwrap(),
        Outcome::NoUpdate,
        "build 42 is banned and below max_build_seen"
    );
    let after = Preferences::load(&d);
    assert_eq!(
        after.bad_builds.iter().copied().collect::<Vec<_>>(),
        [7, 42],
        "bad_builds union kept"
    );
    assert_eq!(after.max_build_seen, 100, "max_build_seen only moves forward");
    assert_eq!(after.last_check, Some(now_secs() + 60), "last_check never moves backwards");
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(data["update_outcome"]["build"], 31);
    assert_eq!(data["future"]["x"], 1);
    assert_eq!(
        crate::directory_servers::SavedServers::load(&d).for_account(1),
        ["matrix.example"]
    );
}

/// The effective base can change between the fetch and the commit: the old endpoint's offer and
/// its metadata are dropped whole, never mixed into the new endpoint's state.
#[test]
fn an_endpoint_change_during_the_fetch_discards_the_offer_and_its_metadata() {
    let (base, _, arrived, release) = serve_gated("valid.minisig");
    let d = store("endpoint-change", &base_prefs(&base));
    let p = Preferences::load(&d);
    let check = go_blocking(d.clone(), p, 41);
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    Preferences::update(&d, |p| p.update_base = Some("http://127.0.0.1:9/other".into())).unwrap();
    release.send(()).unwrap();
    assert_eq!(
        check.join().unwrap().unwrap(),
        Outcome::NoUpdate,
        "the old endpoint's offer is discarded"
    );
    let after = Preferences::load(&d);
    assert_eq!(
        after.update_base.as_deref(),
        Some("http://127.0.0.1:9/other"),
        "the new base is not overwritten"
    );
    assert_eq!(
        (after.max_build_seen, after.last_check),
        (0, None),
        "no metadata from the old endpoint"
    );
}

/// The Settings action (`Shell::choose_theme` -> `update_prefs`) is exactly one
/// `Preferences::update` on its own field: done while the fetch is suspended, both survive.
#[test]
fn a_ui_theme_write_during_the_fetch_and_the_commit_keep_each_other() {
    let (base, _, arrived, release) = serve_gated("valid.minisig");
    let d = store("theme-in-flight", &base_prefs(&base));
    let p = Preferences::load(&d);
    let check = go_blocking(d.clone(), p, 41);
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    Preferences::update(&d, |p| p.theme = Some(crate::Theme::Dark)).unwrap();
    release.send(()).unwrap();
    assert!(matches!(check.join().unwrap().unwrap(), Outcome::Candidate(_)));
    let after = Preferences::load(&d);
    assert_eq!(after.theme, Some(crate::Theme::Dark), "UI choice kept");
    assert_eq!((after.max_build_seen, after.last_check), (42, Some(now_secs())), "metadata kept");
}

/// Reverse order: the check is done, then the UI writes its field on the current values.
#[test]
fn a_ui_theme_write_after_the_check_keeps_every_metadata() {
    let (base, _, arrived, release) = serve_gated("valid.minisig");
    let d = store("theme-after", &base_prefs(&base));
    startup::add_bad_build(&d, 7).unwrap();
    let p = Preferences::load(&d);
    let check = go_blocking(d.clone(), p, 41);
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    release.send(()).unwrap();
    assert!(matches!(check.join().unwrap().unwrap(), Outcome::Candidate(_)));
    Preferences::update(&d, |p| p.theme = Some(crate::Theme::Dark)).unwrap();
    let after = Preferences::load(&d);
    assert_eq!(after.theme, Some(crate::Theme::Dark));
    assert_eq!(
        (after.max_build_seen, after.last_check, after.bad_builds.iter().copied().collect::<Vec<_>>()),
        (42, Some(now_secs()), vec![7]),
        "check metadata intact"
    );
}

/// Corrupt store before the check: no network, no overwrite.
#[tokio::test]
async fn a_corrupt_store_stops_the_check_before_the_network_and_is_not_rewritten() {
    let (base, seen) = serve_fixture("valid.minisig");
    let d = store("corrupt", &base_prefs(&base));
    let path = d.config.join("settings.json");
    std::fs::write(&path, b"{invalid").unwrap();
    let mut p = Preferences::default();
    let r = check_with(&d, &mut p, &keys(), LINUX_APPIMAGE, "41", UPDATE_HOST, now()).await;
    assert!(matches!(r, Err(CheckError::Persist(_))), "{r:?}");
    assert!(seen.lock().unwrap().is_empty(), "no request");
    assert_eq!(std::fs::read(path).unwrap(), b"{invalid");
}

/// Another writer holds the settings lock when the verified answer lands: the commit fails
/// at once (WouldBlock), nothing is written and no Candidate is offered.
#[test]
fn a_contended_lock_at_commit_persists_nothing_and_offers_nothing() {
    let (base, _, arrived, release) = serve_gated("valid.minisig");
    let d = store("contended", &base_prefs(&base));
    let p = Preferences::load(&d);
    let check = go_blocking(d.clone(), p, 41);
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    let (held_tx, held_rx) = mpsc::channel();
    let (free_tx, free_rx) = mpsc::channel::<()>();
    let dd = d.clone();
    let holder = std::thread::spawn(move || {
        Preferences::update(&dd, |_| {
            held_tx.send(()).unwrap();
            free_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        })
    });
    held_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    release.send(()).unwrap();
    assert_eq!(check.join().unwrap(), Err(CheckError::Persist(std::io::ErrorKind::WouldBlock)));
    free_tx.send(()).unwrap();
    holder.join().unwrap().unwrap();
    let after = Preferences::load(&d);
    assert_eq!((after.max_build_seen, after.last_check), (0, None), "nothing persisted");
}
