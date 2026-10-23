//! Explicit, cancellable single-preview projection. No automatic remote loading.
use matrix_domain::{EventId, RoomScope};
use matrix_engine::{
    ViewRequests,
    media::{MediaCommand, MediaError, MediaEvent, Preview},
};

#[derive(Default)]
pub struct LiveMedia {
    views: ViewRequests,
    event: Option<EventId>,
    command: Option<MediaCommand>,
    loading: bool,
    error: Option<MediaError>,
    preview: Option<Preview>,
}
impl LiveMedia {
    pub fn clear(&mut self) {
        self.views.cancel();
        self.event = None;
        self.command = None;
        self.loading = false;
        self.error = None;
        self.preview = None;
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
        match result {
            Ok(download) => self.preview = Some(download.preview),
            Err(error) => self.error = Some(error),
        }
        true
    }
}
