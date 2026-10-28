//! Remote for a running account worker, kept by whoever must end that one account later.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct WorkerControl {
    stop: Arc<AtomicBool>,
    logout: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}
impl WorkerControl {
    pub fn new(stop: Arc<AtomicBool>, logout: Arc<AtomicBool>, done: Arc<AtomicBool>) -> Self {
        Self { stop, logout, done }
    }

    /// Stops the worker; once its writes are drained it revokes this session on the server
    /// (best effort, bounded) and forgets the session's store.
    pub fn logout_and_stop(&self) {
        self.logout.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Release);
    }

    /// The worker thread has ended: its profile lock and store are free.
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}
