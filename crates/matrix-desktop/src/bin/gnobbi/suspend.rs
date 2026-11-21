//! iOS background: a suspended app must hold no file lock or open SQLite in the App Group container (0xdead10cc), and
//! the crypto-lock lease must not lapse under a live client (the NSE takes the lock and moves the store generation).
//! So on background every account worker is stopped and waited for (its thread drops client, store and `.lock`);
//! on foreground the window starts them again from the saved profiles, and the SDK reloads the OlmMachine if the
//! generation moved. The platform observer (mobile.rs) calls `suspend`/`resume` from the main thread.
use matrix_engine::WorkerControl;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

static CONTROLS: Mutex<Vec<WorkerControl>> = Mutex::new(Vec::new());
static SUSPENDED: AtomicBool = AtomicBool::new(false);
static RESUMED: AtomicBool = AtomicBool::new(false);

/// Every started account worker, so the platform observer can end them without the window.
pub fn register(control: WorkerControl) {
    let mut all = CONTROLS.lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|c| !c.is_done());
    all.push(control);
}

/// Stops every worker and waits up to `wait` for their threads to end. False: some still run (the caller decides; the lock stays held).
// shortcut: a fixed wait; a worker stuck in a long SDK call keeps its lock until the OS ends the background grace period.
pub fn suspend(wait: Duration) -> bool {
    SUSPENDED.store(true, Ordering::Release);
    let all: Vec<WorkerControl> = CONTROLS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    all.iter().for_each(WorkerControl::stop);
    let end = Instant::now() + wait;
    while !all.iter().all(WorkerControl::is_done) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    all.iter().all(WorkerControl::is_done)
}

pub fn resume() {
    if SUSPENDED.swap(false, Ordering::AcqRel) {
        RESUMED.store(true, Ordering::Release);
    }
}

pub fn is_suspended() -> bool {
    SUSPENDED.load(Ordering::Acquire)
}

/// True once per resume: the window must restart its accounts.
pub fn take_resumed() -> bool {
    RESUMED.swap(false, Ordering::AcqRel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn control() -> (WorkerControl, Arc<AtomicBool>, Arc<AtomicBool>) {
        let (stop, done) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let c = WorkerControl::new(stop.clone(), Arc::default(), done.clone(), Arc::default());
        (c, stop, done)
    }

    // One test: the registry and flags are process-wide.
    #[test]
    fn suspend_stops_waits_for_every_worker_and_resume_signals_once() {
        let (a, a_stop, a_done) = control();
        register(a);
        // a worker that ends a little after the stop is waited for
        std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(100)); a_done.store(true, Ordering::Release) });
        assert!(suspend(Duration::from_secs(2)));
        assert!(a_stop.load(Ordering::Acquire) && is_suspended());
        resume();
        assert!(!is_suspended());
        assert!(take_resumed() && !take_resumed(), "one signal per resume");
        resume();
        assert!(!take_resumed(), "a resume without a suspend signals nothing");
        let (b, _, _) = control(); // never ends
        register(b);
        assert!(!suspend(Duration::from_millis(60)), "a worker that does not end is reported");
    }
}
