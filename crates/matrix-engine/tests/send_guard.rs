//! The updater's gate against the UI write paths: a write is counted from `try_send` to the end of its
//! handling, the gate closes only on an idle process, a write after the close is refused (kept by the
//! caller), and dropping the lock (failed install) reopens everything. One process, global gate: tests
//! take `SERIAL`.
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope, TransactionId};
use matrix_engine::{
    TrySendError, bounded_writes, lock_for_update,
    sending::{Attachment, SendRequest},
};
use std::{
    num::NonZeroUsize,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
    },
};

static SERIAL: Mutex<()> = Mutex::new(());

fn request(n: usize, attachment: bool) -> SendRequest {
    SendRequest {
        scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:test") },
        transaction_id: TransactionId::new(format!("t{n}")),
        body: String::new(),
        attachment: attachment.then(|| Attachment { filename: "a.png".into(), data: vec![1, 2, 3], voice: None }),
        poll: None,
        mentions: Vec::new(),
    }
}

fn channel(cap: usize) -> (matrix_engine::Sender<SendRequest>, matrix_engine::Receiver<SendRequest>) {
    bounded_writes(NonZeroUsize::new(cap).unwrap(), |_| true)
}

#[test]
fn idle_locks_and_a_write_after_the_lock_is_refused_then_goes_through_after_a_failed_install() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut tx, mut rx) = channel(4);
    let lock = lock_for_update().expect("nothing sent: the gate closes");
    assert!(lock_for_update().is_none(), "already closed: a second updater does not stack");
    // Between the check and the exit: refused, the value comes back to the caller, nothing is queued.
    let Err(TrySendError::Busy(back)) = tx.try_send(request(1, false)) else { panic!("must be refused") };
    assert_eq!(back.transaction_id.as_str(), "t1");
    assert!(rx.try_recv().is_err(), "nothing reached the engine");
    // The install failed: the lock is dropped and the same write is accepted.
    drop(lock);
    assert!(tx.try_send(back).is_ok());
    assert!(lock_for_update().is_none(), "queued write: no update");
    assert_eq!(rx.try_recv().map(|r| r.transaction_id), Ok(TransactionId::new("t1")));
}

#[test]
fn an_attachment_blocks_the_gate_until_the_engine_is_done_with_it() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut tx, mut rx) = channel(4);
    assert!(tx.try_send(request(2, true)).is_ok());
    assert!(lock_for_update().is_none(), "queued attachment (never in the outbox) counts");
    let received = rx.try_recv().unwrap();
    assert!(received.attachment.is_some());
    // Received and being uploaded (the engine awaits it before asking for the next request).
    assert!(lock_for_update().is_none(), "attachment in flight counts");
    assert!(rx.try_recv().is_err());
    assert!(lock_for_update().is_some(), "handled: idle again");
}

#[test]
fn a_dropped_receiver_does_not_leave_the_gate_stuck() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut tx, rx) = channel(4);
    assert!(tx.try_send(request(3, false)).is_ok());
    assert!(lock_for_update().is_none());
    drop(rx);
    assert!(lock_for_update().is_some(), "queued writes died with the engine: nothing more to protect");
}

#[test]
fn a_write_parked_in_the_ui_blocks_the_gate_until_it_is_sent() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut tx, mut rx) = channel(1);
    assert!(tx.try_send(request(4, false)).is_ok());
    let Err(TrySendError::Full(parked)) = tx.try_send(request(5, false)) else { panic!("must be full") };
    rx.try_recv().unwrap();
    assert!(rx.try_recv().is_err());
    assert!(lock_for_update().is_none(), "the UI still holds a write it could not hand over");
    assert!(tx.try_send(parked).is_ok());
    rx.try_recv().unwrap();
    assert!(rx.try_recv().is_err());
    assert!(lock_for_update().is_some(), "handed over and handled");
    drop(tx);
}

#[test]
fn race_no_write_is_accepted_while_the_lock_is_held() {
    let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut tx, mut rx) = channel(8);
    let (locked, done) = (AtomicBool::new(false), AtomicBool::new(false));
    let (accepted, violations, locks) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0));
    std::thread::scope(|s| {
        s.spawn(|| {
            // The UI: keeps sending, retains what is refused.
            let mut n = 0;
            let mut held = None;
            while !done.load(SeqCst) {
                let r = held.take().unwrap_or_else(|| request(n, n % 2 == 0));
                let before = locked.load(SeqCst);
                let result = tx.try_send(r);
                let after = locked.load(SeqCst);
                match result {
                    Ok(()) => {
                        accepted.fetch_add(1, SeqCst);
                        n += 1;
                        if before && after {
                            violations.fetch_add(1, SeqCst);
                        }
                    }
                    Err(TrySendError::Busy(r) | TrySendError::Full(r)) => held = Some(r),
                    Err(TrySendError::Closed(_)) => break,
                }
            }
        });
        s.spawn(|| {
            // The engine: handles one at a time, takes a moment per write.
            while !done.load(SeqCst) {
                if rx.try_recv().is_ok() {
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
            }
        });
        for _ in 0..300 {
            if let Some(lock) = lock_for_update() {
                locked.store(true, SeqCst);
                locks.fetch_add(1, SeqCst);
                std::thread::sleep(std::time::Duration::from_micros(500));
                locked.store(false, SeqCst);
                drop(lock);
            }
            std::thread::yield_now();
        }
        done.store(true, SeqCst);
    });
    assert_eq!(violations.load(SeqCst), 0, "a write got in between the check and the exit");
    assert!(locks.load(SeqCst) > 0 && accepted.load(SeqCst) > 0, "the race was actually exercised");
}
