use matrix_desktop::LiveMedia;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::media::{DownloadedPng, MediaCommand, MediaEvent, Preview};

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(26),
        room_id: RoomId::new("!media:test"),
    }
}

#[test]
fn completed_preview_rejects_duplicate_result_without_revealing_pixels() {
    let mut view = LiveMedia::default();
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    view.request(scope(), EventId::new("$png"));
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag, event, .. }) = receiver.try_recv() else {
        panic!("read required")
    };
    assert!(view.apply(MediaEvent::Downloaded {
        tag: tag.clone(),
        event: event.clone(),
        result: Err(matrix_engine::media::MediaError::Deadline)
    }));
    assert!(!view.apply(MediaEvent::Downloaded {
        tag,
        event,
        result: Ok(DownloadedPng {
            variant: matrix_engine::media::MediaVariant::Original,
            bytes: vec![1],
            preview: Preview {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
                anim: None,
            }
        })
    }));
    assert_eq!(
        view.error(),
        Some(matrix_engine::media::MediaError::Deadline)
    );
    assert!(view.take_preview().is_none());
}

#[test]
fn switching_room_cancels_queued_read_and_rejects_stale_pixels() {
    let mut view = LiveMedia::default();
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    view.request(scope(), EventId::new("$old"));
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag, event, .. }) = receiver.try_recv() else {
        panic!("read required");
    };
    view.clear();
    assert!(tag.is_cancelled());
    assert!(!view.apply(MediaEvent::Downloaded {
        tag,
        event,
        result: Err(matrix_engine::media::MediaError::Failed)
    }));
    assert!(!view.loading());
    assert!(view.error().is_none());
    assert!(view.take_preview().is_none());
}

#[test]
fn preview_preserves_request_when_backend_queue_is_full() {
    let mut view = LiveMedia::default();
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    view.request(scope(), EventId::new("$first"));
    view.flush(&mut sender);
    view.request(scope(), EventId::new("$second"));
    view.flush(&mut sender);
    let Ok(MediaCommand::Download {
        tag: stale, event, ..
    }) = receiver.try_recv()
    else {
        panic!("first queued read required");
    };
    assert_eq!(event.as_str(), "$first");
    assert!(stale.is_cancelled());
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag, event, .. }) = receiver.try_recv() else {
        panic!("full queue must not drop new request");
    };
    assert_eq!(event.as_str(), "$second");
    assert!(!tag.is_cancelled());
    assert!(!view.apply(MediaEvent::Downloaded {
        tag: tag.clone(),
        event: EventId::new("$wrong"),
        result: Err(matrix_engine::media::MediaError::Failed)
    }));
    assert!(view.loading());
    assert!(view.apply(MediaEvent::Downloaded {
        tag,
        event,
        result: Err(matrix_engine::media::MediaError::UnsupportedType)
    }));
    assert_eq!(
        view.error(),
        Some(matrix_engine::media::MediaError::UnsupportedType)
    );
    assert!(view.take_preview().is_none());
    view.request(scope(), EventId::new("$third"));
    drop(receiver);
    view.flush(&mut sender);
    assert_eq!(view.error(), Some(matrix_engine::media::MediaError::Failed));
    assert!(!view.loading());
}

#[test]
fn explicit_preview_request_accepts_only_matching_scoped_result() {
    let mut view = LiveMedia::default();
    view.request(scope(), EventId::new("$png"));
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag, event, .. }) = receiver.try_recv() else {
        panic!("explicit read required");
    };
    assert!(view.loading());
    assert!(view.apply(MediaEvent::Downloaded {
        tag,
        event,
        result: Ok(DownloadedPng {
            variant: matrix_engine::media::MediaVariant::Original,
            bytes: vec![1],
            preview: Preview {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
                anim: None,
            }
        })
    }));
    assert!(!view.loading());
    assert_eq!(view.take_preview().unwrap().rgba, [255, 0, 0, 255]);
}

#[test]
fn inline_loading_asks_for_one_image_at_a_time_and_never_repeats_a_settled_one() {
    let mut view = LiveMedia::default();
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    let (a, b) = (EventId::new("$a"), EventId::new("$b"));
    let order = || [b.clone(), a.clone()];
    assert!(view.next_inline(scope(), order()));
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag, event, .. }) = receiver.try_recv() else {
        panic!("first inline read required");
    };
    assert_eq!(event.as_str(), "$b");
    assert!(!view.next_inline(scope(), order()), "one read at a time");
    assert!(view.apply(MediaEvent::Downloaded {
        tag,
        event,
        result: Err(matrix_engine::media::MediaError::Failed)
    }));
    assert!(view.next_inline(scope(), order()));
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag, event, .. }) = receiver.try_recv() else {
        panic!("second inline read required");
    };
    assert_eq!(event.as_str(), "$a", "failed $b is not retried");
    assert!(view.apply(MediaEvent::Downloaded {
        tag,
        event,
        result: Ok(DownloadedPng {
            variant: matrix_engine::media::MediaVariant::Original,
            bytes: vec![1],
            preview: Preview {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
                anim: None,
            }
        })
    }));
    assert!(view.take_preview().is_some());
    assert!(!view.next_inline(scope(), order()), "all settled");
}

#[test]
fn large_view_original_is_its_own_lane_and_larger_than_the_thumbnail() {
    let png = |side: u32| DownloadedPng {
        variant: matrix_engine::media::MediaVariant::Original,
        bytes: vec![1],
        preview: Preview {
            width: side,
            height: side,
            rgba: vec![0; (side * side * 4) as usize],
            anim: None,
        },
    };
    let mut view = LiveMedia::default();
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(2).unwrap());
    view.request(scope(), EventId::new("$png"));
    view.request_original(scope(), EventId::new("$png"));
    assert!(view.original_pending());
    view.flush(&mut sender);
    let Ok(MediaCommand::Download {
        tag: large_tag,
        original: true,
        ..
    }) = receiver.try_recv()
    else {
        panic!("the original is asked first, with original set")
    };
    let Ok(MediaCommand::Download {
        tag,
        event,
        original: false,
    }) = receiver.try_recv()
    else {
        panic!("the bubble read is still asked, as a thumbnail")
    };
    assert!(view.apply(MediaEvent::Downloaded {
        tag,
        event: event.clone(),
        result: Ok(png(2)),
    }));
    let thumbnail = view.take_preview().unwrap();
    assert!(
        view.original_pending(),
        "bubble answer leaves the original pending"
    );
    assert!(view.apply(MediaEvent::Downloaded {
        tag: large_tag,
        event,
        result: Ok(png(8)),
    }));
    let (id, original) = view.take_original().unwrap();
    let original = original.unwrap();
    assert_eq!(id, EventId::new("$png"));
    assert!(original.width > thumbnail.width && original.height > thumbnail.height);
    assert!(!view.original_pending());
}

#[test]
fn second_original_replaces_the_first_and_closing_drops_the_running_read() {
    let png = || DownloadedPng {
        variant: matrix_engine::media::MediaVariant::Original,
        bytes: vec![1],
        preview: Preview {
            width: 8,
            height: 8,
            rgba: vec![0; 256],
            anim: None,
        },
    };
    let mut view = LiveMedia::default();
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(4).unwrap());
    view.request_original(scope(), EventId::new("$a"));
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag: first, .. }) = receiver.try_recv() else {
        panic!()
    };
    view.request_original(scope(), EventId::new("$b"));
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag: second, .. }) = receiver.try_recv() else {
        panic!()
    };
    assert!(
        first.is_cancelled(),
        "opening another view cancels the first read"
    );
    assert!(!view.apply(MediaEvent::Downloaded {
        tag: first,
        event: EventId::new("$a"),
        result: Ok(png())
    }));
    assert!(
        view.take_original().is_none(),
        "never two originals decoded at once"
    );
    view.cancel_original();
    assert!(second.is_cancelled(), "closing cancels the read");
    assert!(!view.apply(MediaEvent::Downloaded {
        tag: second,
        event: EventId::new("$b"),
        result: Ok(png())
    }));
    assert!(view.take_original().is_none() && !view.original_pending());
}

#[test]
fn a_video_row_asks_for_its_thumbnail_and_an_image_row_for_the_picture() {
    let mut view = LiveMedia::default();
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(2).unwrap());
    view.mark_videos([EventId::new("$video")]);
    view.request(scope(), EventId::new("$video"));
    view.flush(&mut sender);
    assert!(matches!(
        receiver.try_recv(),
        Ok(MediaCommand::VideoThumbnail { event, .. }) if event == EventId::new("$video")
    ));
    view.clear();
    view.request(scope(), EventId::new("$png"));
    view.flush(&mut sender);
    assert!(matches!(
        receiver.try_recv(),
        Ok(MediaCommand::Download {
            original: false,
            ..
        })
    ));
}
