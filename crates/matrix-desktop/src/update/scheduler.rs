//! Background update check (AUTO_UPDATE §1.4): 30 s after start, then at most every 24 h, only on
//! channels that update themselves and with `update_check_enabled`. Any other case opens no
//! connection and builds no client. Errors go to stderr only; no UI here.
use super::channel::Channel;
#[cfg(feature = "live")]
use super::{channel::{Env, detect_channel}, stage::UpdateState};
use crate::{AppDirs, Preferences};
#[cfg(feature = "live")]
use std::{path::Path, sync::{Arc, Mutex}};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub start_delay: Duration,
    pub interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self { start_delay: Duration::from_secs(30), interval: Duration::from_secs(24 * 3600) }
    }
}

/// Pure decision. A `last_check` in the future (clock moved back) counts as due.
pub fn should_check(channel: &Channel, prefs: &Preferences, now: u64) -> bool {
    due(channel, prefs, now, Config::default().interval)
}

fn due(channel: &Channel, prefs: &Preferences, now: u64, interval: Duration) -> bool {
    channel.self_updates()
        && prefs.update_check_enabled
        && prefs.last_check.is_none_or(|t| t > now || now - t >= interval.as_secs())
}

#[cfg_attr(not(feature = "live"), allow(dead_code))]
/// Loads the settings fresh (the switch may have changed) and runs `work` only when due.
fn tick(dirs: &AppDirs, channel: &Channel, interval: Duration, work: impl FnOnce(Preferences)) -> bool {
    let prefs = Preferences::load(dirs);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let go = due(channel, &prefs, now, interval);
    if go {
        work(prefs);
    }
    go
}

#[cfg_attr(not(feature = "live"), allow(dead_code))]
fn run_loop(dirs: &AppDirs, channel: &Channel, cfg: Config, mut work: impl FnMut(Preferences)) {
    std::thread::sleep(cfg.start_delay);
    loop {
        tick(dirs, channel, cfg.interval, &mut work);
        std::thread::sleep(cfg.interval);
    }
}

/// Starts the scheduler thread. `None` (nothing spawned) when the channel does not self-update.
#[cfg(feature = "live")]
pub fn start(dirs: AppDirs, state: Arc<Mutex<UpdateState>>, cfg: Config) -> Option<std::thread::JoinHandle<()>> {
    let channel = detect_channel(&Env::from_process());
    if !channel.self_updates() {
        return None;
    }
    std::thread::Builder::new().name("update-check".into()).spawn(move || {
        // shortcut: leave unproven orphans untouched; add interprocess ownership before cleanup.
        // Built lazily: a disabled switch never creates the runtime or the HTTP client.
        let mut rt: Option<tokio::runtime::Runtime> = None;
        run_loop(&dirs, &channel, cfg, |mut prefs| {
            if rt.is_none() {
                match tokio::runtime::Builder::new_current_thread().enable_time().enable_all().build() {
                    Ok(r) => rt = Some(r),
                    Err(e) => return eprintln!("update: runtime: {e}"),
                }
            }
            if let Some(rt) = &rt {
                online::check_and_stage(rt, &dirs, &state, &channel, &mut prefs);
            }
        });
    }).ok()
}

#[cfg(feature = "live")]
fn target(channel: &Channel) -> &Path {
    match channel {
        Channel::AppImage { target } => target,
        Channel::WindowsPortable { exe } => exe,
        _ => Path::new(""),
    }
}

#[cfg(feature = "live")]
mod online {
    use super::*;
    use crate::update::{check::{self, Outcome}, verify::{Candidate, UpdateKeys, LINUX_APPIMAGE, WINDOWS_ZIP}};
    use std::io::Read;

    const KEYS: &str = include_str!("../../update-keys.pub");
    const READ_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn check_and_stage(rt: &tokio::runtime::Runtime, dirs: &AppDirs, state: &Mutex<UpdateState>, channel: &Channel, prefs: &mut Preferences) {
        let Ok(keys) = UpdateKeys::parse(KEYS) else { return eprintln!("update: bad embedded keys") };
        let platform = if matches!(channel, Channel::AppImage { .. }) { LINUX_APPIMAGE } else { WINDOWS_ZIP };
        // check() logs its own errors and stamps/saves last_check only after a valid answer.
        let Ok(Outcome::Candidate(o)) = rt.block_on(check::check(dirs, prefs, &keys, platform)) else { return };
        if state.lock().unwrap_or_else(|p| p.into_inner()).bad_session.contains(&o.build) {
            return;
        }
        let c = Candidate { version: o.version, build: o.build, file_name: o.url.rsplit('/').next().unwrap_or_default().into(), sha256: o.sha256, size: o.size };
        static ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let staging = target(channel).with_file_name(format!(".gnobbi.{}.{}.{}.new", c.build, std::process::id(), id));
        let client = match reqwest::Client::builder().user_agent("Gnobbi").connect_timeout(READ_TIMEOUT).read_timeout(READ_TIMEOUT).redirect(reqwest::redirect::Policy::none()).build() {
            Ok(c) => c,
            Err(e) => return eprintln!("update: client: {e}"),
        };
        match rt.block_on(client.get(&o.url).send()) {
            Ok(resp) if resp.status() == reqwest::StatusCode::OK => {
                let _ = super::super::stage::stage_candidate(dirs, state, &c, Body { rt, resp, buf: Vec::new(), pos: 0 }, &staging);
            }
            Ok(resp) => eprintln!("update: download status {}", resp.status()),
            Err(e) => eprintln!("update: download: {e}"),
        }
    }

    /// Sync `Read` over a streaming response (the staging code is blocking).
    struct Body<'a> { rt: &'a tokio::runtime::Runtime, resp: reqwest::Response, buf: Vec<u8>, pos: usize }
    impl Read for Body<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos == self.buf.len() {
                match self.rt.block_on(self.resp.chunk()).map_err(std::io::Error::other)? {
                    Some(c) => (self.buf, self.pos) = (c.to_vec(), 0),
                    None => return Ok(0),
                }
            }
            let n = out.len().min(self.buf.len() - self.pos);
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};

    const H: u64 = 3600;
    fn appimage() -> Channel { Channel::AppImage { target: PathBuf::from("/x/g.AppImage") } }
    fn prefs(enabled: bool, last: Option<u64>) -> Preferences {
        Preferences { update_check_enabled: enabled, last_check: last, ..Preferences::default() }
    }

    #[test]
    fn decision_table() {
        let now = 1_000_000;
        let win = Channel::WindowsPortable { exe: PathBuf::from("g.exe") };
        for ch in [appimage(), win] {
            assert!(should_check(&ch, &prefs(true, None), now));
            assert!(should_check(&ch, &prefs(true, Some(now - 24 * H)), now));
            assert!(!should_check(&ch, &prefs(true, Some(now - 24 * H + 1)), now));
            assert!(!should_check(&ch, &prefs(false, None), now));
            assert!(should_check(&ch, &prefs(true, Some(now + 5)), now), "clock moved back");
        }
        for ch in [Channel::MacStore, Channel::Flatpak, Channel::SystemPackage, Channel::Unknown] {
            assert!(!should_check(&ch, &prefs(true, None), now));
        }
    }

    #[test]
    fn defaults_are_30s_and_24h() {
        let c = Config::default();
        assert_eq!((c.start_delay.as_secs(), c.interval.as_secs()), (30, 86_400));
    }

    #[test]
    fn tick_runs_work_only_when_due() {
        let tmp = std::env::temp_dir().join(format!("gnobbi-sched-{}", std::process::id()));
        let dirs = AppDirs { config: tmp.join("c"), data: tmp.join("d"), cache: tmp.join("k") };
        let _ = dirs.ensure();
        let n = AtomicUsize::new(0);
        let h24 = Duration::from_secs(24 * H);
        assert!(tick(&dirs, &appimage(), h24, |_| { n.fetch_add(1, Ordering::SeqCst); }));
        assert!(!tick(&dirs, &Channel::Unknown, h24, |_| { n.fetch_add(1, Ordering::SeqCst); }));
        let off = Preferences { update_check_enabled: false, ..Default::default() };
        off.save(&dirs).unwrap();
        assert!(!tick(&dirs, &appimage(), h24, |_| { n.fetch_add(1, Ordering::SeqCst); }));
        assert_eq!(n.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[cfg(feature = "live")]
    #[test]
    fn start_is_a_noop_off_channel() {
        // Test binary is not an AppImage/Windows portable: nothing is spawned.
        let dirs = AppDirs { config: "c".into(), data: "d".into(), cache: "k".into() };
        assert!(start(dirs, Default::default(), Config::default()).is_none());
    }
}
