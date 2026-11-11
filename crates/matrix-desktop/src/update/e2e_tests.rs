//! HTTP end to end through the scheduler seam (`online::check_and_stage_with`, `tick`): real signed
//! manifests (throw-away key, tests/fixtures/update/e2e/gen.sh) served by a `TcpListener` thread
//! that logs every request. No internet. Linux x86_64 only: the platform key is the AppImage one.
use super::{online::{Seam, check_and_stage_with}, *};
use crate::update::verify::{MAX_MANIFEST_BYTES, UpdateKeys};
use std::{io::{Read, Write}, net::TcpListener, path::PathBuf};

type Files = Vec<(&'static str, Vec<u8>)>;

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/update/e2e").join(name)).unwrap()
}

/// Serves `files` under `<prefix>/<name>`, 404 otherwise. `hits` = request paths seen.
struct Server { base: String, hits: Arc<Mutex<Vec<String>>> }
impl Server {
    fn hits(&self) -> Vec<String> { self.hits.lock().unwrap().clone() }
    fn asked(&self, name: &str) -> bool { self.hits().iter().any(|p| p.ends_with(name)) }
}
fn serve(prefix: &'static str, files: Files) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}{prefix}", l.local_addr().unwrap());
    let hits = Arc::new(Mutex::new(Vec::new()));
    let log = hits.clone();
    std::thread::spawn(move || for mut s in l.incoming().flatten() {
        let mut buf = vec![0u8; 4096];
        let n = s.read(&mut buf).unwrap_or(0);
        let path = String::from_utf8_lossy(&buf[..n]).split_whitespace().nth(1).unwrap_or("").to_owned();
        log.lock().unwrap().push(path.clone());
        let body = files.iter().find(|(f, _)| path == format!("{prefix}/{f}")).map(|(_, b)| b.as_slice());
        let head = if body.is_some() { "200 OK" } else { "404 Not Found" };
        let body = body.unwrap_or_default();
        let _ = s.write_all(format!("HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
        let _ = s.write_all(body);
    });
    Server { base, hits }
}

fn files(manifest: &str, sig: &str, artifact: (&'static str, Vec<u8>)) -> Files {
    vec![("latest.json", fx(manifest)), ("latest.json.minisig", fx(sig)), artifact]
}
fn good(name: &'static str) -> (&'static str, Vec<u8>) { (name, fx("artifact.sh")) }

struct Rig { dirs: AppDirs, root: PathBuf, channel: Channel, state: Mutex<UpdateState>, server: Server }
impl Drop for Rig { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }
impl Rig {
    fn new(name: &str, prefix: &'static str, files: Files) -> Self {
        let root = std::env::temp_dir().join(format!("gnobbi-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs { config: root.join("c"), data: root.join("d"), cache: root.join("k") };
        dirs.ensure().unwrap();
        let server = serve(prefix, files);
        Preferences { update_base: Some(server.base.clone()), ..Preferences::default() }.save(&dirs).unwrap();
        Self { channel: Channel::AppImage { target: root.join("g.AppImage") }, dirs, root, state: Mutex::default(), server }
    }
    /// One scheduler run as the build 41 binary, inside the fixtures' validity (valid_until 2026-12-05).
    fn go(&self) {
        let keys = UpdateKeys::parse(&String::from_utf8(fx("e2e.pub")).unwrap()).unwrap();
        let seam = Seam { keys: &keys, build: "41", host: "gnobbi.meskatech.net", now: UNIX_EPOCH + Duration::from_secs(1_796_496_000 - 10 * 86_400), before_download_gate: &|| {} };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        check_and_stage_with(&rt, &self.dirs, &self.state, &self.channel, &mut Preferences::load(&self.dirs), &seam);
    }
    fn prefs(&self) -> Preferences { Preferences::load(&self.dirs) }
    /// Staging leftovers next to the target.
    fn staging(&self) -> Vec<String> {
        std::fs::read_dir(&self.root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(".gnobbi.")).collect()
    }
}

#[test]
fn signed_manifest_hash_and_selftest_give_ready() {
    let r = Rig::new("ready", "/dl", files("ok.json", "ok.minisig", good("Gnobbi-x86_64-42.AppImage")));
    r.go();
    let st = r.state.lock().unwrap();
    let ready = st.ready.as_ref().expect("Ready");
    assert_eq!((ready.version.as_str(), ready.build), ("0.1.0", 42));
    assert!(ready.path.exists() && st.bad_session.is_empty());
    assert_eq!(r.prefs().max_build_seen, 42);
}

#[test]
fn wrong_signature_never_requests_the_artifact() {
    let r = Rig::new("badsig", "/dl", files("ok.json", "ok.other.minisig", good("Gnobbi-x86_64-42.AppImage")));
    r.go();
    assert_eq!(r.server.hits(), ["/dl/latest.json", "/dl/latest.json.minisig"]);
    assert!(r.state.lock().unwrap().ready.is_none() && r.staging().is_empty());
}

#[test]
fn wrong_hash_deletes_staging_and_marks_build_bad_for_the_session_only() {
    let r = Rig::new("badhash", "/dl", files("badhash.json", "badhash.minisig", good("Gnobbi-x86_64-43.AppImage")));
    r.go();
    assert!(r.server.asked("Gnobbi-x86_64-43.AppImage"));
    assert!(r.staging().is_empty(), "{:?}", r.staging());
    let st = r.state.lock().unwrap();
    assert!(st.ready.is_none() && st.bad_session.contains(&43));
    assert!(r.prefs().bad_builds.is_empty(), "hash failures are never persisted");
}

#[test]
fn artifact_longer_than_signed_size_is_cut_and_rejected() {
    let r = Rig::new("size", "/dl", files("small.json", "small.minisig", ("Gnobbi-x86_64-44.AppImage", vec![b'#'; 5 * 1024 * 1024])));
    r.go();
    assert!(r.staging().is_empty());
    let st = r.state.lock().unwrap();
    assert!(st.ready.is_none() && st.bad_session.contains(&44));
}

#[test]
fn build_not_above_max_build_seen_is_discarded_without_download() {
    let r = Rig::new("rollback", "/dl", files("ok.json", "ok.minisig", good("Gnobbi-x86_64-42.AppImage")));
    Preferences::update(&r.dirs, |p| p.max_build_seen = 50).unwrap();
    r.go();
    assert!(!r.server.asked("AppImage"));
    assert_eq!(r.prefs().max_build_seen, 50);
    assert!(r.state.lock().unwrap().ready.is_none());
}

#[test]
fn same_manifest_works_from_two_origins_and_paths() {
    for (n, prefix) in [("o1", "/dl"), ("o2", "/very/other/place")] {
        let r = Rig::new(n, prefix, files("ok.json", "ok.minisig", good("Gnobbi-x86_64-42.AppImage")));
        r.go();
        // The manifest names gnobbi.meskatech.net; the server asked is the configured one.
        assert!(r.server.asked("Gnobbi-x86_64-42.AppImage"), "{prefix}");
        assert_eq!(r.state.lock().unwrap().ready.as_ref().map(|x| x.build), Some(42), "{prefix}");
    }
}

#[test]
fn manifest_limit_is_64_kib() {
    let sig = fx("ok.minisig");
    // One byte over: aborted while streaming, signature and artifact never requested.
    let r = Rig::new("cap1", "/dl", vec![("latest.json", vec![b'x'; MAX_MANIFEST_BYTES + 1]), ("latest.json.minisig", sig.clone())]);
    r.go();
    assert_eq!(r.server.hits(), ["/dl/latest.json"]);
    // Exactly at the cap: read in full, then rejected by the signature, not by size.
    let r = Rig::new("cap2", "/dl", vec![("latest.json", vec![b'x'; MAX_MANIFEST_BYTES]), ("latest.json.minisig", sig)]);
    r.go();
    assert_eq!(r.server.hits(), ["/dl/latest.json", "/dl/latest.json.minisig"]);
}

#[test]
fn no_connection_when_switch_off_or_channel_does_not_update() {
    let mut r = Rig::new("sched", "/dl", files("ok.json", "ok.minisig", good("Gnobbi-x86_64-42.AppImage")));
    let h24 = Duration::from_secs(24 * 3600);
    for ch in [Channel::MacStore, Channel::Flatpak, Channel::SystemPackage, Channel::Unknown] {
        r.channel = ch;
        assert!(!tick(&r.dirs, &r.channel, h24, |_| r.go()));
        r.go(); // direct call too: no artifact key for these channels
    }
    r.channel = Channel::AppImage { target: r.root.join("g.AppImage") };
    Preferences::update(&r.dirs, |p| p.update_check_enabled = false).unwrap();
    assert!(!tick(&r.dirs, &r.channel, h24, |_| r.go()));
    assert!(r.server.hits().is_empty(), "{:?}", r.server.hits());
    // Control: the same server is reached once everything is on.
    Preferences::update(&r.dirs, |p| p.update_check_enabled = true).unwrap();
    assert!(tick(&r.dirs, &r.channel, h24, |_| r.go()));
    assert!(r.server.asked("latest.json"));
}
