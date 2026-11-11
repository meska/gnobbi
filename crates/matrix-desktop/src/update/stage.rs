//! Download into staging, SHA-256, `--self-test`, state `Ready` (AUTO_UPDATE §1.4 steps 7-10).
//! The HTTP client is not here (no new dependency): the caller passes the response body as a
//! `Read`; this module enforces the size cap, verifies, runs the self-test and cleans up.
//! `bad_builds`: hash/size/network/timeout/spawn failures stay in memory for the session;
//! self-test with another build or exit != 0 is persisted (`startup::add_bad_build`).
use super::{startup, verify::{self, Candidate, UpdateError}};
use crate::AppDirs;
use std::{collections::BTreeSet, io::{Read, Write}, path::{Path, PathBuf}, sync::Mutex, time::{Duration, Instant}};

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
    // Step 7: hard cap. One byte past `size` is enough for verify_file to reject it.
    let mut out = std::fs::OpenOptions::new();
    out.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut out, 0o600);
    let out = out.open(staging).map_err(StageError::Io)?;
    let mut guard = Guard(staging, false);
    {
        let mut out = out;
        std::io::copy(&mut body.take(c.size.saturating_add(1)), &mut out).map_err(StageError::Io)?;
        out.flush().map_err(StageError::Io)?;
    } // close before guard cleanup on errors, including Windows
    // Step 8: length then hash, before any chmod or exec.
    verify::verify_file(staging, c).map_err(StageError::Verify)?;
    // Step 9.
    #[cfg(unix)]
    std::fs::set_permissions(staging, std::os::unix::fs::PermissionsExt::from_mode(0o755)).map_err(StageError::Io)?;
    // Step 10.
    self_test(staging, c.build, timeout)?;
    guard.1 = true;
    Ok(Ready { version: c.version.clone(), build: c.build, path: staging.to_owned(), size: c.size, sha256: c.sha256.clone() })
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
