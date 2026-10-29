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
    assert_eq!(
        room_label(Some(N::Aliased("#team:example.org".into())), id),
        "#team:example.org"
    );
    assert_eq!(
        room_label(Some(N::Calculated("Anna, Luca".into())), id),
        "Anna, Luca"
    );
    assert_eq!(room_label(Some(N::EmptyWas("Anna".into())), id), "Anna");
    assert_eq!(room_label(Some(N::Empty), id), id);
    assert_eq!(room_label(None, id), id);
}

fn label_of(json: serde_json::Value) -> String {
    message_label(&serde_json::from_value::<MessageType>(json).unwrap())
}

#[test]
fn non_text_message_types_get_real_labels() {
    let table = [
        (
            serde_json::json!({"msgtype":"m.video","body":"clip.mp4","filename":"clip.mp4","url":"mxc://t/v",
                "info":{"size":307200,"duration":12000,"mimetype":"video/mp4"},
                "com.beeper.disappearing_timer":{"type":"after_read","timer":60000}}),
            "Video · 0:12 · 300 KB",
        ),
        (
            serde_json::json!({"msgtype":"m.video","body":"v","url":"mxc://t/v"}),
            "Video",
        ),
        (
            serde_json::json!({"msgtype":"m.audio","body":"a.ogg","url":"mxc://t/a","org.matrix.msc3245.voice":{}}),
            "Nota vocale",
        ),
        (
            serde_json::json!({"msgtype":"m.audio","body":"a.mp3","url":"mxc://t/a"}),
            "Audio",
        ),
        (
            serde_json::json!({"msgtype":"m.audio","body":"a.ogg","url":"mxc://t/a","info":{"duration":75000},"org.matrix.msc3245.voice":{}}),
            "Nota vocale · 1:15",
        ),
        (
            serde_json::json!({"msgtype":"m.audio","body":"a.mp3","url":"mxc://t/a","info":{"duration":12000}}),
            "Audio · 0:12",
        ),
        (
            serde_json::json!({"msgtype":"m.file","body":"r.pdf","filename":"relazione.pdf","url":"mxc://t/f","info":{"size":2097152}}),
            "File · relazione.pdf · 2.0 MB",
        ),
        (
            serde_json::json!({"msgtype":"m.file","body":"r.pdf","url":"mxc://t/f"}),
            "File · r.pdf",
        ),
        (
            serde_json::json!({"msgtype":"m.location","body":"qui","geo_uri":"geo:1,2"}),
            "Posizione",
        ),
        (serde_json::json!({"msgtype":"m.foo","body":"x"}), "m.foo"),
        (
            serde_json::json!({"msgtype":"m.text","body":""}),
            "Messaggio senza testo",
        ),
        (
            serde_json::json!({"msgtype":"m.text","body":"ciao"}),
            "ciao",
        ),
    ];
    for (json, expected) in table {
        let label = label_of(json.clone());
        assert_eq!(label, expected, "{json}");
        assert!(!label.contains("supporto non disponibile"));
    }
}

#[test]
fn video_info_is_a_typed_value_for_every_combination() {
    use matrix_sdk::ruma::events::room::MediaSource;
    let video = |info: serde_json::Value| {
        let mut json = serde_json::json!({"msgtype":"m.video","body":"c.mp4","url":"mxc://t/v"});
        if !info.is_null() {
            json["info"] = info;
        }
        match serde_json::from_value::<MessageType>(json).unwrap() {
            MessageType::Video(v) => video_info(&v),
            _ => unreachable!(),
        }
    };
    let all = video(serde_json::json!({"size":307200,"duration":12000,
        "thumbnail_url":"mxc://t/th","thumbnail_info":{"mimetype":"image/jpeg"}}));
    assert_eq!(all.duration_ms, Some(12000));
    assert_eq!(all.size, Some(307200));
    assert!(matches!(all.thumbnail, Some(MediaSource::Plain(u)) if u.as_str() == "mxc://t/th"));
    let bare = video(serde_json::json!({"size":10}));
    assert_eq!(
        (bare.duration_ms, bare.size, bare.thumbnail.is_none()),
        (None, Some(10), true)
    );
    let no_size = video(serde_json::json!({"duration":500}));
    assert_eq!((no_size.duration_ms, no_size.size), (Some(500), None));
    let none = video(serde_json::Value::Null);
    assert_eq!(
        (none.duration_ms, none.size, none.thumbnail.is_none()),
        (None, None, true)
    );
}

#[test]
fn video_info_table_gives_the_values_the_bubble_shows() {
    // (info, duration_ms, size, has_thumbnail, row label)
    let table = [
        (
            serde_json::json!({"size":307200,"duration":75000,"thumbnail_url":"mxc://t/th",
                "thumbnail_info":{"mimetype":"image/jpeg"}}),
            Some(75000),
            Some(307200),
            true,
            "Video · 1:15 · 300 KB",
        ),
        (
            serde_json::json!({"size":3145728,"duration":12000}),
            Some(12000),
            Some(3145728),
            false,
            "Video · 0:12 · 3.0 MB",
        ),
        (
            serde_json::json!({"size":10,"thumbnail_url":"mxc://t/th"}),
            None,
            Some(10),
            true,
            "Video · 10 B",
        ),
        (
            serde_json::json!({"duration":500}),
            Some(500),
            None,
            false,
            "Video · 0:00",
        ),
        (serde_json::Value::Null, None, None, false, "Video"),
    ];
    for (info, duration_ms, size, thumb, label) in table {
        let mut json = serde_json::json!({"msgtype":"m.video","body":"c.mp4","url":"mxc://t/v"});
        if !info.is_null() {
            json["info"] = info.clone();
        }
        let msgtype = serde_json::from_value::<MessageType>(json).unwrap();
        let MessageType::Video(v) = &msgtype else {
            unreachable!()
        };
        let got = video_info(v);
        assert_eq!(
            (got.duration_ms, got.size, got.thumbnail.is_some()),
            (duration_ms, size, thumb),
            "{info}"
        );
        assert_eq!(message_label(&msgtype), label, "{info}");
    }
}

#[test]
fn audio_info_takes_duration_and_size_from_info_and_never_has_a_thumbnail() {
    let audio = |json: serde_json::Value| match serde_json::from_value::<MessageType>(json).unwrap() {
        MessageType::Audio(a) => audio_info(&a),
        _ => unreachable!(),
    };
    let got = audio(serde_json::json!({"msgtype":"m.audio","body":"a","url":"mxc://t/a","info":{"duration":3000,"size":10}}));
    assert_eq!((got.duration_ms, got.size, got.thumbnail.is_none()), (Some(3000), Some(10), true));
    let bare = audio(serde_json::json!({"msgtype":"m.audio","body":"a","url":"mxc://t/a"}));
    assert_eq!((bare.duration_ms, bare.size), (None, None));
}
