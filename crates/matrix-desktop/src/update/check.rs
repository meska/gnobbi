//! Network half of the update check (AUTO_UPDATE.md §1.4, client steps 1-6): fetch `latest.json`
//! and `latest.json.minisig`, verify, apply the stateful rules (R8 `max_build_seen`,
//! `bad_builds`), persist. Errors go to stderr only; the caller never shows them in the UI.
//! The fetch and the cryptographic check run outside any lock; only the decision is taken and
//! persisted inside one `Preferences::update` transaction, on the values current at commit time.
//! No download of the artifact here.

use super::verify::{
    Candidate, MAX_MANIFEST_BYTES, UpdateError, UpdateKeys, VerifiedManifest, authenticate_manifest,
};
use super::{build_number, effective_base, github};
use crate::{AppDirs, Preferences};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Host the signed artifact `url` must name; the download itself comes from the effective base.
pub const UPDATE_HOST: &str = "gnobbi.meskatech.net";
const TIMEOUT: Duration = Duration::from_secs(20);

/// A verified, newer, not-blocked build, ready for the download step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub build: u64,
    /// Effective base + signed file name (never the host written in the manifest).
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    NoUpdate,
    Candidate(Offer),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// Client could not be built, connection/timeout/body error.
    Network,
    /// Anything but 200, redirects included (never followed).
    Status(u16),
    /// A response exceeded [`MAX_MANIFEST_BYTES`] while streaming.
    TooLarge,
    /// Signature or manifest rejected.
    Verify(UpdateError),
    /// The settings transaction failed (lock contended, unreadable store, write error):
    /// nothing was persisted and no candidate is offered.
    Persist(std::io::ErrorKind),
}

/// Full check with the running build and the real clock.
pub async fn check(
    dirs: &AppDirs,
    prefs: &mut Preferences,
    keys: &UpdateKeys,
    platform: &str,
) -> Result<Outcome, CheckError> {
    check_with(dirs, prefs, keys, platform, crate::build_info::BUILD_INFO.build, UPDATE_HOST, SystemTime::now()).await
}

/// [`check`] with build, host and clock injected: the only body, so tests drive the same path.
pub(super) async fn check_with(
    dirs: &AppDirs,
    prefs: &mut Preferences,
    keys: &UpdateKeys,
    platform: &str,
    build: &str,
    host: &str,
    now: SystemTime,
) -> Result<Outcome, CheckError> {
    // An unparsable build ("sconosciuto") never updates: 0 would offer everything.
    let Some(running) = build_number(build) else {
        return Ok(Outcome::NoUpdate);
    };
    // The scheduler's snapshot may already be stale before the network starts.
    let current = Preferences::load_checked(dirs).map_err(|e| CheckError::Persist(e.kind()))?;
    *prefs = current.clone();
    let result = run(
        dirs,
        &current,
        keys,
        running,
        platform,
        host,
        now,
    )
    .await;
    if let Err(e) = &result {
        eprintln!("update check failed: {e:?}");
    }
    result
}

/// Production path with the running build and clock injected (tests drive it directly). `prefs`
/// is the configuration read before the network; it is never written back. Steps: gate on the
/// switch, fetch + verify outside the lock, then decide and persist in one transaction.
pub(super) async fn run(
    dirs: &AppDirs,
    prefs: &Preferences,
    keys: &UpdateKeys,
    running_build: u64,
    platform: &str,
    expected_host: &str,
    now: SystemTime,
) -> Result<Outcome, CheckError> {
    // Before the network: a disabled switch opens no connection at all.
    if !prefs.update_check_enabled {
        return Ok(Outcome::NoUpdate);
    }
    let base = effective_base(prefs);
    let now_secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // The stamp is the ATTEMPT (B5): written before the network, so failures and restarts cannot
    // retry within 24 h. A stamp in the future (clock moved back) is replaced by `now`.
    // It never touches `max_build_seen`/`bad_builds`.
    let mut go = false;
    Preferences::update(dirs, |p| {
        go = p.update_check_enabled && effective_base(p) == base;
        if go {
            p.last_check = Some(now_secs);
        }
    })
    .map_err(|e| CheckError::Persist(e.kind()))?;
    if !go {
        return Ok(Outcome::NoUpdate);
    }
    // GitHub Releases first, only for the default source (a custom `update_base` is a mirror).
    // The site answers only when GitHub is unreachable or failing server-side.
    if prefs.update_base.is_none() {
        match github::fetch(platform, crate::build_info::BUILD_INFO.version).await {
            Ok(None) => return Ok(Outcome::NoUpdate),
            Ok(Some(f)) => {
                let (build, url) = (f.candidate.build, f.url);
                let candidate = Some(f.candidate).filter(|c| c.build > running_build);
                return commit(dirs, &base, build, candidate, Some(url));
            }
            Err(CheckError::Network | CheckError::Status(500..=599)) => eprintln!("update: GitHub unavailable, trying the site"),
            Err(e) => return Err(e),
        }
    }
    let verified = fetch(&base, keys, now).await?;
    let build = verified.build;
    // Every authenticated manifest raises the floor (B3), even without our artifact.
    let (candidate, error) = match verified.select(running_build, platform, expected_host) {
        Ok(c) => (Some(c), None),
        Err(UpdateError::NotNewer { .. } | UpdateError::NoArtifact) => (None, None),
        Err(e) => (None, Some(CheckError::Verify(e))),
    };
    let outcome = commit(dirs, &base, build, candidate, None)?;
    error.map_or(Ok(outcome), Err)
}

/// Fetch `latest.json` + `latest.json.minisig` and run the cryptographic verification: the whole
/// network half, no store read or written.
async fn fetch(
    base: &str,
    keys: &UpdateKeys,
    now: SystemTime,
) -> Result<VerifiedManifest, CheckError> {
    let client = reqwest::Client::builder()
        .user_agent("Gnobbi")
        .timeout(TIMEOUT)
        // Our URLs are fixed and relative: a redirect is never legitimate.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| CheckError::Network)?;
    let manifest = get_capped(&client, &format!("{base}latest.json")).await?;
    let minisig = get_capped(&client, &format!("{base}latest.json.minisig")).await?;
    authenticate_manifest(keys, &manifest, &minisig, now).map_err(CheckError::Verify)
}

/// GET with a hard byte cap, counted while streaming (Content-Length is not trusted).
async fn get_capped(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, CheckError> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|_| CheckError::Network)?;
    if resp.status() != reqwest::StatusCode::OK {
        return Err(CheckError::Status(resp.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| CheckError::Network)? {
        if body.len() + chunk.len() > MAX_MANIFEST_BYTES {
            return Err(CheckError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Decide and persist in ONE transaction on the values the store holds NOW (R8 `max_build_seen`
/// and `bad_builds` on top of the verification result). The fetched verdict is dropped whole —
/// no metadata mixed in — when the effective base changed while fetching. `max_build_seen` only moves forward; `bad_builds` is the current set, never replaced by the check.
/// A failed transaction is [`CheckError::Persist`] and offers no candidate.
fn commit(
    dirs: &AppDirs,
    base: &str,
    build: u64,
    candidate: Option<Candidate>,
    url: Option<String>,
) -> Result<Outcome, CheckError> {
    let mut outcome = Outcome::NoUpdate;
    Preferences::update(dirs, |prefs| {
        if effective_base(prefs) != base {
            return;
        }
        let rollback = build < prefs.max_build_seen;
        prefs.max_build_seen = prefs.max_build_seen.max(build);
        // Switch off since the fetch, a rollback or a known-bad build: metadata only.
        if prefs.update_check_enabled
            && !rollback
            && let Some(c) = candidate
            && !prefs.bad_builds.contains(&c.build)
        {
            outcome = Outcome::Candidate(Offer {
                version: c.version,
                build: c.build,
                url: url.unwrap_or_else(|| format!("{base}{}", c.file_name)),
                sha256: c.sha256,
                size: c.size,
            });
        }
    })
    .map_err(|e| CheckError::Persist(e.kind()))?;
    Ok(outcome)
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
