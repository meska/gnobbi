//! Update verification contract: signature first, then the manifest
//! rules, then the downloaded file. No network, no clock, no GUI: every input is passed in.
//!
//! Rules R1-R13 and the manifest format are in the internal `MANIFEST_SIGNING.md`.
//!
//! Order inside [`verify_manifest`] (stops at the first failure):
//! 1. sizes (R1);
//! 2. signature on the raw bytes, key chosen by key id (R2-R4);
//! 3. trusted comment `gnobbi build=<N>`, as a string (R5);
//! 4. JSON parse (R6);
//! 5. `N == build` (R5);
//! 6. `valid_until` (R7);
//! 7. `build > running_build` (R9);
//! 8. platform artifact (R10);
//! 9. https, host, file name (R11).
//!
//! Nothing from the JSON is read before step 2 succeeds.
//!
//! Left to the caller (stateful): `max_build_seen` (R8: reject `build < max_build_seen`,
//! read `build` from `Ok` or from `NotNewer`), `bad_builds` (R9), size cap while downloading (R12).

use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// R1: larger inputs are rejected before any decoding.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_SIGNATURE_BYTES: usize = 4 * 1024;
/// The only manifest schema this client understands.
pub const SCHEMA: u64 = 1;
/// Artifact keys of `artifacts` (MANIFEST_SIGNING.md §2); pass one as `platform`.
pub const LINUX_APPIMAGE: &str = "linux-x86_64-appimage";
pub const WINDOWS_ZIP: &str = "windows-x64-zip";

/// Trusted public keys: 1 or 2, in file order. The first signs releases, the second is the
/// next key (rotation). A signature is accepted only from the key whose key id it carries.
///
/// Rotation: ship a build whose keys file is `[current, next]`; once enough clients run it,
/// sign with `next` and ship `[next, new_next]`. A key is never removed before the builds
/// that trust only it are gone. Revocation = drop it from the file in a build signed by the other.
pub struct UpdateKeys {
    keys: Vec<PublicKey>,
}

impl UpdateKeys {
    /// `pub_file` = concatenation of `minisign -G` `.pub` files: per key one
    /// `untrusted comment:` line + one base64 line, no blank lines, 1 or 2 keys.
    /// Anything else → [`UpdateError::BadKeyFile`]. The comment's key id is ignored:
    /// the real one is inside the base64.
    ///
    /// Production keys come from `update-keys.pub` via `include_str!`, added by card
    /// the task together with the real keys; tests build their own from fixtures.
    pub fn parse(pub_file: &str) -> Result<Self, UpdateError> {
        let lines: Vec<&str> = pub_file.lines().collect();
        if lines.is_empty() || lines.len() > 4 || !lines.len().is_multiple_of(2) {
            return Err(UpdateError::BadKeyFile);
        }
        let keys = lines
            .chunks(2)
            .map(|pair| PublicKey::decode(&pair.join("\n")).map_err(|_| UpdateError::BadKeyFile))
            .collect::<Result<_, _>>()?;
        Ok(Self { keys })
    }
}

/// A manifest that passed every rule: the only thing the download step may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Display only, never compared.
    pub version: String,
    pub build: u64,
    /// Last segment of the artifact `url`, already checked against `[A-Za-z0-9._-]+`
    /// and not `.`/`..`. The caller resolves it against the folder `latest.json` came
    /// from (public site or LAN mirror), never against the host in the manifest.
    pub file_name: String,
    /// 64 lowercase hex chars.
    pub sha256: String,
    /// Bytes, > 0.
    pub size: u64,
}

/// One variant per negative outcome; log the variant, never the manifest content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// Keys file not 1-2 decodable keys (build-time defect, not attacker input).
    BadKeyFile,
    /// R1: manifest > [`MAX_MANIFEST_BYTES`] or signature > [`MAX_SIGNATURE_BYTES`].
    TooLarge,
    /// R2: `.minisig` not UTF-8 / not decodable, or legacy (non-prehashed) signature.
    MalformedSignature,
    /// R3: no embedded key has the signature's key id.
    UnknownKey,
    /// R4: the key matches but the signature does not (altered manifest or signature).
    InvalidSignature,
    /// R5: trusted comment is not exactly `gnobbi build=<N>`, or `N` != JSON `build`.
    TrustedComment,
    /// R6: not JSON, or a required field missing / wrong type / wrong format.
    /// The `&str` names the field (for the log line).
    Malformed(&'static str),
    /// R6: `schema` is an integer other than [`SCHEMA`].
    UnsupportedSchema(u64),
    /// R7: `now >= valid_until`. No tolerance.
    Expired,
    /// R9 ("rollback"): `build <= running_build`. Normal outcome,
    /// not an attack; `build` is returned so the caller can still apply R8.
    NotNewer { build: u64, running: u64 },
    /// R10: no `artifacts[platform]`.
    NoArtifact,
    /// `url` does not parse or its scheme is not `https`.
    NotHttps,
    /// `url` host differs from `expected_host`.
    WrongHost,
    /// R11: last `url` segment empty, `.`, `..` or outside `[A-Za-z0-9._-]`.
    BadFileName,
    /// R13: file length != `size` (includes "longer than size", read stops at size+1).
    SizeMismatch { expected: u64, actual: u64 },
    /// R13: SHA-256 of the file != `sha256`.
    Sha256Mismatch,
    /// Reading the downloaded file failed.
    Io(std::io::ErrorKind),
}

/// Verifies `latest.json` against `latest.json.minisig`, then the rules on the verified
/// manifest, in the order of the module doc.
///
/// - `manifest`: exact bytes of `latest.json` (signed as-is, trailing `\n` included).
/// - `minisig`: bytes of `latest.json.minisig`.
/// - `running_build`: `BUILD_INFO.build` parsed as u64 by the caller.
/// - `now`: current time, injected (tests use `UNIX_EPOCH + Duration`).
/// - `platform`: artifact key, e.g. [`LINUX_APPIMAGE`].
/// - `expected_host`: public update host the signed `url` must name (`gnobbi.meskatech.net`).
///
/// Required fields: `schema`, `version`, `build`, `commit`, `released`, `valid_until`,
/// `channel`, `artifacts`; `version`/`commit`/`released`/`channel` only need to be strings.
/// `build` u64 ≥ 1; `valid_until` exactly `YYYY-MM-DDTHH:MM:SSZ` (UTC); artifact
/// `url` string, `sha256` 64 lowercase hex, `size` u64 > 0. Unknown fields and other
/// artifact keys are ignored. Must not panic on any input.
pub fn verify_manifest(
    keys: &UpdateKeys,
    manifest: &[u8],
    minisig: &[u8],
    running_build: u64,
    now: SystemTime,
    platform: &str,
    expected_host: &str,
) -> Result<Candidate, UpdateError> {
    if manifest.len() > MAX_MANIFEST_BYTES || minisig.len() > MAX_SIGNATURE_BYTES {
        return Err(UpdateError::TooLarge);
    }
    let sig = std::str::from_utf8(minisig)
        .ok()
        .and_then(|t| Signature::decode(t).ok())
        .ok_or(UpdateError::MalformedSignature)?;
    keys.check(manifest, &sig)?;

    let signed_build = sig
        .trusted_comment()
        .strip_prefix("gnobbi build=")
        .and_then(|n| n.parse::<u64>().ok().filter(|v| v.to_string() == n))
        .ok_or(UpdateError::TrustedComment)?;

    let json: Value =
        serde_json::from_slice(manifest).map_err(|_| UpdateError::Malformed("json"))?;
    let root = json.as_object().ok_or(UpdateError::Malformed("root"))?;
    let schema = root
        .get("schema")
        .and_then(Value::as_u64)
        .ok_or(UpdateError::Malformed("schema"))?;
    if schema != SCHEMA {
        return Err(UpdateError::UnsupportedSchema(schema));
    }
    let text = |field: &'static str| {
        root.get(field)
            .and_then(Value::as_str)
            .ok_or(UpdateError::Malformed(field))
    };
    let version = text("version")?;
    text("commit")?;
    text("released")?;
    text("channel")?;
    let valid_until =
        parse_utc(text("valid_until")?).ok_or(UpdateError::Malformed("valid_until"))?;
    let artifacts = root
        .get("artifacts")
        .and_then(Value::as_object)
        .ok_or(UpdateError::Malformed("artifacts"))?;
    let build = root
        .get("build")
        .and_then(Value::as_u64)
        .filter(|b| *b >= 1)
        .ok_or(UpdateError::Malformed("build"))?;
    if build != signed_build {
        return Err(UpdateError::TrustedComment);
    }

    let now = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if now >= valid_until {
        return Err(UpdateError::Expired);
    }
    if build <= running_build {
        return Err(UpdateError::NotNewer {
            build,
            running: running_build,
        });
    }

    let artifact = artifacts.get(platform).ok_or(UpdateError::NoArtifact)?;
    let url = artifact
        .get("url")
        .and_then(Value::as_str)
        .ok_or(UpdateError::Malformed("url"))?;
    let sha256 = artifact
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|h| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .ok_or(UpdateError::Malformed("sha256"))?;
    let size = artifact
        .get("size")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0)
        .ok_or(UpdateError::Malformed("size"))?;

    let url = url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "https")
        .ok_or(UpdateError::NotHttps)?;
    if url.host_str() != Some(expected_host) {
        return Err(UpdateError::WrongHost);
    }
    let file_name = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|n| !matches!(*n, "" | "." | "..") && n.bytes().all(is_name_byte))
        .ok_or(UpdateError::BadFileName)?;

    Ok(Candidate {
        version: version.to_owned(),
        build,
        file_name: file_name.to_owned(),
        sha256: sha256.to_owned(),
        size,
    })
}

/// Checks a downloaded file against the verified candidate before any chmod, rename
/// or execution. Streams the file, reads at most `size + 1` bytes; length first, then hash.
/// On error the caller deletes the file.
pub fn verify_file(path: &Path, candidate: &Candidate) -> Result<(), UpdateError> {
    let mut file = std::fs::File::open(path).map_err(io_err)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut actual = 0u64;
    // Never read past size + 1: a longer file is rejected without reading it all.
    let limit = candidate.size.saturating_add(1);
    while actual < limit {
        let want = usize::try_from(limit - actual).map_or(buf.len(), |w| w.min(buf.len()));
        let n = match file.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_err(e)),
        };
        hasher.update(&buf[..n]);
        actual += n as u64;
    }
    if actual != candidate.size {
        return Err(UpdateError::SizeMismatch {
            expected: candidate.size,
            actual,
        });
    }
    if hex(&hasher.finalize()) != candidate.sha256 {
        return Err(UpdateError::Sha256Mismatch);
    }
    Ok(())
}

fn io_err(e: std::io::Error) -> UpdateError {
    UpdateError::Io(e.kind())
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl UpdateKeys {
    /// R3/R4: the key is picked by key id by trying each one; `UnexpectedKeyId` moves on.
    fn check(&self, manifest: &[u8], sig: &Signature) -> Result<(), UpdateError> {
        for key in &self.keys {
            match key.verify(manifest, sig, false) {
                Ok(()) => return Ok(()),
                Err(minisign_verify::Error::UnexpectedKeyId) => {}
                Err(minisign_verify::Error::InvalidSignature) => {
                    return Err(UpdateError::InvalidSignature)
                }
                Err(_) => return Err(UpdateError::MalformedSignature),
            }
        }
        Err(UpdateError::UnknownKey)
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` (UTC, nothing else) to Unix seconds.
fn parse_utc(t: &str) -> Option<u64> {
    let b = t.as_bytes();
    let shape = b"dddd-dd-ddTdd:dd:ddZ";
    if b.len() != shape.len()
        || !b.iter().zip(shape).all(|(c, s)| match s {
            b'd' => c.is_ascii_digit(),
            _ => c == s,
        })
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| t[r].parse::<u64>().ok();
    let (y, mo, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, s) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    // Days since 1970-01-01 (civil-from-days, Howard Hinnant); y >= 1970 or None.
    let y = if mo <= 2 { y.checked_sub(1)? } else { y };
    let era = y / 400;
    let yoe = y % 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era * 146_097 + doe).checked_sub(719_468)?;
    Some(days * 86_400 + h * 3_600 + mi * 60 + s)
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
