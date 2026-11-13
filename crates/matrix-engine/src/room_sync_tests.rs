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
    assert_eq!(room_label(Some(N::Empty), id), "Stanza vuota");
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
fn audio_info_takes_duration_size_voice_and_waveform_and_never_a_thumbnail() {
    let audio = |json: serde_json::Value| match serde_json::from_value::<MessageType>(json).unwrap() {
        MessageType::Audio(a) => audio_info(&a, vec![0.0, 0.5, 1.0]),
        _ => unreachable!(),
    };
    let got = audio(serde_json::json!({"msgtype":"m.audio","body":"a","url":"mxc://t/a","info":{"duration":3000,"size":10}}));
    assert_eq!((got.duration_ms, got.size, got.voice), (Some(3000), Some(10), false));
    assert_eq!(got.waveform, vec![0.0, 0.5, 1.0]);
    // The MSC3245 marker is what makes the row a voice note.
    let voice = audio(serde_json::json!({"msgtype":"m.audio","body":"a","url":"mxc://t/a","org.matrix.msc3245.voice":{}}));
    assert!(voice.voice);
    let bare = audio(serde_json::json!({"msgtype":"m.audio","body":"a","url":"mxc://t/a"}));
    assert_eq!((bare.duration_ms, bare.size), (None, None));
}

#[test]
fn nonempty_treats_missing_and_empty_avatar_as_absent() {
    use matrix_sdk::ruma::MxcUri;
    assert_eq!(nonempty(None), None);
    assert_eq!(nonempty(Some(<&MxcUri>::from(""))), None);
    assert_eq!(
        nonempty(Some(<&MxcUri>::from("mxc://s/abc"))).as_deref(),
        Some("mxc://s/abc")
    );
}

#[test]
fn unique_name_alone_ambiguous_keeps_id_missing_name_none() {
    let id = |n: &str| format!("@{n}:example.org");
    assert_eq!(
        super::disambiguated_name(Some("Anna"), true, &id("anna1")).as_deref(),
        Some("Anna (@anna1:example.org)")
    );
    assert_eq!(
        super::disambiguated_name(Some("Anna"), true, &id("anna2")).as_deref(),
        Some("Anna (@anna2:example.org)")
    );
    assert_eq!(
        super::disambiguated_name(Some("Anna"), false, &id("anna1")).as_deref(),
        Some("Anna")
    );
    assert_eq!(super::disambiguated_name(None, true, &id("x")), None);
}

mod poll_mapping {
    use super::super::poll_from_parts;
    use matrix_domain::Poll;

    fn poll(disclosed: bool, max: u64, ended: bool, own_event: bool, votes: &[(&str, &[&str])]) -> Poll {
        let votes = votes
            .iter()
            .map(|(a, u)| (a.to_string(), u.iter().map(|u| u.to_string()).collect()))
            .collect();
        poll_from_parts("Q", disclosed, max, [("a", "A"), ("b", "B"), ("c", "C")].into_iter(), &votes, ended, "@me:x", own_event)
    }

    #[test]
    fn distinct_voters_and_own_vote() {
        let p = poll(true, 2, false, false, &[("a", &["@me:x", "@u:x"]), ("b", &["@u:x"])]);
        assert_eq!(p.voters, 2);
        assert_eq!(p.answers.iter().map(|a| (a.votes, a.own)).collect::<Vec<_>>(), [(2, true), (1, false), (0, false)]);
        assert_eq!((p.multiple, p.max_selections, p.disclosed, p.ended, p.can_end), (true, 2, true, false, false));
        assert_eq!(p.question, "Q");
    }

    #[test]
    fn undisclosed_closed_and_can_end() {
        let open = poll(false, 1, false, true, &[]);
        assert!(!open.disclosed && !open.results_visible() && open.can_end);
        let closed = poll(false, 1, true, true, &[]);
        assert!(closed.ended && closed.results_visible() && !closed.can_end);
    }

    #[test]
    fn zero_max_selections_becomes_single() {
        let p = poll(true, 0, false, false, &[]);
        assert_eq!((p.max_selections, p.multiple), (1, false));
    }
}

fn msgtype_of(json: serde_json::Value) -> MessageType {
    serde_json::from_value(json).unwrap()
}

#[test]
fn attachment_caption_comes_from_the_sdk_not_from_the_file_name() {
    for kind in ["m.image", "m.video", "m.audio", "m.file"] {
        let with = |extra: serde_json::Value| {
            let mut j = serde_json::json!({"msgtype":kind,"url":"mxc://t/x"});
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            msgtype_of(j)
        };
        let t = with(serde_json::json!({"body":"Ecco i dati 😀","filename":"a.png"}));
        assert_eq!(caption_sources(&t).0, Some("Ecco i dati 😀"), "{kind}");
        let t = with(serde_json::json!({"body":"a.png","filename":"a.png"}));
        assert!(caption_sources(&t).0.is_none() && caption_sources(&t).1.is_none(), "{kind}: body = file name");
        let t = with(serde_json::json!({"body":"a.png"}));
        assert!(caption_sources(&t).0.is_none() && caption_sources(&t).1.is_none(), "{kind}: old style");
        let t = with(serde_json::json!({"body":"*x*","filename":"a.png","format":"org.matrix.custom.html","formatted_body":"<em>x</em>"}));
        assert_eq!(caption_sources(&t).1.map(|f| f.body.as_str()), Some("<em>x</em>"), "{kind}");
        let m = media_of(&t).unwrap();
        assert!(!m.label.contains("Ecco") && !m.label.is_empty(), "{kind}");
    }
    assert!(media_of(&msgtype_of(serde_json::json!({"msgtype":"m.text","body":"ciao"}))).is_none());
    assert!(caption_sources(&msgtype_of(serde_json::json!({"msgtype":"m.text","body":"ciao"}))).0.is_none());
    let image = media_of(&msgtype_of(serde_json::json!({"msgtype":"m.image","body":"x","filename":"a.png","url":"mxc://t/x"}))).unwrap();
    assert_eq!((image.kind, image.label.as_str()), (crate::history::MediaKind::Image, "Immagine"));
}

#[test]
fn latest_event_ts_is_none_for_no_event_and_for_invites() {
    use matrix_sdk::ruma::MilliSecondsSinceUnixEpoch;
    use matrix_sdk_ui::timeline::{LatestEventValue, TimelineDetails};
    assert_eq!(latest_event_ts(&LatestEventValue::None), None);
    let invite = LatestEventValue::RemoteInvite {
        timestamp: MilliSecondsSinceUnixEpoch(1_700_000_000_123_u64.try_into().unwrap()),
        inviter: None,
        inviter_profile: TimelineDetails::Unavailable,
    };
    assert_eq!(latest_event_ts(&invite), None, "invite stamp is client receipt time");
}
