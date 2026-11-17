//! Which server the public-room directory is read from: the choices, the servers the user added
//! (kept per account in `settings.json`, never on the server) and the menu state. No GPUI, no network.
use crate::AppDirs;
use crate::add_server::{AddError, AddState};
use crate::preferences::{read_settings, update_settings};
use matrix_engine::search::SearchError;
use serde_json::Value;
use std::collections::BTreeMap;

pub const ADD_LABEL: &str = "Aggiungi nuovo server…";
const MINE: &str = "Il tuo server";
const FALLBACK: &str = "matrix.org";
const KEY: &str = "directory_servers";

/// Active account's server, the other accounts' servers, the added ones, then `matrix.org`
/// when absent. Case-insensitive on the server name, first occurrence wins.
pub fn choices(active: &str, other_accounts: &[String], saved: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in std::iter::once(active)
        .chain(other_accounts.iter().map(String::as_str))
        .chain(saved.iter().map(String::as_str))
        .chain(std::iter::once(FALLBACK))
    {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(s)) {
            out.push(s.to_owned());
        }
    }
    out
}

/// Added servers by account id, as stored under `directory_servers`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedServers(BTreeMap<u64, Vec<String>>);

impl SavedServers {
    /// Missing or malformed input yields what can be read; never fails.
    pub fn load(dirs: &AppDirs) -> Self {
        let Some(Value::Object(map)) = read_settings(dirs).remove(KEY) else {
            return Self::default();
        };
        let accounts = map.into_iter().filter_map(|(id, list)| {
            let list: Vec<String> = list
                .as_array()?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            Some((id.parse().ok()?, list))
        });
        Self(accounts.collect())
    }

    /// Replaces only the `directory_servers` key; the other settings stay.
    pub fn save(&self, dirs: &AppDirs) -> std::io::Result<()> {
        let body: serde_json::Map<String, Value> = self
            .0
            .iter()
            .filter(|(_, list)| !list.is_empty())
            .map(|(id, list)| (id.to_string(), list.clone().into()))
            .collect();
        update_settings(dirs, |m| {
            m.insert(KEY.into(), Value::Object(body));
        })
    }

    pub fn for_account(&self, account: u64) -> &[String] {
        self.0.get(&account).map_or(&[], Vec::as_slice)
    }

    pub fn add(&mut self, account: u64, server: &str) {
        let list = self.0.entry(account).or_default();
        if !list.iter().any(|s| s.eq_ignore_ascii_case(server)) {
            list.push(server.to_owned());
        }
    }

    /// The account is gone: its servers must not stay in `settings.json`.
    pub fn remove_account(&mut self, account: u64) {
        self.0.remove(&account);
    }

    pub fn remove(&mut self, account: u64, server: &str) {
        if let Some(list) = self.0.get_mut(&account) {
            list.retain(|s| !s.eq_ignore_ascii_case(server));
        }
    }
}

/// One line of the open menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Server(String),
    Add,
    /// Only for servers the user added.
    Remove(String),
}

/// What a choice asks the owner to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Nothing,
    /// `value()` changed or was confirmed.
    Selected,
    /// The name field must be shown.
    AddStarted,
    /// Persist `store()`.
    Removed,
}

pub struct ServerPicker {
    account: u64,
    active: String,
    others: Vec<String>,
    store: SavedServers,
    selected: Option<String>,
    open: bool,
    highlighted: usize,
    adding: Option<AddState>,
}

impl ServerPicker {
    pub fn new(account: u64, active: &str, others: Vec<String>, store: SavedServers) -> Self {
        Self {
            account,
            active: active.to_owned(),
            others,
            store,
            selected: None,
            open: false,
            highlighted: 0,
            adding: None,
        }
    }

    /// A different account became active: its server is chosen again.
    pub fn switch_account(&mut self, account: u64, active: &str, others: Vec<String>) {
        self.account = account;
        self.active = active.to_owned();
        self.others = others;
        self.selected = None;
        self.open = false;
        self.adding = None;
    }

    /// `None` = the active account's server, `Some(name)` = any other.
    pub fn value(&self) -> Option<String> {
        self.selected.clone().filter(|s| *s != self.active)
    }

    /// Name on the button.
    pub fn shown(&self) -> &str {
        self.selected.as_deref().unwrap_or(&self.active)
    }

    pub fn store(&self) -> &SavedServers {
        &self.store
    }

    fn saved(&self) -> &[String] {
        self.store.for_account(self.account)
    }

    pub fn rows(&self) -> Vec<Row> {
        let list = choices(&self.active, &self.others, self.saved());
        let mut rows: Vec<Row> = list.into_iter().map(Row::Server).collect();
        rows.push(Row::Add);
        rows.extend(self.saved().iter().filter(|s| !s.eq_ignore_ascii_case(&self.active)).cloned().map(Row::Remove));
        rows
    }

    pub fn label(&self, row: &Row) -> String {
        match row {
            Row::Server(s) if *s == self.active => format!("{s} — {MINE}"),
            Row::Server(s) => s.clone(),
            Row::Add => ADD_LABEL.to_owned(),
            Row::Remove(s) => format!("Rimuovi {s}"),
        }
    }

    pub fn is_selected(&self, row: &Row) -> bool {
        matches!(row, Row::Server(s) if s == self.shown())
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    pub fn open(&mut self) {
        self.open = true;
        self.highlighted = self.rows().iter().position(|r| self.is_selected(r)).unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn set_highlighted(&mut self, i: usize) {
        self.highlighted = i.min(self.rows().len() - 1);
    }

    /// ↑/↓ wrap, Enter chooses, Esc closes; a closed menu opens on Enter/↓/↑.
    /// `None` = key not handled.
    pub fn key(&mut self, key: &str) -> Option<Choice> {
        if !self.open {
            return matches!(key, "enter" | "space" | "down" | "up").then(|| {
                self.open();
                Choice::Nothing
            });
        }
        let n = self.rows().len();
        match key {
            "down" => self.highlighted = (self.highlighted + 1) % n,
            "up" => self.highlighted = (self.highlighted + n - 1) % n,
            "enter" | "space" => return Some(self.choose(self.highlighted)),
            "escape" => self.open = false,
            _ => return None,
        }
        Some(Choice::Nothing)
    }

    pub fn choose(&mut self, i: usize) -> Choice {
        self.open = false;
        match self.rows().get(i).cloned() {
            Some(Row::Server(s)) => {
                self.selected = Some(s);
                self.adding = None;
                Choice::Selected
            }
            Some(Row::Add) => {
                self.adding = Some(AddState::Idle);
                Choice::AddStarted
            }
            Some(Row::Remove(s)) if !s.eq_ignore_ascii_case(&self.active) => {
                self.store.remove(self.account, &s);
                self.adding = None;
                if self.selected.as_deref() == Some(s.as_str()) && !self.rows().iter().any(|r| matches!(r, Row::Server(x) if x.eq_ignore_ascii_case(&s))) {
                    self.selected = None;
                }
                Choice::Removed
            }
            _ => Choice::Nothing,
        }
    }

    pub fn is_adding(&self) -> bool {
        self.adding.is_some()
    }

    /// The error line under the name field, if any.
    pub fn add_error(&self) -> Option<&'static str> {
        self.adding.as_ref()?.error().map(AddError::text)
    }

    pub fn is_probing(&self) -> bool {
        matches!(self.adding, Some(AddState::Probing(_)))
    }

    /// «Aggiungi» is enabled: a non-blank field and no probe in flight.
    pub fn can_add(&self, text: &str) -> bool {
        self.adding.as_ref().is_some_and(|a| a.can_submit(text))
    }

    pub fn cancel_add(&mut self) {
        self.adding = None;
    }

    /// Name confirmed. Returns the canonical server to probe; a name already in the menu
    /// (any case) is just selected, a blank or invalid one shows its error, a probe in flight
    /// does nothing.
    pub fn confirm_add(&mut self, name: &str) -> Option<String> {
        let adding = self.adding.as_mut()?;
        if !adding.can_submit(name) {
            return None;
        }
        let Ok(canonical) = crate::add_server::normalize_server(name) else {
            return adding.submit(name);
        };
        let existing = self.rows().into_iter().find_map(|r| match r {
            Row::Server(s) if s.eq_ignore_ascii_case(&canonical) => Some(s),
            _ => None,
        });
        if let Some(s) = existing {
            self.selected = Some(s);
            // field stays open with the line «Già presente»: Esc or a new name goes on
            self.adding = Some(AddState::Error(AddError::Present));
            return None;
        }
        self.adding.as_mut()?.submit(name)
    }

    /// A room alias/ID was pasted in the name field: the add is over, the caller moves it to «Unisciti».
    pub fn take_join(&mut self) -> Option<String> {
        let target = self.adding.as_mut()?.take_join()?;
        self.adding = None;
        Some(target)
    }

    /// The name field changed: clears a shown error.
    pub fn add_edited(&mut self) {
        if let Some(a) = self.adding.as_mut() {
            a.edited();
        }
    }

    /// Answer of the probe: Ok saves and selects the server, Err shows one line and saves nothing.
    /// Returns true when the store changed (persist it).
    pub fn probed(&mut self, server: &str, result: Result<(), SearchError>) -> bool {
        let Some(adding) = self.adding.as_mut() else {
            return false;
        };
        if !adding.probed(server, result.map_err(AddError::from)) {
            return false;
        }
        self.store.add(self.account, server);
        self.selected = Some(server.to_owned());
        self.adding = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn order_with_one_two_and_three_accounts() {
        assert_eq!(choices("a.org", &[], &[]), v(&["a.org", "matrix.org"]));
        assert_eq!(
            choices("a.org", &v(&["b.org"]), &v(&["z.org"])),
            v(&["a.org", "b.org", "z.org", "matrix.org"])
        );
        // same server on all three accounts, then a third that differs
        assert_eq!(choices("a.org", &v(&["a.org", "a.org"]), &[]), v(&["a.org", "matrix.org"]));
        assert_eq!(
            choices("a.org", &v(&["b.org", "a.org"]), &[]),
            v(&["a.org", "b.org", "matrix.org"])
        );
    }

    #[test]
    fn matrix_org_is_not_repeated_or_moved() {
        assert_eq!(choices("matrix.org", &[], &[]), v(&["matrix.org"]));
        assert_eq!(
            choices("a.org", &v(&["matrix.org"]), &v(&["z.org"])),
            v(&["a.org", "matrix.org", "z.org"])
        );
        assert_eq!(choices("a.org", &[], &v(&["matrix.org"])), v(&["a.org", "matrix.org"]));
    }

    #[test]
    fn dedup_is_case_insensitive_but_keeps_ports() {
        assert_eq!(
            choices("a.org", &[], &v(&["A.org", "a.org:8448", "z.org", "Z.ORG"])),
            v(&["a.org", "a.org:8448", "z.org", "matrix.org"])
        );
    }

    #[test]
    fn removing_an_account_drops_only_its_servers() {
        let mut store = SavedServers::default();
        store.add(1, "a.org");
        store.add(2, "b.org");
        store.remove_account(1);
        assert!(store.for_account(1).is_empty());
        assert_eq!(store.for_account(2), ["b.org"]);
    }

    fn picker(saved: &[&str]) -> ServerPicker {
        let mut store = SavedServers::default();
        for s in saved {
            store.add(1, s);
        }
        ServerPicker::new(1, "a.org", v(&["b.org"]), store)
    }

    #[test]
    fn rows_are_servers_then_add_then_remove_for_added_only() {
        let p = picker(&["z.org"]);
        let labels: Vec<_> = p.rows().iter().map(|r| p.label(r)).collect();
        assert_eq!(
            labels,
            [
                "a.org — Il tuo server",
                "b.org",
                "z.org",
                "matrix.org",
                ADD_LABEL,
                "Rimuovi z.org"
            ]
        );
        assert!(p.is_selected(&p.rows()[0]));
        assert_eq!(picker(&[]).rows().len(), 4, "nothing removable");
    }

    #[test]
    fn value_is_none_for_the_active_server_and_resets_on_account_change() {
        let mut p = picker(&[]);
        assert_eq!(p.value(), None);
        assert_eq!(p.choose(1), Choice::Selected);
        assert_eq!((p.value(), p.shown()), (Some("b.org".into()), "b.org"));
        p.choose(0);
        assert_eq!(p.value(), None);
        p.choose(1);
        p.switch_account(2, "c.org", vec![]);
        assert_eq!((p.value(), p.shown()), (None, "c.org"));
        assert_eq!(p.rows()[0], Row::Server("c.org".into()));
    }

    #[test]
    fn keyboard_wraps_chooses_and_escapes() {
        let mut p = picker(&[]);
        assert_eq!(p.key("x"), None);
        assert_eq!(p.key("down"), Some(Choice::Nothing));
        assert!(p.is_open());
        assert_eq!(p.highlighted(), 0, "opens on the selected server");
        p.key("up");
        assert_eq!(p.highlighted(), 3, "wraps to the last row");
        p.key("down");
        p.key("down");
        assert_eq!(p.highlighted(), 1);
        assert_eq!(p.key("enter"), Some(Choice::Selected));
        assert!(!p.is_open());
        assert_eq!(p.value(), Some("b.org".into()));
        p.key("enter");
        assert_eq!(p.highlighted(), 1);
        p.key("escape");
        assert!(!p.is_open());
        assert_eq!(p.value(), Some("b.org".into()), "Esc keeps the choice");
        // Space behaves like Enter (the keyboard click on the button is ignored by the view)
        assert_eq!(p.key("space"), Some(Choice::Nothing));
        assert!(p.is_open());
        assert_eq!(p.key("space"), Some(Choice::Selected));
    }

    #[test]
    fn add_probe_ok_saves_and_selects_err_shows_one_line() {
        let mut p = picker(&[]);
        assert_eq!(p.choose(3), Choice::AddStarted);
        assert_eq!(p.confirm_add("  "), None);
        assert_eq!(p.confirm_add(" x.org "), Some("x.org".into()));
        assert_eq!(p.confirm_add("x.org"), None, "probe in flight");
        assert!(!p.probed("x.org", Err(SearchError::Refused)));
        assert_eq!(p.add_error(), Some("Il server non mostra le sue stanze"));
        assert_eq!(p.store().for_account(1), &[] as &[String]);
        assert_eq!(p.confirm_add("x.org"), Some("x.org".into()));
        assert_eq!(p.add_error(), None);
        assert!(p.probed("x.org", Ok(())));
        assert!(!p.is_adding());
        assert_eq!(p.value(), Some("x.org".into()));
        assert_eq!(p.store().for_account(1), ["x.org"]);
        // a late answer for another server is ignored
        assert!(!p.probed("y.org", Ok(())));
    }

    #[test]
    fn invalid_name_shows_its_error_and_editing_clears_it() {
        let mut p = picker(&[]);
        p.choose(3);
        assert_eq!(p.confirm_add("http://x.org"), None);
        assert_eq!(p.add_error(), Some("Sono ammessi solo indirizzi https"));
        p.add_edited();
        assert_eq!(p.add_error(), None);
        assert_eq!(p.confirm_add(" HTTPS://X.org/ "), Some("x.org".into()));
    }

    #[test]
    fn existing_name_in_any_case_selects_the_existing_one() {
        let mut p = picker(&["z.org"]);
        p.choose(4);
        assert_eq!(p.confirm_add("https://B.ORG/"), None);
        assert_eq!(p.value(), Some("b.org".into()));
        p.choose(4);
        assert_eq!(p.confirm_add("Z.org"), None);
        assert_eq!(p.value(), Some("z.org".into()));
        assert_eq!(p.add_error(), Some("Già presente"));
        assert!(!p.is_probing() && p.can_add("w.org"));
    }

    #[test]
    fn store_dedups_and_removes_case_insensitively() {
        let mut s = SavedServers::default();
        s.add(1, "x.org");
        s.add(1, "X.ORG");
        assert_eq!(s.for_account(1), ["x.org"]);
        s.remove(1, "X.org");
        assert!(s.for_account(1).is_empty());
    }

    #[test]
    fn your_own_server_is_never_removable() {
        let mut p = picker(&["a.org", "z.org"]);
        let removable: Vec<_> = p.rows().into_iter().filter_map(|r| match r {
            Row::Remove(s) => Some(s),
            _ => None,
        }).collect();
        assert_eq!(removable, ["z.org"]);
        assert_eq!(p.store().for_account(1).len(), 2, "stored, just not offered");
        let _ = &mut p;
    }

    #[test]
    fn a_name_already_in_the_menu_is_selected_without_probe() {
        let mut p = picker(&[]);
        p.choose(3);
        assert_eq!(p.confirm_add("b.org"), None);
        assert_eq!(p.value(), Some("b.org".into()));
        assert!(p.is_adding(), "field stays open with «Già presente»");
    }

    #[test]
    fn removing_an_existing_server_clears_the_duplicate_error() {
        let mut p = picker(&["b.org"]);
        let add = p.rows().iter().position(|r| *r == Row::Add).unwrap();
        p.choose(add);
        assert_eq!(p.confirm_add("b.org"), None);
        assert_eq!(p.add_error(), Some("Già presente"));
        let remove = p.rows().iter().position(|r| *r == Row::Remove("b.org".into())).unwrap();
        assert_eq!(p.choose(remove), Choice::Removed);
        assert_eq!(p.add_error(), None);
    }

    #[test]
    fn remove_drops_the_server_and_falls_back_to_the_active_one() {
        let mut p = picker(&["z.org"]);
        p.choose(2);
        assert_eq!(p.value(), Some("z.org".into()));
        assert_eq!(p.choose(5), Choice::Removed);
        assert_eq!(p.value(), None);
        assert!(!p.rows().contains(&Row::Server("z.org".into())));
    }

    fn dirs(name: &str) -> AppDirs {
        let root = std::env::temp_dir().join(format!("gnobbi-dirsrv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        AppDirs { config: root.join("cfg"), data: root.join("d"), cache: root.join("c") }
    }

    #[test]
    fn persistence_is_per_account_and_keeps_other_settings() {
        let d = dirs("persist");
        crate::Preferences { debug: true, ..Default::default() }.save(&d).unwrap();
        let mut s = SavedServers::load(&d);
        assert_eq!(s, SavedServers::default());
        s.add(1, "x.org");
        s.add(1, "x.org");
        s.add(2, "y.org");
        s.save(&d).unwrap();
        let mut again = SavedServers::load(&d);
        assert_eq!(again.for_account(1), ["x.org"]);
        assert_eq!(again.for_account(2), ["y.org"]);
        assert_eq!(again.for_account(3), &[] as &[String]);
        again.remove(1, "x.org");
        again.save(&d).unwrap();
        let last = SavedServers::load(&d);
        assert_eq!((last.for_account(1).len(), last.for_account(2)), (0, &v(&["y.org"])[..]));
        // the preferences written first survive, and saving them later keeps the servers
        assert!(crate::Preferences::load(&d).debug);
        crate::Preferences::default().save(&d).unwrap();
        assert_eq!(SavedServers::load(&d).for_account(2), ["y.org"]);
        let raw = std::fs::read_to_string(d.config.join("settings.json")).unwrap();
        assert!(raw.contains(r#""directory_servers":{"2":["y.org"]}"#), "{raw}");
    }

    #[test]
    fn malformed_stored_value_is_ignored() {
        let d = dirs("bad");
        std::fs::create_dir_all(&d.config).unwrap();
        std::fs::write(
            d.config.join("settings.json"),
            r#"{"directory_servers":{"1":["ok.org",3],"x":["no"],"2":"no"}}"#,
        )
        .unwrap();
        let s = SavedServers::load(&d);
        assert_eq!(s.for_account(1), ["ok.org"]);
        assert_eq!(s.for_account(2).len(), 0);
    }
}
