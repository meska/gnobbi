//! Primary update source: `latest.json` (schema 1) of the GitHub prerelease `nightly`, see the
//! internal NIGHTLY.md. The manifest is not minisign-signed: trust is TLS to GitHub plus the
//! release owner, so it is never handed to the signed-site path in `verify.rs`. An update exists
//! when its `build` is greater than the running one (`check.rs`); `version` is only displayed.
//! The asset URL must be exactly `<DOWNLOAD>/<name>` and the staging step verifies size and hash.

use super::check::CheckError;
use super::verify::{Candidate, MAX_MANIFEST_BYTES};
use serde_json::Value;
use std::time::Duration;

const DOWNLOAD: &str = "https://github.com/meska/gnobbi/releases/download/nightly";
const TIMEOUT: Duration = Duration::from_secs(20);
/// Absolute ceiling for one artifact, whatever the manifest declares (checked before any download).
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// What the check needs: the offer plus the exact URL of its file.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub candidate: Candidate,
    pub url: String,
}

fn safe_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The nightly `latest.json` → the offer for `platform`. `Ok(None)` = nothing to offer (other
/// schema, no asset for the platform, malformed or oversized asset); not JSON = `Err`.
/// Whether `build` is newer than the running one is the caller's decision.
pub(super) fn parse_manifest(manifest: &[u8], platform: &str) -> Result<Option<Found>, CheckError> {
    let v: Value = serde_json::from_slice(manifest).map_err(|_| CheckError::Network)?;
    if v.get("schema").and_then(Value::as_u64) != Some(1) {
        return Ok(None);
    }
    let build = v.get("build").and_then(Value::as_u64).filter(|b| *b > 0);
    let version = v.get("version").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b)));
    let a = v.get("artifacts").and_then(|a| a.get(platform));
    let text = |k: &str| a.and_then(|a| a.get(k)).and_then(Value::as_str);
    let size = a.and_then(|a| a.get("size")).and_then(Value::as_u64).filter(|s| (1..=MAX_ARTIFACT_BYTES).contains(s));
    let sha256 = text("sha256").filter(|h| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    let name = text("name").filter(|n| safe_name(n));
    let (Some(build), Some(version), Some(size), Some(sha256), Some(name)) = (build, version, size, sha256, name) else { return Ok(None) };
    let url = format!("{DOWNLOAD}/{name}");
    if text("url") != Some(url.as_str()) {
        return Ok(None);
    }
    Ok(Some(Found { candidate: Candidate { version: version.to_owned(), build, file_name: name.to_owned(), sha256: sha256.to_owned(), size }, url }))
}

/// Follow redirects only from github.com (assets live on a CDN host) and only to https.
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|a| {
        let from_github = a.previous().first().is_some_and(|u| u.host_str() == Some("github.com"));
        if from_github && a.url().scheme() == "https" && a.previous().len() < 4 { a.follow() } else { a.stop() }
    })
}

async fn get(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, CheckError> {
    let mut resp = client.get(url).send().await.map_err(|_| CheckError::Network)?;
    if resp.status() != reqwest::StatusCode::OK {
        return Err(CheckError::Status(resp.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(c) = resp.chunk().await.map_err(|_| CheckError::Network)? {
        if body.len() + c.len() > MAX_MANIFEST_BYTES {
            return Err(CheckError::TooLarge);
        }
        body.extend_from_slice(&c);
    }
    Ok(body)
}

/// Network half. 404 = no nightly published yet = `Ok(None)`, silently. Rate limit and other
/// statuses are `CheckError::Status`; connection problems and 5xx are the caller's cue to fall
/// back to the site.
pub(super) async fn fetch(platform: &str, running_version: &str) -> Result<Option<Found>, CheckError> {
    let client = crate::http::client_builder()
        .user_agent(format!("Gnobbi/{running_version}"))
        .timeout(TIMEOUT)
        .redirect(redirect_policy())
        .build()
        .map_err(|_| CheckError::Network)?;
    match get(&client, &format!("{DOWNLOAD}/latest.json")).await {
        Ok(b) => parse_manifest(&b, platform),
        Err(CheckError::Status(404)) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::verify::{LINUX_APPIMAGE, WINDOWS_ZIP};

    const H: &str = "d2358579a99e69a151932fe9805328e88c226dbf8fdd72cc5aff1085931fad40";

    fn manifest(patch: impl FnOnce(&mut Value)) -> Vec<u8> {
        let base = "https://github.com/meska/gnobbi/releases/download/nightly";
        let mut v = serde_json::json!({
            "schema": 1, "build": 812, "version": "0.1.0-nightly.20261124.812", "ignored": true,
            "artifacts": {
                "linux-x86_64-appimage": {"name": "Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage", "url": format!("{base}/Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage"), "sha256": H, "size": 1000},
                "windows-x64-zip": {"name": "gnobbi-0.1.0-nightly.20261124.812-windows-x64.zip", "url": format!("{base}/gnobbi-0.1.0-nightly.20261124.812-windows-x64.zip"), "sha256": H, "size": 2000},
                "macos-arm64-dmg": {"name": "Gnobbi-x-macos-arm64.dmg", "url": format!("{base}/Gnobbi-x-macos-arm64.dmg"), "sha256": H, "size": 3000}
            }
        });
        patch(&mut v);
        v.to_string().into_bytes()
    }
    fn parse(m: &[u8], k: &str) -> Option<Found> { parse_manifest(m, k).unwrap() }

    #[test]
    fn picks_the_asset_of_the_platform() {
        let m = manifest(|_| {});
        let l = parse(&m, LINUX_APPIMAGE).unwrap();
        assert_eq!((l.candidate.build, l.candidate.size, l.candidate.version.as_str()), (812, 1000, "0.1.0-nightly.20261124.812"));
        assert_eq!(l.url, "https://github.com/meska/gnobbi/releases/download/nightly/Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage");
        assert_eq!(l.candidate.file_name, "Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage");
        assert!(parse(&m, WINDOWS_ZIP).unwrap().candidate.file_name.ends_with("-windows-x64.zip"));
        assert_eq!(parse(&m, "linux-aarch64"), None);
    }

    #[test]
    fn rejects_what_must_never_become_a_download() {
        let k = |f: &str, v: Value| manifest(|m| m["artifacts"]["linux-x86_64-appimage"][f] = v);
        assert_eq!(parse(&manifest(|m| m["schema"] = 2.into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&manifest(|m| m["build"] = 0.into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&manifest(|m| m["version"] = "a b".into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("size", 0.into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("size", (MAX_ARTIFACT_BYTES + 1).into()), LINUX_APPIMAGE), None, "absolute cap, before any request");
        assert!(parse(&k("size", MAX_ARTIFACT_BYTES.into()), LINUX_APPIMAGE).is_some());
        assert_eq!(parse(&k("sha256", H.to_uppercase().into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("sha256", "abc".into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("name", "a/../b".into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("url", "https://evil.example/x.AppImage".into()), LINUX_APPIMAGE), None, "url must be the nightly folder + name");
        assert_eq!(parse(&k("url", "http://github.com/meska/gnobbi/releases/download/nightly/Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage".into()), LINUX_APPIMAGE), None);
        assert!(parse_manifest(b"not json", LINUX_APPIMAGE).is_err());
        assert_eq!(parse(b"{}", LINUX_APPIMAGE), None);
    }

    #[test]
    fn sha_mismatch_is_refused() {
        // The hash from the manifest goes into Candidate; other bytes must be rejected.
        let c = Candidate { version: "0.3.0".into(), build: 9, file_name: "a".into(), sha256: H.into(), size: 4 };
        let f = std::env::temp_dir().join(format!("gnobbi-gh-sha-{}", std::process::id()));
        std::fs::write(&f, b"data").unwrap();
        let r = super::super::verify::verify_file(&f, &c);
        let _ = std::fs::remove_file(&f);
        assert_eq!(r, Err(super::super::verify::UpdateError::Sha256Mismatch));
    }

    #[test]
    fn malformed_or_hostile_manifests_never_panic_and_never_offer() {
        let ok = |v: Value| manifest(|m| m["artifacts"]["linux-x86_64-appimage"] = v);
        // JSON that is valid but of the wrong shape: nothing to offer.
        for body in [&b"null"[..], b"[]", b"42", b"\"x\"", b"{\"schema\":1}", b"{\"schema\":\"1\"}", b"{\"schema\":1,\"build\":\"812\"}"] {
            assert_eq!(parse_manifest(body, LINUX_APPIMAGE).unwrap(), None, "{}", String::from_utf8_lossy(body));
        }
        // Not JSON / cut JSON / empty: error (the caller falls back or stays silent).
        for body in [&b""[..], b"{\"schema\":1,", b"\xff\xfe", b"<html>"] {
            assert!(parse_manifest(body, LINUX_APPIMAGE).is_err());
        }
        for bad in [Value::Null, "x".into(), 7.into(), serde_json::json!([]), serde_json::json!({}), serde_json::json!({"name": "a", "size": 1.5, "sha256": H, "url": "u"})] {
            assert_eq!(parse(&ok(bad), LINUX_APPIMAGE), None);
        }
        let k = |f: &str, v: Value| manifest(|m| m["artifacts"]["linux-x86_64-appimage"][f] = v);
        assert_eq!(parse(&k("size", (-1).into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("size", "1000".into()), LINUX_APPIMAGE), None, "size as a string");
        assert_eq!(parse(&k("name", "".into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("name", "..".into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&k("name", "a\\b.zip".into()), LINUX_APPIMAGE), None, "backslash");
        assert_eq!(parse(&k("name", "a b".into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&manifest(|m| m["version"] = "x".repeat(65).into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&manifest(|m| m["build"] = (-3).into()), LINUX_APPIMAGE), None);
        assert_eq!(parse(&manifest(|m| m["build"] = 1.5.into()), LINUX_APPIMAGE), None);
        // The URL may not smuggle a query/fragment or a trailing slash onto the exact asset path.
        let base = "https://github.com/meska/gnobbi/releases/download/nightly/Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage";
        for u in [format!("{base}?x=1"), format!("{base}#f"), format!("{base}/"), format!(" {base}"), base.replace("github.com", "github.com.evil.test")] {
            assert_eq!(parse(&k("url", u.clone().into()), LINUX_APPIMAGE), None, "{u}");
        }
    }

    #[test]
    fn a_manifest_without_our_platform_offers_nothing() {
        let only_windows = manifest(|m| { m["artifacts"].as_object_mut().unwrap().retain(|k, _| k.starts_with("windows")); });
        assert_eq!(parse(&only_windows, LINUX_APPIMAGE), None);
        assert!(parse(&only_windows, WINDOWS_ZIP).is_some());
        assert_eq!(parse(&manifest(|m| { m.as_object_mut().unwrap().remove("artifacts"); }), LINUX_APPIMAGE), None);
        // macOS has an asset in the manifest but is out of scope: no key maps to it.
        assert_eq!(parse(&manifest(|_| {}), "macos-arm64-dmg").map(|f| f.candidate.size), Some(3000), "parser is generic");
        assert_ne!(LINUX_APPIMAGE, WINDOWS_ZIP);
    }

    #[test]
    fn the_same_asset_resolves_to_one_url_whatever_the_order_of_keys() {
        let a = parse(&manifest(|_| {}), LINUX_APPIMAGE).unwrap();
        let b = parse(br#"{"artifacts":{"linux-x86_64-appimage":{"size":1000,"sha256":"d2358579a99e69a151932fe9805328e88c226dbf8fdd72cc5aff1085931fad40","url":"https://github.com/meska/gnobbi/releases/download/nightly/Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage","name":"Gnobbi-0.1.0-nightly.20261124.812-linux-x86_64.AppImage"}},"version":"0.1.0-nightly.20261124.812","build":812,"schema":1}"#, LINUX_APPIMAGE).unwrap();
        assert_eq!(a, b);
    }

    /// One-shot local HTTP server: answers every connection with `raw` (no real network).
    fn raw_server(raw: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || for mut s in l.incoming().flatten() {
            let _ = s.read(&mut [0u8; 2048]);
            let _ = s.write_all(&raw);
        });
        format!("http://{addr}/latest.json")
    }
    fn client() -> reqwest::Client { reqwest::Client::builder().redirect(redirect_policy()).timeout(Duration::from_secs(5)).build().unwrap() }
    fn http(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut v = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len()).into_bytes();
        v.extend_from_slice(body);
        v
    }

    #[tokio::test]
    async fn get_maps_statuses_without_reading_a_body_as_manifest() {
        assert_eq!(get(&client(), &raw_server(http("200 OK", "", b"{}"))).await, Ok(b"{}".to_vec()));
        for (line, code) in [("404 Not Found", 404), ("403 Forbidden", 403), ("429 Too Many Requests", 429), ("500 Internal Server Error", 500), ("503 Service Unavailable", 503)] {
            assert_eq!(get(&client(), &raw_server(http(line, "", b"{\"schema\":1}"))).await, Err(CheckError::Status(code)), "{line}");
        }
    }

    #[tokio::test]
    async fn get_aborts_a_body_over_the_cap_even_if_content_length_lies() {
        let big = vec![b'x'; MAX_MANIFEST_BYTES + 1];
        assert_eq!(get(&client(), &raw_server(http("200 OK", "", &big))).await, Err(CheckError::TooLarge));
        // Declares 10 bytes, sends the oversize body anyway (chunked: no length at all).
        let mut chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        for _ in 0..3 { chunked.extend_from_slice(format!("{:x}\r\n", MAX_MANIFEST_BYTES / 2).as_bytes()); chunked.extend(vec![b'y'; MAX_MANIFEST_BYTES / 2]); chunked.extend_from_slice(b"\r\n"); }
        assert_eq!(get(&client(), &raw_server(chunked)).await, Err(CheckError::TooLarge));
        let exact = vec![b' '; MAX_MANIFEST_BYTES];
        assert_eq!(get(&client(), &raw_server(http("200 OK", "", &exact))).await.map(|b| b.len()), Ok(MAX_MANIFEST_BYTES));
    }

    #[tokio::test]
    async fn get_without_network_is_a_network_error() {
        let dead = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap() };
        assert_eq!(get(&client(), &format!("http://{dead}/latest.json")).await, Err(CheckError::Network));
        // Connection cut in the middle of the body.
        let cut = http("200 OK", "", b"{}");
        let head = String::from_utf8_lossy(&cut).replace("Content-Length: 2", "Content-Length: 500").into_bytes();
        assert_eq!(get(&client(), &raw_server(head)).await, Err(CheckError::Network));
    }

    #[tokio::test]
    async fn redirects_are_followed_only_from_github_and_never_to_http() {
        // 127.0.0.1 is not github.com: a redirect from it is not followed, the 302 surfaces as a status.
        let r = http("302 Found", "Location: http://127.0.0.1:1/evil\r\n", b"");
        assert_eq!(get(&client(), &raw_server(r)).await, Err(CheckError::Status(302)));
    }
}
