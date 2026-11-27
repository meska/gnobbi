//! Process-wide guard between the updater's last «nothing unsent» check and the exit. Every UI→engine
//! write (text, attachment, poll, reply/edit, thread, sticker) goes through a `bounded_writes` channel,
//! which counts it here from `try_send` until the engine has finished handling it (queued in the SDK
//! queue or answered). `lock` closes the gate only if nothing is counted, under the same mutex that
//! `try_send` takes: a write is either counted before the lock (the lock refuses) or refused after it
//! (`Busy`, the UI keeps it and retries once the gate reopens).
use std::sync::{Mutex, MutexGuard, TryLockError};

#[derive(Default)]
struct Gate {
    closed: bool,
    /// Writes sitting in a channel or being handled by the engine.
    writes: usize,
    /// Writes a UI sender was told `Full`/`Busy` for and still holds in memory.
    parked: usize,
}

static GATE: Mutex<Gate> = Mutex::new(Gate { closed: false, writes: 0, parked: 0 });

fn gate() -> MutexGuard<'static, Gate> {
    // Counters stay meaningful after a panic elsewhere; `closed` is only ever set by `lock`.
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Held from the last check until the app exits; dropping it (install failed) reopens the writes.
#[derive(Debug)]
pub struct UpdateLock(());

impl Drop for UpdateLock {
    fn drop(&mut self) {
        gate().closed = false;
    }
}

/// Closes the gate if no write is counted or parked and `idle()` (e.g. the outbox is empty) says so.
/// `None` = busy or cannot tell: the caller must not restart. `idle` runs under the gate, after the counters.
pub fn lock(idle: impl FnOnce() -> bool) -> Option<UpdateLock> {
    let mut g = gate();
    if g.closed || g.writes > 0 || g.parked > 0 || !idle() {
        return None;
    }
    g.closed = true;
    Some(UpdateLock(()))
}

/// Admits one write (UI side, never waits). `false` = refused: gate closed or momentarily busy.
pub(crate) fn admit(counted: bool) -> bool {
    let mut g = match GATE.try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(e)) => e.into_inner(),
        Err(TryLockError::WouldBlock) => return false,
    };
    if g.closed {
        return false;
    }
    g.writes += counted as usize;
    true
}

/// Backend senders wait anyway: they are counted but not refused (they are not the UI race).
pub(crate) fn count(n: usize) {
    gate().writes += n;
}

pub(crate) fn release(n: usize) {
    let mut g = gate();
    g.writes = g.writes.saturating_sub(n);
}

pub(crate) fn park(on: bool) {
    let mut g = gate();
    g.parked = if on { g.parked + 1 } else { g.parked.saturating_sub(1) };
}
