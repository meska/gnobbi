//! User preferences in `settings.json` inside the config dir. Never holds secrets.
use crate::{Accent, AppDirs, DARK, DAY, DesignTokens, LIGHT, NIGHT};
use serde_json::{Value, json};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// «Non chiedere più» on the launcher-entry proposal (Linux); «Aggiungi al menu» clears it.
    pub desktop_entry_dont_ask: bool,
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
            desktop_entry_dont_ask: false,
        }
    }
}

const FILE: &str = "settings.json";

impl Preferences {
    /// Missing, corrupt or invalid input yields defaults, field by field; unknown keys are ignored.
    pub fn load(dirs: &AppDirs) -> Preferences {
        let Ok(bytes) = std::fs::read(dirs.config.join(FILE)) else {
            return Self::default();
        };
        let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) else {
            return Self::default();
        };
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
            desktop_entry_dont_ask: map
                .get("desktop_entry_dont_ask")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        }
    }

    /// Atomic write (temp file then rename), owner-only. Keys it does not own stay as they are.
    pub fn save(&self, dirs: &AppDirs) -> std::io::Result<()> {
        update_settings(dirs, |map| {
            let body = json!({ "theme": self.theme.map_or("system", Theme::name), "accent": self.accent.map_or("system", Accent::name), "debug": self.debug, "read_receipts": self.read_receipts, "auto_night": self.auto_night.name(), "font": self.font.name(), "icon": self.icon.name(), "hide_deleted": self.hide_redacted, "close_to_tray": self.close_to_tray, "desktop_entry_dont_ask": self.desktop_entry_dont_ask });
            if let Value::Object(own) = body {
                map.extend(own);
            }
        })
    }
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
    #[cfg(unix)]
    crate::create_private_dir(&dirs.config)?;
    #[cfg(not(unix))]
    std::fs::create_dir_all(&dirs.config)?;
    let mut map = read_settings(dirs);
    change(&mut map);
    let tmp = dirs.config.join(format!("{FILE}.tmp"));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&tmp)?
        .write_all(Value::Object(map).to_string().as_bytes())?;
    std::fs::rename(tmp, dirs.config.join(FILE))
}
