//! Network half of the update check (AUTO_UPDATE.md §1.4, client steps 1-6): fetch `latest.json`
//! and `latest.json.minisig`, verify, apply the stateful rules (R8 `max_build_seen`,
//! `bad_builds`), persist. Errors go to stderr only; the caller never shows them in the UI.
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
    /// `Preferences::save` failed.
    Persist(std::io::ErrorKind),
}

/// Full check with the running build and the real clock; saves `prefs` after a valid decision.
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
    let result = run(
        prefs,
        keys,
        running,
        platform,
        UPDATE_HOST,
        SystemTime::now(),
    )
    .await;
    let result = match result {
        Ok(out) => prefs
            .save(dirs)
            .map(|()| out)
            .map_err(|e| CheckError::Persist(e.kind())),
        Err(e) => Err(e),
    };
    if let Err(e) = &result {
        eprintln!("update check failed: {e:?}");
    }
    result
}

/// Fetch + verify + decide; mutates `prefs` (not saved) only after a valid decision.
pub(super) async fn run(
    prefs: &mut Preferences,
    keys: &UpdateKeys,
    running_build: u64,
    platform: &str,
    expected_host: &str,
    now: SystemTime,
) -> Result<Outcome, CheckError> {
    let base = effective_base(prefs);
    let client = reqwest::Client::builder()
        .user_agent("Gnobbi")
        .timeout(TIMEOUT)
        // Our URLs are fixed and relative: a redirect is never legitimate.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| CheckError::Network)?;
    let manifest = get_capped(&client, &format!("{base}latest.json")).await?;
    let minisig = get_capped(&client, &format!("{base}latest.json.minisig")).await?;
    let verdict = verify_manifest(
        keys,
        &manifest,
        &minisig,
        running_build,
        now,
        platform,
        expected_host,
    );
    let now_secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    decide(prefs, verdict, &base, now_secs).map_err(CheckError::Verify)
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

/// R8 and `bad_builds` on top of the verification result. A signed manifest older than
/// `max_build_seen` is discarded; any valid one (including "not newer") raises `max_build_seen`
/// and stamps `last_check`. Verification failures leave `prefs` untouched.
fn decide(
    prefs: &mut Preferences,
    verdict: Result<Candidate, UpdateError>,
    base: &str,
    now_secs: u64,
) -> Result<Outcome, UpdateError> {
    let (build, candidate) = match verdict {
        Ok(c) => (c.build, Some(c)),
        Err(UpdateError::NotNewer { build, .. }) => (build, None),
        Err(e) => return Err(e),
    };
    let rollback = build < prefs.max_build_seen;
    prefs.max_build_seen = prefs.max_build_seen.max(build);
    prefs.last_check = Some(now_secs);
    Ok(match candidate {
        Some(c) if !rollback && !prefs.bad_builds.contains(&c.build) => Outcome::Candidate(Offer {
            url: format!("{base}{}", c.file_name),
            version: c.version,
            build: c.build,
            sha256: c.sha256,
            size: c.size,
        }),
        _ => Outcome::NoUpdate,
    })
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
