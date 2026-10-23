use matrix_engine::{TryReceiveError, TrySendError, bounded};
use std::{
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

#[derive(Default)]
struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn poll<F: Future>(future: Pin<&mut F>, wakes: &Arc<WakeCount>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(&Waker::from(wakes.clone())))
}

#[test]
fn closing_receiver_wakes_blocked_producer_and_async_drain_finishes() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
    sender.try_send(22).unwrap();
    let wakes = Arc::new(WakeCount::default());
    let mut send = std::pin::pin!(sender.send(23));
    assert_eq!(poll(send.as_mut(), &wakes), Poll::Pending);
    receiver.close();
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        poll(send.as_mut(), &wakes),
        Poll::Ready(Err(TrySendError::Closed(23)))
    );
    run(async {
        assert_eq!(receiver.recv().await, Some(22));
        assert_eq!(receiver.recv().await, None);
    });
}

#[test]
fn cancelling_receive_releases_waiter_without_consuming_any_item() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
    let wakes = Arc::new(WakeCount::default());
    {
        let mut receive = std::pin::pin!(receiver.recv());
        assert_eq!(poll(receive.as_mut(), &wakes), Poll::Pending);
    }
    assert_eq!(Arc::strong_count(&wakes), 1);
    sender.try_send(8).unwrap();
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert_eq!(receiver.try_recv(), Ok(8));
}

#[test]
fn cancelling_send_releases_waiter_without_enqueuing_unsent_value() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
    sender.try_send(1).unwrap();
    let wakes = Arc::new(WakeCount::default());
    {
        let mut send = std::pin::pin!(sender.send(2));
        assert_eq!(poll(send.as_mut(), &wakes), Poll::Pending);
    }
    assert_eq!(Arc::strong_count(&wakes), 1);
    assert_eq!(receiver.try_recv(), Ok(1));
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert_eq!(receiver.try_recv(), Err(TryReceiveError::Empty));
    sender.try_send(3).unwrap();
    assert_eq!(receiver.try_recv(), Ok(3));
}

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
// Real futures on real threads, not a simulated channel or network runtime.
// Each executor has a finite deadline, so regressions cannot leave a cron worker.
fn run<F: Future>(future: F) -> F::Output {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                let remaining = deadline
                    .checked_duration_since(std::time::Instant::now())
                    .expect("channel test stalled");
                std::thread::park_timeout(remaining);
            }
        }
    }
}

#[test]
fn async_channel_delivers_ordered_items_across_threads() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(3).unwrap());
    let producer = std::thread::spawn(move || {
        run(async {
            for value in 0..256 {
                assert_eq!(sender.send(value).await, Ok(()));
            }
        })
    });
    let consumer = std::thread::spawn(move || {
        run(async {
            let mut values = Vec::new();
            while let Some(value) = receiver.recv().await {
                values.push(value);
            }
            values
        })
    });
    producer.join().unwrap();
    assert_eq!(consumer.join().unwrap(), (0..256).collect::<Vec<_>>());
}

#[test]
fn repoll_replaces_receive_waker_and_releases_previous_executor() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
    let old = Arc::new(WakeCount::default());
    let new = Arc::new(WakeCount::default());
    let mut receive = std::pin::pin!(receiver.recv());
    assert_eq!(poll(receive.as_mut(), &old), Poll::Pending);
    assert_eq!(poll(receive.as_mut(), &new), Poll::Pending);
    assert_eq!(Arc::strong_count(&old), 1);
    sender.try_send(42).unwrap();
    assert_eq!(old.0.load(Ordering::SeqCst), 0);
    assert_eq!(new.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(receive.as_mut(), &new), Poll::Ready(Some(42)));
}

#[test]
fn async_receive_parks_until_producer_wakes_it() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
    let wakes = Arc::new(WakeCount::default());
    let mut receive = std::pin::pin!(receiver.recv());
    assert_eq!(poll(receive.as_mut(), &wakes), Poll::Pending);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    sender.try_send(42).unwrap();
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(receive.as_mut(), &wakes), Poll::Ready(Some(42)));
}

#[test]
fn async_send_backpressures_until_consumer_frees_capacity() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
    sender.try_send(1).unwrap();
    let wakes = Arc::new(WakeCount::default());
    let mut send = std::pin::pin!(sender.send(2));
    assert_eq!(poll(send.as_mut(), &wakes), Poll::Pending);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert_eq!(receiver.try_recv(), Ok(1));
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(send.as_mut(), &wakes), Poll::Ready(Ok(())));
    assert_eq!(receiver.try_recv(), Ok(2));
}

#[test]
fn producer_drop_wakes_receiver_then_drains_accepted_items() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
    sender.try_send(9).unwrap();
    drop(sender);
    assert_eq!(receiver.try_recv(), Ok(9));
    assert_eq!(receiver.try_recv(), Err(TryReceiveError::Closed));
    let (sender, mut receiver) = bounded::<u8>(NonZeroUsize::new(1).unwrap());
    let wakes = Arc::new(WakeCount::default());
    let mut receive = std::pin::pin!(receiver.recv());
    assert_eq!(poll(receive.as_mut(), &wakes), Poll::Pending);
    drop(sender);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(poll(receive.as_mut(), &wakes), Poll::Ready(None));
}

#[test]
fn consumer_drop_wakes_pending_send_and_returns_its_value() {
    let (mut sender, receiver) = bounded(NonZeroUsize::new(1).unwrap());
    sender.try_send(1).unwrap();
    let wakes = Arc::new(WakeCount::default());
    {
        let mut send = std::pin::pin!(sender.send(2));
        assert_eq!(poll(send.as_mut(), &wakes), Poll::Pending);
        drop(receiver);
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            poll(send.as_mut(), &wakes),
            Poll::Ready(Err(TrySendError::Closed(2)))
        );
    }
    assert_eq!(sender.try_send(3), Err(TrySendError::Closed(3)));
}

#[test]
fn full_queue_returns_unsent_value_without_dropping_or_reordering() {
    let (mut sender, mut receiver) = bounded(NonZeroUsize::new(2).unwrap());
    assert_eq!(receiver.try_recv(), Err(TryReceiveError::Empty));
    assert_eq!(sender.try_send("first"), Ok(()));
    assert_eq!(sender.try_send("second"), Ok(()));
    assert_eq!(sender.try_send("third"), Err(TrySendError::Full("third")));
    assert_eq!(receiver.try_recv(), Ok("first"));
    assert_eq!(sender.try_send("third"), Ok(()));
    assert_eq!(receiver.try_recv(), Ok("second"));
    assert_eq!(receiver.try_recv(), Ok("third"));
    assert_eq!(receiver.try_recv(), Err(TryReceiveError::Empty));
}
