//! User preferences in `settings.json` inside the config dir. Never holds secrets.
use crate::{Accent, AppDirs, DARK, DAY, DesignTokens, LIGHT, NIGHT};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};

/// The four themes; each maps to one semantic palette in `DesignTokens`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Classic,
    Day,
    Dark,
    Night,
}

impl Theme {
    pub const ALL: [Theme; 4] = [Theme::Classic, Theme::Day, Theme::Dark, Theme::Night];

    /// Saved value; `light`/`dark` are the names written before the four themes existed.
    fn name(self) -> &'static str {
        match self {
            Theme::Classic => "light",
            Theme::Day => "day",
            Theme::Dark => "dark",
            Theme::Night => "night",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Theme::Classic => "Classico",
            Theme::Day => "Giorno",
            Theme::Dark => "Scuro",
            Theme::Night => "Notte",
        }
    }
    /// The palette with the accent chosen in Settings.
    pub fn tokens(self) -> DesignTokens {
        self.tokens_with(crate::current_accent())
    }
    pub(crate) fn base_tokens(self) -> DesignTokens {
        match self {
            Theme::Classic => LIGHT,
            Theme::Day => DAY,
            Theme::Dark => DARK,
            Theme::Night => NIGHT,
        }
    }
    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark | Theme::Night)
    }
    fn parse(s: &str) -> Option<Theme> {
        Self::ALL.into_iter().find(|t| t.name() == s)
    }
}

impl Preferences {
    /// The accent to paint with: the chosen one, or the OS accent (blue if unreadable).
    pub fn effective_accent(&self) -> Accent {
        self.accent.unwrap_or_else(|| crate::system_accent().unwrap_or(Accent::Blue))
    }
}

/// «Notte automatica»: when it applies, the Night theme replaces the chosen one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoNight {
    #[default]
    Off,
    /// Local time between 20:00 and 07:00.
    Schedule,
    /// The OS appearance is dark.
    System,
}

impl AutoNight {
    pub const ALL: [AutoNight; 3] = [AutoNight::Off, AutoNight::Schedule, AutoNight::System];
    fn name(self) -> &'static str {
        match self {
            AutoNight::Off => "off",
            AutoNight::Schedule => "schedule",
            AutoNight::System => "system",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AutoNight::Off => "No",
            AutoNight::Schedule => "Orario",
            AutoNight::System => "Sistema",
        }
    }
    fn parse(s: &str) -> Option<AutoNight> {
        Self::ALL.into_iter().find(|a| a.name() == s)
    }
}

/// Font family of the interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontChoice {
    #[default]
    Sans,
    Serif,
    Mono,
}

impl FontChoice {
    pub const ALL: [FontChoice; 3] = [FontChoice::Sans, FontChoice::Serif, FontChoice::Mono];
    fn name(self) -> &'static str {
        match self {
            FontChoice::Sans => "sans",
            FontChoice::Serif => "serif",
            FontChoice::Mono => "mono",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            FontChoice::Sans => "Sans",
            FontChoice::Serif => "Serif",
            FontChoice::Mono => "Mono",
        }
    }
    pub fn family(self) -> &'static str {
        match self {
            FontChoice::Sans => crate::UI_FONT,
            FontChoice::Serif => crate::SERIF_FONT,
            FontChoice::Mono => crate::MONO_FONT,
        }
    }
    fn parse(s: &str) -> Option<FontChoice> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }
}

/// App icon variants (assets from the icon set); the minimal mark is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconChoice {
    #[default]
    MinimalMark,
    Gnome,
    Monster,
}

impl IconChoice {
    pub const ALL: [IconChoice; 3] = [IconChoice::MinimalMark, IconChoice::Gnome, IconChoice::Monster];
    fn name(self) -> &'static str {
        match self {
            IconChoice::MinimalMark => "segno-minimo",
            IconChoice::Gnome => "gnomo",
            IconChoice::Monster => "mostriciattolo",
        }
    }
    /// Accessible name; the interface shows only the thumbnail.
    pub fn label(self) -> &'static str {
        match self {
            IconChoice::MinimalMark => "Segno minimo",
            IconChoice::Gnome => "Gnomo",
            IconChoice::Monster => "Mostriciattolo",
        }
    }
    /// 64 px PNG thumbnail.
    pub fn thumbnail(self) -> &'static [u8] {
        match self {
            IconChoice::MinimalMark => include_bytes!("../assets/icon-segno-minimo-64.png"),
            IconChoice::Gnome => include_bytes!("../assets/icon-gnomo-64.png"),
            IconChoice::Monster => include_bytes!("../assets/icon-mostriciattolo-64.png"),
        }
    }
    fn parse(s: &str) -> Option<IconChoice> {
        Self::ALL.into_iter().find(|i| i.name() == s)
    }
}

static FONT: AtomicU8 = AtomicU8::new(0);

/// The font is process-wide, like the accent, so every view reads it while rendering.
pub fn set_font(f: FontChoice) {
    FONT.store(f as u8, Ordering::Relaxed);
}

pub fn ui_font() -> &'static str {
    FontChoice::ALL[usize::from(FONT.load(Ordering::Relaxed)).min(2)].family()
}

/// Hours 20..24 and 0..7 are night.
pub fn is_night_hour(hour: u32) -> bool {
    !(7..20).contains(&hour)
}

pub fn local_hour() -> u32 {
    chrono::Timelike::hour(&chrono::Local::now())
}

impl Preferences {
    /// The theme to paint now: the saved one (or the OS appearance), Night when «notte automatica» applies.
    pub fn effective_theme(&self, system_dark: bool, hour: u32) -> Theme {
        let night = match self.auto_night {
            AutoNight::Off => false,
            AutoNight::Schedule => is_night_hour(hour),
            AutoNight::System => system_dark,
        };
        match (night, self.theme) {
            (true, _) => Theme::Night,
            (_, Some(t)) => t,
            (_, None) if system_dark => Theme::Dark,
            _ => Theme::Classic,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    /// `None` follows the OS appearance (first start, or a missing/unknown saved value).
    pub theme: Option<Theme>,
    /// `None` follows the OS accent (falling back to blue).
    pub accent: Option<Accent>,
    pub debug: bool,
    /// «Conferme di lettura»: on by default, also when the saved value is missing or not a boolean.
    pub read_receipts: bool,
    pub auto_night: AutoNight,
    pub font: FontChoice,
    pub icon: IconChoice,
    /// «Nascondi messaggi eliminati»: on by default, also when the saved value is missing or not a boolean.
    pub hide_redacted: bool,
    /// «Riduci nel tray alla chiusura»: off by default, so closing the window keeps quitting.
    pub close_to_tray: bool,
    /// «Notifiche» (tray and panel): one choice for all accounts; off by default.
    pub notifications: bool,
    /// «Non chiedere più» on the launcher-entry proposal (Linux); «Aggiungi al menu» clears it.
    pub desktop_entry_dont_ask: bool,
    /// «Cerca aggiornamenti»: on by default, also when the saved value is missing or not a boolean.
    pub update_check_enabled: bool,
    /// Unix seconds of the last update check; `None` = never.
    pub last_check: Option<u64>,
    /// Highest manifest `build` seen (anti-rollback); 0 = none yet.
    pub max_build_seen: u64,
    /// Builds that failed the self-test or the restore; never offered again.
    pub bad_builds: BTreeSet<u64>,
    /// Download base override (LAN), hand-edited in settings.json; no UI.
    pub update_base: Option<String>,
    /// Width of the room list as dragged by the user; `None` = the design default. Per installation.
    pub list_width: Option<u32>,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            theme: None,
            accent: Some(Accent::Blue),
            debug: false,
            read_receipts: true,
            auto_night: AutoNight::Off,
            font: FontChoice::Sans,
            icon: IconChoice::MinimalMark,
            hide_redacted: true,
            close_to_tray: false,
            notifications: false,
            desktop_entry_dont_ask: false,
            update_check_enabled: true,
            last_check: None,
            max_build_seen: 0,
            bad_builds: BTreeSet::new(),
            update_base: None,
            list_width: None,
        }
    }
}

const FILE: &str = "settings.json";

/// Tests only. A process spawned while another thread holds the settings lock inherits that fd
/// until its exec completes (seen on macOS), so the lock reads as held and the write fails with
/// WouldBlock. Lock holders (`no_spawn`) hold this shared, a spawn holds it exclusively (`spawn_gate`).
#[cfg(test)]
static SPAWN_GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());

#[cfg(test)]
pub(crate) fn no_spawn() -> std::sync::RwLockReadGuard<'static, ()> {
    SPAWN_GATE.read().unwrap_or_else(|e| e.into_inner())
}

/// Polls instead of queueing as a writer: a queued writer would block the second shared reader of
/// a test that nests two writers (one waits on the other) and deadlock it.
#[cfg(test)]
pub(crate) fn spawn_gate() -> std::sync::RwLockWriteGuard<'static, ()> {
    loop {
        match SPAWN_GATE.try_write() {
            Ok(guard) => return guard,
            Err(std::sync::TryLockError::Poisoned(e)) => return e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
}

impl Preferences {
    /// Missing, corrupt or invalid input yields defaults, field by field; unknown keys are ignored.
    pub fn load(dirs: &AppDirs) -> Preferences {
        let read = || -> std::io::Result<Option<serde_json::Map<String, Value>>> {
            match std::fs::read(dirs.config.join(FILE)) {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(Value::Object(map)) => Ok(Some(map)),
                    _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid settings")),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        };
        // Steady state: current schema, no lock needed. Otherwise migrate once (best effort: a busy
        // lock or invalid update fields must not hide the user's notification choices).
        let current = |m: &serde_json::Map<String, Value>| m.get("settings_version").and_then(Value::as_u64) == Some(1);
        if let Ok(Some(map)) = read() {
            if current(&map) {
                return Self::from_map(&map);
            }
        }
        if let Err(error) = prepare_settings(dirs, None) {
            eprintln!("{}", diag("settings_migrate", &error));
        }
        match read() {
            Ok(Some(map)) => Self::from_map(&map),
            _ => Self::default(),
        }
    }

    /// Strict read for automatic network operations; only a missing file uses defaults.
    pub fn load_checked(dirs: &AppDirs) -> std::io::Result<Preferences> {
        let bytes = match std::fs::read(dirs.config.join(FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => {
                validate_update_settings(&map)?;
                Ok(Self::from_map(&map))
            },
            _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid settings")),
        }
    }

    fn from_map(map: &serde_json::Map<String, Value>) -> Preferences {
        Preferences {
            theme: map
                .get("theme")
                .and_then(Value::as_str)
                .and_then(Theme::parse),
            // Missing or unknown: blue, as before accents existed; "system" follows the OS.
            accent: match map.get("accent").and_then(Value::as_str) {
                Some("system") => None,
                v => Some(v.and_then(Accent::parse).unwrap_or(Accent::Blue)),
            },
            debug: map
                .get("debug")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            read_receipts: map
                .get("read_receipts")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            auto_night: map
                .get("auto_night")
                .and_then(Value::as_str)
                .and_then(AutoNight::parse)
                .unwrap_or_default(),
            font: map
                .get("font")
                .and_then(Value::as_str)
                .and_then(FontChoice::parse)
                .unwrap_or_default(),
            icon: map
                .get("icon")
                .and_then(Value::as_str)
                .and_then(IconChoice::parse)
                .unwrap_or_default(),
            // Key renamed from "hide_redacted": old files always saved false (the old default),
            // so reading them would pin everyone to «show»; this resets once to the new default.
            hide_redacted: map
                .get("hide_deleted")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            close_to_tray: map
                .get("close_to_tray")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            notifications: map
                .get("notifications")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            desktop_entry_dont_ask: map
                .get("desktop_entry_dont_ask")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
            update_check_enabled: map
                .get("update_check_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            last_check: map.get("last_check").and_then(Value::as_u64),
            max_build_seen: map
                .get("max_build_seen")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            bad_builds: map
                .get("bad_builds")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default(),
            update_base: map
                .get("update_base")
                .and_then(Value::as_str)
                .filter(|b| !b.trim().is_empty())
                .map(str::to_owned),
            list_width: map.get("list_width").and_then(Value::as_u64).and_then(|w| u32::try_from(w).ok()),
        }
    }

    pub fn update(dirs: &AppDirs, change: impl FnOnce(&mut Preferences)) -> std::io::Result<Preferences> {
        let mut saved = None;
        update_settings(dirs, |map| {
            let mut prefs = Self::from_map(map);
            change(&mut prefs);
            prefs.encode(map);
            saved = Some(prefs);
        })?;
        Ok(saved.expect("transaction closure executed"))
    }

    /// Legacy snapshot writer; runtime callers migrate to field-level updates.
    pub fn save(&self, dirs: &AppDirs) -> std::io::Result<()> {
        update_settings(dirs, |map| self.encode(map))
    }

    fn encode(&self, map: &mut serde_json::Map<String, Value>) {
        let body = json!({ "theme": self.theme.map_or("system", Theme::name), "accent": self.accent.map_or("system", Accent::name), "debug": self.debug, "read_receipts": self.read_receipts, "auto_night": self.auto_night.name(), "font": self.font.name(), "icon": self.icon.name(), "hide_deleted": self.hide_redacted, "close_to_tray": self.close_to_tray, "notifications": self.notifications, "desktop_entry_dont_ask": self.desktop_entry_dont_ask, "update_check_enabled": self.update_check_enabled, "last_check": self.last_check, "max_build_seen": self.max_build_seen, "bad_builds": self.bad_builds, "update_base": self.update_base, "list_width": self.list_width });
        if let Value::Object(own) = body {
            map.extend(own);
        }
    }
}

/// One log line: stable code + ErrorKind only, never paths, JSON or ids.
fn diag(code: &str, error: &std::io::Error) -> String {
    format!("Gnobbi: {code} {:?}", error.kind())
}

fn validate_update_settings(map: &serde_json::Map<String, Value>) -> std::io::Result<()> {
    let valid = map.get("settings_version").is_none_or(|v| v.as_u64().is_some_and(|n| n <= 1))
        && map.get("update_check_enabled").is_none_or(Value::is_boolean)
        && map.get("update_base").is_none_or(|v| v.is_null() || v.is_string())
        && map.get("last_check").is_none_or(|v| v.is_null() || v.as_u64().is_some())
        && map.get("max_build_seen").is_none_or(|v| v.as_u64().is_some())
        && map.get("bad_builds").is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(|n| n.as_u64().is_some())));
    if valid { Ok(()) } else { Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid update settings")) }
}

/// Reads `settings.json` as an object (empty when missing or corrupt).
pub(crate) fn read_settings(dirs: &AppDirs) -> serde_json::Map<String, Value> {
    match std::fs::read(dirs.config.join(FILE)).map(|b| serde_json::from_slice::<Value>(&b)) {
        Ok(Ok(Value::Object(map))) => map,
        _ => Default::default(),
    }
}

/// Read-modify-write of `settings.json`, atomic and owner-only; several stores share the file.
pub(crate) fn update_settings(
    dirs: &AppDirs,
    change: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> std::io::Result<()> {
    prepare_settings(dirs, Some(Box::new(change)))
}

fn prepare_settings(
    dirs: &AppDirs,
    change: Option<Box<dyn FnOnce(&mut serde_json::Map<String, Value>) + '_>>,
) -> std::io::Result<()> {
    #[cfg(test)]
    let _no_spawn = no_spawn();
    #[cfg(unix)]
    crate::create_private_dir(&dirs.config)?;
    #[cfg(not(unix))]
    std::fs::create_dir_all(&dirs.config)?;
    let path = dirs.config.join(FILE);
    reject_non_file(&path)?;
    let lock_path = dirs.config.join("settings.json.lock");
    reject_non_file(&lock_path)?;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true).create(true);
    no_follow(&mut opts);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let lock = opts.open(&lock_path)?;
    validate_regular(&lock.metadata()?)?;
    lock.try_lock().map_err(std::io::Error::from)?;
    reject_non_file(&path)?;
    let read = |path: &std::path::Path| -> std::io::Result<Vec<u8>> {
        reject_non_file(path)?;
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true);
        no_follow(&mut opts);
        let mut file = opts.open(path)?;
        validate_regular(&file.metadata()?)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes)?;
        Ok(bytes)
    };
    let legacy = if dirs.config.file_name().is_some_and(|n| n == if cfg!(target_os = "linux") { "gnobbi" } else { "Gnobbi" }) {
        Some(dirs.config.with_file_name(if cfg!(target_os = "linux") { "dime" } else { "Dime" }).join(FILE))
    } else {
        None
    };
    let bytes = match read(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match legacy {
            Some(ref path) => match read(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                result => Some(result?),
            },
            None => None,
        },
        result => Some(result?),
    };
    let had_file = bytes.is_some();
    let mut map = match bytes {
        Some(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => map,
            _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid settings")),
        },
        None => Default::default(),
    };
    validate_update_settings(&map)?;
    let needs_migration = had_file && (map.get("settings_version").is_none() || !path.exists());
    if change.is_none() && !needs_migration {
        return Ok(());
    }
    map.insert("settings_version".into(), json!(1));
    if let Some(change) = change { change(&mut map); }
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (tmp, mut file) = loop {
        let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let tmp = dirs.config.join(format!("{FILE}.{}.{}.tmp", std::process::id(), n));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        file.write_all(Value::Object(map).to_string().as_bytes())?;
        file.sync_all()?;
        drop(file);
        reject_non_file(&path)?;
        std::fs::rename(&tmp, &path)?;
        #[cfg(unix)]
        std::fs::File::open(&dirs.config)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn reject_non_file(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => validate_regular(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_regular(metadata: &std::fs::Metadata) -> std::io::Result<()> {
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "hard link rejected"));
        }
    }
    Ok(())
}

fn no_follow(opts: &mut std::fs::OpenOptions) {
    #[cfg(target_os = "linux")]
    std::os::unix::fs::OpenOptionsExt::custom_flags(opts, libc::O_NOFOLLOW | libc::O_NONBLOCK);
    #[cfg(target_os = "macos")]
    std::os::unix::fs::OpenOptionsExt::custom_flags(opts, 0x100 | 0x4);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::custom_flags(opts, 0x0020_0000);
}

#[cfg(test)]
mod diag_tests {
    #[test]
    fn diag_line_has_code_and_kind_only() {
        let e = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/home/u/secret.json @user:srv");
        assert_eq!(super::diag("settings_migrate", &e), "Gnobbi: settings_migrate PermissionDenied");
    }
}
