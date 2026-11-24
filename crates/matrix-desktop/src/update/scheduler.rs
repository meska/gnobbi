//! Background update check: 30 s after start, then every 6 h, only on channels that update
//! themselves, in distributed builds (`GNOBBI_BUILD`) and with `update_check_enabled`. Any other
//! case opens no connection and builds no client. The check only records the offer in the shared
//! state; the download starts when the user asks (`apply`). Errors go to stderr only.
use super::channel::Channel;
use super::{channel::{Env, detect_channel}, stage::UpdateState};
use crate::{AppDirs, Preferences};
use std::{path::Path, sync::{Arc, Mutex}};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub start_delay: Duration,
    pub interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self { start_delay: Duration::from_secs(30), interval: Duration::from_secs(6 * 3600) }
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

/// Loads the settings fresh (the switch may have changed) and runs `work` only when due.
fn tick(dirs: &AppDirs, channel: &Channel, interval: Duration, work: impl FnOnce(Preferences)) -> bool {
    let Ok(prefs) = Preferences::load_checked(dirs) else { return false };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let go = due(channel, &prefs, now, interval);
    if go {
        work(prefs);
    }
    go
}

fn download_allowed(dirs: &AppDirs, channel: &Channel, initial: &Preferences) -> bool {
    let Ok(current) = Preferences::load_checked(dirs) else { return false };
    channel.self_updates() && current.update_check_enabled
        && super::effective_base(&current) == super::effective_base(initial)
}

fn run_loop(dirs: &AppDirs, channel: &Channel, cfg: Config, mut work: impl FnMut(Preferences)) {
    std::thread::sleep(cfg.start_delay);
    loop {
        // The loop is the timer (start + every `interval`): `last_check` must not skip the check at start.
        tick(dirs, channel, Duration::ZERO, &mut work);
        std::thread::sleep(cfg.interval);
    }
}

/// Starts the scheduler thread. `None` (nothing spawned) when the channel does not self-update.
pub fn start(dirs: AppDirs, state: Arc<Mutex<UpdateState>>, cfg: Config) -> Option<std::thread::JoinHandle<()>> {
    let channel = detect_channel(&Env::from_process());
    if !channel.self_updates() || !release_build(crate::build_info::DISTRIBUTED, crate::build_info::BUILD_INFO.build) {
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
                online::check(rt, &dirs, &state, &channel, &mut prefs);
            }
        });
    }).ok()
}

/// Local development builds (no `GNOBBI_BUILD`, or no numeric build) neither check nor offer anything.
pub fn release_build(distributed: bool, build: &str) -> bool {
    distributed && super::build_number(build).is_some()
}

fn target(channel: &Channel) -> &Path {
    match channel {
        Channel::AppImage { target } => target,
        Channel::WindowsPortable { exe } => exe,
        _ => Path::new(""),
    }
}

fn artifact_key(channel: &Channel, os: &str, arch: &str) -> Option<&'static str> {
    use super::verify::{LINUX_APPIMAGE, WINDOWS_ZIP};
    match (channel, os, arch) {
        (Channel::AppImage { .. }, "linux", "x86_64") => Some(LINUX_APPIMAGE),
        (Channel::WindowsPortable { .. }, "windows", "x86_64") => Some(WINDOWS_ZIP),
        _ => None,
    }
}

pub use online::stage;

mod online {
    use super::*;
    use crate::update::{check::{self, Offer, Outcome}, install, stage::Ready, verify::{Candidate, UpdateKeys}};
    use std::io::Read;

    const KEYS: &str = include_str!("../../update-keys.pub");
    const READ_TIMEOUT: Duration = Duration::from_secs(30);

    /// Production check: records the offer (if any) in the shared state, downloads nothing.
    pub fn check(rt: &tokio::runtime::Runtime, dirs: &AppDirs, state: &Mutex<UpdateState>, channel: &Channel, prefs: &mut Preferences) {
        let Ok(keys) = UpdateKeys::parse(KEYS) else { return eprintln!("update: bad embedded keys") };
        let seam = Seam { keys: &keys, build: crate::build_info::BUILD_INFO.build, host: check::UPDATE_HOST, now: SystemTime::now() };
        let offer = check_offer(rt, dirs, state, channel, prefs, &seam);
        state.lock().unwrap_or_else(|p| p.into_inner()).available = offer;
    }

    /// «Aggiorna»: download, verify, stage; the caller installs. `Err` is one line for the UI.
    pub fn stage(dirs: &AppDirs, state: &Mutex<UpdateState>, channel: &Channel, o: &Offer) -> Result<Ready, String> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().enable_all().build().map_err(|_| install::failed(""))?;
        // shortcut: prefs read now, not at check time; a changed `update_base` in between is not noticed, the offer URL stays that of its source.
        let prefs = Preferences::load_checked(dirs).map_err(|_| install::failed(""))?;
        stage_offer(&rt, dirs, state, channel, o, &prefs, &|| {}).ok_or_else(|| install::failed(""))
    }

    /// Internal injection point (production fills it with embedded keys, real build, host and
    /// clock; never read from env/config).
    pub(super) struct Seam<'a> {
        pub keys: &'a UpdateKeys,
        pub build: &'a str,
        pub host: &'a str,
        pub now: SystemTime,
    }

    pub(super) fn check_offer(rt: &tokio::runtime::Runtime, dirs: &AppDirs, state: &Mutex<UpdateState>, channel: &Channel, prefs: &mut Preferences, seam: &Seam) -> Option<Offer> {
        let platform = artifact_key(channel, std::env::consts::OS, std::env::consts::ARCH)?;
        // check() logs its own errors and stamps last_check as an attempt before any connection.
        let Ok(Outcome::Candidate(o)) = rt.block_on(check::check_with(dirs, prefs, seam.keys, platform, seam.build, seam.host, seam.now)) else { return None };
        (!state.lock().unwrap_or_else(|p| p.into_inner()).bad_session.contains(&o.build)).then_some(o)
    }

    pub(super) fn stage_offer(rt: &tokio::runtime::Runtime, dirs: &AppDirs, state: &Mutex<UpdateState>, channel: &Channel, o: &Offer, prefs: &Preferences, gate: &dyn Fn()) -> Option<Ready> {
        let c = Candidate { version: o.version.clone(), build: o.build, file_name: o.url.rsplit('/').next().unwrap_or_default().into(), sha256: o.sha256.clone(), size: o.size };
        static ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let staging = target(channel).with_file_name(format!(".gnobbi.{}.{}.{}.new", c.build, std::process::id(), id));
        let client = match reqwest::Client::builder().user_agent("Gnobbi").connect_timeout(READ_TIMEOUT).read_timeout(READ_TIMEOUT).redirect(crate::update::github::redirect_policy()).build() {
            Ok(c) => c,
            Err(e) => { eprintln!("update: client: {e}"); return None; }
        };
        gate();
        if !download_allowed(dirs, channel, prefs) { return None; }
        // send() arms its timers eagerly: it needs the runtime context, hence the async block.
        match rt.block_on(async { client.get(&o.url).send().await }) {
            Ok(resp) if resp.status() == reqwest::StatusCode::OK => {
                super::super::stage::stage_candidate(dirs, state, &c, Body { rt, resp, buf: Vec::new(), pos: 0 }, &staging).ok()
            }
            Ok(resp) => { eprintln!("update: download status {}", resp.status()); None }
            Err(e) => { eprintln!("update: download: {e}"); None }
        }
    }

    /// Test seam of the whole old flow: check, then download as soon as there is an offer.
    /// `before_download_gate` runs after the offer and before the download authorization, so a
    /// test can commit a settings change there deterministically. No lock is held across it.
    #[cfg(test)]
    pub(super) fn check_and_stage_with(rt: &tokio::runtime::Runtime, dirs: &AppDirs, state: &Mutex<UpdateState>, channel: &Channel, prefs: &mut Preferences, seam: &Seam, before_download_gate: &dyn Fn()) {
        if let Some(o) = check_offer(rt, dirs, state, channel, prefs, seam) {
            let _ = stage_offer(rt, dirs, state, channel, &o, prefs, before_download_gate);
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

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "e2e_tests.rs"]
mod e2e_tests;

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
    fn artifact_key_requires_matching_os_and_architecture() {
        let win = Channel::WindowsPortable { exe: PathBuf::from("g.exe") };
        assert_eq!(artifact_key(&appimage(), "linux", "x86_64"), Some(super::super::verify::LINUX_APPIMAGE));
        assert_eq!(artifact_key(&win, "windows", "x86_64"), Some(super::super::verify::WINDOWS_ZIP));
        assert_eq!(artifact_key(&appimage(), "linux", "aarch64"), None);
        assert_eq!(artifact_key(&win, "windows", "aarch64"), None);
        assert_eq!(artifact_key(&win, "linux", "x86_64"), None);
    }

    #[test]
    fn decision_table() {
        let now = 1_000_000;
        let win = Channel::WindowsPortable { exe: PathBuf::from("g.exe") };
        for ch in [appimage(), win] {
            assert!(should_check(&ch, &prefs(true, None), now));
            assert!(should_check(&ch, &prefs(true, Some(now - 6 * H)), now));
            assert!(!should_check(&ch, &prefs(true, Some(now - 6 * H + 1)), now));
            assert!(!should_check(&ch, &prefs(false, None), now));
            assert!(should_check(&ch, &prefs(true, Some(now + 5)), now), "clock moved back");
        }
        for ch in [Channel::MacStore, Channel::Flatpak, Channel::SystemPackage, Channel::Unknown] {
            assert!(!should_check(&ch, &prefs(true, None), now));
        }
    }

    #[test]
    fn defaults_are_30s_and_6h() {
        let c = Config::default();
        assert_eq!((c.start_delay.as_secs(), c.interval.as_secs()), (30, 6 * 3600));
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

    #[test]
    fn tick_rejects_corrupt_settings_without_work() {
        let tmp = std::env::temp_dir().join(format!("gnobbi-sched-corrupt-{}", std::process::id()));
        let dirs = AppDirs { config: tmp.join("c"), data: tmp.join("d"), cache: tmp.join("k") };
        dirs.ensure().unwrap();
        std::fs::write(dirs.config.join("settings.json"), b"{").unwrap();
        assert!(!tick(&dirs, &appimage(), Duration::from_secs(0), |_| panic!("network work")));
        assert_eq!(std::fs::read(dirs.config.join("settings.json")).unwrap(), b"{");
        std::fs::write(dirs.config.join("settings.json"), br#"{"update_check_enabled":"false"}"#).unwrap();
        assert!(!tick(&dirs, &appimage(), Duration::from_secs(0), |_| panic!("network work")));
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn download_gate_rechecks_current_switch_and_base() {
        let tmp = std::env::temp_dir().join(format!("gnobbi-sched-gate-{}", std::process::id()));
        let dirs = AppDirs { config: tmp.join("c"), data: tmp.join("d"), cache: tmp.join("k") };
        let initial = Preferences { update_base: Some("https://example.test/".into()), ..Preferences::default() };
        initial.save(&dirs).unwrap();
        assert!(download_allowed(&dirs, &appimage(), &initial));
        Preferences::update(&dirs, |p| p.update_check_enabled = false).unwrap();
        assert!(!download_allowed(&dirs, &appimage(), &initial));
        Preferences::update(&dirs, |p| { p.update_check_enabled = true; p.update_base = Some("https://other.test/".into()); }).unwrap();
        assert!(!download_allowed(&dirs, &appimage(), &initial));
        std::fs::write(dirs.config.join("settings.json"), b"{").unwrap();
        assert!(!download_allowed(&dirs, &appimage(), &initial));
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Entry-point example for QA: the production `check_and_stage_with` fed a signed fixture,
    /// build/clock and a deterministic hook that runs between the Candidate and the download gate.
    /// Only on linux-x86_64: `artifact_key` resolves an AppImage artifact nowhere else, so the
    /// check would return before any request.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn drive(name: &str, minisig: &'static str, hook: &dyn Fn(&AppDirs)) -> (Vec<String>, std::path::PathBuf) {
        use std::io::{Read, Write};
        let root = std::env::temp_dir().join(format!("gnobbi-seam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs { config: root.join("c"), data: root.join("d"), cache: root.join("k") };
        let fx = |n: &str| std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/update").join(n)).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/dl", listener.local_addr().unwrap());
        Preferences { update_base: Some(base), ..Preferences::default() }.save(&dirs).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let sig = fx(minisig);
        std::thread::spawn(move || for mut s in listener.incoming().flatten() {
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let path = String::from_utf8_lossy(&buf[..n]).split_whitespace().nth(1).unwrap_or("").to_owned();
            let body = match path.as_str() { "/dl/latest.json" => fx("valid.json"), "/dl/latest.json.minisig" => sig.clone(), _ => Vec::new() };
            let head = if body.is_empty() { "404 Not Found" } else { "200 OK" };
            log.lock().unwrap().push(path);
            let _ = s.write_all(format!("HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
            let _ = s.write_all(&body);
        });
        let keys = crate::update::verify::UpdateKeys::parse(&String::from_utf8(fx("keys.pub")).unwrap()).unwrap();
        let channel = Channel::AppImage { target: root.join("g.AppImage") };
        let mut prefs = Preferences::load(&dirs);
        let seam = online::Seam { keys: &keys, build: "41", host: "gnobbi.meskatech.net", now: UNIX_EPOCH + Duration::from_secs(1_796_496_000 - 10 * 86_400) };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        online::check_and_stage_with(&rt, &dirs, &Default::default(), &channel, &mut prefs, &seam, &|| hook(&dirs));
        let got = seen.lock().unwrap().clone();
        (got, root)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn scheduler_requests_the_artifact_only_while_the_switch_is_on() {
        const ART: &str = "/dl/Gnobbi-x86_64-42.AppImage";
        let (seen, root) = drive("on", "valid.minisig", &|_| {});
        assert!(seen.iter().any(|p| p == ART), "{seen:?}");
        let _ = std::fs::remove_dir_all(root);
        let (seen, root) = drive("off", "valid.minisig", &|d| { Preferences::update(d, |p| p.update_check_enabled = false).unwrap(); });
        assert_eq!(seen, ["/dl/latest.json", "/dl/latest.json.minisig"], "OFF before the gate: no artifact request");
        let _ = std::fs::remove_dir_all(root);
        let (seen, root) = drive("badsig", "valid.other.minisig", &|_| {});
        assert!(!seen.iter().any(|p| p == ART), "{seen:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn local_builds_never_check() {
        assert!(release_build(true, "812"));
        assert!(!release_build(false, "812"), "commit count without GNOBBI_BUILD");
        assert!(!release_build(true, "sconosciuto"));
        assert!(!release_build(true, ""));
    }

    #[test]
    fn start_is_a_noop_off_channel() {
        // Test binary is not an AppImage/Windows portable: nothing is spawned.
        let dirs = AppDirs { config: "c".into(), data: "d".into(), cache: "k".into() };
        assert!(start(dirs, Default::default(), Config::default()).is_none());
    }
}
