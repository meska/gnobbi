use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn hit(id: &str) -> UserHit {
    UserHit {
        user_id: id.into(),
        display_name: None,
        avatar_url: None,
    }
}

/// Returns `hits` for any term, counting calls; `delay` simulates a slow server.
fn search(
    hits: Vec<UserHit>,
    delay: Duration,
    all: bool,
) -> (UserSearch, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let terms = Arc::new(Mutex::new(Vec::new()));
    let (c, t) = (calls.clone(), terms.clone());
    let s = UserSearch::new("a.test".into(), all, move |term| {
        c.fetch_add(1, Ordering::SeqCst);
        t.lock().unwrap().push(term);
        let hits = hits.clone();
        async move {
            tokio::time::sleep(delay).await;
            Ok(hits)
        }
    });
    (s, calls, terms)
}

async fn ms(n: u64) {
    tokio::time::sleep(Duration::from_millis(n)).await;
}

#[tokio::test(start_paused = true)]
async fn below_two_chars_never_calls() {
    let (mut s, calls, _) = search(vec![], Duration::ZERO, false);
    s.set_term(" a ");
    ms(1000).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(*s.subscribe().borrow(), SearchState::Idle);
}

#[tokio::test(start_paused = true)]
async fn debounce_collapses_typing_into_one_call() {
    let (mut s, calls, terms) = search(vec![hit("@bo:a.test")], Duration::ZERO, false);
    let rx = s.subscribe();
    for t in ["bo", "bob", "bobb"] {
        s.set_term(t);
        ms(100).await;
    }
    assert_eq!(*rx.borrow(), SearchState::Searching);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    ms(300).await;
    assert_eq!(*terms.lock().unwrap(), ["bobb"]);
    assert!(matches!(&*rx.borrow(), SearchState::Results { .. }));
}

#[tokio::test(start_paused = true)]
async fn superseded_request_never_publishes() {
    let (mut s, calls, _) = search(vec![hit("@old:a.test")], Duration::from_millis(500), false);
    let rx = s.subscribe();
    s.set_term("old");
    ms(400).await; // debounce done, fetch in flight
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    s.set_term("x"); // too short: back to idle, the in-flight one is dropped
    ms(2000).await;
    assert_eq!(*rx.borrow(), SearchState::Idle);
}

#[tokio::test(start_paused = true)]
async fn same_instance_filters_other_servers() {
    let hits = vec![hit("@bob:a.test"), hit("@eve:b.test")];
    let (mut s, _, _) = search(hits, Duration::ZERO, true);
    let rx = s.subscribe();
    s.set_term("bo");
    ms(300).await;
    assert_eq!(
        *rx.borrow(),
        SearchState::Results {
            hits: vec![hit("@bob:a.test")],
            direct: None
        }
    );
    s.set_filter(Filter::All);
    ms(300).await;
    assert!(matches!(&*rx.borrow(), SearchState::Results { hits, .. } if hits.len() == 2));
}

#[tokio::test(start_paused = true)]
async fn all_filter_ignored_when_unavailable() {
    let (mut s, _, _) = search(vec![hit("@eve:b.test")], Duration::ZERO, false);
    assert!(!s.all_available());
    s.set_filter(Filter::All);
    s.set_term("ev");
    ms(300).await;
    assert_eq!(*s.subscribe().borrow(), SearchState::NoResults);
}

#[tokio::test(start_paused = true)]
async fn full_id_offers_direct_entry_even_if_absent() {
    let (mut s, _, _) = search(vec![], Duration::ZERO, false);
    let rx = s.subscribe();
    s.set_term("@zed:far.test");
    ms(300).await;
    assert_eq!(
        *rx.borrow(),
        SearchState::Results {
            hits: vec![],
            direct: Some("@zed:far.test".into())
        }
    );
}

#[tokio::test(start_paused = true)]
async fn failure_is_error_state() {
    let mut s = UserSearch::new("a.test".into(), false, |_| async { Err(()) });
    let rx = s.subscribe();
    s.set_term("bob");
    ms(300).await;
    assert_eq!(*rx.borrow(), SearchState::Error);
}
