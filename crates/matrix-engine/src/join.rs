//! Join of a room that may take longer than one HTTP request (large federated rooms).
//!
//! A request timeout is NOT a failure: the server keeps joining and the sync delivers the room.
//! So only a definitive 4xx answer is a refusal; everything else becomes `Slow` and the membership
//! is watched until `JOIN_CONFIRM_WINDOW`. Spec: the JOIN_STATE_MACHINE notes.
use crate::operations::OperationError;
use matrix_domain::RoomScope;
use std::time::Duration;

/// How long after the click the join is watched before it is «non confermato».
/// Configured here only; the HTTP timeout of a single request stays the client's 10 s
/// (`persistence.rs` / `auth.rs`, `RequestConfig::timeout`) because the SDK join call takes no per-call config.
pub const JOIN_CONFIRM_WINDOW: Duration = Duration::from_secs(150);
/// Period of the local membership recheck while watching (sync updates end the wait sooner).
pub const JOIN_RECHECK: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinPhase {
    /// Request timed out or got no answer: still in progress, not an error.
    Slow,
    /// Joined, also «was already joined».
    Joined(RoomScope),
    /// The server (or the input) said no: `Forbidden`, `Unavailable` or `InvalidInput`.
    Refused(OperationError),
    /// Window over without proof of membership.
    Unconfirmed,
}

/// Definitive answers only: 400, 403, 404. 429, 5xx, proxy 502/504 and the rest are «still in progress».
pub fn refusal_for_status(status: u16) -> Option<OperationError> {
    match status {
        400 => Some(OperationError::InvalidInput),
        403 => Some(OperationError::Forbidden),
        404 => Some(OperationError::Unavailable),
        _ => None,
    }
}

#[cfg(feature = "sdk")]
mod driver {
    use super::*;
    use crate::operations::{join_via, membership, Membership};
    use matrix_sdk::{
        ruma::{OwnedRoomId, OwnedRoomOrAliasId, RoomAliasId, RoomId},
        Client, RoomState,
    };
    use tokio::{sync::broadcast, time::Instant};

    fn refusal(error: &matrix_sdk::Error) -> Option<OperationError> {
        error.as_client_api_error().and_then(|e| refusal_for_status(e.status_code.as_u16()))
    }

    /// Waits for proof of membership until `deadline`: a matching update, a positive `local` recheck every
    /// `recheck`, and at the end one `last` (server-side) check. `false` = unconfirmed.
    pub(crate) async fn confirm<U: Clone>(
        updates: broadcast::Receiver<U>,
        is_join: impl Fn(&U) -> bool,
        local: impl Fn() -> bool,
        last: impl AsyncFnOnce() -> bool,
        deadline: Instant,
        recheck: Duration,
    ) -> bool {
        let mut updates = Some(updates);
        loop {
            if local() {
                return true;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return last().await,
                _ = tokio::time::sleep(recheck) => {}
                update = async {
                    match &mut updates {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => match update {
                    Ok(u) if is_join(&u) => return true,
                    Err(broadcast::error::RecvError::Closed) => updates = None,
                    _ => {}
                },
            }
        }
    }

    /// Joins through the SDK and reports `Slow` (at most once) through `emit`; returns the final phase.
    /// A room already joined, here or per the server, gives `Joined` without a new join request.
    pub async fn join_tracked(
        client: &Client,
        account: matrix_domain::AccountId,
        target: &str,
        via: &[String],
        mut emit: impl FnMut(JoinPhase),
    ) -> JoinPhase {
        let deadline = Instant::now() + JOIN_CONFIRM_WINDOW;
        let Ok(target) = OwnedRoomOrAliasId::try_from(target.trim()) else {
            return JoinPhase::Refused(OperationError::InvalidInput);
        };
        let mut via = match join_via(&target, via) {
            Ok(via) => via,
            Err(e) => return JoinPhase::Refused(e),
        };
        let id: OwnedRoomId = if target.is_room_id() {
            match RoomId::parse(target.as_str()) {
                Ok(id) => id,
                Err(_) => return JoinPhase::Refused(OperationError::InvalidInput),
            }
        } else {
            let Ok(alias) = RoomAliasId::parse(target.as_str()) else {
                return JoinPhase::Refused(OperationError::InvalidInput);
            };
            match client.resolve_room_alias(&alias).await {
                Ok(r) => {
                    for s in r.servers {
                        if !via.contains(&s) {
                            via.push(s);
                        }
                    }
                    r.room_id
                }
                // No join was sent yet: «Riprova» is safe.
                Err(e) => {
                    return match e.as_client_api_error().and_then(|a| refusal_for_status(a.status_code.as_u16())) {
                        Some(r) => JoinPhase::Refused(r),
                        None => JoinPhase::Unconfirmed,
                    };
                }
            }
        };
        let scope = || RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(id.as_str()) };
        let local = || client.get_room(&id).is_some_and(|r| r.state() == RoomState::Joined);
        // «Riprova» and double entry points land here: an existing membership is never joined again.
        // shortcut: an unreadable answer (Unknown, e.g. M_FORBIDDEN of a never-joined room) still joins, a join is idempotent server-side.
        if local() || matches!(membership(client, &id).await, Membership::Joined) {
            return JoinPhase::Joined(scope());
        }
        // Subscribed BEFORE the request: the sync can deliver the room while the request is still open.
        let updates = client.subscribe_to_room_updates(&id);
        match client.join_room_by_id_or_alias(&target, &via).await {
            Ok(_) => return JoinPhase::Joined(scope()),
            Err(e) => {
                if let Some(r) = refusal(&e) {
                    return JoinPhase::Refused(r);
                }
            }
        }
        emit(JoinPhase::Slow);
        let joined = confirm(
            updates,
            |u| matches!(u, matrix_sdk::sync::RoomUpdate::Joined { .. }),
            local,
            async || matches!(membership(client, &id).await, Membership::Joined),
            deadline,
            JOIN_RECHECK,
        )
        .await;
        if joined { JoinPhase::Joined(scope()) } else { JoinPhase::Unconfirmed }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

        const W: Duration = JOIN_CONFIRM_WINDOW;

        #[tokio::test(start_paused = true)]
        async fn sync_update_confirms_at_once() {
            let (tx, rx) = broadcast::channel(8);
            let start = Instant::now();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(40)).await;
                tx.send(true).unwrap();
            });
            assert!(confirm(rx, |u| *u, || false, async || false, start + W, JOIN_RECHECK).await);
            assert_eq!(start.elapsed(), Duration::from_secs(40));
        }

        #[tokio::test(start_paused = true)]
        async fn local_recheck_confirms_on_the_next_tick() {
            let (_tx, rx) = broadcast::channel::<bool>(8);
            let joined = std::sync::Arc::new(AtomicBool::new(false));
            let flag = joined.clone();
            let start = Instant::now();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(25)).await;
                flag.store(true, SeqCst);
            });
            assert!(confirm(rx, |u| *u, || joined.load(SeqCst), async || false, start + W, JOIN_RECHECK).await);
            assert_eq!(start.elapsed(), Duration::from_secs(30));
        }

        #[tokio::test(start_paused = true)]
        async fn nothing_in_the_window_is_unconfirmed_after_one_last_check() {
            let (_tx, rx) = broadcast::channel::<bool>(8);
            let asked = AtomicUsize::new(0);
            let start = Instant::now();
            let ok = confirm(rx, |u| *u, || false, async || { asked.fetch_add(1, SeqCst); false }, start + W, JOIN_RECHECK).await;
            assert!(!ok);
            assert_eq!((asked.load(SeqCst), start.elapsed()), (1, W));
        }

        #[tokio::test(start_paused = true)]
        async fn last_server_check_can_still_confirm() {
            let (_tx, rx) = broadcast::channel::<bool>(8);
            let ok = confirm(rx, |u| *u, || false, async || true, Instant::now() + W, JOIN_RECHECK).await;
            assert!(ok);
        }

        #[tokio::test(start_paused = true)]
        async fn closed_or_foreign_updates_do_not_spin_or_confirm() {
            let (tx, rx) = broadcast::channel(8);
            tx.send(false).unwrap(); // not a join
            drop(tx); // closed: polling only
            let start = Instant::now();
            assert!(!confirm(rx, |u| *u, || false, async || false, start + W, JOIN_RECHECK).await);
            assert_eq!(start.elapsed(), W);
        }
    }
}
#[cfg(feature = "sdk")]
pub use driver::join_tracked;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_definitive_4xx_refuse() {
        assert_eq!(refusal_for_status(403), Some(OperationError::Forbidden));
        assert_eq!(refusal_for_status(404), Some(OperationError::Unavailable));
        assert_eq!(refusal_for_status(400), Some(OperationError::InvalidInput));
        for slow in [408, 429, 500, 502, 504] {
            assert_eq!(refusal_for_status(slow), None, "{slow} is still in progress");
        }
    }
}
