//! iOS background: a suspended app must hold no file lock or open SQLite in the App Group container (0xdead10cc), and
//! the crypto-lock lease must not lapse under a live client (the NSE takes the lock and moves the store generation).
//! So on background every account worker is stopped and waited for (its thread drops client, store and `.lock`);
//! on foreground the window starts them again from the saved profiles, and the SDK reloads the OlmMachine if the
//! generation moved. The platform observer (mobile.rs) calls `suspend`/`resume` from the main thread.
use matrix_engine::WorkerControl;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Foreground, Grace, Released }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event { Background, Foregrounded, Deadline }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { Begin, Cancel, Release, Restart, None }

pub fn transition(state: State, event: Event) -> (State, Action) {
    use Action::*;
    match (state, event) {
        (State::Foreground, Event::Background) => (State::Grace, Begin),
        (State::Grace, Event::Foregrounded) => (State::Foreground, Cancel),
        (State::Grace, Event::Deadline) => (State::Released, Release),
        (State::Released, Event::Foregrounded) => (State::Foreground, Restart),
        _ => (state, None),
    }
}

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

pub fn background() {
    RESUMED.store(false, Ordering::Release);
}

pub fn resume() {
    if SUSPENDED.load(Ordering::Acquire) {
        RESUMED.store(true, Ordering::Release);
    }
}

pub fn is_suspended() -> bool {
    SUSPENDED.load(Ordering::Acquire)
}

/// True once per resume: the window must restart its accounts.
pub fn take_resumed() -> bool {
    if !RESUMED.load(Ordering::Acquire) || !CONTROLS.lock().unwrap_or_else(|e| e.into_inner()).iter().all(WorkerControl::is_done) {
        return false;
    }
    SUSPENDED.store(false, Ordering::Release);
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

    #[test]
    fn lifecycle_cancels_grace_and_restarts_only_after_release() {
        use Event::*;
        use State::*;
        assert_eq!(transition(Foreground, Background), (Grace, Action::Begin));
        assert_eq!(transition(Grace, Foregrounded), (Foreground, Action::Cancel));
        assert_eq!(transition(Grace, Deadline), (Released, Action::Release));
        assert_eq!(transition(Grace, Background), (Grace, Action::None));
        assert_eq!(transition(Released, Background), (Released, Action::None));
        assert_eq!(transition(Released, Foregrounded), (Foreground, Action::Restart));
        assert_eq!(transition(Foreground, Deadline), (Foreground, Action::None));
        assert_eq!(transition(Released, Deadline), (Released, Action::None));
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
        assert!(is_suspended(), "stay paused until restart is taken");
        assert!(take_resumed() && !take_resumed(), "one signal per resume");
        assert!(!is_suspended());
        resume();
        assert!(!take_resumed(), "a resume without a suspend signals nothing");
        let (b, _, b_done) = control(); // never ends until explicitly released
        register(b);
        assert!(!suspend(Duration::from_millis(60)), "a worker that does not end is reported");
        resume();
        assert!(!take_resumed() && is_suspended(), "never reopen a locked store");
        background();
        b_done.store(true, Ordering::Release);
        assert!(!take_resumed(), "background cancels a pending restart");
        resume();
        assert!(take_resumed() && !is_suspended());
    }
}
