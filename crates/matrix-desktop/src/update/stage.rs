//! Download into staging, SHA-256, `--self-test`, state `Ready` (AUTO_UPDATE §1.4 steps 7-10).
//! The HTTP client is not here (no new dependency): the caller passes the response body as a
//! `Read`; this module enforces the size cap, verifies, runs the self-test and cleans up.
//! `bad_builds`: hash/size/network/timeout/spawn failures stay in memory for the session;
//! self-test with another build or exit != 0 is persisted (`startup::add_bad_build`).
use super::{startup, verify::{self, Candidate, UpdateError}};
use crate::AppDirs;
use std::{collections::BTreeSet, io::{Read, Write}, path::{Path, PathBuf}, sync::Mutex, time::{Duration, Instant}};

pub use super::github::MAX_ARTIFACT_BYTES;
/// The one file taken from the Windows zip.
#[cfg(not(target_os = "android"))]
const ZIP_EXE: &str = "gnobbi.exe";
pub const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Downloaded, verified and self-tested: what the app may offer with «Aggiorna ora».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub version: String,
    pub build: u64,
    /// The staging file, still named `.gnobbi.*.new`.
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
}

/// Shared with the app as `Arc<Mutex<UpdateState>>`.
#[derive(Debug, Default)]
pub struct UpdateState {
    pub ready: Option<Ready>,
    /// Found by the background check; nothing downloaded yet.
    pub available: Option<super::check::Offer>,
    /// «Aggiorna» is running.
    pub working: bool,
    /// One line of the last failed «Aggiorna».
    pub error: Option<String>,
    /// Transient failures: skipped for this session only.
    pub bad_session: BTreeSet<u64>,
}

#[derive(Debug)]
pub enum StageError {
    Io(std::io::Error),
    /// Body longer or shorter than `size`, or wrong hash.
    Verify(UpdateError),
    SelfTestSpawn(std::io::Error),
    SelfTestTimeout,
    /// Declared size above [`MAX_ARTIFACT_BYTES`]: refused before any byte is read.
    TooLarge,
    /// The Windows zip has no usable `gnobbi.exe`.
    BadZip,
    /// Exit != 0 or a line other than `gnobbi build=<N>`.
    SelfTestFailed,
}

/// Deletes the staging file on drop unless disarmed: covers every early return and panic.
struct Guard<'a>(&'a Path, bool);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if !self.1 {
            let _ = std::fs::remove_file(self.0);
        }
    }
}

pub fn stage_candidate(dirs: &AppDirs, state: &Mutex<UpdateState>, c: &Candidate, body: impl Read, staging: &Path) -> Result<Ready, StageError> {
    stage_with_timeout(dirs, state, c, body, staging, SELF_TEST_TIMEOUT)
}

pub(super) fn stage_with_timeout(dirs: &AppDirs, state: &Mutex<UpdateState>, c: &Candidate, body: impl Read, staging: &Path, timeout: Duration) -> Result<Ready, StageError> {
    let res = run(c, body, staging, timeout);
    if let Err(e) = &res {
        eprintln!("update: build {} not staged: {e:?}", c.build);
        if matches!(e, StageError::SelfTestFailed)
            && let Err(e) = startup::add_bad_build(dirs, c.build)
        {
            eprintln!("update: bad_builds not saved: {e}");
        }
        state.lock().unwrap_or_else(|p| p.into_inner()).bad_session.insert(c.build);
    }
    let ready = res?;
    state.lock().unwrap_or_else(|p| p.into_inner()).ready = Some(ready.clone());
    Ok(ready)
}

fn run(c: &Candidate, body: impl Read, staging: &Path, timeout: Duration) -> Result<Ready, StageError> {
    if c.size > MAX_ARTIFACT_BYTES {
        return Err(StageError::TooLarge);
    }
    // The Windows asset is a zip: the zip is verified as declared, then only `gnobbi.exe` is taken from it.
    let zip = c.file_name.ends_with(".zip");
    let dl = if zip { with_suffix(staging, ".zip") } else { staging.to_owned() };
    download(&dl, c, body)?;
    let (size, sha256) = if zip {
        let r = extract_exe(&dl, staging);
        let _ = std::fs::remove_file(&dl);
        r?
    } else {
        (c.size, c.sha256.clone())
    };
    let mut guard = Guard(staging, false);
    // An APK is not executable: the system installer checks it (hash here, signature in `Native.installApk`).
    if !c.file_name.ends_with(".apk") {
        // Step 9.
        #[cfg(unix)]
        std::fs::set_permissions(staging, std::os::unix::fs::PermissionsExt::from_mode(0o755)).map_err(StageError::Io)?;
        // Step 10.
        self_test(staging, c.build, timeout)?;
    }
    guard.1 = true;
    Ok(Ready { version: c.version.clone(), build: c.build, path: staging.to_owned(), size, sha256 })
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Steps 7-8: receive at most `size + 1` bytes (the cap applies while receiving), then length and hash,
/// before any chmod or exec. A failure leaves nothing behind; an existing `path` is never touched.
fn download(path: &Path, c: &Candidate, body: impl Read) -> Result<(), StageError> {
    let mut out = std::fs::OpenOptions::new();
    out.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut out, 0o600);
    let out = out.open(path).map_err(StageError::Io)?;
    let mut guard = Guard(path, false);
    {
        let mut out = out;
        std::io::copy(&mut body.take(c.size.saturating_add(1)), &mut out).map_err(StageError::Io)?;
        out.flush().map_err(StageError::Io)?;
    } // close before guard cleanup on errors, including Windows
    verify::verify_file(path, c).map_err(StageError::Verify)?;
    guard.1 = true;
    Ok(())
}

/// `gnobbi.exe` of the (already verified) zip into `dest`, never more than [`MAX_ARTIFACT_BYTES`]
/// whatever the zip header claims. Returns size and SHA-256 of the extracted file.
#[cfg(target_os = "android")] // no zip crate there (and no zip asset: the Android artifact is the APK itself)
fn extract_exe(_: &Path, _: &Path) -> Result<(u64, String), StageError> {
    Err(StageError::BadZip)
}

#[cfg(not(target_os = "android"))]
fn extract_exe(zip: &Path, dest: &Path) -> Result<(u64, String), StageError> {
    let mut ar = zip::ZipArchive::new(std::fs::File::open(zip).map_err(StageError::Io)?).map_err(|_| StageError::BadZip)?;
    let entry = ar.by_name(ZIP_EXE).map_err(|_| StageError::BadZip)?;
    if !entry.is_file() || entry.size() > MAX_ARTIFACT_BYTES {
        return Err(StageError::BadZip);
    }
    let mut out = std::fs::OpenOptions::new();
    out.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut out, 0o600);
    let mut out = out.open(dest).map_err(StageError::Io)?;
    let mut guard = Guard(dest, false);
    let n = std::io::copy(&mut entry.take(MAX_ARTIFACT_BYTES + 1), &mut out).map_err(StageError::Io)?;
    out.flush().map_err(StageError::Io)?;
    drop(out);
    if n > MAX_ARTIFACT_BYTES {
        return Err(StageError::TooLarge);
    }
    let sha256 = verify::sha256_file(dest).map_err(StageError::Io)?;
    guard.1 = true; // kept: the caller's guard takes over
    Ok((n, sha256))
}

fn self_test(path: &Path, build: u64, timeout: Duration) -> Result<(), StageError> {
    use std::process::{Command, Stdio};
    #[cfg(test)]
    let gate = crate::preferences::spawn_gate();
    let mut child = Command::new(path).arg("--self-test").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(StageError::SelfTestSpawn)?;
    #[cfg(test)]
    drop(gate);
    let start = Instant::now();
    let status = loop {
        match child.try_wait().map_err(StageError::Io)? {
            Some(s) => break s,
            None if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(StageError::SelfTestTimeout);
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut line = String::new();
    // The line is tiny, so the pipe never fills while we poll. Capped anyway.
    let _ = child.stdout.take().map(|s| s.take(256).read_to_string(&mut line));
    let expected = startup::self_test_line(&["--self-test".into()], &build.to_string()).unwrap_or_default();
    if status.success() && line.trim() == expected { Ok(()) } else { Err(StageError::SelfTestFailed) }
}
