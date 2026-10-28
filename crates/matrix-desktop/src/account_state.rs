//! What belongs to one signed-in account while the window shows it, or while it waits in the
//! background: the worker routes and the projections fed by them. A parked account keeps
//! syncing (`pump`) so no bounded route ever fills up and stalls its worker.
use crate::{DeviceVerification, LiveDelivery, LiveMedia, LiveOperations, Sidebar};
use matrix_domain::{
    AccountId,
    notifications::{Notification, NotificationPolicy},
};
use matrix_engine::{
    Receiver, RoomListDiff, RoomListEvent, RoomSyncStatus, Sender,
    history::{HistoryRequest, HistoryResult},
    media::{MediaCommand, MediaEvent},
    operations::{OperationCommand, OperationResult},
    sending::{SendRequest, SendResult},
    verification::{VerificationCommand, VerificationEvent},
};
use std::path::PathBuf;

type Route<C, E> = Option<(Sender<C>, Receiver<E>)>;

/// Every channel to one account's worker. Dropping it does not stop the worker.
#[derive(Default)]
pub struct AccountRoutes {
    pub events: Option<Receiver<RoomListEvent>>,
    pub media: Route<MediaCommand, MediaEvent>,
    pub verification: Route<VerificationCommand, VerificationEvent>,
    pub operation: Route<OperationCommand, OperationResult>,
    pub send: Route<SendRequest, SendResult>,
    pub history: Route<HistoryRequest, HistoryResult>,
}

#[derive(Default)]
pub struct Drained {
    pub events: Vec<RoomListEvent>,
    pub media: Vec<MediaEvent>,
    pub verifications: Vec<VerificationEvent>,
    pub operations: Vec<OperationResult>,
    pub deliveries: Vec<SendResult>,
    pub pages: Vec<HistoryResult>,
    /// The room-list route is closed: the worker is gone.
    pub closed: bool,
}

fn take<E>(route: &mut Receiver<E>, max: usize, out: &mut Vec<E>) {
    out.extend(std::iter::from_fn(|| route.try_recv().ok()).take(max));
}

impl AccountRoutes {
    /// Non-blocking, bounded per kind (the per-tick limits of the window).
    pub fn drain(&mut self) -> Drained {
        let mut d = Drained::default();
        if let Some((_, r)) = &mut self.media {
            take(r, 2, &mut d.media)
        }
        if let Some((_, r)) = &mut self.verification {
            take(r, 8, &mut d.verifications)
        }
        if let Some((_, r)) = &mut self.operation {
            take(r, 8, &mut d.operations)
        }
        if let Some((_, r)) = &mut self.send {
            take(r, 8, &mut d.deliveries)
        }
        if let Some((_, r)) = &mut self.history {
            take(r, 8, &mut d.pages)
        }
        if let Some(route) = &mut self.events {
            for _ in 0..32 {
                match route.try_recv() {
                    Ok(event) => d.events.push(event),
                    Err(matrix_engine::TryReceiveError::Closed) => {
                        d.closed = true;
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
        d
    }
}

pub struct AccountState {
    /// The id the worker's session reports; room scopes of this account carry it.
    pub account_id: AccountId,
    /// Profile folder and trust root of this account.
    pub root: Option<(PathBuf, Option<PathBuf>)>,
    pub routes: AccountRoutes,
    pub sidebar: Sidebar,
    pub status: RoomSyncStatus,
    pub notifications: NotificationPolicy,
    pub verification: DeviceVerification,
    pub delivery: LiveDelivery,
    pub operations: LiveOperations,
    pub media: LiveMedia,
}

impl AccountState {
    pub fn new(
        account_id: AccountId,
        root: Option<(PathBuf, Option<PathBuf>)>,
        routes: AccountRoutes,
    ) -> Self {
        Self {
            account_id,
            root,
            routes,
            sidebar: Sidebar::connected(account_id),
            status: RoomSyncStatus::Connecting,
            notifications: NotificationPolicy::new(account_id),
            verification: DeviceVerification::new(account_id),
            delivery: LiveDelivery::new(),
            operations: LiveOperations::default(),
            media: LiveMedia::default(),
        }
    }

    /// The account leaves the screen: nothing of it may show in the next view. Its sidebar keeps
    /// the rooms (they are the account's own), but no search text, picture or message selection.
    pub fn park(&mut self) {
        self.sidebar.set_query("");
        self.media.clear();
        self.operations.clear_selection();
    }

    /// One tick of a parked account: apply what its worker sent and send what is queued.
    /// Returns the alerts for new unread messages.
    pub fn pump(&mut self) -> Vec<Notification> {
        let d = self.routes.drain();
        let mut alerts = Vec::new();
        for result in d.operations {
            if !matches!(result, OperationResult::Members { .. }) {
                self.operations.apply(result);
            }
        }
        for event in d.media {
            self.media.apply(event);
        }
        for event in d.verifications {
            self.verification.apply(event);
        }
        for result in d.deliveries {
            self.delivery.apply(result);
        }
        for event in d.events {
            match event {
                RoomListEvent::Diff(diff) => {
                    match &diff {
                        RoomListDiff::Upsert(room) => {
                            alerts.extend(self.notifications.update(&room.scope, room.unread));
                        }
                        RoomListDiff::Remove(scope) => self.notifications.remove(scope),
                    }
                    self.sidebar.apply_diff(diff);
                }
                RoomListEvent::Order(order) => self.sidebar.set_order(order),
                RoomListEvent::Status(status) => {
                    if status == RoomSyncStatus::AuthenticationFailed {
                        self.notifications.set_enabled(false);
                    }
                    self.status = status;
                }
                RoomListEvent::TimelineActivity(_) => {}
            }
        }
        if d.closed
            && matches!(
                self.status,
                RoomSyncStatus::Connecting | RoomSyncStatus::Ready
            )
        {
            self.status = RoomSyncStatus::SyncFailed;
        }
        if let Some((sender, _)) = &mut self.routes.media {
            self.media.flush(sender);
        }
        if let Some((sender, _)) = &mut self.routes.verification {
            self.verification.flush(sender);
        }
        if let Some((sender, _)) = &mut self.routes.send {
            self.delivery.flush(sender);
        }
        alerts
    }
}
