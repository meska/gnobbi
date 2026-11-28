//! Cancellable media projection: one read at a time, images of the open room loaded on arrival.
use matrix_domain::{EventId, RoomScope};
use matrix_engine::{
    ViewRequests,
    media::{MediaCommand, MediaError, MediaEvent, OpenedAudio, Preview, VideoFile},
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
    /// The in-app audio player: its own lane, so a video or a save never cancels the note.
    audio_views: ViewRequests,
    audio: Option<EventId>,
    audio_command: Option<MediaCommand>,
    audio_done: Option<(EventId, Result<OpenedAudio, MediaError>)>,
    /// The auto download and the download button: one fetch into the engine's memory at a time, on its own lane.
    fetch_views: ViewRequests,
    fetching: Option<(EventId, bool)>,
    fetch_command: Option<MediaCommand>,
    /// Events whose original is in memory (until the engine drops it for room: then Open and Save fetch it again).
    fetched: HashSet<EventId>,
    /// Fetches that failed or were cancelled by the user: never started again by themselves.
    stopped: HashSet<EventId>,
    /// A fetch the user asked for failed (not cancelled): the app says so once.
    fetch_failed: bool,
    /// The download of a file (save or open) now running: `(file, bytes received, size the sender declared)`.
    transfer: Option<(EventId, u64, Option<u64>)>,
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
        self.cancel_audio();
        self.stop_fetch(false);
    }
    /// Fetch the original of `event` into memory (`max_bytes` at most). One at a time: a fetch the user asked for replaces
    /// the one the app started, and an automatic one waits for the lane and is never started again after it failed.
    pub fn request_fetch(&mut self, scope: RoomScope, event: EventId, max_bytes: usize, manual: bool) -> bool {
        if self.fetched.contains(&event) || (!manual && self.stopped.contains(&event)) {
            return false;
        }
        if self.fetching.as_ref().is_some_and(|(_, was_manual)| manual && !was_manual) {
            self.stop_fetch(false);
        }
        if self.fetching.is_some() {
            return false;
        }
        let Ok(tag) = self.fetch_views.start(scope) else {
            return false;
        };
        self.stopped.remove(&event);
        self.fetching = Some((event.clone(), manual));
        self.fetch_command = Some(MediaCommand::Prefetch { tag, event, max_bytes });
        true
    }
    /// Stop the running fetch; `declined` keeps it from starting again by itself (the user cancelled it).
    fn stop_fetch(&mut self, declined: bool) {
        if let Some((event, _)) = self.fetching.take() {
            if self.transfer.as_ref().map(|t| &t.0) == Some(&event) { self.transfer = None; }
            if declined { self.stopped.insert(event); }
        }
        self.fetch_views.cancel();
        self.fetch_command = None;
    }
    pub fn fetched(&self, event: &EventId) -> bool {
        self.fetched.contains(event)
    }
    /// A fetch that failed or was cancelled, and that only the user's button starts again.
    pub fn fetch_stopped(&self, event: &EventId) -> bool {
        self.stopped.contains(event)
    }
    pub fn fetching(&self, event: &EventId) -> bool {
        self.fetching.as_ref().is_some_and(|(e, _)| e == event)
    }
    pub fn fetched_ids(&self) -> HashSet<EventId> {
        self.fetched.clone()
    }
    pub fn stopped_ids(&self) -> HashSet<EventId> {
        self.stopped.clone()
    }
    /// How far the running save or open of a file has got.
    pub fn transfer(&self) -> Option<&(EventId, u64, Option<u64>)> {
        self.transfer.as_ref()
    }
    /// Stop the running download of a file, whichever lane it runs on.
    pub fn cancel_transfer(&mut self) {
        self.cancel_save();
        self.cancel_original();
        if self.fetching.as_ref().is_some_and(|(_, manual)| *manual) {
            self.stop_fetch(true);
        }
    }
    /// Download and decode the note of `event` for the in-app player, on its own lane.
    pub fn request_audio(&mut self, scope: RoomScope, event: EventId) {
        let Ok(tag) = self.audio_views.start(scope) else {
            return;
        };
        self.audio = Some(event.clone());
        self.audio_done = None;
        self.audio_command = Some(MediaCommand::OpenAudio { tag, event });
    }
    /// Stop a read that is still running: its answer, if it comes, is dropped.
    pub fn cancel_audio(&mut self) {
        self.audio_views.cancel();
        self.audio = None;
        self.audio_command = None;
        self.audio_done = None;
    }
    /// The finished decode (samples and waveform, or the error), once.
    pub fn take_audio(&mut self) -> Option<(EventId, Result<OpenedAudio, MediaError>)> {
        self.audio_done.take()
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
        if self.transfer.as_ref().map(|t| &t.0) == self.saving.as_ref() { self.transfer = None; }
        self.save_views.cancel();
        self.saving = None;
        self.save_command = None;
        self.save_done = None;
    }
    /// The finished save (ok or error), once.
    pub fn take_saved(&mut self) -> Option<(EventId, Result<(), MediaError>)> {
        self.save_done.take()
    }
    /// True once after a download the user asked for with the button failed.
    pub fn take_fetch_failed(&mut self) -> bool {
        std::mem::take(&mut self.fetch_failed)
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
        if self.transfer.as_ref().map(|t| &t.0) == self.large.as_ref() { self.transfer = None; }
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
        if let Some(command) = self.fetch_command.take() {
            match sender.try_send(command) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(c) | matrix_engine::TrySendError::Busy(c),
                ) => self.fetch_command = Some(c),
                Err(matrix_engine::TrySendError::Closed(_)) => self.stop_fetch(true),
            }
        }
        if let Some(command) = self.audio_command.take() {
            match sender.try_send(command) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(c) | matrix_engine::TrySendError::Busy(c),
                ) => self.audio_command = Some(c),
                Err(matrix_engine::TrySendError::Closed(_)) => {
                    self.audio_done = self.audio.take().map(|e| (e, Err(MediaError::Failed)));
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
                self.transfer = None;
                self.video_done = Some((event, result));
                return true;
            }
            MediaEvent::Saved { tag, event, result } => {
                if self.saving.as_ref() != Some(&event) || !self.save_views.accepts(&tag) {
                    return false;
                }
                self.saving = None;
                self.transfer = None;
                self.save_done = Some((event, result));
                return true;
            }
            MediaEvent::Prefetched { tag, event, result } => {
                if !self.fetching(&event) || !self.fetch_views.accepts(&tag) {
                    return false;
                }
                let manual = self.fetching.take().is_some_and(|(_, manual)| manual);
                if self.transfer.as_ref().map(|t| &t.0) == Some(&event) { self.transfer = None; }
                match result {
                    Ok(()) => { self.fetched.insert(event); }
                    Err(e) => {
                        self.fetch_failed |= manual && !matches!(e, MediaError::Cancelled);
                        self.stopped.insert(event);
                    }
                }
                return true;
            }
            MediaEvent::Progress { tag, event, received, total } => {
                // Only a fetch the user asked for shows its progress: the ones the app starts run quietly.
                let fetch = self.fetching.as_ref().is_some_and(|(e, manual)| *manual && *e == event) && self.fetch_views.accepts(&tag);
                let ours = fetch || (self.saving.as_ref() == Some(&event) && self.save_views.accepts(&tag)) || (self.large.as_ref() == Some(&event) && self.large_views.accepts(&tag));
                if ours {
                    self.transfer = Some((event, received, total));
                }
                return ours;
            }
            MediaEvent::AudioOpened { tag, event, result } => {
                if self.audio.as_ref() != Some(&event) || !self.audio_views.accepts(&tag) {
                    return false;
                }
                self.audio = None;
                self.audio_done = Some((event, result));
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

    fn audio_request() -> (LiveMedia, matrix_engine::RequestTag, EventId) {
        let mut media = LiveMedia::default();
        let event = EventId::new("$note:fixture.invalid");
        let scope = RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!r:fixture.invalid"),
        };
        media.request_audio(scope, event.clone());
        let Some(MediaCommand::OpenAudio { tag, .. }) = media.audio_command.take() else {
            panic!("an audio request asks the engine to open and decode the note");
        };
        (media, tag, event)
    }

    #[test]
    fn a_finished_note_keeps_its_samples_and_a_cancelled_one_drops_them() {
        let (mut media, tag, event) = audio_request();
        assert!(media.apply(MediaEvent::AudioOpened {
            tag,
            event: event.clone(),
            result: Ok(OpenedAudio {
                samples: vec![0.0; 8],
                sample_rate: 48_000,
                waveform: vec![0.5; 4],
            }),
        }));
        assert!(media
            .take_audio()
            .is_some_and(|(e, result)| e == event && result.is_ok()));
        assert!(media.take_audio().is_none());

        let (mut media, tag, event) = audio_request();
        media.cancel_audio();
        assert!(!media.apply(MediaEvent::AudioOpened {
            tag,
            event,
            result: Err(MediaError::Cancelled),
        }));
        assert!(media.take_audio().is_none());
    }

    #[test]
    fn a_closed_channel_reports_a_note_that_never_loaded() {
        let (mut media, tag, event) = audio_request();
        media.audio_command = Some(MediaCommand::OpenAudio {
            tag,
            event: event.clone(),
        });
        let (mut sender, receiver) =
            matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
        drop(receiver);
        media.flush(&mut sender);
        assert!(matches!(
            media.take_audio(),
            Some((e, Err(MediaError::Failed))) if e == event
        ));
    }

    fn fetch_scope() -> RoomScope {
        RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:fixture.invalid") }
    }

    fn started(media: &mut LiveMedia, event: &str, manual: bool) -> Option<(matrix_engine::RequestTag, usize)> {
        if !media.request_fetch(fetch_scope(), EventId::new(event), 1000, manual) {
            return None;
        }
        let Some(MediaCommand::Prefetch { tag, max_bytes, event: asked }) = media.fetch_command.take() else {
            panic!("a fetch asks the engine to prefetch");
        };
        assert_eq!(asked.as_str(), event);
        Some((tag, max_bytes))
    }

    #[test]
    fn a_fetch_runs_alone_and_a_finished_one_is_remembered_and_not_asked_again() {
        let mut media = LiveMedia::default();
        let (tag, max) = started(&mut media, "$a", false).unwrap();
        assert_eq!(max, 1000);
        assert!(started(&mut media, "$b", false).is_none(), "one at a time");
        assert!(media.apply(MediaEvent::Prefetched { tag, event: EventId::new("$a"), result: Ok(()) }));
        assert!(media.fetched(&EventId::new("$a")) && !media.fetching(&EventId::new("$a")));
        assert!(started(&mut media, "$a", true).is_none(), "already in memory");
        assert!(started(&mut media, "$b", false).is_some(), "the lane is free again");
    }

    #[test]
    fn a_failed_fetch_is_not_retried_by_itself_but_the_button_asks_again() {
        let mut media = LiveMedia::default();
        let (tag, _) = started(&mut media, "$a", false).unwrap();
        assert!(media.apply(MediaEvent::Prefetched { tag, event: EventId::new("$a"), result: Err(MediaError::TooLarge) }));
        assert!(media.fetch_stopped(&EventId::new("$a")) && !media.fetched(&EventId::new("$a")));
        assert!(!media.take_fetch_failed(), "an automatic fetch fails quietly");
        assert!(started(&mut media, "$a", false).is_none());
        let (tag, _) = started(&mut media, "$a", true).expect("the user's button starts it again");
        assert!(media.apply(MediaEvent::Prefetched { tag, event: EventId::new("$a"), result: Err(MediaError::Failed) }));
        assert!(media.take_fetch_failed() && !media.take_fetch_failed(), "the user's failed download is told once");
        let (tag, _) = started(&mut media, "$a", true).unwrap();
        assert!(media.apply(MediaEvent::Prefetched { tag, event: EventId::new("$a"), result: Err(MediaError::Cancelled) }));
        assert!(!media.take_fetch_failed(), "a cancelled one is not a failure");
        started(&mut media, "$a", true).unwrap();
        assert!(!media.fetch_stopped(&EventId::new("$a")));
    }

    #[test]
    fn a_fetch_the_user_asks_for_replaces_the_one_the_app_started_and_only_it_shows_progress() {
        let mut media = LiveMedia::default();
        let (quiet, _) = started(&mut media, "$auto", false).unwrap();
        let progress = |tag: &matrix_engine::RequestTag, event: &str| MediaEvent::Progress { tag: tag.clone(), event: EventId::new(event), received: 5, total: Some(10) };
        assert!(!media.apply(progress(&quiet, "$auto")) && media.transfer().is_none(), "the app's own fetches run quietly");
        let (shown, _) = started(&mut media, "$mine", true).unwrap();
        assert!(quiet.is_cancelled() && !media.fetching(&EventId::new("$auto")));
        assert!(!media.fetch_stopped(&EventId::new("$auto")), "it may start again later");
        assert!(media.apply(progress(&shown, "$mine")));
        assert_eq!(media.transfer().map(|t| (t.0.as_str(), t.1, t.2)), Some(("$mine", 5, Some(10))));
        // A late answer of the replaced one is dropped.
        assert!(!media.apply(MediaEvent::Prefetched { tag: quiet, event: EventId::new("$auto"), result: Ok(()) }));
        // Cancel: the transfer goes, and it does not start again by itself.
        media.cancel_transfer();
        assert!(shown.is_cancelled() && media.transfer().is_none() && media.fetch_stopped(&EventId::new("$mine")));
        assert!(!media.apply(MediaEvent::Prefetched { tag: shown, event: EventId::new("$mine"), result: Ok(()) }));
        assert!(!media.fetched(&EventId::new("$mine")));
    }

    #[test]
    fn cancelling_a_download_of_another_file_leaves_the_quiet_fetch_alone() {
        let mut media = LiveMedia::default();
        let (tag, _) = started(&mut media, "$auto", false).unwrap();
        media.cancel_transfer();
        assert!(!tag.is_cancelled() && media.fetching(&EventId::new("$auto")) && !media.fetch_stopped(&EventId::new("$auto")));
    }

    #[test]
    fn a_closed_channel_stops_the_fetch_and_leaving_the_room_cancels_it() {
        let mut media = LiveMedia::default();
        let (tag, _) = started(&mut media, "$a", false).unwrap();
        media.fetch_command = Some(MediaCommand::Prefetch { tag, event: EventId::new("$a"), max_bytes: 1 });
        let (mut sender, receiver) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
        drop(receiver);
        media.flush(&mut sender);
        assert!(!media.fetching(&EventId::new("$a")) && media.fetch_stopped(&EventId::new("$a")));
        let (tag, _) = started(&mut media, "$b", false).unwrap();
        media.clear();
        assert!(tag.is_cancelled() && !media.fetching(&EventId::new("$b")));
    }
}
