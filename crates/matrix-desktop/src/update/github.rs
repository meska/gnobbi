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
    let client = reqwest::Client::builder()
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
}
