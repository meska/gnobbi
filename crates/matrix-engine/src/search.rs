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

#[cfg(feature = "sdk")]
pub use driver::{Fetch, Search};

#[cfg(feature = "sdk")]
mod driver {
    use super::*;
    use std::{
        future::Future,
        pin::Pin,
        sync::{Arc, Mutex},
    };
    use tokio::{sync::watch, task::JoinHandle};

    /// The injected provider: `(term, scope, since)` -> one page. `since` is `None` for the first page.
    pub type Fetch<S, H> = dyn Fn(
            String,
            S,
            Option<String>,
        ) -> Pin<Box<dyn Future<Output = Result<Page<H>, SearchError>> + Send>>
        + Send
        + Sync;

    struct Shared<H> {
        /// A task publishes only while its generation is the current one.
        generation: u64,
        hits: Vec<H>,
        next: Option<String>,
    }

    /// Debounced, cancellable, paginated search over any provider. Superseded requests are aborted
    /// and cannot publish; the term is never logged.
    pub struct Search<S, H> {
        fetch: Arc<Fetch<S, H>>,
        scope: S,
        term: String,
        shared: Arc<Mutex<Shared<H>>>,
        tx: Arc<watch::Sender<SearchState<H>>>,
        task: Option<JoinHandle<()>>,
    }

    impl<S: Clone + Send + 'static, H: Clone + Send + Sync + 'static> Search<S, H> {
        pub fn new<F, Fut>(scope: S, fetch: F) -> Self
        where
            F: Fn(String, S, Option<String>) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Page<H>, SearchError>> + Send + 'static,
        {
            Self {
                fetch: Arc::new(move |t, s, since| Box::pin(fetch(t, s, since))),
                scope,
                term: String::new(),
                shared: Arc::new(Mutex::new(Shared {
                    generation: 0,
                    hits: Vec::new(),
                    next: None,
                })),
                tx: Arc::new(watch::channel(SearchState::Idle).0),
                task: None,
            }
        }

        pub fn subscribe(&self) -> watch::Receiver<SearchState<H>> {
            self.tx.subscribe()
        }

        /// New term and/or scope: the running request is dropped, a new one starts after `DEBOUNCE`.
        /// Below `MIN_CHARS` (after trim) nothing is asked and the state is `Idle`.
        pub fn set_query(&mut self, term: &str, scope: S) {
            self.term = term.trim().to_owned();
            self.scope = scope;
            self.abort();
            let generation = self.bump(|s| {
                s.hits.clear();
                s.next = None;
            });
            if self.term.chars().count() < MIN_CHARS {
                self.tx.send_replace(SearchState::Idle);
                return;
            }
            self.tx.send_replace(SearchState::Searching {
                term: self.term.clone(),
            });
            self.spawn(generation, true);
        }

        /// «Altri risultati»: the next page, without debounce. No-op unless a next page is known
        /// and none is loading.
        pub fn more(&mut self) {
            if self.task.as_ref().is_some_and(|t| !t.is_finished()) {
                return;
            }
            let (generation, hits) = {
                let mut s = self.shared.lock().unwrap();
                if s.next.is_none() {
                    return;
                }
                s.generation += 1;
                (s.generation, s.hits.clone())
            };
            self.tx.send_replace(SearchState::Results {
                term: self.term.clone(),
                hits,
                more: More::Loading,
            });
            self.spawn(generation, false);
        }

        fn abort(&mut self) {
            if let Some(t) = self.task.take() {
                t.abort();
            }
        }

        fn bump(&self, f: impl FnOnce(&mut Shared<H>)) -> u64 {
            let mut s = self.shared.lock().unwrap();
            s.generation += 1;
            f(&mut s);
            s.generation
        }

        fn spawn(&mut self, generation: u64, first: bool) {
            let (term, scope) = (self.term.clone(), self.scope.clone());
            let (fetch, shared, tx) = (self.fetch.clone(), self.shared.clone(), self.tx.clone());
            let since = if first {
                None
            } else {
                shared.lock().unwrap().next.clone()
            };
            self.task = Some(tokio::spawn(async move {
                if first {
                    tokio::time::sleep(DEBOUNCE).await;
                }
                let answer = tokio::time::timeout(TIMEOUT, fetch(term.clone(), scope, since))
                    .await
                    .unwrap_or(Err(SearchError::Timeout));
                let mut s = shared.lock().unwrap();
                if s.generation != generation {
                    return;
                }
                let state = match answer {
                    Ok(page) => {
                        s.hits.extend(page.hits);
                        s.next = page.next_batch;
                        if s.hits.is_empty() && s.next.is_none() {
                            SearchState::NoResults { term }
                        } else {
                            SearchState::Results {
                                term,
                                hits: s.hits.clone(),
                                more: if s.next.is_some() {
                                    More::Available
                                } else {
                                    More::None
                                },
                            }
                        }
                    }
                    Err(error) if first => SearchState::Error { term, error },
                    Err(error) => SearchState::Results {
                        term,
                        hits: s.hits.clone(),
                        more: More::Failed(error),
                    },
                };
                tx.send_replace(state);
            }));
        }
    }

    impl<S, H> Drop for Search<S, H> {
        fn drop(&mut self) {
            if let Some(t) = self.task.take() {
                t.abort();
            }
        }
    }
}

#[cfg(all(test, feature = "sdk"))]
#[path = "search_tests.rs"]
mod driver_tests;

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
