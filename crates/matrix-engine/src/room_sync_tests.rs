use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn activity_sync() -> RoomSync {
    let mut sync = RoomSync::new(AccountSession {
        account_id: matrix_domain::AccountId::new(55),
        client: None,
        #[cfg(unix)]
        store: None,
    });
    let scope = RoomScope {
        account_id: matrix_domain::AccountId::new(55),
        room_id: RoomId::new("!activity:test"),
    };
    sync.previous.insert(
        scope.room_id.as_str().to_owned(),
        RoomSummary {
            scope,
            name: String::new(),
            unread: 0,
            info: Default::default(),
        },
    );
    sync
}

#[test]
fn repeated_timeline_head_does_not_repeat_activity() {
    let mut sync = activity_sync();
    let update = || {
        vec![(
            "!activity:test".to_owned(),
            matrix_domain::EventId::new("$one"),
        )]
    };
    assert_eq!(sync.timeline_activity(update()).len(), 1);
    assert!(
        sync.timeline_activity(update()).is_empty(),
        "duplicate sync delivery must be idempotent"
    );
}

#[test]
fn departed_room_retires_timeline_head() {
    let mut sync = activity_sync();
    let update = || {
        vec![(
            "!activity:test".to_owned(),
            matrix_domain::EventId::new("$one"),
        )]
    };
    assert_eq!(sync.timeline_activity(update()).len(), 1);
    let joined = sync.previous.clone();
    sync.previous.clear();
    assert!(sync.timeline_activity(update()).is_empty());
    assert!(
        sync.timeline_heads.is_empty(),
        "departed rooms must not accumulate heads"
    );
    sync.previous = joined;
    assert_eq!(
        sync.timeline_activity(update()).len(),
        1,
        "rejoined room must invalidate again"
    );
}

#[test]
fn timeline_activity_uses_only_current_account_joined_rooms() {
    let mut sync = activity_sync();
    let scopes = sync.timeline_activity(vec![
        ("!activity:test".into(), matrix_domain::EventId::new("$one")),
        ("!foreign:test".into(), matrix_domain::EventId::new("$two")),
    ]);
    assert_eq!(
        scopes,
        vec![RoomScope {
            account_id: matrix_domain::AccountId::new(55),
            room_id: RoomId::new("!activity:test"),
        }]
    );
    assert_eq!(sync.timeline_heads.len(), 1);
    assert!(sync.timeline_activity(Vec::new()).is_empty());
    let new = sync.timeline_activity(vec![(
        "!activity:test".into(),
        matrix_domain::EventId::new("$three"),
    )]);
    assert_eq!(
        new, scopes,
        "new tail invalidates even with unchanged summary"
    );
    let mut other = activity_sync();
    other.previous.values_mut().next().unwrap().scope.account_id =
        matrix_domain::AccountId::new(56);
    let scopes = other.timeline_activity(vec![(
        "!activity:test".into(),
        matrix_domain::EventId::new("$one"),
    )]);
    assert_eq!(
        scopes[0].account_id,
        matrix_domain::AccountId::new(56),
        "heads are owned by one session, not shared across accounts"
    );
}

#[test]
fn closed_activity_route_returns_unsent_scope() {
    runtime().block_on(async {
        let (mut sender, receiver) = crate::bounded(std::num::NonZeroUsize::new(1).unwrap());
        drop(receiver);
        let scope = activity_sync().previous.values().next().unwrap().scope.clone();
        let result = sender.send(RoomListEvent::TimelineActivity(scope.clone())).await;
        assert!(matches!(result, Err(crate::TrySendError::Closed(RoomListEvent::TimelineActivity(actual))) if actual == scope));
    });
}

#[test]
fn shutdown_reports_unconfirmed_media_accepted_before_login_finishes() {
    let (mut worker, _status) = RoomListWorker::spawn(|| {
        std::future::pending::<Result<AccountSession, crate::auth::AuthError>>()
    })
    .unwrap();
    let (mut commands, _results) = worker.take_media_route().unwrap();
    assert!(
        commands
            .try_send(crate::media::MediaCommand::Send {
                scope: RoomScope {
                    account_id: matrix_domain::AccountId::new(26),
                    room_id: RoomId::new("!media:test")
                },
                transaction: matrix_domain::TransactionId::new("p026-unstarted"),
                bytes: vec![1],
            })
            .is_ok()
    );
    assert_eq!(worker.shutdown(), Err(ShutdownError::WriteUnconfirmed));
}

#[test]
fn shutdown_interrupts_the_maximum_reconnect_delay() {
    runtime().block_on(async {
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let mut retry = crate::reconnect::Backoff::new(22);
        for _ in 0..100 {
            retry.next_delay();
        }
        let delay = retry.next_delay();
        assert!(delay >= Duration::from_secs(15));
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            signal.store(true, Ordering::Release);
        });
        let start = Instant::now();
        assert!(
            until_stopped(&stop, tokio::time::sleep(delay))
                .await
                .is_none()
        );
        assert!(start.elapsed() < Duration::from_millis(500));
    });
}

#[test]
fn shutdown_interrupts_a_full_status_route_without_dropping_accepted_items() {
    runtime().block_on(async {
        let (mut sender, mut receiver) = crate::bounded(std::num::NonZeroUsize::new(1).unwrap());
        sender.send(22).await.unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            signal.store(true, Ordering::Release);
        });
        let start = Instant::now();
        assert!(until_stopped(&stop, sender.send(23)).await.is_none());
        assert!(start.elapsed() < Duration::from_millis(500));
        assert_eq!(receiver.recv().await, Some(22));
        drop(sender);
        assert_eq!(receiver.recv().await, None);
    });
}

#[test]
fn shutdown_receive_closes_intake_before_draining_with_a_live_producer() {
    runtime().block_on(async {
        let stop = AtomicBool::new(true);
        let (mut sender, mut receiver) = crate::bounded(std::num::NonZeroUsize::new(2).unwrap());
        sender.send(22).await.unwrap();
        assert_eq!(shutdown_receive(&stop, &mut receiver).await, Some(22));
        assert_eq!(sender.send(23).await, Err(crate::TrySendError::Closed(23)));
        assert_eq!(shutdown_receive(&stop, &mut receiver).await, None);
    });
}

#[test]
fn room_label_prefers_the_computed_name_and_uses_the_id_last() {
    use matrix_sdk::RoomDisplayName as N;
    let id = "!AHaFoh:example.org";
    assert_eq!(room_label(Some(N::Named("Team".into())), id), "Team");
    assert_eq!(room_label(Some(N::Aliased("#team:example.org".into())), id), "#team:example.org");
    assert_eq!(room_label(Some(N::Calculated("Anna, Luca".into())), id), "Anna, Luca");
    assert_eq!(room_label(Some(N::EmptyWas("Anna".into())), id), "Anna");
    assert_eq!(room_label(Some(N::Empty), id), id);
    assert_eq!(room_label(None, id), id);
}

#[test]
fn preview_follows_real_messages_and_ignores_edits_and_deleted_ones() {
    let msg = |id: &str, content: &str, extra: &str| {
        format!(r#"{{"type":"m.room.message","event_id":"{id}","sender":"@a:t","content":{content}{extra}}}"#)
    };
    let set = |id: &str, text: &str| PreviewChange::Set { event_id: id.into(), text: text.into() };
    assert_eq!(preview_change(&msg("$1", r#"{"msgtype":"m.text","body":"ciao"}"#, "")), set("$1", "ciao"));
    assert_eq!(preview_change(&msg("$2", r#"{"msgtype":"m.notice","body":"avviso"}"#, "")), set("$2", "avviso"));
    assert_eq!(preview_change(&msg("$3", r#"{"msgtype":"m.image","body":"IMG_1.png"}"#, "")), set("$3", "Immagine"));
    assert_eq!(preview_change(&msg("$4", r#"{"msgtype":"m.file","body":"a.pdf"}"#, "")), set("$4", "a.pdf"));
    let edit = r#"{"msgtype":"m.text","body":"* x","m.relates_to":{"rel_type":"m.replace","event_id":"$1"}}"#;
    assert_eq!(preview_change(&msg("$5", edit, "")), PreviewChange::Ignore);
    assert_eq!(preview_change(&msg("$6", "{}", r#","unsigned":{"redacted_because":{}}"#)), PreviewChange::Ignore);
    assert_eq!(preview_change(r#"{"type":"m.room.redaction","event_id":"$7","redacts":"$1","content":{}}"#),
        PreviewChange::Remove { target: "$1".into() });
    assert_eq!(preview_change(r#"{"type":"m.room.redaction","event_id":"$8","content":{"redacts":"$2"}}"#),
        PreviewChange::Remove { target: "$2".into() });
    assert_eq!(preview_change(r#"{"type":"m.room.member","event_id":"$9","content":{}}"#), PreviewChange::Ignore);
    assert_eq!(preview_change("not json"), PreviewChange::Ignore);
}
