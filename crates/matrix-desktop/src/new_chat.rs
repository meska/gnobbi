//! «Nuova chat»: what the result list shows and how the keyboard moves through it. Pure, so the
//! dialog, the accessibility tree and the tests all read the same entries.
use crate::search_list::{Choice, Hit, Row, SearchList};
use matrix_engine::{
    operations::SearchState,
    search::{More, SearchError, SearchState as Shared},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub user_id: String,
    pub name: Option<String>,
    pub avatar: Option<String>,
    /// The «Avvia con @nome:server» entry: a typed ID the directory did not return.
    pub direct: bool,
}

impl Hit for Entry {
    fn key(&self) -> &str {
        &self.user_id
    }
    fn title(&self) -> &str {
        self.name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or(&self.user_id)
    }
    fn detail(&self) -> String {
        if self.direct {
            String::new()
        } else {
            self.user_id.clone()
        }
    }
    fn avatar(&self) -> Option<&str> {
        self.avatar.as_deref()
    }
    /// What a screen reader says: name and ID, or the «Avvia con» line.
    fn label(&self) -> String {
        match (&self.name, self.direct) {
            (_, true) => format!("Avvia con {}", self.user_id),
            (Some(name), _) if !name.is_empty() => format!("{name}, {}", self.user_id),
            _ => self.user_id.clone(),
        }
    }
}

pub use crate::search_list::Status;

/// «Nuova chat» on the shared list. The engine's user state carries no term, so the list keeps
/// the empty one: the dialog already clears it when the field changes.
pub struct Picker {
    list: SearchList<Entry>,
    all_available: bool,
    all: bool,
}

impl Default for Picker {
    fn default() -> Self {
        Self {
            list: SearchList::default(),
            all_available: false,
            all: false,
        }
    }
}

impl Picker {
    pub fn set_state(&mut self, state: SearchState, all_available: bool) {
        self.all_available = all_available;
        if !all_available {
            self.all = false;
        }
        let term = String::new();
        let (state, lead) = match state {
            SearchState::Idle => (Shared::Idle, None),
            SearchState::Searching => (Shared::Searching { term }, None),
            SearchState::NoResults => (Shared::NoResults { term }, None),
            SearchState::Error => (
                Shared::Error {
                    term,
                    error: SearchError::Network,
                },
                None,
            ),
            SearchState::Results { hits, direct } => (
                Shared::Results {
                    term,
                    hits: hits
                        .into_iter()
                        .map(|h| Entry {
                            user_id: h.user_id,
                            name: h.display_name,
                            avatar: h.avatar_url,
                            direct: false,
                        })
                        .collect(),
                    more: More::None,
                },
                direct.map(|user_id| Entry {
                    user_id,
                    name: None,
                    avatar: None,
                    direct: true,
                }),
            ),
        };
        self.list.set_state(state);
        self.list.set_lead(lead);
    }

    /// The typed ID first when the directory did not list it, else the exact match first.
    pub fn entries(&self) -> Vec<Entry> {
        self.list
            .rows()
            .into_iter()
            .filter_map(|r| match r {
                Row::Hit(e) => Some(e.clone()),
                Row::More(_) => None,
            })
            .collect()
    }

    pub fn list(&self) -> &SearchList<Entry> {
        &self.list
    }

    pub fn status(&self) -> Option<Status> {
        self.list.status()
    }

    pub fn active(&self) -> usize {
        self.list.active()
    }

    pub fn set_active(&mut self, index: usize) {
        self.list.set_active(index);
    }

    /// Arrow keys wrap around. Returns true when the key was ours.
    pub fn key(&mut self, key: &str) -> bool {
        self.list.key(key)
    }

    /// The user ID Enter or a click starts a chat with.
    pub fn chosen(&self) -> Option<String> {
        match self.list.chosen()? {
            Choice::Hit(e) => Some(e.user_id.clone()),
            Choice::More => None,
        }
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
        assert_eq!(p.status(), Some(Status::Error(SearchError::Network)));
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
