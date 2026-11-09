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
    /// «Scarica»: one save at a time on its own lane, so it never disturbs the reads above.
    save_views: ViewRequests,
    saving: Option<EventId>,
    save_command: Option<MediaCommand>,
    save_done: Option<(EventId, Result<(), MediaError>)>,
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
        self.cancel_save();
    }
    /// Save the original of `event` to `path`; false while another save runs.
    pub fn request_save(&mut self, scope: RoomScope, event: EventId, path: std::path::PathBuf) -> bool {
        if self.saving.is_some() {
            return false;
        }
        let Ok(tag) = self.save_views.start(scope) else {
            return false;
        };
        self.saving = Some(event.clone());
        self.save_command = Some(MediaCommand::Save { tag, event, path });
        true
    }
    /// Stop the running save; its answer, if it still comes, is dropped.
    pub fn cancel_save(&mut self) {
        self.save_views.cancel();
        self.saving = None;
        self.save_command = None;
        self.save_done = None;
    }
    /// The finished save (ok or error), once.
    pub fn take_saved(&mut self) -> Option<(EventId, Result<(), MediaError>)> {
        self.save_done.take()
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
                Err(matrix_engine::TrySendError::Closed(command)) => {
                    // A video that never reached the engine failed to play: it belongs to the
                    // bubble's play lane, not to the large view's picture lane.
                    match (command, self.large.take()) {
                        (MediaCommand::OpenVideo { .. }, Some(event)) => {
                            self.video_done = Some((event, Err(MediaError::Failed)));
                        }
                        (_, Some(event)) => {
                            self.large_done = Some((event, Err(MediaError::Failed)));
                        }
                        (_, None) => {}
                    }
                }
            }
        }
        if let Some(command) = self.save_command.take() {
            match sender.try_send(command) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(c) | matrix_engine::TrySendError::Busy(c),
                ) => self.save_command = Some(c),
                Err(matrix_engine::TrySendError::Closed(_)) => {
                    self.save_done = self.saving.take().map(|e| (e, Err(MediaError::Failed)));
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
            MediaEvent::Saved { tag, event, result } => {
                if self.saving.as_ref() != Some(&event) || !self.save_views.accepts(&tag) {
                    return false;
                }
                self.saving = None;
                self.save_done = Some((event, result));
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

    #[test]
    fn a_closed_channel_reports_a_video_that_never_started_on_its_own_lane() {
        let (mut media, tag, event) = video_request();
        // The helper hands over the tag; the command goes back on the lane it belongs to.
        media.large_command = Some(MediaCommand::OpenVideo {
            tag,
            event: event.clone(),
        });
        let (mut sender, receiver) =
            matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
        drop(receiver);
        media.flush(&mut sender);
        assert!(matches!(
            media.take_video(),
            Some((e, Err(MediaError::Failed))) if e == event
        ));
        assert!(media.take_original().is_none());
    }

    fn save_request(media: &mut LiveMedia) -> (matrix_engine::RequestTag, EventId) {
        let event = EventId::new("$file:fixture.invalid");
        let scope = RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!r:fixture.invalid"),
        };
        assert!(media.request_save(scope, event.clone(), "/tmp/x".into()));
        let Some(MediaCommand::Save { tag, path, .. }) = media.save_command.take() else {
            panic!("a save asks the engine to save");
        };
        assert_eq!(path, std::path::Path::new("/tmp/x"));
        (tag, event)
    }

    #[test]
    fn a_save_runs_alone_and_its_answer_is_taken_once() {
        let mut media = LiveMedia::default();
        let (tag, event) = save_request(&mut media);
        let scope = tag.scope().clone();
        assert!(!media.request_save(scope, EventId::new("$other"), "/tmp/y".into()), "one at a time");
        assert!(media.apply(MediaEvent::Saved { tag, event: event.clone(), result: Err(MediaError::TooLarge) }));
        assert!(matches!(media.take_saved(), Some((e, Err(MediaError::TooLarge))) if e == event));
        assert!(media.take_saved().is_none());
    }

    #[test]
    fn a_cancelled_save_drops_its_late_answer_and_frees_the_lane() {
        let mut media = LiveMedia::default();
        let (tag, event) = save_request(&mut media);
        let scope = tag.scope().clone();
        media.cancel_save();
        assert!(!media.apply(MediaEvent::Saved { tag, event, result: Ok(()) }));
        assert!(media.take_saved().is_none());
        assert!(media.request_save(scope, EventId::new("$again"), "/tmp/z".into()));
    }
}
