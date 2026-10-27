//! Sliding-sync adapter: the official `SyncService` drives sync and `RoomListService` says when
//! the room list is loaded; the same `RoomListEvent`s as the classic loop come out. Sliding sync
//! only (MSC4186), no classic fallback.
use crate::{RoomListEvent, RoomSyncStatus, Sender, room_sync::RoomSync};
use eyeball_im::Vector;
use futures_util::StreamExt;
use matrix_sdk::sliding_sync::Version;
use matrix_sdk_ui::{
    room_list_service::{RoomListItem, RoomListLoadingState, filters::new_filter_non_left},
    sync_service::{State, SyncService},
};
use tokio::sync::broadcast::error::RecvError;

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
    let service = match SyncService::builder(client.clone()).build().await {
        Ok(service) => service,
        Err(_) => {
            let _ = emit(sender, RoomListEvent::Status(RoomSyncStatus::SyncFailed)).await;
            return;
        }
    };
    let Ok(list) = service.room_list_service().all_rooms().await else {
        let _ = emit(sender, RoomListEvent::Status(RoomSyncStatus::SyncFailed)).await;
        return;
    };
    let mut loading = list.loading_state();
    let (entries, controller) = list.entries_with_dynamic_adapters(usize::from(u16::MAX));
    // Joined and invited rooms (not left or banned), sorted by the SDK.
    controller.set_filter(Box::new(new_filter_non_left()));
    tokio::pin!(entries);
    let mut listed = Vector::new();
    let mut last_order = Vec::new();
    let mut state = service.state();
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
    // SyncService stays Running while a silent server holds its requests open: probe it.
    let mut probe = tokio::time::interval(std::time::Duration::from_secs(5));
    probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Last probe failed: the next success lets `Ready` come back.
    let mut unreachable = false;
    loop {
        tokio::select! {
            _ = probe.tick(), if running => {
                let reachable = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    client.whoami(),
                )
                .await
                .is_ok_and(|answer| answer.is_ok());
                unreachable = !reachable;
                if unreachable && ready {
                    ready = false;
                    if !emit(sender, RoomListEvent::Status(RoomSyncStatus::SyncFailed)).await {
                        return;
                    }
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
            Some(next) = state.next() => match next {
                State::Running => {
                    backoff.reset();
                    running = true;
                }
                State::Error(_) | State::Terminated | State::Offline => {
                    running = false;
                    ready = false;
                    if !emit(sender, RoomListEvent::Status(RoomSyncStatus::SyncFailed)).await {
                        return;
                    }
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
                    tokio::time::sleep(backoff.next_delay()).await;
                    service.start().await;
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
        if running && loaded && !ready && !unreachable {
            ready = true;
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
    let list = service.room_list_service().all_rooms().await.map_err(|_| ())?;
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
    if matches!(done, Ok(true)) { Ok(()) } else { Err(()) }
}
