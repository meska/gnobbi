//! User preferences in `settings.json` inside the config dir. Never holds secrets.
use crate::{Accent, AppDirs, DARK, DAY, DesignTokens, LIGHT, NIGHT};
use serde_json::{Value, json};
use std::io::Write;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    /// `None` follows the OS appearance (first start, or a missing/unknown saved value).
    pub theme: Option<Theme>,
    /// `None` follows the OS accent (falling back to blue).
    pub accent: Option<Accent>,
    pub debug: bool,
    /// «Conferme di lettura»: on by default, also when the saved value is missing or not a boolean.
    pub read_receipts: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            theme: None,
            accent: Some(Accent::Blue),
            debug: false,
            read_receipts: true,
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
        }
    }

    /// Atomic write (temp file then rename), owner-only.
    pub fn save(&self, dirs: &AppDirs) -> std::io::Result<()> {
        #[cfg(unix)]
        crate::create_private_dir(&dirs.config)?;
        #[cfg(not(unix))]
        std::fs::create_dir_all(&dirs.config)?;
        let tmp = dirs.config.join(format!("{FILE}.tmp"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let body = json!({ "theme": self.theme.map_or("system", Theme::name), "accent": self.accent.map_or("system", Accent::name), "debug": self.debug, "read_receipts": self.read_receipts });
        opts.open(&tmp)?.write_all(body.to_string().as_bytes())?;
        std::fs::rename(tmp, dirs.config.join(FILE))
    }
}
