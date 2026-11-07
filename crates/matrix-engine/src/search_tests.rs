use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// Pages of 2 hits; `pages` = how many exist. `delay` simulates a slow server.
fn search(pages: usize, delay: Duration) -> (Search<(), String>, Arc<AtomicUsize>, Log) {
    let calls = Arc::new(AtomicUsize::new(0));
    let log = Log::default();
    let (c, l) = (calls.clone(), log.clone());
    let s = Search::new((), move |term, (), since: Option<String>| {
        c.fetch_add(1, Ordering::SeqCst);
        l.lock().unwrap().push((term.clone(), since.clone()));
        async move {
            tokio::time::sleep(delay).await;
            let n: usize = since.map_or(0, |s| s.parse().unwrap());
            Ok(Page {
                hits: vec![format!("{term}{n}a"), format!("{term}{n}b")],
                next_batch: (n + 1 < pages).then(|| (n + 1).to_string()),
            })
        }
    });
    (s, calls, log)
}

async fn ms(n: u64) {
    tokio::time::sleep(Duration::from_millis(n)).await;
}

#[tokio::test(start_paused = true)]
async fn below_two_chars_never_calls() {
    let (mut s, calls, _) = search(1, Duration::ZERO);
    s.set_query(" a ", ());
    ms(1000).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(*s.subscribe().borrow(), SearchState::Idle);
}

#[tokio::test(start_paused = true)]
async fn nothing_is_asked_until_the_term_is_set() {
    let (s, calls, _) = search(1, Duration::ZERO);
    ms(1000).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(*s.subscribe().borrow(), SearchState::Idle);
}

#[tokio::test(start_paused = true)]
async fn debounce_collapses_typing_into_one_call() {
    let (mut s, calls, log) = search(1, Duration::ZERO);
    let rx = s.subscribe();
    for t in ["bo", "bob", "bobb"] {
        s.set_query(t, ());
        ms(100).await;
    }
    assert_eq!(
        *rx.borrow(),
        SearchState::Searching {
            term: "bobb".into()
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    ms(300).await;
    assert_eq!(*log.lock().unwrap(), [("bobb".to_owned(), None)]);
    assert!(matches!(&*rx.borrow(), SearchState::Results { term, .. } if term == "bobb"));
}

#[tokio::test(start_paused = true)]
async fn superseded_request_never_publishes() {
    let (mut s, calls, _) = search(1, Duration::from_millis(500));
    let rx = s.subscribe();
    s.set_query("old", ());
    ms(400).await; // debounce done, fetch in flight
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    s.set_query("x", ()); // too short: idle, the in-flight one is aborted
    ms(2000).await;
    assert_eq!(*rx.borrow(), SearchState::Idle);
    s.set_query("new", ());
    ms(400).await;
    s.set_query("newer", ());
    ms(2000).await;
    assert!(matches!(&*rx.borrow(), SearchState::Results { term, .. } if term == "newer"));
}

#[tokio::test(start_paused = true)]
async fn more_appends_the_next_page_until_the_last() {
    let (mut s, _, log) = search(2, Duration::from_millis(50));
    let rx = s.subscribe();
    s.set_query("ma", ());
    ms(400).await;
    assert!(
        matches!(&*rx.borrow(), SearchState::Results { hits, more: More::Available, .. } if hits.len() == 2)
    );
    s.more();
    assert!(matches!(
        &*rx.borrow(),
        SearchState::Results {
            more: More::Loading,
            ..
        }
    ));
    s.more(); // already loading: ignored
    ms(100).await;
    assert!(
        matches!(&*rx.borrow(), SearchState::Results { hits, more: More::None, .. } if hits.len() == 4)
    );
    s.more(); // last page: ignored
    ms(100).await;
    assert_eq!(log.lock().unwrap().len(), 2);
    assert_eq!(log.lock().unwrap()[1].1.as_deref(), Some("1"));
}

#[tokio::test(start_paused = true)]
async fn a_page_arriving_after_a_new_term_is_dropped() {
    let (mut s, _, _) = search(2, Duration::from_millis(300));
    let rx = s.subscribe();
    s.set_query("ma", ());
    ms(700).await;
    s.more();
    ms(100).await;
    s.set_query("mo", ());
    ms(2000).await;
    assert!(
        matches!(&*rx.borrow(), SearchState::Results { term, hits, .. } if term == "mo" && hits.len() == 2)
    );
}

#[tokio::test(start_paused = true)]
async fn errors_and_timeout_are_states_and_a_failed_page_keeps_the_hits() {
    let mut s = Search::<(), String>::new((), |_, (), _| async { Err(SearchError::Refused) });
    let rx = s.subscribe();
    s.set_query("bob", ());
    ms(400).await;
    assert_eq!(
        *rx.borrow(),
        SearchState::Error {
            term: "bob".into(),
            error: SearchError::Refused
        }
    );

    let mut s = Search::<(), String>::new((), |_, (), _| async { std::future::pending().await });
    let rx = s.subscribe();
    s.set_query("bob", ());
    ms(DEBOUNCE.as_millis() as u64 + TIMEOUT.as_millis() as u64 + 10).await;
    assert_eq!(
        *rx.borrow(),
        SearchState::Error {
            term: "bob".into(),
            error: SearchError::Timeout
        }
    );

    let fail = Arc::new(AtomicUsize::new(0));
    let f = fail.clone();
    let mut s = Search::<(), String>::new((), move |_, (), since| {
        let n = f.fetch_add(1, Ordering::SeqCst);
        async move {
            match (since, n) {
                (Some(_), 1) => Err(SearchError::Unreachable),
                (None, _) => Ok(Page {
                    hits: vec!["a".into()],
                    next_batch: Some("1".into()),
                }),
                _ => Ok(Page {
                    hits: vec!["b".into()],
                    next_batch: None,
                }),
            }
        }
    });
    let rx = s.subscribe();
    s.set_query("ab", ());
    ms(400).await;
    s.more();
    ms(10).await;
    assert!(
        matches!(&*rx.borrow(), SearchState::Results { hits, more: More::Failed(SearchError::Unreachable), .. } if hits.len() == 1)
    );
    s.more(); // same page again
    ms(10).await;
    assert!(
        matches!(&*rx.borrow(), SearchState::Results { hits, more: More::None, .. } if hits.len() == 2)
    );
}
