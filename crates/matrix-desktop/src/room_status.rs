//! Per-room «Messaggi di stato» choice, local to this device and account (`room_status.json` in the
//! account's profile folder, `{"!room:x": "show" | "hide"}`; «Come nelle impostazioni» = no entry).
//! Never sent to the server. shortcut: not synced between devices; room account data could carry it later.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusChoice {
    #[default]
    Inherit,
    Show,
    Hide,
}

impl StatusChoice {
    pub const ALL: [Self; 3] = [Self::Inherit, Self::Show, Self::Hide];

    /// The room's choice wins over the global «Mostra messaggi di stato».
    pub fn hides(self, global_show: bool) -> bool {
        match self {
            Self::Inherit => !global_show,
            Self::Show => false,
            Self::Hide => true,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Inherit => crate::i18n::t("Same as settings"),
            Self::Show => crate::i18n::t("Show"),
            Self::Hide => crate::i18n::t("Hide"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomStatusChoices {
    rooms: BTreeMap<String, StatusChoice>,
    file: PathBuf,
}

impl RoomStatusChoices {
    /// Missing or corrupt file: every room follows the settings; an unknown value drops only that room.
    pub fn load(profile: &Path) -> Self {
        let file = profile.join("room_status.json");
        let map: BTreeMap<String, serde_json::Value> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let rooms = map
            .into_iter()
            .filter_map(|(room, v)| match v.as_str() {
                Some("show") => Some((room, StatusChoice::Show)),
                Some("hide") => Some((room, StatusChoice::Hide)),
                _ => None,
            })
            .collect();
        Self { rooms, file }
    }

    pub fn get(&self, room: &str) -> StatusChoice {
        self.rooms.get(room).copied().unwrap_or_default()
    }

    pub fn set(&mut self, room: &str, choice: StatusChoice) {
        if choice == StatusChoice::Inherit {
            self.rooms.remove(room);
        } else {
            self.rooms.insert(room.to_owned(), choice);
        }
        let body: BTreeMap<_, _> = self.rooms.iter().map(|(r, c)| (r, if *c == StatusChoice::Hide { "hide" } else { "show" })).collect();
        let tmp = self.file.with_extension("json.tmp");
        // A failed write only loses the choice at the next start.
        if std::fs::write(&tmp, serde_json::to_vec(&body).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, &self.file);
        }
    }
}
