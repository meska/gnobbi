//! Provider-independent result list of a search field (the SEARCH_CONTRACT notes §1b).
//! Pure logic: the dialog view, the accessibility tree and the tests read the same rows.
//!
//! Use: implement [`Hit`] for the provider's result type, feed the field text to [`set_term`]
//! (instant local state) and the engine's `SearchState<H>` to [`set_state`] (dropped when it
//! answers another term). Keys: [`key`] moves, [`chosen`] says what Enter or a click picked;
//! `Choice::More` means «Altri risultati» → call the driver's `more()`.
//!
//! [`set_term`]: SearchList::set_term
//! [`set_state`]: SearchList::set_state
//! [`key`]: SearchList::key
//! [`chosen`]: SearchList::chosen
use matrix_engine::search::{MIN_CHARS, More, SearchError, SearchState};

/// One result as the list shows and announces it.
pub trait Hit {
    /// Identity: dedupes the lead row and survives paging.
    fn key(&self) -> &str;
    fn title(&self) -> &str;
    /// Secondary line (alias, members, user ID).
    fn detail(&self) -> String;
    fn avatar(&self) -> Option<&str>;
    /// What a screen reader says.
    fn label(&self) -> String {
        format!("{}, {}", self.title(), self.detail())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Row<'a, H> {
    Hit(&'a H),
    /// The last row, «Altri risultati».
    More(More),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Choice<'a, H> {
    Hit(&'a H),
    More,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Searching,
    NoResults,
    Error(SearchError),
    /// Message search of the open room: failed, no cause to tell.
    Failed,
    Partial,
}

impl Status {
    /// One line, no help text. Also the announcement made when the state changes.
    pub fn text(self) -> &'static str {
        match self {
            Self::Searching => crate::i18n::t("Searching"),
            Self::NoResults => crate::i18n::t("No results"),
            Self::Failed => crate::i18n::t("Search failed"),
            Self::Partial => crate::i18n::t("Partial results"),
            Self::Error(e) => match e {
                SearchError::InvalidServer => crate::i18n::t("Invalid server name"),
                SearchError::NotMatrix => crate::i18n::t("Not a Matrix server"),
                SearchError::Refused => crate::i18n::t("The server does not show its rooms"),
                SearchError::Unreachable => crate::i18n::t("The server does not respond"),
                SearchError::OurServerBlocked => crate::i18n::t("Your server cannot reach this server"),
                SearchError::Tls => crate::i18n::t("Invalid server certificate"),
                SearchError::Timeout => crate::i18n::t("Timed out"),
                SearchError::Network => crate::i18n::t("Connection failed"),
            },
        }
    }
}

/// Text of the «Altri risultati» row.
pub fn more_text(more: More) -> &'static str {
    match more {
        More::None | More::Available => crate::i18n::t("More results"),
        More::Loading => crate::i18n::t("Loading"),
        More::Failed(_) => crate::i18n::t("More results not loaded"),
    }
}

pub struct SearchList<H> {
    term: String,
    state: SearchState<H>,
    lead: Option<H>,
    active: usize,
}

impl<H> Default for SearchList<H> {
    fn default() -> Self {
        Self {
            term: String::new(),
            state: SearchState::Idle,
            lead: None,
            active: 0,
        }
    }
}

impl<H: Hit> SearchList<H> {
    /// The field changed. Old results no longer match: Idle below `MIN_CHARS`, else Searching.
    pub fn set_term(&mut self, term: &str) {
        self.term = term.trim().to_owned();
        self.state = if self.term.chars().count() < MIN_CHARS {
            SearchState::Idle
        } else {
            SearchState::Searching {
                term: self.term.clone(),
            }
        };
        self.active = 0;
    }

    /// An empty field that lists the provider's first page: Searching on the empty term.
    pub fn set_suggesting(&mut self) {
        self.term.clear();
        self.state = SearchState::Searching { term: String::new() };
        self.active = 0;
    }

    /// An engine answer. Dropped (false) when it answers a term the field no longer holds.
    /// The active row stays while the rows only grow (next page), else it restarts from the first.
    pub fn set_state(&mut self, state: SearchState<H>) -> bool {
        if state.term().is_some_and(|t| t != self.term) {
            return false;
        }
        self.change(|s| s.state = state);
        true
    }

    /// A fixed first row that did not come from the server (e.g. «Avvia con @id»), deduped by key:
    /// when the server returned it too, that hit goes first instead.
    pub fn set_lead(&mut self, lead: Option<H>) {
        self.change(|s| s.lead = lead);
    }

    fn change(&mut self, f: impl FnOnce(&mut Self)) {
        let before = self.keys();
        f(self);
        let after = self.keys();
        if after.len() < before.len() || after[..before.len()] != before[..] {
            self.active = 0;
        }
        self.active = self.active.min(self.len().saturating_sub(1));
    }

    fn keys(&self) -> Vec<String> {
        self.hits().iter().map(|h| h.key().to_owned()).collect()
    }

    fn hits(&self) -> Vec<&H> {
        let SearchState::Results { hits, .. } = &self.state else {
            return Vec::new();
        };
        let mut out: Vec<&H> = hits.iter().collect();
        if let Some(lead) = &self.lead {
            match out.iter().position(|h| h.key() == lead.key()) {
                Some(at) => out[..=at].rotate_right(1),
                None => out.insert(0, lead),
            }
        }
        out
    }

    fn more(&self) -> More {
        match &self.state {
            SearchState::Results { more, .. } => *more,
            _ => More::None,
        }
    }

    pub fn rows(&self) -> Vec<Row<'_, H>> {
        let mut rows: Vec<_> = self.hits().into_iter().map(Row::Hit).collect();
        if self.more() != More::None {
            rows.push(Row::More(self.more()));
        }
        rows
    }

    pub fn len(&self) -> usize {
        self.rows().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn status(&self) -> Option<Status> {
        match &self.state {
            SearchState::Searching { .. } => Some(Status::Searching),
            SearchState::NoResults { .. } => Some(Status::NoResults),
            SearchState::Error { error, .. } => Some(Status::Error(*error)),
            _ => None,
        }
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.len().saturating_sub(1));
    }

    /// ↑/↓ wrap around, «Altri risultati» included. Returns true when the key was ours.
    pub fn key(&mut self, key: &str) -> bool {
        let len = self.len();
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

    /// What Enter or a click on the active row picks.
    pub fn chosen(&self) -> Option<Choice<'_, H>> {
        match self.rows().into_iter().nth(self.active)? {
            Row::Hit(h) => Some(Choice::Hit(h)),
            Row::More(_) => Some(Choice::More),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct R(&'static str);
    impl Hit for R {
        fn key(&self) -> &str {
            self.0
        }
        fn title(&self) -> &str {
            self.0
        }
        fn detail(&self) -> String {
            "3 membri".into()
        }
        fn avatar(&self) -> Option<&str> {
            None
        }
    }

    fn results(term: &str, hits: &[&'static str], more: More) -> SearchState<R> {
        SearchState::Results {
            term: term.into(),
            hits: hits.iter().map(|h| R(h)).collect(),
            more,
        }
    }

    fn list(hits: &[&'static str], more: More) -> SearchList<R> {
        let mut l = SearchList::default();
        l.set_term("ma");
        assert!(l.set_state(results("ma", hits, more)));
        l
    }

    #[test]
    fn short_term_is_idle_longer_is_searching_without_a_server() {
        let mut l = SearchList::<R>::default();
        l.set_term(" a ");
        assert_eq!(l.status(), None);
        l.set_term("ma");
        assert_eq!(l.status(), Some(Status::Searching));
    }

    #[test]
    fn an_answer_for_another_term_is_dropped() {
        let mut l = list(&["a"], More::None);
        l.set_term("mar");
        assert!(!l.set_state(results("ma", &["old"], More::None)));
        assert!(l.is_empty());
        assert_eq!(l.status(), Some(Status::Searching));
    }

    #[test]
    fn arrows_wrap_and_enter_picks_the_active_hit() {
        let mut l = list(&["a", "b"], More::None);
        assert_eq!(l.chosen(), Some(Choice::Hit(&R("a"))));
        assert!(l.key("down") && l.key("down"));
        assert_eq!(l.chosen(), Some(Choice::Hit(&R("a"))));
        assert!(l.key("up"));
        assert_eq!(l.chosen(), Some(Choice::Hit(&R("b"))));
        assert!(!l.key("left"));
        assert!(!SearchList::<R>::default().key("down"));
    }

    #[test]
    fn more_row_is_last_reachable_and_chosen_as_more() {
        let mut l = list(&["a"], More::Available);
        assert_eq!(l.rows(), [Row::Hit(&R("a")), Row::More(More::Available)]);
        l.key("up"); // wraps onto the last row
        assert_eq!(l.chosen(), Some(Choice::More));
    }

    #[test]
    fn next_page_keeps_the_active_row_a_new_term_result_restarts() {
        let mut l = list(&["a", "b"], More::Available);
        l.set_active(1);
        l.set_state(results("ma", &["a", "b", "c"], More::None));
        assert_eq!(l.active(), 1);
        assert_eq!(l.len(), 3);
        l.set_state(results("ma", &["z"], More::None));
        assert_eq!(l.active(), 0);
    }

    #[test]
    fn lead_goes_first_and_the_servers_copy_replaces_it() {
        let mut l = list(&["a", "b"], More::None);
        l.set_lead(Some(R("x")));
        assert_eq!(l.rows()[0], Row::Hit(&R("x")));
        assert_eq!(l.len(), 3);
        l.set_lead(Some(R("b")));
        assert_eq!(l.rows(), [Row::Hit(&R("b")), Row::Hit(&R("a"))]);
    }

    #[test]
    fn one_line_statuses() {
        let mut l = SearchList::<R>::default();
        l.set_term("ma");
        l.set_state(SearchState::NoResults { term: "ma".into() });
        assert_eq!(l.status().unwrap().text(), "No results");
        l.set_state(SearchState::Error {
            term: "ma".into(),
            error: SearchError::Timeout,
        });
        assert_eq!(l.status().unwrap().text(), "Timed out");
    }
}
