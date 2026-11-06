//! «Nuova chat»: what the result list shows and how the keyboard moves through it. Pure, so the
//! dialog, the accessibility tree and the tests all read the same entries.
use matrix_engine::operations::SearchState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub user_id: String,
    pub name: Option<String>,
    pub avatar: Option<String>,
    /// The «Avvia con @nome:server» entry: a typed ID the directory did not return.
    pub direct: bool,
}

impl Entry {
    /// What a screen reader says: name and ID, or the «Avvia con» line.
    pub fn label(&self) -> String {
        match (&self.name, self.direct) {
            (_, true) => format!("Avvia con {}", self.user_id),
            (Some(name), _) if !name.is_empty() => format!("{name}, {}", self.user_id),
            _ => self.user_id.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Searching,
    NoResults,
    Error,
    Partial,
}

impl Status {
    /// One line, no help text. Also the announcement made when the state changes.
    pub fn text(self) -> &'static str {
        match self {
            Self::Searching => "Ricerca in corso",
            Self::NoResults => "Nessun risultato",
            Self::Error => "Ricerca non riuscita",
            Self::Partial => "Risultati parziali",
        }
    }
}

pub struct Picker {
    state: SearchState,
    all_available: bool,
    all: bool,
    active: usize,
}

impl Default for Picker {
    fn default() -> Self {
        Self {
            state: SearchState::Idle,
            all_available: false,
            all: false,
            active: 0,
        }
    }
}

impl Picker {
    pub fn set_state(&mut self, state: SearchState, all_available: bool) {
        self.all_available = all_available;
        if !all_available {
            self.all = false;
        }
        if self.state != state {
            self.active = 0;
            self.state = state;
        }
    }

    /// The typed ID first when the directory did not list it, else the exact match first.
    pub fn entries(&self) -> Vec<Entry> {
        let SearchState::Results { hits, direct } = &self.state else {
            return Vec::new();
        };
        let mut out: Vec<Entry> = hits
            .iter()
            .map(|h| Entry {
                user_id: h.user_id.clone(),
                name: h.display_name.clone(),
                avatar: h.avatar_url.clone(),
                direct: false,
            })
            .collect();
        if let Some(id) = direct {
            match out.iter().position(|e| &e.user_id == id) {
                Some(at) => out[..=at].rotate_right(1),
                None => out.insert(
                    0,
                    Entry {
                        user_id: id.clone(),
                        name: None,
                        avatar: None,
                        direct: true,
                    },
                ),
            }
        }
        out
    }

    pub fn status(&self) -> Option<Status> {
        match self.state {
            SearchState::Searching => Some(Status::Searching),
            SearchState::NoResults => Some(Status::NoResults),
            SearchState::Error => Some(Status::Error),
            _ => None,
        }
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.entries().len().saturating_sub(1));
    }

    /// Arrow keys wrap around. Returns true when the key was ours.
    pub fn key(&mut self, key: &str) -> bool {
        let len = self.entries().len();
        if len == 0 {
            return false;
        }
        match key {
            "down" => self.active = (self.active + 1) % len,
            "up" => self.active = (self.active + len - 1) % len,
            _ => return false,
        }
        true
    }

    /// The user ID Enter or a click starts a chat with.
    pub fn chosen(&self) -> Option<String> {
        self.entries()
            .into_iter()
            .nth(self.active)
            .map(|e| e.user_id)
    }

    pub fn all_available(&self) -> bool {
        self.all_available
    }

    pub fn all(&self) -> bool {
        self.all
    }

    pub fn toggle_all(&mut self) {
        if self.all_available {
            self.all = !self.all;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_engine::operations::UserHit;

    fn hit(id: &str, name: &str) -> UserHit {
        UserHit {
            user_id: id.into(),
            display_name: Some(name.into()),
            avatar_url: None,
        }
    }
    fn results(hits: Vec<UserHit>, direct: Option<&str>) -> Picker {
        let mut p = Picker::default();
        p.set_state(
            SearchState::Results {
                hits,
                direct: direct.map(Into::into),
            },
            false,
        );
        p
    }

    #[test]
    fn arrows_wrap_and_enter_picks_the_active_user() {
        let mut p = results(vec![hit("@ma:s", "Mara"), hit("@mo:s", "Moro")], None);
        assert_eq!(p.chosen().as_deref(), Some("@ma:s"));
        assert!(p.key("down"));
        assert_eq!(p.chosen().as_deref(), Some("@mo:s"));
        assert!(p.key("down"));
        assert_eq!(p.chosen().as_deref(), Some("@ma:s"));
        assert!(p.key("up"));
        assert_eq!(p.chosen().as_deref(), Some("@mo:s"));
        assert!(!p.key("left"));
    }

    #[test]
    fn typed_id_missing_from_the_list_is_offered_first() {
        let p = results(vec![hit("@ma:s", "Mara")], Some("@zed:s"));
        let e = p.entries();
        assert!(e[0].direct);
        assert_eq!(e[0].label(), "Avvia con @zed:s");
        assert_eq!(p.chosen().as_deref(), Some("@zed:s"));
    }

    #[test]
    fn typed_id_present_in_the_list_is_the_exact_match_first() {
        let p = results(
            vec![hit("@ma:s", "Mara"), hit("@mo:s", "Moro")],
            Some("@mo:s"),
        );
        let e = p.entries();
        assert_eq!(e.len(), 2);
        assert!(!e[0].direct);
        assert_eq!(e[0].label(), "Moro, @mo:s");
        assert_eq!(e[1].user_id, "@ma:s");
    }

    #[test]
    fn no_list_no_keys_and_one_line_statuses() {
        let mut p = Picker::default();
        assert!(!p.key("down") && p.chosen().is_none() && p.status().is_none());
        p.set_state(SearchState::NoResults, false);
        assert_eq!(p.status().unwrap().text(), "Nessun risultato");
        p.set_state(SearchState::Searching, false);
        assert_eq!(p.status(), Some(Status::Searching));
        p.set_state(SearchState::Error, false);
        assert_eq!(p.status(), Some(Status::Error));
    }

    #[test]
    fn all_switch_only_when_the_server_allows_it() {
        let mut p = Picker::default();
        p.toggle_all();
        assert!(!p.all());
        p.set_state(SearchState::Idle, true);
        p.toggle_all();
        assert!(p.all());
        p.set_state(SearchState::Idle, false);
        assert!(!p.all());
    }

    #[test]
    fn a_new_result_set_restarts_from_the_first_entry() {
        let mut p = results(vec![hit("@ma:s", "Mara"), hit("@mo:s", "Moro")], None);
        p.set_active(1);
        p.set_state(
            SearchState::Results {
                hits: vec![hit("@mi:s", "Mia")],
                direct: None,
            },
            false,
        );
        assert_eq!(p.chosen().as_deref(), Some("@mi:s"));
    }
}
