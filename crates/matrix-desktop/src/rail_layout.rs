//! Local order and visibility of the spaces in the rail and the «Non lette» / «Tutte le chat» choice, per account (`rail-layout.json` in the
//! config dir). Never sent to the server; «Tutte» is not part of it and is always shown first.
use crate::{AppDirs, NavigationEntry};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;

const FILE: &str = "rail-layout.json";

/// Space room ids: `order` lists the spaces the user arranged, `hidden` the ones off the rail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RailLayout {
    order: Vec<String>,
    hidden: HashSet<String>,
    /// The shown view is «Non lette» instead of «Tutte le chat» (a chosen Space is not remembered).
    pub unread: bool,
}

/// One list of strings, or None when the value is not a list of distinct strings.
fn ids(value: Option<&Value>) -> Option<Vec<String>> {
    let list: Vec<String> = value?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    (list.iter().collect::<HashSet<_>>().len() == list.len()).then_some(list)
}

impl RailLayout {
    /// Spaces in rail order with their hidden flag; «Tutte» stays first. Spaces not arranged yet
    /// follow the arranged ones in the server order.
    pub fn arrange(&self, entries: Vec<NavigationEntry>) -> Vec<(NavigationEntry, bool)> {
        let rank = |e: &NavigationEntry| {
            e.scope.as_ref().map_or(0, |s| {
                1 + self
                    .order
                    .iter()
                    .position(|id| id == s.room_id.as_str())
                    .unwrap_or(self.order.len())
            })
        };
        let mut entries = entries;
        entries.sort_by_key(rank); // stable
        entries
            .into_iter()
            .map(|e| {
                let hidden = e
                    .scope
                    .as_ref()
                    .is_some_and(|s| self.hidden.contains(s.room_id.as_str()));
                (e, hidden)
            })
            .collect()
    }

    /// Moves the space `id` to position `to` among `current`, the ids as arranged now (hidden
    /// ones included, so `to` counts them). False, and nothing changes, when `id` is unknown,
    /// `to` is out of range or it is already there.
    pub fn move_to(&mut self, current: &[String], id: &str, to: usize) -> bool {
        let Some(from) = current.iter().position(|c| c == id) else {
            return false;
        };
        if to >= current.len() || to == from {
            return false;
        }
        let mut order = current.to_vec();
        let moved = order.remove(from);
        order.insert(to, moved);
        self.order = order;
        true
    }

    /// Moves a space one step (-1 up, +1 down); wrapper of `move_to` until the UI drags rows.
    pub fn shift(&mut self, current: &[String], id: &str, delta: isize) -> bool {
        let Some(at) = current.iter().position(|c| c == id) else {
            return false;
        };
        at.checked_add_signed(delta).is_some_and(|to| self.move_to(current, id, to))
    }

    pub fn toggle_hidden(&mut self, id: &str) {
        if !self.hidden.remove(id) {
            self.hidden.insert(id.to_owned());
        }
    }
}

/// Layouts of all accounts, keyed by catalog account id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RailLayouts(BTreeMap<u64, RailLayout>);

impl RailLayouts {
    /// Missing or corrupt file: every account in default order. A bad field resets only that
    /// field of that account.
    pub fn load(dirs: &AppDirs) -> Self {
        let Ok(bytes) = std::fs::read(dirs.config.join(FILE)) else {
            return Self::default();
        };
        let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&bytes) else {
            return Self::default();
        };
        Self(
            map.iter()
                .filter_map(|(key, value)| {
                    let layout = RailLayout {
                        order: ids(value.get("order")).unwrap_or_default(),
                        hidden: ids(value.get("hidden"))
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                        unread: value.get("unread").and_then(Value::as_bool).unwrap_or(false),
                    };
                    Some((key.parse().ok()?, layout))
                })
                .collect(),
        )
    }

    pub fn get(&self, account: u64) -> RailLayout {
        self.0.get(&account).cloned().unwrap_or_default()
    }

    pub fn remove(&mut self, account: u64) {
        self.0.remove(&account);
    }

    pub fn set(&mut self, account: u64, layout: RailLayout) {
        self.0.insert(account, layout);
    }

    /// Atomic write (temp file then rename), owner-only.
    pub fn save(&self, dirs: &AppDirs) -> std::io::Result<()> {
        #[cfg(unix)]
        crate::create_private_dir(&dirs.config)?;
        #[cfg(not(unix))]
        std::fs::create_dir_all(&dirs.config)?;
        let body: Map<String, Value> = self
            .0
            .iter()
            .map(|(id, l)| {
                let mut hidden: Vec<_> = l.hidden.iter().collect();
                hidden.sort();
                (
                    id.to_string(),
                    json!({ "order": l.order, "hidden": hidden, "unread": l.unread }),
                )
            })
            .collect();
        let tmp = dirs.config.join(format!("{FILE}.tmp"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        opts.open(&tmp)?
            .write_all(Value::Object(body).to_string().as_bytes())?;
        std::fs::rename(tmp, dirs.config.join(FILE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unread_view_choice_survives_save_and_load_per_account_and_defaults_to_off() {
        let dir = std::env::temp_dir().join(format!("rail-layout-{}", std::process::id()));
        let dirs = AppDirs { config: dir.join("c"), data: dir.join("d"), cache: dir.join("k") };
        let mut layouts = RailLayouts::default();
        layouts.set(1, RailLayout { unread: true, ..Default::default() });
        layouts.set(2, RailLayout::default());
        layouts.save(&dirs).unwrap();
        let back = RailLayouts::load(&dirs);
        assert!(back.get(1).unread && !back.get(2).unread && !back.get(3).unread);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
