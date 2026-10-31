//! «Nuova chat» user search over `Client::search_users` only. Debounced, superseded requests are
//! aborted and can never publish. The search term is private: nothing here logs it, and the
//! error state carries no text.
use matrix_sdk::{Client, ruma::UserId};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};

#[cfg(test)]
#[path = "user_search_tests.rs"]
mod tests;

pub const DEBOUNCE: Duration = Duration::from_millis(275);
pub const MIN_CHARS: usize = 2;
const LIMIT: u64 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserHit {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    SameInstance,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchState {
    Idle,
    Searching,
    /// `direct` is the «Avvia con @nome:server» entry: set when the term is a valid user ID,
    /// even if the directory did not return it (then `hits` may be empty).
    Results {
        hits: Vec<UserHit>,
        direct: Option<String>,
    },
    NoResults,
    /// One line in the UI, no detail.
    Error,
}

type Fetch =
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Vec<UserHit>, ()>> + Send>> + Send + Sync;

pub struct UserSearch {
    own_server: String,
    fetch: Arc<Fetch>,
    all_available: bool,
    term: String,
    filter: Filter,
    /// Guards publication: a task writes only while its generation is the current one.
    generation: Arc<Mutex<u64>>,
    tx: Arc<watch::Sender<SearchState>>,
    task: Option<JoinHandle<()>>,
}

impl UserSearch {
    /// Search as the client's active account. `Filter::All` is not offered: the user directory
    /// never leaves the own homeserver (the user-directory-search notes).
    pub fn for_client(client: Client) -> Option<Self> {
        let own_server = client.user_id()?.server_name().to_string();
        let fetch = move |term: String| {
            let client = client.clone();
            Box::pin(async move {
                let found = client.search_users(&term, LIMIT).await.map_err(|_| ())?;
                Ok(found
                    .results
                    .into_iter()
                    .map(|u| UserHit {
                        user_id: u.user_id.to_string(),
                        display_name: u.display_name,
                        avatar_url: u.avatar_url.map(|m| m.to_string()),
                    })
                    .collect())
            }) as Pin<Box<dyn Future<Output = _> + Send>>
        };
        Some(Self::new(own_server, false, fetch))
    }

    pub fn new<F, Fut>(own_server: String, all_available: bool, fetch: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<UserHit>, ()>> + Send + 'static,
    {
        Self {
            own_server,
            fetch: Arc::new(move |t| Box::pin(fetch(t))),
            all_available,
            term: String::new(),
            filter: Filter::default(),
            generation: Arc::default(),
            tx: Arc::new(watch::channel(SearchState::Idle).0),
            task: None,
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
        self.restart();
    }

    /// Ignored when `All` is not available.
    pub fn set_filter(&mut self, filter: Filter) {
        if filter == Filter::All && !self.all_available {
            return;
        }
        self.filter = filter;
        self.restart();
    }

    fn restart(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        let gen_now = {
            let mut g = self.generation.lock().unwrap();
            *g += 1;
            let state = if self.term.chars().count() < MIN_CHARS {
                SearchState::Idle
            } else {
                SearchState::Searching
            };
            self.tx.send_replace(state);
            *g
        };
        if self.term.chars().count() < MIN_CHARS {
            return;
        }
        let (term, filter, own) = (self.term.clone(), self.filter, self.own_server.clone());
        let (fetch, generation, tx) =
            (self.fetch.clone(), self.generation.clone(), self.tx.clone());
        self.task = Some(tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            let direct = UserId::parse(&term).ok().map(|u| u.to_string());
            let found = fetch(term).await;
            let state = outcome(found, direct, filter, &own);
            let g = generation.lock().unwrap();
            if *g == gen_now {
                tx.send_replace(state);
            }
        }));
    }
}

impl Drop for UserSearch {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

fn outcome(
    found: Result<Vec<UserHit>, ()>,
    direct: Option<String>,
    filter: Filter,
    own: &str,
) -> SearchState {
    let mut hits = match found {
        Ok(h) => h,
        // A typed ID can still be started even if the directory failed.
        Err(()) if direct.is_none() => return SearchState::Error,
        Err(()) => Vec::new(),
    };
    if filter == Filter::SameInstance {
        hits.retain(|h| UserId::parse(&h.user_id).is_ok_and(|u| u.server_name().as_str() == own));
    }
    if hits.is_empty() && direct.is_none() {
        SearchState::NoResults
    } else {
        SearchState::Results { hits, direct }
    }
}
