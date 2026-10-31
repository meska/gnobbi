//! Shared search contract (project the SEARCH_CONTRACT notes): one debounced driver and one
//! result list for «Nuova chat» (user directory) and «Unisciti» (public room directory).
//! SDK-free so the desktop can show these types; the driver itself comes behind `sdk`.
//! Privacy: never log a term, nor format (`{:?}`) a value that carries one.
use std::time::Duration;

/// Wait after the last change of the term before a request starts.
pub const DEBOUNCE: Duration = Duration::from_millis(275);
/// Terms shorter than this (chars, after trim) never reach the server.
pub const MIN_CHARS: usize = 2;
/// Results asked per request; the page size of the room directory.
pub const PAGE: u32 = 20;
/// Every request (first page or «Altri risultati») gives up after this.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// One answer of a provider. `next_batch` is the opaque `since` of the next page; `None` = last page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<H> {
    pub hits: Vec<H>,
    pub next_batch: Option<String>,
}

/// Why a request failed; the UI shows one fixed line per kind and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchError {
    /// Not a Matrix server name; caught before any request.
    InvalidServer,
    /// The remote server refused to show its directory to us (401/403/404 relayed by our server).
    Refused,
    /// Our server could not reach the remote one (5xx relayed by our server).
    Unreachable,
    /// No answer within `TIMEOUT`.
    Timeout,
    /// Our own homeserver is not reachable.
    Network,
}

/// The «Altri risultati» row under the hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum More {
    /// Last page: no row.
    None,
    Available,
    Loading,
    /// The hits stay; choosing the row asks the same page again.
    Failed(SearchError),
}

/// What the driver publishes. Every non-idle state names its term, so the UI drops an answer
/// for a term the field no longer holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchState<H> {
    Idle,
    Searching {
        term: String,
    },
    Results {
        term: String,
        hits: Vec<H>,
        more: More,
    },
    NoResults {
        term: String,
    },
    Error {
        term: String,
        error: SearchError,
    },
}

impl<H> SearchState<H> {
    pub fn term(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Searching { term }
            | Self::Results { term, .. }
            | Self::NoResults { term }
            | Self::Error { term, .. } => Some(term),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_but_idle_names_its_term() {
        assert_eq!(SearchState::<()>::Idle.term(), None);
        let s = SearchState::<()>::Results {
            term: "ma".into(),
            hits: vec![],
            more: More::None,
        };
        assert_eq!(s.term(), Some("ma"));
        let e = SearchState::<()>::Error {
            term: "mo".into(),
            error: SearchError::Timeout,
        };
        assert_eq!(e.term(), Some("mo"));
    }
}
