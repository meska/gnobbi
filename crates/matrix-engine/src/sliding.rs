//! Sliding-sync adapter: the official `SyncService` drives sync and `RoomListService` says when
//! the room list is loaded; the same `RoomListEvent`s as the classic loop come out. Sliding sync
//! only (MSC4186), no classic fallback.
use crate::{RoomListEvent, RoomSyncStatus, Sender, room_sync::RoomSync};
use eyeball_im::Vector;
use futures_util::StreamExt;
use matrix_sdk::sliding_sync::Version;
use matrix_sdk_ui::{
    room_list_service::{
        RoomListItem, RoomListLoadingState, State as RoomListState, filters::new_filter_non_left,
    },
    sync_service::{State, SyncService},
};
use tokio::{sync::broadcast::error::RecvError, time::Instant};

/// Traces each shown-status change: previous, new, cause, time since the last one. Nothing else
/// is logged (no token, header, error text or user data).
struct StatusTrace {
    shown: &'static str,
    since: std::time::Instant,
}

impl StatusTrace {
    fn change(&mut self, new: &'static str, cause: &'static str) {
        tracing::info!(
            target: "gnobbi::sync_status",
            previous = self.shown,
            new,
            cause,
            elapsed_ms = self.since.elapsed().as_millis() as u64,
            "sync status"
        );
        self.shown = new;
        self.since = std::time::Instant::now();
    }
}

const OUTAGE: std::time::Duration = std::time::Duration::from_secs(20);
const HOLD: std::time::Duration = std::time::Duration::from_secs(5);
const EPISODES: u32 = 3;
const PROBE_MISSES: u32 = 3;

/// Decides when the bar says «sync failed»: only after 20 s without a success since the first
/// failure, 3 failure episodes with no success between them, or 3 probe misses spanning 20 s;
/// back at the first success. The shown text holds at least 5 s. Pure: the clock is injected.
/// shortcut: the first change after start is not held, so a first Ready may flip at once.
#[derive(Default)]
pub(crate) struct StatusGate {
    failed: bool,
    changed: Option<Instant>,
    episodes: u32,
    since: Option<Instant>,
    probe_misses: u32,
    probe_since: Option<Instant>,
}

impl StatusGate {
    /// One SyncService Offline/Error episode.
    pub(crate) fn fail(&mut self, now: Instant) {
        self.episodes += 1;
        self.since.get_or_insert(now);
    }

    /// A valid sliding response (or the SDK back from offline).
    pub(crate) fn succeed(&mut self) {
        (self.episodes, self.since) = (0, None);
    }

    /// A reachability probe; one slow answer changes nothing.
    pub(crate) fn probe(&mut self, now: Instant, ok: bool) {
        if ok {
            (self.probe_misses, self.probe_since) = (0, None);
        } else {
            self.probe_misses += 1;
            self.probe_since.get_or_insert(now);
        }
    }

    /// When failure becomes due, if it is pending.
    fn due(&self) -> Option<Instant> {
        let by_probe = self
            .probe_since
            .filter(|_| self.probe_misses >= PROBE_MISSES)
            .map(|t| t + OUTAGE);
        let by_sync = self.since.map(|t| {
            if self.episodes >= EPISODES {
                t
            } else {
                t + OUTAGE
            }
        });
        by_probe.into_iter().chain(by_sync).min()
    }

    /// `Some(failed)` when the shown text must change now.
    pub(crate) fn poll(&mut self, now: Instant) -> Option<bool> {
        let want = self.due().is_some_and(|due| due <= now);
        if want == self.failed || self.changed.is_some_and(|t| now < t + HOLD) {
            return None;
        }
        (self.failed, self.changed) = (want, Some(now));
        Some(want)
    }

    /// The next instant `poll` may answer differently; None if only an input can change it.
    pub(crate) fn wake(&self, now: Instant) -> Option<Instant> {
        let want = self.due().is_some_and(|due| due <= now);
        let at = if want != self.failed {
            self.changed.map(|t| t + HOLD)
        } else if self.failed {
            None
        } else {
            self.due()
        };
        at.filter(|at| *at > now)
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed
    }
}

/// Send, stopping the adapter when the consumer is gone.
async fn emit(sender: &mut Sender<RoomListEvent>, event: RoomListEvent) -> bool {
    sender.send(event).await.is_ok()
}

/// Projects diffs, order and activity for the rooms `RoomListService` lists (joined and invited,
/// sorted by the SDK); false when the consumer is gone.
async fn publish(
    sync: &mut RoomSync,
    sender: &mut Sender<RoomListEvent>,
    listed: &Vector<RoomListItem>,
    last_order: &mut Vec<matrix_domain::RoomScope>,
    heads: Vec<(String, matrix_domain::EventId)>,
) -> bool {
    let rooms = listed.iter().map(|item| (**item).clone()).collect();
    let Ok(changes) = sync.summarize_rooms(rooms).await else {
        return true;
    };
    for change in changes {
        if !emit(sender, RoomListEvent::Diff(change)).await {
            return false;
        }
    }
    let order: Vec<_> = listed
        .iter()
        .filter_map(|item| sync.scope_of(item.room_id().as_str()))
        .collect();
    if order != *last_order {
        last_order.clone_from(&order);
        if !emit(sender, RoomListEvent::Order(order)).await {
            return false;
        }
    }
    for scope in sync.timeline_activity(heads) {
        if !emit(sender, RoomListEvent::TimelineActivity(scope)).await {
            return false;
        }
    }
    true
}

/// Runs until the consumer drops or the session is logged out. `on_ready` fires once, at the
/// first `Ready`. Await only on the backend runtime.
pub async fn run(
    sync: &mut RoomSync,
    sender: &mut Sender<RoomListEvent>,
    mut on_ready: impl FnMut(),
) {
    let mut trace = StatusTrace {
        shown: "connecting",
        since: std::time::Instant::now(),
    };
    let Some(client) = sync.client().cloned() else {
        let _ = emit(
            sender,
            RoomListEvent::Status(RoomSyncStatus::AuthenticationFailed),
        )
        .await;
        return;
    };
    sync.sliding_counts = true;
    // The session store may predate sliding sync: the version is not persisted with it.
    client.set_sliding_sync_version(Version::Native);
    // Before start, so no delivery is missed; the broadcast carries what classic /sync returned.
    let mut updates = client.subscribe_to_all_room_updates();
    let _ = client.event_cache().subscribe();
    let service = match SyncService::builder(client.clone())
        // The SDK itself pings /versions while offline and restarts: no restart of ours for it.
        .with_offline_mode()
        .build()
        .await
    {
        Ok(service) => service,
        Err(_) => {
            trace.change("sync_failed", "start_error");
            let _ = emit(sender, RoomListEvent::Status(RoomSyncStatus::SyncFailed)).await;
            return;
        }
    };
    let room_list = service.room_list_service();
    let Ok(list) = room_list.all_rooms().await else {
        let _ = emit(sender, RoomListEvent::Status(RoomSyncStatus::SyncFailed)).await;
        return;
    };
    let (alert_tx, mut alert_rx) = tokio::sync::mpsc::unbounded_channel();
    crate::notify::watch(&client, sync.account_id(), alert_tx).await;
    let mut loading = list.loading_state();
    let (entries, controller) = list.entries_with_dynamic_adapters(usize::from(u16::MAX));
    // Joined and invited rooms (not left or banned), sorted by the SDK.
    controller.set_filter(Box::new(new_filter_non_left()));
    tokio::pin!(entries);
    let mut listed = Vector::new();
    let mut last_order = Vec::new();
    let mut state = service.state();
    // Leaves `Error` only on a valid sliding response: the proof of success (`Running` of the
    // service is set at `start`, before any response).
    let mut list_state = room_list.state();
    service.start().await;
    use std::hash::{Hash, Hasher};
    let mut seed = std::collections::hash_map::DefaultHasher::new();
    crate::sending::fresh_transaction_id()
        .as_str()
        .hash(&mut seed);
    let mut backoff = crate::reconnect::Backoff::new(seed.finish());
    // Streams may only deliver changes: seed from the current values.
    let mut running = matches!(state.get(), State::Running);
    let mut loaded = matches!(loading.get(), RoomListLoadingState::Loaded { .. });
    let (mut ready, mut warmed) = (false, false);
    let mut gate = StatusGate::default();
    let mut cause = "";
    // Restart after Error/Terminated, with backoff, without blocking the loop.
    let mut restart_at: Option<Instant> = None;
    // SyncService stays Running while a silent server holds its requests open: probe it.
    let mut probe = tokio::time::interval(std::time::Duration::from_secs(5));
    probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let far = Instant::now() + std::time::Duration::from_secs(86_400);
        tokio::select! {
            _ = tokio::time::sleep_until(gate.wake(Instant::now()).unwrap_or(far)) => {}
            _ = tokio::time::sleep_until(restart_at.unwrap_or(far)), if restart_at.is_some() => {
                restart_at = None;
                service.start().await;
            }
            _ = probe.tick(), if running => {
                let answer = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    client.whoami(),
                )
                .await;
                let ok = answer.as_ref().is_ok_and(|answer| answer.is_ok());
                if !ok {
                    cause = if answer.is_err() { "probe_timeout" } else { "probe_error" };
                }
                gate.probe(Instant::now(), ok);
            }
            Some(alert) = alert_rx.recv() => {
                if !emit(sender, alert).await {
                    return;
                }
            }
            update = updates.recv() => {
                let heads = match update {
                    Ok(rooms) => sync.observe_joined(&rooms),
                    // Missed deliveries: rebuild previews from the store, activity resumes.
                    Err(RecvError::Lagged(_)) => {
                        let _ = sync.saved_batch().await;
                        Vec::new()
                    }
                    Err(RecvError::Closed) => return,
                };
                if !publish(sync, sender, &listed, &mut last_order, heads).await {
                    return;
                }
            }
            Some(diffs) = entries.next() => {
                for diff in diffs {
                    diff.apply(&mut listed);
                }
                if !publish(sync, sender, &listed, &mut last_order, Vec::new()).await {
                    return;
                }
            }
            Some(next) = list_state.next() => {
                if !matches!(next, RoomListState::Error { .. } | RoomListState::Terminated { .. } | RoomListState::Init) {
                    gate.succeed();
                    backoff.reset();
                }
            }
            Some(next) = state.next() => match next {
                State::Running => {
                    // Not a success: /versions can answer while sliding sync fails.
                    running = true;
                }
                State::Offline => {
                    (running, cause) = (false, "sync_service_offline");
                    gate.fail(Instant::now());
                }
                State::Error(_) | State::Terminated => {
                    running = false;
                    cause = if matches!(next, State::Error(_)) { "sync_service_error" } else { "sync_service_terminated" };
                    gate.fail(Instant::now());
                    // An expired token is the one failure that must not retry.
                    if let Err(error) = client.whoami().await
                        && matches!(
                            error.client_api_error_kind(),
                            Some(matrix_sdk::ruma::api::error::ErrorKind::UnknownToken { .. })
                        )
                    {
                        let _ = emit(
                            sender,
                            RoomListEvent::Status(RoomSyncStatus::AuthenticationFailed),
                        )
                        .await;
                        return;
                    }
                    restart_at = Some(Instant::now() + backoff.next_delay());
                }
                State::Idle => {}
            },
            Some(next) = loading.next() => {
                loaded = matches!(next, RoomListLoadingState::Loaded { .. });
                if loaded
                    && !publish(sync, sender, &listed, &mut last_order, Vec::new()).await
                {
                    return;
                }
            }
        }
        if gate.poll(Instant::now()) == Some(true) {
            ready = false;
            trace.change("sync_failed", cause);
            if !emit(sender, RoomListEvent::Status(RoomSyncStatus::SyncFailed)).await {
                return;
            }
        }
        if running && loaded && !ready && !gate.failed() {
            ready = true;
            trace.change(
                "ready",
                if trace.shown == "sync_failed" {
                    "recovered"
                } else {
                    "first_ready"
                },
            );
            if !emit(sender, RoomListEvent::Status(RoomSyncStatus::Ready)).await {
                return;
            }
            if !std::mem::replace(&mut warmed, true) {
                on_ready();
            }
        }
    }
}

/// One bounded sliding-sync round for callers that run before the room worker (key recovery on
/// a fresh sign-in): start, wait until the room list is loaded, stop. Never beside `run`.
pub(crate) async fn first_sync(client: &matrix_sdk::Client) -> Result<(), ()> {
    client.set_sliding_sync_version(Version::Native);
    let service = SyncService::builder(client.clone())
        .build()
        .await
        .map_err(|_| ())?;
    let list = service
        .room_list_service()
        .all_rooms()
        .await
        .map_err(|_| ())?;
    let mut loading = list.loading_state();
    service.start().await;
    let loaded = async {
        while let Some(next) = loading.next().await {
            if matches!(next, RoomListLoadingState::Loaded { .. }) {
                return true;
            }
        }
        false
    };
    let done = tokio::time::timeout(std::time::Duration::from_secs(30), loaded).await;
    service.stop().await;
    if matches!(done, Ok(true)) {
        Ok(())
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Instant, StatusGate};
    use std::time::Duration;

    #[derive(Clone, Copy)]
    enum In {
        Fail,
        Succeed,
        Probe(bool),
    }
    use In::*;

    /// Feeds `(second, input)` to a gate on a fake clock, polling every 100 ms (and so at every
    /// `wake`); returns the (second, failed) text changes.
    fn run(inputs: &[(u64, In)], end: u64) -> Vec<(u64, bool)> {
        let t0 = Instant::now();
        let mut gate = StatusGate::default();
        let mut changes = Vec::new();
        for ms in (0..=end * 1000).step_by(100) {
            let now = t0 + Duration::from_millis(ms);
            for (_, input) in inputs.iter().filter(|(s, _)| s * 1000 == ms) {
                match input {
                    Fail => gate.fail(now),
                    Succeed => gate.succeed(),
                    Probe(ok) => gate.probe(now, *ok),
                }
            }
            if let Some(failed) = gate.poll(now) {
                changes.push((ms / 1000, failed));
            }
            if let Some(at) = gate.wake(now) {
                assert!(at > now);
            }
        }
        changes
    }

    #[test]
    fn transitions_table() {
        let flaky: Vec<_> = (0..20)
            .map(|i| (i * 2, if i % 2 == 0 { Fail } else { Succeed }))
            .collect();
        let cases: Vec<(&str, Vec<(u64, In)>, Vec<(u64, bool)>)> = vec![
            ("outage 1 s", vec![(0, Fail), (1, Succeed)], vec![]),
            ("outage 3 s", vec![(0, Fail), (3, Succeed)], vec![]),
            ("outage 5 s", vec![(0, Fail), (5, Succeed)], vec![]),
            ("outage 19 s", vec![(0, Fail), (19, Succeed)], vec![]),
            (
                "outage 21 s: failed at 20, back held to 25",
                vec![(0, Fail), (21, Succeed)],
                vec![(20, true), (25, false)],
            ),
            (
                "outage 40 s",
                vec![(0, Fail), (40, Succeed)],
                vec![(20, true), (40, false)],
            ),
            ("single failures between successes", flaky, vec![]),
            (
                "3 consecutive failures",
                vec![(0, Fail), (1, Fail), (2, Fail), (30, Succeed)],
                vec![(2, true), (30, false)],
            ),
            (
                "2 failures only",
                vec![(0, Fail), (1, Fail), (3, Succeed)],
                vec![],
            ),
            (
                "3 failures, success at once: hold 5 s",
                vec![(0, Fail), (1, Fail), (2, Fail), (3, Succeed)],
                vec![(2, true), (7, false)],
            ),
            (
                "failed again inside the hold: no flip back",
                vec![
                    (0, Fail),
                    (0, Fail),
                    (0, Fail),
                    (1, Succeed),
                    (2, Fail),
                    (2, Fail),
                    (2, Fail),
                ],
                vec![(0, true)],
            ),
            (
                "one slow probe",
                vec![(5, Probe(false)), (10, Probe(true))],
                vec![],
            ),
            (
                "3 probe misses over 10 s",
                vec![
                    (5, Probe(false)),
                    (10, Probe(false)),
                    (15, Probe(false)),
                    (16, Probe(true)),
                ],
                vec![],
            ),
            (
                "silent server: 3 probe misses and 20 s",
                (1..=8).map(|i| (i * 5, Probe(false))).collect(),
                vec![(25, true)],
            ),
        ];
        for (name, inputs, want) in cases {
            assert_eq!(run(&inputs, 60), want, "{name}");
        }
    }
}
