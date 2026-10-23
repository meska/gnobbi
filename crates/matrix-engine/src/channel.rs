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
}
/// Single consumer; deliberately not Clone.
pub struct Receiver<T> {
    state: Arc<Mutex<State<T>>>,
}

/// Bounded by item count, not payload bytes. No silent eviction or coalescing.
pub fn bounded<T>(capacity: NonZeroUsize) -> (Sender<T>, Receiver<T>) {
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
        },
        Receiver { state },
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
            state
                .queue
                .push_back(value.take().expect("send polled after completion"));
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
        let _registration = ReceiveRegistration(&self.state);
        poll_fn(|cx| {
            let mut state = self.state.lock().expect("channel state poisoned");
            if let Some(value) = state.queue.pop_front() {
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
