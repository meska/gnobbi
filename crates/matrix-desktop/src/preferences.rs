//! User preferences in `settings.json` inside the config dir. Never holds secrets.
use crate::AppDirs;
use serde_json::{Value, json};
use std::io::Write;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    fn name(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
    fn parse(s: &str) -> Option<Theme> {
        [Theme::System, Theme::Light, Theme::Dark].into_iter().find(|t| t.name() == s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub theme: Theme,
    pub debug: bool,
}

const FILE: &str = "settings.json";

impl Preferences {
    /// Missing, corrupt or invalid input yields defaults, field by field; unknown keys are ignored.
    pub fn load(dirs: &AppDirs) -> Preferences {
        let Ok(bytes) = std::fs::read(dirs.config.join(FILE)) else { return Self::default() };
        let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) else {
            return Self::default();
        };
        Preferences {
            theme: map.get("theme").and_then(Value::as_str).and_then(Theme::parse).unwrap_or_default(),
            debug: map.get("debug").and_then(Value::as_bool).unwrap_or_default(),
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
        let body = json!({ "theme": self.theme.name(), "debug": self.debug });
        opts.open(&tmp)?.write_all(body.to_string().as_bytes())?;
        std::fs::rename(tmp, dirs.config.join(FILE))
    }
}
