//! Local TCP server, real fixtures, no internet.
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// 2026-12-05T18:40:00Z minus 10 days: inside the fixtures' validity.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_796_496_000 - 10 * 86_400)
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
fn prefs(base: &str) -> Preferences {
    Preferences {
        update_base: Some(base.into()),
        ..Preferences::default()
    }
}
async fn go(p: &mut Preferences, running: u64) -> Result<Outcome, CheckError> {
    run(
        p,
        &keys(),
        running,
        super::super::verify::LINUX_APPIMAGE,
        UPDATE_HOST,
        now(),
    )
    .await
}
fn serve_fixture(minisig: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    serve(move |path| match path {
        "/dl/latest.json" => ok(&fx("valid.json")),
        "/dl/latest.json.minisig" => ok(&fx(minisig)),
        _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
    })
}

#[tokio::test]
async fn valid_manifest_gives_candidate_resolved_against_effective_base() {
    let (base, seen) = serve_fixture("valid.minisig");
    let mut p = prefs(&base);
    let Ok(Outcome::Candidate(o)) = go(&mut p, 41).await else {
        panic!("no candidate")
    };
    assert_eq!(o.url, format!("{base}/Gnobbi-x86_64-42.AppImage"));
    assert_eq!((o.build, o.size, o.version.as_str()), (42, 1000, "0.1.0"));
    assert_eq!(
        (p.max_build_seen, p.last_check),
        (42, Some(1_796_496_000 - 10 * 86_400))
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
async fn redirect_is_not_followed() {
    let (base, seen) = serve(|_| {
        b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\r\n".to_vec()
    });
    let mut p = prefs(&base);
    assert_eq!(go(&mut p, 41).await, Err(CheckError::Status(302)));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(p.last_check, None);
}

#[tokio::test]
async fn body_over_cap_without_content_length_is_aborted() {
    let (base, _) = serve(|_| {
        let mut r = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        r.extend(std::iter::repeat_n(b'x', MAX_MANIFEST_BYTES + 1));
        r
    });
    let mut p = prefs(&base);
    assert_eq!(go(&mut p, 41).await, Err(CheckError::TooLarge));
}

#[tokio::test]
async fn wrong_signature_is_an_error_and_changes_nothing() {
    let (base, _) = serve_fixture("valid.other.minisig");
    let mut p = prefs(&base);
    assert_eq!(
        go(&mut p, 41).await,
        Err(CheckError::Verify(UpdateError::UnknownKey))
    );
    assert_eq!((p.max_build_seen, p.last_check), (0, None));
}

#[tokio::test]
async fn not_newer_is_no_update_but_records_build() {
    let (base, _) = serve_fixture("valid.minisig");
    let mut p = prefs(&base);
    assert_eq!(go(&mut p, 42).await, Ok(Outcome::NoUpdate));
    assert_eq!(p.max_build_seen, 42);
}

#[tokio::test]
async fn older_than_max_build_seen_is_discarded_and_max_kept() {
    let (base, _) = serve_fixture("valid.minisig");
    let mut p = prefs(&base);
    p.max_build_seen = 50;
    assert_eq!(go(&mut p, 41).await, Ok(Outcome::NoUpdate));
    assert_eq!(p.max_build_seen, 50);
}

#[tokio::test]
async fn bad_build_is_discarded() {
    let (base, _) = serve_fixture("valid.minisig");
    let mut p = prefs(&base);
    p.bad_builds.insert(42);
    assert_eq!(go(&mut p, 41).await, Ok(Outcome::NoUpdate));
    assert_eq!(p.max_build_seen, 42);
}
