use matrix_engine::reconnect::Backoff;
use std::time::Duration;

#[test]
fn a_successful_connection_resets_delay_without_repeating_the_jitter_sequence() {
    let mut retry = Backoff::new(22);
    let first = retry.next_delay();
    for _ in 0..8 {
        retry.next_delay();
    }
    retry.reset();
    let recovered = retry.next_delay();
    assert!(recovered >= Duration::from_millis(250));
    assert!(recovered <= Duration::from_millis(500));
    assert_ne!(first, recovered);
}

#[test]
fn distinct_seeds_desynchronize_clients_and_zero_is_valid() {
    let samples = |seed| {
        let mut retry = Backoff::new(seed);
        (0..10).map(|_| retry.next_delay()).collect::<Vec<_>>()
    };
    assert_ne!(samples(22), samples(23));
    assert!(samples(0).iter().all(|d| *d >= Duration::from_millis(250)));
}

#[test]
fn retries_grow_exponentially_with_bounded_jitter_and_saturate() {
    let mut retry = Backoff::new(22);
    for ceiling in [500, 1000, 2000, 4000, 8000, 16000, 30000]
        .into_iter()
        .chain(std::iter::repeat_n(30000, 10000))
    {
        let delay = retry.next_delay();
        assert!(delay >= Duration::from_millis(ceiling / 2));
        assert!(delay <= Duration::from_millis(ceiling));
    }
}
