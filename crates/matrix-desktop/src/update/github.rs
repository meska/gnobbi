//! Primary update source: the newest non-draft GitHub Release of `meska/gnobbi` (pre-releases
//! included, because `/releases/latest` hides them while we are 0.x). The asset is picked by
//! file name and its SHA-256 comes from the Release's `SHA256SUMS`; the staging step verifies
//! size and hash of the download. Unlike the site manifest this source is not minisign-signed:
//! trust is TLS to GitHub plus the Release owner.

use super::check::CheckError;
use super::verify::{Candidate, LINUX_APPIMAGE, WINDOWS_ZIP};
use serde_json::Value;
use std::time::Duration;

const API: &str = "https://api.github.com/repos/meska/gnobbi";
const DOWNLOAD: &str = "https://github.com/meska/gnobbi/releases/download";
const SUMS: &str = "SHA256SUMS";
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BYTES: usize = 512 * 1024;
pub const MACOS_DMG: &str = "macos-dmg";

/// What the check needs: the offer plus the exact URL of its file.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub candidate: Candidate,
    pub url: String,
}

/// Release asset suffix of a platform key (names are fixed by `release.yml`).
fn suffix(platform: &str) -> Option<&'static str> {
    match platform {
        LINUX_APPIMAGE => Some("-linux-x86_64.AppImage"),
        WINDOWS_ZIP => Some("-windows-x64.zip"),
        MACOS_DMG => Some(".dmg"),
        _ => None,
    }
}

/// `1.2.3` or `v1.2.3-rc1` → (1, 2, 3); `None` for anything else.
fn triple(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim_start_matches('v').split(['-', '+']).next()?;
    let mut p = core.split('.').map(|n| n.parse::<u64>().ok());
    let triple = (p.next()??, p.next()??, p.next()??);
    p.next().is_none().then_some(triple)
}

fn safe_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Newest non-draft release (highest version; API order breaks no tie we care about) and its
/// asset for `platform`. `Ok(None)` = nothing to offer (no release, not newer, no asset).
pub(super) fn pick(releases: &[u8], platform: &str, running_version: &str) -> Result<Option<(String, String, u64)>, CheckError> {
    let list: Vec<Value> = serde_json::from_slice(releases).map_err(|_| CheckError::Network)?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let Some((t, rel)) = list
        .into_iter()
        .filter(|r| r.get("draft").and_then(Value::as_bool) != Some(true))
        .filter_map(|r| Some((triple(&text(&r, "tag_name")?)?, r)))
        .max_by_key(|(t, _)| *t)
    else {
        return Ok(None);
    };
    if t <= triple(running_version).unwrap_or((u64::MAX, 0, 0)) {
        return Ok(None);
    }
    let tag = text(&rel, "tag_name").unwrap_or_default();
    let Some(suffix) = suffix(platform) else { return Ok(None) };
    let found = rel.get("assets").and_then(Value::as_array).into_iter().flatten().find_map(|a| {
        let name = text(a, "name").filter(|n| n.ends_with(suffix) && safe_name(n))?;
        Some((name, a.get("size").and_then(Value::as_u64).filter(|s| *s > 0)?))
    });
    let Some((name, size)) = found else { return Ok(None) };
    if !safe_name(&tag) {
        return Ok(None);
    }
    Ok(Some((tag, name, size)))
}

/// SHA-256 of `name` in a `sha256sum ./*` file (`<hex>  ./<name>`); lowercase 64-hex only.
pub(super) fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (h, f) = l.split_once(char::is_whitespace)?;
        let f = f.trim_start_matches([' ', '*']).trim_start_matches("./");
        (f == name && h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then(|| h.to_owned())
    })
}

/// Number of commits behind `sha`/tag: the last page of a `per_page=1` listing, i.e. the
/// `git rev-list --count` that `build.rs` embeds as the build number.
pub(super) fn build_from_commits(link: Option<&str>, body: &[u8]) -> Option<u64> {
    if let Some(l) = link {
        let last = l.split(',').find(|p| p.contains("rel=\"last\""))?;
        let q = last.split("page=").last()?;
        return q.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok();
    }
    let n = serde_json::from_slice::<Vec<serde_json::Value>>(body).ok()?.len() as u64;
    (n > 0).then_some(n)
}

/// Follow redirects only from github.com (assets live on a CDN host) and only to https.
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|a| {
        let from_github = a.previous().first().is_some_and(|u| u.host_str() == Some("github.com"));
        if from_github && a.url().scheme() == "https" && a.previous().len() < 4 { a.follow() } else { a.stop() }
    })
}

async fn get(client: &reqwest::Client, url: &str) -> Result<(Vec<u8>, Option<String>), CheckError> {
    let mut resp = client.get(url).send().await.map_err(|_| CheckError::Network)?;
    if resp.status() != reqwest::StatusCode::OK {
        return Err(CheckError::Status(resp.status().as_u16()));
    }
    let link = resp.headers().get(reqwest::header::LINK).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let mut body = Vec::new();
    while let Some(c) = resp.chunk().await.map_err(|_| CheckError::Network)? {
        if body.len() + c.len() > MAX_BYTES {
            return Err(CheckError::TooLarge);
        }
        body.extend_from_slice(&c);
    }
    Ok((body, link))
}

/// Network half. 404 or an empty list = no release yet = `Ok(None)`, silently. Rate limit
/// (403/429) and other statuses are `CheckError::Status`; connection problems and 5xx are the
/// caller's cue to fall back to the site.
pub(super) async fn fetch(platform: &str, running_version: &str) -> Result<Option<Found>, CheckError> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Gnobbi/{running_version}"))
        .default_headers(reqwest::header::HeaderMap::from_iter([
            (reqwest::header::ACCEPT, "application/vnd.github+json".parse().unwrap()),
        ]))
        .timeout(TIMEOUT)
        .redirect(redirect_policy())
        .build()
        .map_err(|_| CheckError::Network)?;
    let list = match get(&client, &format!("{API}/releases?per_page=10")).await {
        Ok((b, _)) => b,
        Err(CheckError::Status(404)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some((tag, name, size)) = pick(&list, platform, running_version)? else { return Ok(None) };
    let (sums, _) = get(&client, &format!("{DOWNLOAD}/{tag}/{SUMS}")).await?;
    let sha256 = std::str::from_utf8(&sums).ok().and_then(|s| sum_for(s, &name)).ok_or(CheckError::Network)?;
    let (body, link) = get(&client, &format!("{API}/commits?sha={tag}&per_page=1")).await?;
    let build = build_from_commits(link.as_deref(), &body).ok_or(CheckError::Network)?;
    let version = tag.trim_start_matches('v').to_owned();
    let url = format!("{DOWNLOAD}/{tag}/{name}");
    Ok(Some(Found { candidate: Candidate { version, build, file_name: name, sha256, size }, url }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "d2358579a99e69a151932fe9805328e88c226dbf8fdd72cc5aff1085931fad40";
    // Newest first, as GitHub returns it: a draft, a prerelease, an older release.
    const LIST: &str = r#"[
      {"tag_name":"v0.9.0","draft":true,"prerelease":true,"assets":[{"name":"Gnobbi-0.9.0-linux-x86_64.AppImage","size":5}]},
      {"tag_name":"v0.3.0","draft":false,"prerelease":true,"assets":[
        {"name":"Gnobbi-0.3.0-linux-x86_64.AppImage","size":1000},
        {"name":"gnobbi-0.3.0-windows-x64.zip","size":2000},
        {"name":"Gnobbi-0.3.0.dmg","size":3000},
        {"name":"SHA256SUMS","size":300}]},
      {"tag_name":"v0.2.0","draft":false,"prerelease":false,"assets":[{"name":"Gnobbi-0.2.0-linux-x86_64.AppImage","size":900}]}
    ]"#;

    #[test]
    fn prerelease_wins_draft_ignored_per_platform() {
        let p = |k| pick(LIST.as_bytes(), k, "0.2.0").unwrap();
        assert_eq!(p(LINUX_APPIMAGE), Some(("v0.3.0".into(), "Gnobbi-0.3.0-linux-x86_64.AppImage".into(), 1000)));
        assert_eq!(p(WINDOWS_ZIP).unwrap().1, "gnobbi-0.3.0-windows-x64.zip");
        assert_eq!(p(MACOS_DMG).unwrap().1, "Gnobbi-0.3.0.dmg");
    }

    #[test]
    fn nothing_to_offer() {
        // not newer, equal version, no release, only a draft, unknown platform
        assert_eq!(pick(LIST.as_bytes(), LINUX_APPIMAGE, "0.3.0").unwrap(), None);
        assert_eq!(pick(LIST.as_bytes(), LINUX_APPIMAGE, "1.0.0").unwrap(), None);
        assert_eq!(pick(b"[]", LINUX_APPIMAGE, "0.1.0").unwrap(), None);
        assert_eq!(pick(br#"[{"tag_name":"v0.5.0","draft":true,"assets":[]}]"#, LINUX_APPIMAGE, "0.1.0").unwrap(), None);
        assert_eq!(pick(LIST.as_bytes(), "linux-aarch64", "0.1.0").unwrap(), None);
        // newest release has no asset for this platform
        let no_win = br#"[{"tag_name":"v0.3.0","assets":[{"name":"Gnobbi-0.3.0-linux-x86_64.AppImage","size":1}]}]"#;
        assert_eq!(pick(no_win, WINDOWS_ZIP, "0.1.0").unwrap(), None);
        // hostile names never become a URL
        let evil = br#"[{"tag_name":"v0.3.0","assets":[{"name":"a/../b-linux-x86_64.AppImage","size":1}]}]"#;
        assert_eq!(pick(evil, LINUX_APPIMAGE, "0.1.0").unwrap(), None);
        assert!(pick(b"not json", LINUX_APPIMAGE, "0.1.0").is_err());
    }

    #[test]
    fn sums_lookup() {
        let s = format!("{H}  ./Gnobbi-0.3.0-linux-x86_64.AppImage\n{}  ./other.zip\n", "A".repeat(64));
        assert_eq!(sum_for(&s, "Gnobbi-0.3.0-linux-x86_64.AppImage").as_deref(), Some(H));
        assert_eq!(sum_for(&s, "other.zip"), None, "uppercase/odd hash rejected");
        assert_eq!(sum_for(&s, "missing"), None);
        assert_eq!(sum_for(&format!("{H} *plain.bin"), "plain.bin").as_deref(), Some(H));
    }

    #[test]
    fn build_from_link_or_body() {
        let link = r#"<https://api.github.com/repositories/1/commits?sha=v0.3.0&per_page=1&page=2>; rel="next", <https://api.github.com/repositories/1/commits?sha=v0.3.0&per_page=1&page=1234>; rel="last""#;
        assert_eq!(build_from_commits(Some(link), b"[{}]"), Some(1234));
        assert_eq!(build_from_commits(None, b"[{}]"), Some(1));
        assert_eq!(build_from_commits(None, b"[]"), None);
    }

    #[test]
    fn triple_parsing() {
        assert_eq!(triple("v0.10.0"), Some((0, 10, 0)));
        assert_eq!(triple("1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(triple("1.2"), None);
        assert_eq!(triple("nightly"), None);
    }

    #[test]
    fn sha_mismatch_is_refused() {
        // The hash taken from SHA256SUMS goes into Candidate; other bytes must be rejected.
        let c = Candidate { version: "0.3.0".into(), build: 9, file_name: "a".into(), sha256: H.into(), size: 4 };
        let f = std::env::temp_dir().join(format!("gnobbi-gh-sha-{}", std::process::id()));
        std::fs::write(&f, b"data").unwrap();
        let r = super::super::verify::verify_file(&f, &c);
        let _ = std::fs::remove_file(&f);
        assert_eq!(r, Err(super::super::verify::UpdateError::Sha256Mismatch));
    }
}
