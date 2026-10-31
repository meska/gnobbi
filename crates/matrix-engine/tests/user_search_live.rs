//! «Nuova chat» user search against a disposable Synapse or Tuwunel seeded by
//! scripts/userdir-live (alice+bob share a room, carol shares none). Run with
//! `scripts/userdir-live/run.sh live`, which sets USERDIR_URL and USERDIR_SERVER.
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    operations::{OperationError, SearchState, start_dm},
    sdk::Client,
    user_search::{DEBOUNCE, Filter, UserHit, UserSearch},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const PASSWORD: &str = "probe-userdir-pass";

async fn login(url: &str, user: &str) -> Client {
    let client = Client::builder().homeserver_url(url).build().await.unwrap();
    client.matrix_auth().login_username(user, PASSWORD).send().await.unwrap();
    client
}

/// Same fetch as `UserSearch::for_client`, but counting the HTTP calls.
fn counted(client: &Client, server: &str) -> (UserSearch, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let (c, n) = (client.clone(), calls.clone());
    let search = UserSearch::new(server.into(), false, move |term: String| {
        let (c, n) = (c.clone(), n.clone());
        async move {
            n.fetch_add(1, Ordering::SeqCst);
            let r = c.search_users(&term, 20).await.map_err(|_| ())?;
            Ok(r.results
                .into_iter()
                .map(|u| UserHit {
                    user_id: u.user_id.to_string(),
                    display_name: u.display_name,
                    avatar_url: None,
                })
                .collect())
        }
    });
    (search, calls)
}

/// Types `term` and waits for the settled state.
async fn settled(search: &mut UserSearch, term: &str) -> SearchState {
    let mut rx = search.subscribe();
    search.set_term(term);
    tokio::time::sleep(DEBOUNCE).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let s = rx.borrow_and_update().clone();
            if s != SearchState::Searching {
                return s;
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("search never settled")
}

fn ids(s: &SearchState) -> Vec<String> {
    match s {
        SearchState::Results { hits, .. } => hits.iter().map(|h| h.user_id.clone()).collect(),
        _ => vec![],
    }
}

#[test]
#[ignore = "requires scripts/userdir-live/run.sh live"]
fn new_chat_search_on_a_live_homeserver() {
    let url = std::env::var("USERDIR_URL").expect("USERDIR_URL required");
    let server = std::env::var("USERDIR_SERVER").expect("USERDIR_SERVER required");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (alice, bob) = (format!("@alice:{server}"), format!("@bob:{server}"));
        let bob_client = login(&url, "bob").await;
        let (mut s, calls) = counted(&bob_client, &server);

        // By localpart, by display-name part, by 2 characters, and the shared-room user only.
        for term in ["alice", "ross", "Rossini", "al"] {
            let st = settled(&mut s, term).await;
            println!("[{server}] bob «{term}» -> {st:?}");
            assert!(ids(&st).contains(&alice), "{term}: {st:?}");
            assert!(!ids(&st).contains(&bob), "requester must not appear");
        }
        // Not sharing a room: invisible with the default directory settings.
        let st = settled(&mut s, "carol").await;
        println!("[{server}] bob «carol» -> {st:?}");
        assert_eq!(st, SearchState::NoResults);

        // Same-instance filter keeps the own-server user; «Tutti» is refused for a client.
        s.set_filter(Filter::All);
        let st = settled(&mut s, "alice").await;
        assert!(ids(&st).contains(&alice));
        assert!(!s.all_available());

        // Full pasted ID: the directory finds it and the «Avvia con» entry is set.
        let st = settled(&mut s, &alice).await;
        println!("[{server}] bob «{alice}» -> {st:?}");
        match &st {
            SearchState::Results { direct, .. } => assert_eq!(direct.as_deref(), Some(alice.as_str())),
            _ => panic!("{st:?}"),
        }
        assert!(ids(&st).contains(&alice));

        // Unknown user: offered as direct entry, starting the chat fails (current behaviour).
        let ghost = format!("@ghost-nobody:{server}");
        let st = settled(&mut s, &ghost).await;
        println!("[{server}] bob «{ghost}» -> {st:?}");
        assert_eq!(st, SearchState::Results { hits: vec![], direct: Some(ghost.clone()) });
        let err = start_dm(&bob_client, AccountId::new(1), &ghost).await;
        println!("[{server}] start_dm(ghost) -> {err:?}");
        assert!(matches!(err, Err(OperationError::Unavailable)));

        // Threshold: 0 and 1 characters never hit the network.
        let before = calls.load(Ordering::SeqCst);
        assert_eq!(settled(&mut s, "a").await, SearchState::Idle);
        assert_eq!(settled(&mut s, " ").await, SearchState::Idle);
        tokio::time::sleep(DEBOUNCE * 2).await;
        assert_eq!(calls.load(Ordering::SeqCst), before, "no call under 2 characters");

        // Fast typing: only the last term is fetched.
        let before = calls.load(Ordering::SeqCst);
        let mut rx = s.subscribe();
        for t in ["al", "ali", "alic", "alice"] {
            s.set_term(t);
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        tokio::time::sleep(DEBOUNCE + Duration::from_millis(100)).await;
        let _ = rx.changed().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(calls.load(Ordering::SeqCst), before + 1, "debounce + cancellation");
        assert!(ids(&rx.borrow()).contains(&alice));

        // Carol shares no room: she sees nobody, not even by exact name.
        let carol_client = login(&url, "carol").await;
        let (mut c, _) = counted(&carol_client, &server);
        for term in ["alice", "bob", "Rossini"] {
            let st = settled(&mut c, term).await;
            println!("[{server}] carol «{term}» -> {st:?}");
            assert_eq!(st, SearchState::NoResults);
        }
    });
}
