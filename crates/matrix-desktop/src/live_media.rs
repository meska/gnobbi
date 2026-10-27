//! Cancellable media projection: one read at a time, images of the open room loaded on arrival.
use matrix_domain::{EventId, RoomScope};
use matrix_engine::{
    ViewRequests,
    media::{MediaCommand, MediaError, MediaEvent, Preview},
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
    pub fn request(&mut self, scope: RoomScope, event: EventId) {
        let Ok(tag) = self.views.start(scope) else {
            return;
        };
        self.event = Some(event.clone());
        self.command = Some(MediaCommand::Download { tag, event });
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
        let MediaEvent::Downloaded { tag, event, result } = result else {
            return false;
        };
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
