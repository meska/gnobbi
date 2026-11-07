use matrix_desktop::{AccountRoutes, AccountState, Composer, RoomDrafts};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};
use matrix_engine::{
    RoomListDiff, RoomListEvent, RoomSummary, RoomSyncStatus, Sender, bounded,
    verification::VerificationEvent,
};
use std::num::NonZeroUsize;

fn scope(account: u64, room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new(room),
    }
}

fn upsert(account: u64, room: &str, name: &str) -> RoomListEvent {
    RoomListEvent::Diff(RoomListDiff::Upsert(Box::new(RoomSummary {
        scope: scope(account, room),
        name: name.to_owned(),
        unread: 0,
        info: Default::default(),
    })))
}

/// A state whose worker is replaced by the test: it returns the room-list sender.
fn account(capacity: usize) -> (AccountState, Sender<RoomListEvent>) {
    let (tx, rx) = bounded(NonZeroUsize::new(capacity).unwrap());
    let routes = AccountRoutes {
        events: Some(rx),
        ..Default::default()
    };
    (AccountState::new(AccountId::new(1), None, routes), tx)
}

#[test]
fn parked_account_keeps_syncing_and_its_worker_never_blocks() {
    let (mut shown, _shown_tx) = account(4);
    let (mut parked, mut tx) = account(4);
    parked.park();
    // The worker fills the whole route while nobody looks at this account.
    for event in [
        upsert(1, "!a:test", "Alpha"),
        upsert(1, "!b:test", "Beta"),
        RoomListEvent::Order(vec![scope(1, "!b:test"), scope(1, "!a:test")]),
        RoomListEvent::Status(RoomSyncStatus::Ready),
    ] {
        assert!(tx.try_send(event).is_ok());
    }
    assert!(
        tx.try_send(upsert(1, "!c:test", "Gamma")).is_err(),
        "route is full"
    );
    parked.pump();
    assert!(
        tx.try_send(upsert(1, "!c:test", "Gamma")).is_ok(),
        "pump freed the route"
    );
    parked.pump();
    let names: Vec<_> = parked
        .sidebar
        .rooms()
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names.len(), 3);
    assert!(parked.status == RoomSyncStatus::Ready);
    // Nothing leaked into the other account, which has the same engine account ID.
    shown.pump();
    assert!(shown.sidebar.rooms().is_empty());
    assert!(shown.status == RoomSyncStatus::Connecting);
}

#[test]
fn parked_account_learns_its_profile_and_a_closed_worker_is_noticed() {
    let (mut state, tx) = account(4);
    let (mut vtx, vrx) = bounded(NonZeroUsize::new(4).unwrap());
    let (cmd, _cmd_rx) = bounded(NonZeroUsize::new(4).unwrap());
    state.routes.verification = Some((cmd, vrx));
    assert!(
        vtx.try_send(VerificationEvent::Profile {
            account_id: AccountId::new(1),
            user_id: "@gnobbi-test-bob:test".into(),
            display_name: Some("gnobbi-test-bob".into()),
            device_id: Some("BOBDEV".into()),
            avatar: None,
        })
        .is_ok()
    );
    state.pump();
    assert_eq!(
        state.verification.profile().unwrap().0,
        "@gnobbi-test-bob:test"
    );
    state.status = RoomSyncStatus::Ready;
    drop(tx);
    state.pump();
    assert!(state.status == RoomSyncStatus::SyncFailed);
}

#[test]
fn leaving_the_screen_clears_search_selection_and_media_but_keeps_the_rooms() {
    let (mut state, mut tx) = account(8);
    assert!(tx.try_send(upsert(1, "!a:test", "Alpha")).is_ok());
    assert!(tx.try_send(upsert(1, "!b:test", "Beta")).is_ok());
    state.pump();
    state.sidebar.set_query("alpha");
    assert_eq!(state.sidebar.visible_indices().len(), 1);
    let target = ScopedTimelineId {
        scope: scope(1, "!a:test"),
        id: TimelineId::Server(EventId::new("$e:test")),
    };
    assert!(state.operations.select(target));
    state
        .media
        .request(scope(1, "!a:test"), EventId::new("$img:test"));
    assert!(state.media.loading() && state.operations.selection().is_some());

    state.park();

    assert_eq!(state.sidebar.query(), "");
    assert_eq!(
        state.sidebar.visible_indices().len(),
        2,
        "rooms are the account's own"
    );
    assert!(state.operations.selection().is_none());
    assert!(!state.media.loading());
    assert!(state.media.take_preview().is_none());
}

#[test]
fn drafts_follow_the_account_even_when_both_report_the_same_account_and_room_id() {
    // Workers report an ID that is only unique inside their own profile folder, so the window
    // keys drafts by the catalog ID. Account 1 and 2 are both in !shared (same engine ID 1).
    let (a, b) = (scope(1, "!shared:test"), scope(2, "!shared:test"));
    let placeholder = |id| scope(id, "!not-selected:local.invalid");
    let mut drafts = RoomDrafts::new(placeholder(1));
    let mut editor = Composer::default();
    drafts.switch(a.clone(), &mut editor);
    editor.replace(None, "bozza di alice");

    drafts.switch(placeholder(2), &mut editor); // account switch: nothing of alice stays
    assert_eq!(editor.text(), "");
    drafts.switch(b.clone(), &mut editor);
    assert_eq!(
        editor.text(),
        "",
        "bob does not see alice's draft in the same room"
    );
    editor.replace(None, "bozza di bob");

    drafts.switch(placeholder(1), &mut editor);
    assert_eq!(editor.text(), "");
    drafts.switch(a, &mut editor);
    assert_eq!(editor.text(), "bozza di alice");
    drafts.switch(b, &mut editor);
    assert_eq!(editor.text(), "bozza di bob");
}

fn room(account: u64, id: &str, unread: u64, mentions: u64) -> RoomListEvent {
    RoomListEvent::Diff(RoomListDiff::Upsert(Box::new(RoomSummary {
        scope: scope(account, id),
        name: id.to_owned(),
        unread,
        info: matrix_engine::RoomInfo {
            mentions,
            ..Default::default()
        },
    })))
}

#[test]
fn each_account_counts_its_own_unread_from_its_own_sync() {
    let (mut a, mut a_tx) = account(8);
    let (mut b, mut b_tx) = account(8);
    assert_eq!(a.unread(), (0, 0));
    assert!(a_tx.try_send(room(1, "!x:test", 3, 1)).is_ok());
    assert!(a_tx.try_send(room(1, "!y:test", 2, 0)).is_ok());
    assert!(b_tx.try_send(room(1, "!x:test", 7, 4)).is_ok());
    a.pump();
    b.pump();
    // Same engine account ID and room ID in both: nothing is shared.
    assert_eq!((a.unread(), b.unread()), ((5, 1), (7, 4)));
    // A later sync of one account moves only that account's badge.
    assert!(a_tx.try_send(room(1, "!x:test", 0, 0)).is_ok());
    a.pump();
    assert_eq!((a.unread(), b.unread()), ((2, 0), (7, 4)));
}

#[test]
fn an_alert_from_a_background_account_opens_that_room_when_the_account_is_shown() {
    let (mut state, mut tx) = account(8);
    state.notifications.set_enabled(true);
    assert!(tx.try_send(room(1, "!x:test", 0, 0)).is_ok());
    assert!(tx.try_send(room(1, "!y:test", 0, 0)).is_ok());
    state.pump();
    assert!(
        tx.try_send(RoomListEvent::Alert(scope(1, "!y:test")))
            .is_ok()
    );
    let alerts = state.pump();
    let (account_id, target) = alerts[0].target();
    assert_eq!((account_id, target), (None, &scope(1, "!y:test")));
    assert_ne!(
        state.sidebar.selected().map(|r| &r.scope),
        Some(target),
        "the room is not open yet"
    );
    assert!(state.open_room(target));
    assert_eq!(state.sidebar.selected().unwrap().scope, scope(1, "!y:test"));
    assert!(!state.open_room(&scope(1, "!gone:test")), "unknown room");
}
