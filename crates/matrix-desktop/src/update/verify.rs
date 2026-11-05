//! Update verification contract: signature first, then the manifest
//! rules, then the downloaded file. No network, no clock, no GUI: every input is passed in.
//!
//! Rules R1-R13 and the manifest format are in the internal `MANIFEST_SIGNING.md`.
//! Bodies are `todo!()` until; signatures and error variants are fixed.
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

use std::path::Path;
use std::time::SystemTime;

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
    #[allow(dead_code)] // stub only: the task reads it and drops this allow.
    keys: Vec<minisign_verify::PublicKey>,
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
        let _ = pub_file;
        todo!("the task")
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
    let _ = (
        keys,
        manifest,
        minisig,
        running_build,
        now,
        platform,
        expected_host,
    );
    todo!("the task")
}

/// Checks a downloaded file against the verified candidate before any chmod, rename
/// or execution. Streams the file, reads at most `size + 1` bytes; length first, then hash.
/// On error the caller deletes the file.
pub fn verify_file(path: &Path, candidate: &Candidate) -> Result<(), UpdateError> {
    let _ = (path, candidate);
    todo!("the task")
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
