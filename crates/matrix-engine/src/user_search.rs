//! «Nuova chat» user search over `Client::search_users` only: a `search::Search` driver (debounce,
//! abort of superseded requests) plus the user-specific part: same-instance filter and the typed ID. The search term is private: nothing here logs it, and the
//! error state carries no text.
use crate::search::{Page, Search, SearchError, SearchState as Driver};
use matrix_sdk::{Client, ruma::UserId};
use std::future::Future;
use tokio::sync::watch;

#[cfg(test)]
#[path = "user_search_tests.rs"]
mod tests;

pub use crate::operations::{Filter, SearchState, UserHit};
pub use crate::search::{DEBOUNCE, MIN_CHARS};

pub struct UserSearch {
    all_available: bool,
    term: String,
    filter: Filter,
    search: Search<Filter, UserHit>,
    tx: watch::Sender<SearchState>,
}

impl UserSearch {
    /// Search as the client's active account. `Filter::All` is not offered: the user directory
    /// never leaves the own homeserver (the user-directory-search notes).
    pub fn for_client(client: Client) -> Option<Self> {
        let own_server = client.user_id()?.server_name().to_string();
        let fetch = move |term: String| {
            let client = client.clone();
            async move {
                let found = client
                    .search_users(&term, crate::search::PAGE.into())
                    .await
                    .map_err(|_| ())?;
                Ok(found
                    .results
                    .into_iter()
                    .map(|u| UserHit {
                        user_id: u.user_id.to_string(),
                        display_name: u.display_name,
                        avatar_url: u.avatar_url.map(|m| m.to_string()),
                    })
                    .collect())
            }
        };
        Some(Self::new(own_server, false, fetch))
    }

    pub fn new<F, Fut>(own_server: String, all_available: bool, fetch: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<UserHit>, ()>> + Send + 'static,
    {
        let own = own_server.clone();
        let search = Search::new(Filter::default(), move |term, filter, _since| {
            let (found, own) = (fetch(term), own.clone());
            async move {
                let mut hits = found.await.map_err(|()| SearchError::Network)?;
                if filter == Filter::SameInstance {
                    hits.retain(|h| {
                        UserId::parse(&h.user_id).is_ok_and(|u| u.server_name().as_str() == own)
                    });
                }
                Ok(Page {
                    hits,
                    next_batch: None,
                })
            }
        });
        let (tx, _) = watch::channel(SearchState::Idle);
        // Mirror the driver in the legacy shape the dialog already consumes; ends when the driver drops.
        let mut rx = search.subscribe();
        let out = tx.clone();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let state = legacy(rx.borrow_and_update().clone());
                out.send_replace(state);
            }
        });
        Self {
            all_available,
            term: String::new(),
            filter: Filter::default(),
            search,
            tx,
        }
    }

    /// The UI hides or disables the «Tutti» switch when this is false.
    pub fn all_available(&self) -> bool {
        self.all_available
    }

    pub fn subscribe(&self) -> watch::Receiver<SearchState> {
        self.tx.subscribe()
    }

    pub fn set_term(&mut self, term: &str) {
        self.term = term.trim().to_owned();
        self.search.set_query(&self.term, self.filter);
    }

    /// Ignored when `All` is not available.
    pub fn set_filter(&mut self, filter: Filter) {
        if filter == Filter::All && !self.all_available {
            return;
        }
        self.filter = filter;
        self.search.set_query(&self.term, self.filter);
    }
}

/// A full user ID typed in the field stays startable even if the directory failed or omitted it.
fn legacy(state: Driver<UserHit>) -> SearchState {
    let direct = state
        .term()
        .and_then(|t| UserId::parse(t).ok())
        .map(|u| u.to_string());
    match state {
        Driver::Idle => SearchState::Idle,
        Driver::Searching { .. } => SearchState::Searching,
        Driver::Results { hits, .. } => SearchState::Results { hits, direct },
        Driver::NoResults { .. } | Driver::Error { .. } if direct.is_some() => {
            SearchState::Results {
                hits: Vec::new(),
                direct,
            }
        }
        Driver::NoResults { .. } => SearchState::NoResults,
        Driver::Error { .. } => SearchState::Error,
    }
}
