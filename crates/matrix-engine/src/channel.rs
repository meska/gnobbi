use std::{
    collections::VecDeque,
    future::poll_fn,
    num::NonZeroUsize,
    sync::{Arc, Mutex, TryLockError},
    task::{Poll, Waker},
};

#[derive(Debug, PartialEq, Eq)]
pub enum TrySendError<T> {
    Busy(T),
    Full(T),
    Closed(T),
}
#[derive(Debug, PartialEq, Eq)]
pub enum TryReceiveError {
    Busy,
    Empty,
    Closed,
}

struct State<T> {
    queue: VecDeque<T>,
    capacity: usize,
    receive_waker: Option<Waker>,
    send_waker: Option<Waker>,
    sender_alive: bool,
    receiver_alive: bool,
}
/// Single producer; deliberately not Clone. UI must use try_send, never wait.
pub struct Sender<T> {
    state: Arc<Mutex<State<T>>>,
    writes: Writes<T>,
    /// A write was refused with Full/Busy and the caller still holds it (counted in the send guard).
    parked: bool,
}
/// `Some` on channels that carry user writes: those are counted in `send_guard` while queued/handled.
type Writes<T> = Option<fn(&T) -> bool>;
/// Single consumer; deliberately not Clone.
pub struct Receiver<T> {
    state: Arc<Mutex<State<T>>>,
    writes: Writes<T>,
    /// The write last received is being handled: it stays counted until the next receive (or the drop).
    held: bool,
}

/// Bounded by item count, not payload bytes. No silent eviction or coalescing.
pub fn bounded<T>(capacity: NonZeroUsize) -> (Sender<T>, Receiver<T>) {
    channel(capacity, None)
}

/// Like [`bounded`], for channels whose `is_write` items must survive an updater restart: see `send_guard`.
/// While the gate is closed `try_send` of such an item answers `Busy` and keeps nothing.
pub fn bounded_writes<T>(capacity: NonZeroUsize, is_write: fn(&T) -> bool) -> (Sender<T>, Receiver<T>) {
    channel(capacity, Some(is_write))
}

fn channel<T>(capacity: NonZeroUsize, writes: Writes<T>) -> (Sender<T>, Receiver<T>) {
    let state = Arc::new(Mutex::new(State {
        queue: VecDeque::new(),
        capacity: capacity.get(),
        receive_waker: None,
        send_waker: None,
        sender_alive: true,
        receiver_alive: true,
    }));
    (
        Sender {
            state: state.clone(),
            writes,
            parked: false,
        },
        Receiver { state, writes, held: false },
    )
}
struct SendRegistration<'a, T>(&'a Mutex<State<T>>);
impl<T> Drop for SendRegistration<'_, T> {
    fn drop(&mut self) {
        let wake = self
            .0
            .lock()
            .expect("channel state poisoned")
            .send_waker
            .take();
        drop(wake);
    }
}
impl<T> Sender<T> {
    /// True once the receiving side is gone: nothing sent from now on can be delivered.
    pub fn is_closed(&self) -> bool {
        !self.state.lock().expect("channel state poisoned").receiver_alive
    }
    /// Backend-only backpressure. Dropping this future abandons its unsent value;
    /// durable state owners must retain/persist that work until delivery succeeds.
    pub async fn send(&mut self, value: T) -> Result<(), TrySendError<T>> {
        let _registration = SendRegistration(&self.state);
        let mut value = Some(value);
        poll_fn(|cx| {
            let mut state = self.state.lock().expect("channel state poisoned");
            if !state.receiver_alive {
                state.send_waker = None;
                return Poll::Ready(Err(TrySendError::Closed(
                    value.take().expect("send polled after completion"),
                )));
            }
            if state.queue.len() == state.capacity {
                state.send_waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
            let value = value.take().expect("send polled after completion");
            if self.writes.is_some_and(|w| w(&value)) {
                crate::send_guard::count(1);
            }
            state.queue.push_back(value);
            state.send_waker = None;
            let wake = state.receive_waker.take();
            drop(state);
            if let Some(wake) = wake {
                wake.wake();
            }
            Poll::Ready(Ok(()))
        })
        .await
    }
    /// UI-safe capacity/mutex try: retain returned values on Busy/Full and
    /// schedule a later attempt, never spin. Closed requires explicit recovery.
    pub fn try_send(&mut self, value: T) -> Result<(), TrySendError<T>> {
        let counted = self.writes.is_some_and(|w| w(&value));
        let result = self.admit(value, counted);
        if counted && self.parked != matches!(result, Err(TrySendError::Full(_) | TrySendError::Busy(_))) {
            self.parked = !self.parked;
            crate::send_guard::park(self.parked);
        }
        result
    }
    fn admit(&mut self, value: T, counted: bool) -> Result<(), TrySendError<T>> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Err(TrySendError::Busy(value)),
            Err(TryLockError::Poisoned(_)) => panic!("channel state poisoned"),
        };
        if !state.receiver_alive {
            return Err(TrySendError::Closed(value));
        }
        if state.queue.len() == state.capacity {
            return Err(TrySendError::Full(value));
        }
        // Counted under the channel lock: a refused (closed gate) or counted write, never a lost one.
        if self.writes.is_some() && !crate::send_guard::admit(counted) {
            return Err(TrySendError::Busy(value));
        }
        state.queue.push_back(value);
        let wake = state.receive_waker.take();
        drop(state);
        if let Some(wake) = wake {
            wake.wake();
        }
        Ok(())
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if self.parked {
            crate::send_guard::park(false);
        }
        let mut state = self.state.lock().expect("channel state poisoned");
        state.sender_alive = false;
        let wake = state.receive_waker.take();
        drop(state);
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut state = self.state.lock().expect("channel state poisoned");
        state.receiver_alive = false;
        let wake = state.send_waker.take();
        let abandoned = std::mem::take(&mut state.queue);
        drop(state);
        let queued = abandoned.iter().filter(|v| self.writes.is_some_and(|w| w(v))).count();
        self.release(queued);
        if let Some(wake) = wake {
            wake.wake();
        }
        drop(abandoned);
    }
}
struct ReceiveRegistration<'a, T>(&'a Mutex<State<T>>);
impl<T> Drop for ReceiveRegistration<'_, T> {
    fn drop(&mut self) {
        let wake = self
            .0
            .lock()
            .expect("channel state poisoned")
            .receive_waker
            .take();
        drop(wake);
    }
}
impl<T> Receiver<T> {
    /// Uncounts the write handled so far (plus `more`); called when the next one is asked for.
    fn release(&mut self, more: usize) {
        let n = std::mem::take(&mut self.held) as usize + more;
        if n > 0 {
            crate::send_guard::release(n);
        }
    }
    /// Backend shutdown: reject new sends atomically, retain queued values to drain.
    pub fn close(&mut self) {
        let mut state = self.state.lock().expect("channel state poisoned");
        state.receiver_alive = false;
        let send = state.send_waker.take();
        let receive = state.receive_waker.take();
        drop(state);
        if let Some(wake) = send {
            wake.wake();
        }
        if let Some(wake) = receive {
            wake.wake();
        }
    }

    /// Backend wait only; UI drains with try_recv. No busy polling.
    pub async fn recv(&mut self) -> Option<T> {
        self.release(0);
        let _registration = ReceiveRegistration(&self.state);
        let (state, writes, held) = (&self.state, self.writes, &mut self.held);
        poll_fn(|cx| {
            let mut state = state.lock().expect("channel state poisoned");
            if let Some(value) = state.queue.pop_front() {
                *held = writes.is_some_and(|w| w(&value));
                state.receive_waker = None;
                let wake = state.send_waker.take();
                drop(state);
                if let Some(wake) = wake {
                    wake.wake();
                }
                Poll::Ready(Some(value))
            } else if !state.sender_alive || !state.receiver_alive {
                state.receive_waker = None;
                Poll::Ready(None)
            } else {
                state.receive_waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
    /// UI drain: stop at Empty/Busy or the caller's frame budget, not a busy loop.
    pub fn try_recv(&mut self) -> Result<T, TryReceiveError> {
        self.release(0);
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Err(TryReceiveError::Busy),
            Err(TryLockError::Poisoned(_)) => panic!("channel state poisoned"),
        };
        let value =
            state
                .queue
                .pop_front()
                .ok_or(if state.sender_alive && state.receiver_alive {
                    TryReceiveError::Empty
                } else {
                    TryReceiveError::Closed
                })?;
        let wake = state.send_waker.take();
        drop(state);
        self.held = self.writes.is_some_and(|w| w(&value));
        if let Some(wake) = wake {
            wake.wake();
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_try_operations_return_busy_instead_of_waiting_for_mutex() {
        let (mut sender, mut receiver) = bounded(NonZeroUsize::new(1).unwrap());
        let state = sender.state.clone();
        let guard = state.lock().unwrap();
        assert_eq!(sender.try_send(1), Err(TrySendError::Busy(1)));
        assert_eq!(receiver.try_recv(), Err(TryReceiveError::Busy));
        drop(guard);
        assert_eq!(sender.try_send(1), Ok(()));
        assert_eq!(receiver.try_recv(), Ok(1));
    }
}
