//! Local per-room preferences (`room-prefs.json` in the config dir). Never sent to the server.
use crate::AppDirs;
use matrix_domain::RoomScope;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

const FILE: &str = "room-prefs.json";

/// Rooms whose status rows («X è entrato/a…») are hidden; absent = shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomPrefs {
    hide_status: BTreeSet<String>,
    /// Explicit per-room choice for deleted messages; absent = no choice (the global default applies).
    hide_redacted: BTreeMap<String, bool>,
}

fn key(scope: &RoomScope) -> String {
    format!("{}|{}", scope.account_id.as_u64(), scope.room_id.as_str())
}

impl RoomPrefs {
    /// Missing or corrupt file: every room shows its status rows.
    pub fn load(dirs: &AppDirs) -> Self {
        let v = std::fs::read(dirs.config.join(FILE)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        let list = v.as_ref().and_then(|v| v.get("hide_status")?.as_array().cloned()).unwrap_or_default();
        let redacted = v.as_ref().and_then(|v| v.get("hide_redacted")?.as_object().cloned()).unwrap_or_default();
        Self {
            hide_status: list.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            hide_redacted: redacted.iter().filter_map(|(k, v)| Some((k.clone(), v.as_bool()?))).collect(),
        }
    }

    pub fn hide_status(&self, scope: &RoomScope) -> bool {
        self.hide_status.contains(&key(scope))
    }

    /// The room's explicit choice, or `None` when it never chose.
    pub fn hide_redacted(&self, scope: &RoomScope) -> Option<bool> {
        self.hide_redacted.get(&key(scope)).copied()
    }

    pub fn set_hide_redacted(&mut self, scope: &RoomScope, hide: bool) {
        self.hide_redacted.insert(key(scope), hide);
    }

    pub fn toggle_hide_status(&mut self, scope: &RoomScope) {
        let k = key(scope);
        if !self.hide_status.remove(&k) {
            self.hide_status.insert(k);
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
        opts.open(&tmp)?
            .write_all(json!({ "hide_status": self.hide_status, "hide_redacted": self.hide_redacted }).to_string().as_bytes())?;
        std::fs::rename(tmp, dirs.config.join(FILE))
    }
}
