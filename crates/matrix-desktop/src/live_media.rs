//! Cancellable media projection: one read at a time, images of the open room loaded on arrival.
use matrix_domain::{EventId, RoomScope};
use matrix_engine::{
    ViewRequests,
    media::{MediaCommand, MediaError, MediaEvent, Preview, VideoFile},
};
use std::collections::HashSet;

#[derive(Default)]
pub struct LiveMedia {
    views: ViewRequests,
    event: Option<EventId>,
    command: Option<MediaCommand>,
    loading: bool,
    error: Option<MediaError>,
    preview: Option<Preview>,
    /// Events already answered (picture or error); never asked again.
    settled: HashSet<EventId>,
    /// Candidates that are videos: their bubble asks for the thumbnail only.
    videos: HashSet<EventId>,
    /// The large view's original: its own request lane, so it never disturbs the bubble reads.
    large_views: ViewRequests,
    large: Option<EventId>,
    large_command: Option<MediaCommand>,
    large_done: Option<(EventId, Result<Preview, MediaError>)>,
    video_done: Option<(EventId, Result<VideoFile, MediaError>)>,
    /// The file the system player was opened on; deleted when replaced or when the app exits.
    playing: Option<VideoFile>,
}
impl LiveMedia {
    pub fn clear(&mut self) {
        self.views.cancel();
        self.event = None;
        self.command = None;
        self.loading = false;
        self.error = None;
        self.preview = None;
        self.settled.clear();
        self.videos.clear();
        self.cancel_original();
    }
    /// Ask for the full-resolution picture of `event`, replacing any earlier original request.
    pub fn request_original(&mut self, scope: RoomScope, event: EventId) {
        let Ok(tag) = self.large_views.start(scope) else {
            return;
        };
        self.large = Some(event.clone());
        self.large_done = None;
        self.large_command = Some(MediaCommand::Download {
            tag,
            event,
            original: true,
        });
    }
    /// Ask for the original video of `event` as a private temp file, on the large view's lane.
    pub fn request_video(&mut self, scope: RoomScope, event: EventId) {
        let Ok(tag) = self.large_views.start(scope) else {
            return;
        };
        self.large = Some(event.clone());
        self.large_done = None;
        self.video_done = None;
        self.large_command = Some(MediaCommand::OpenVideo { tag, event });
    }
    /// The finished video download (file or error), once.
    pub fn take_video(&mut self) -> Option<(EventId, Result<VideoFile, MediaError>)> {
        self.video_done.take()
    }
    pub fn hold_playing(&mut self, file: VideoFile) {
        self.playing = Some(file);
    }
    pub fn cancel_original(&mut self) {
        self.large_views.cancel();
        self.large = None;
        self.large_done = None;
        self.video_done = None;
        self.large_command = None;
    }
    pub fn original_pending(&self) -> bool {
        self.large.is_some()
    }
    /// The finished original (picture or error), once.
    pub fn take_original(&mut self) -> Option<(EventId, Result<Preview, MediaError>)> {
        self.large_done.take()
    }
    /// Ask for the first candidate not yet answered, unless a read is in flight.
    pub fn next_inline(
        &mut self,
        scope: RoomScope,
        candidates: impl IntoIterator<Item = EventId>,
    ) -> bool {
        if self.loading || self.command.is_some() {
            return false;
        }
        let Some(event) = candidates.into_iter().find(|e| !self.settled.contains(e)) else {
            return false;
        };
        self.request(scope, event);
        self.loading
    }
    pub fn event(&self) -> Option<&EventId> {
        self.event.as_ref()
    }
    pub fn mark_videos(&mut self, events: impl IntoIterator<Item = EventId>) {
        self.videos.extend(events);
    }
    pub fn request(&mut self, scope: RoomScope, event: EventId) {
        let Ok(tag) = self.views.start(scope) else {
            return;
        };
        self.event = Some(event.clone());
        self.command = Some(if self.videos.contains(&event) {
            MediaCommand::VideoThumbnail { tag, event }
        } else {
            MediaCommand::Download {
                tag,
                event,
                original: false,
            }
        });
        self.loading = true;
        self.error = None;
        self.preview = None;
    }
    pub fn loading(&self) -> bool {
        self.loading
    }
    pub fn error(&self) -> Option<MediaError> {
        self.error
    }
    pub fn take_preview(&mut self) -> Option<Preview> {
        self.preview.take()
    }
    pub fn flush(&mut self, sender: &mut matrix_engine::Sender<MediaCommand>) {
        if let Some(command) = self.large_command.take() {
            match sender.try_send(command) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(c) | matrix_engine::TrySendError::Busy(c),
                ) => self.large_command = Some(c),
                Err(matrix_engine::TrySendError::Closed(_)) => {
                    self.large_done = self.large.take().map(|e| (e, Err(MediaError::Failed)));
                }
            }
        }
        if let Some(command) = self.command.take() {
            match sender.try_send(command) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(c) | matrix_engine::TrySendError::Busy(c),
                ) => self.command = Some(c),
                Err(matrix_engine::TrySendError::Closed(_)) => {
                    self.error = Some(MediaError::Failed);
                    self.loading = false;
                    self.settled.extend(self.event.clone());
                }
            }
        }
    }
    pub fn apply(&mut self, result: MediaEvent) -> bool {
        let (tag, event, result) = match result {
            MediaEvent::Downloaded { tag, event, result } => (tag, event, result),
            MediaEvent::VideoDownloaded { tag, event, result } => {
                if self.large.as_ref() != Some(&event) || !self.large_views.accepts(&tag) {
                    return false;
                }
                self.large = None;
                self.video_done = Some((event, result));
                return true;
            }
            _ => return false,
        };
        if self.large.as_ref() == Some(&event) && self.large_views.accepts(&tag) {
            self.large = None;
            self.large_done = Some((event, result.map(|download| download.preview)));
            return true;
        }
        if !self.loading || !self.views.accepts(&tag) || self.event.as_ref() != Some(&event) {
            return false;
        }
        self.loading = false;
        self.settled.insert(event);
        match result {
            Ok(download) => self.preview = Some(download.preview),
            Err(error) => self.error = Some(error),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};

    fn video_request() -> (LiveMedia, matrix_engine::RequestTag, EventId) {
        let mut media = LiveMedia::default();
        let event = EventId::new("$clip:fixture.invalid");
        let scope = RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!r:fixture.invalid"),
        };
        media.request_video(scope, event.clone());
        let Some(MediaCommand::OpenVideo { tag, .. }) = media.large_command.take() else {
            panic!("a video request asks the engine to open the video");
        };
        (media, tag, event)
    }

    #[test]
    fn a_finished_video_download_is_taken_once_and_an_error_is_kept() {
        let (mut media, tag, event) = video_request();
        assert!(media.original_pending());
        assert!(media.apply(MediaEvent::VideoDownloaded {
            tag,
            event: event.clone(),
            result: Err(MediaError::TooLarge),
        }));
        assert!(!media.original_pending());
        assert!(matches!(
            media.take_video(),
            Some((e, Err(MediaError::TooLarge))) if e == event
        ));
        assert!(media.take_video().is_none());
    }

    #[test]
    fn closing_the_view_drops_a_download_that_is_still_running() {
        let (mut media, tag, event) = video_request();
        media.cancel_original();
        assert!(!media.original_pending());
        assert!(!media.apply(MediaEvent::VideoDownloaded {
            tag,
            event,
            result: Err(MediaError::Cancelled),
        }));
        assert!(media.take_video().is_none());
    }
}
