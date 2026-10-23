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
    let Ok(MediaCommand::Download { tag, event }) = receiver.try_recv() else {
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
            bytes: vec![1],
            preview: Preview {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255]
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
    let Ok(MediaCommand::Download { tag, event }) = receiver.try_recv() else {
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
    let Ok(MediaCommand::Download { tag: stale, event }) = receiver.try_recv() else {
        panic!("first queued read required");
    };
    assert_eq!(event.as_str(), "$first");
    assert!(stale.is_cancelled());
    view.flush(&mut sender);
    let Ok(MediaCommand::Download { tag, event }) = receiver.try_recv() else {
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
    let Ok(MediaCommand::Download { tag, event }) = receiver.try_recv() else {
        panic!("explicit read required");
    };
    assert!(view.loading());
    assert!(view.apply(MediaEvent::Downloaded {
        tag,
        event,
        result: Ok(DownloadedPng {
            bytes: vec![1],
            preview: Preview {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255]
            }
        })
    }));
    assert!(!view.loading());
    assert_eq!(view.take_preview().unwrap().rgba, [255, 0, 0, 255]);
}
