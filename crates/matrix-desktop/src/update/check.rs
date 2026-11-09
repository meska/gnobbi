//! Network half of the update check (AUTO_UPDATE.md §1.4, client steps 1-6): fetch `latest.json`
//! and `latest.json.minisig`, verify, apply the stateful rules (R8 `max_build_seen`,
//! `bad_builds`), persist. Errors go to stderr only; the caller never shows them in the UI.
//! The fetch and the cryptographic check run outside any lock; only the decision is taken and
//! persisted inside one `Preferences::update` transaction, on the values current at commit time.
//! No download of the artifact here.

use super::verify::{Candidate, MAX_MANIFEST_BYTES, UpdateError, UpdateKeys, verify_manifest};
use super::{build_number, effective_base};
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
    // An unparsable build ("sconosciuto") never updates: 0 would offer everything.
    let Some(running) = build_number(crate::build_info::BUILD_INFO.build) else {
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
        UPDATE_HOST,
        SystemTime::now(),
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
    let verdict = fetch(&base, keys, running_build, now, platform, expected_host).await?;
    let (build, candidate) = match verdict {
        Ok(c) => (c.build, Some(c)),
        Err(UpdateError::NotNewer { build, .. }) => (build, None),
        // Invalid answer or network error: metadata is never modified.
        Err(e) => return Err(CheckError::Verify(e)),
    };
    commit(dirs, &base, build, candidate, now_secs)
}

/// Fetch `latest.json` + `latest.json.minisig` and run the cryptographic verification: the whole
/// network half, no store read or written.
async fn fetch(
    base: &str,
    keys: &UpdateKeys,
    running_build: u64,
    now: SystemTime,
    platform: &str,
    expected_host: &str,
) -> Result<Result<Candidate, UpdateError>, CheckError> {
    let client = reqwest::Client::builder()
        .user_agent("Gnobbi")
        .timeout(TIMEOUT)
        // Our URLs are fixed and relative: a redirect is never legitimate.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| CheckError::Network)?;
    let manifest = get_capped(&client, &format!("{base}latest.json")).await?;
    let minisig = get_capped(&client, &format!("{base}latest.json.minisig")).await?;
    Ok(verify_manifest(
        keys,
        &manifest,
        &minisig,
        running_build,
        now,
        platform,
        expected_host,
    ))
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
/// no metadata mixed in — when the effective base changed while fetching. `max_build_seen` and
/// `last_check` only move forward; `bad_builds` is the current set, never replaced by the check.
/// A failed transaction is [`CheckError::Persist`] and offers no candidate.
fn commit(
    dirs: &AppDirs,
    base: &str,
    build: u64,
    candidate: Option<Candidate>,
    now_secs: u64,
) -> Result<Outcome, CheckError> {
    let mut outcome = Outcome::NoUpdate;
    Preferences::update(dirs, |prefs| {
        if effective_base(prefs) != base {
            return;
        }
        let rollback = build < prefs.max_build_seen;
        prefs.max_build_seen = prefs.max_build_seen.max(build);
        prefs.last_check = Some(prefs.last_check.unwrap_or(0).max(now_secs));
        // Switch off since the fetch, a rollback or a known-bad build: metadata only.
        if prefs.update_check_enabled
            && !rollback
            && let Some(c) = candidate
            && !prefs.bad_builds.contains(&c.build)
        {
            outcome = Outcome::Candidate(Offer {
                version: c.version,
                build: c.build,
                url: format!("{base}{}", c.file_name),
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
