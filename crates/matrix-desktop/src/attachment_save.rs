//! «Scarica» on received attachments: when a bubble offers it and what each save is doing.
use matrix_domain::{EventId, TimelineId};
use matrix_engine::media::MediaError;
use std::collections::HashMap;

/// A save that is not idle. A finished one leaves no trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveState {
    Running,
    Failed(MediaError),
}

/// Received, confirmed, not deleted, and with a file name: text, deleted and still-sending rows get none.
pub fn offers_save(row: &crate::DemoEvent, live: bool) -> bool {
    offers(
        live,
        &row.id.id,
        row.redacted,
        row.send_state.is_some(),
        row.attachment_name.is_some(),
    )
}

fn offers(live: bool, id: &TimelineId, redacted: bool, sending: bool, named: bool) -> bool {
    live && matches!(id, TimelineId::Server(_)) && !redacted && !sending && named
}

pub fn failure_label(error: MediaError) -> &'static str {
    match error {
        MediaError::TooLarge => "File troppo grande",
        _ => "Download non riuscito",
    }
}

/// The sender declared more than we save: say so before the picker opens.
pub fn over_cap(declared: Option<u64>) -> bool {
    declared.is_some_and(|size| size > matrix_engine::media::MAX_SAVE_BYTES as u64)
}

/// Over the cap another try gives the same answer.
pub fn retryable(error: MediaError) -> bool {
    error != MediaError::TooLarge
}

/// Where the picker opens: Downloads when it exists.
pub fn start_dir() -> std::path::PathBuf {
    let home = std::env::home_dir().unwrap_or_default();
    let downloads = home.join("Downloads");
    if downloads.is_dir() { downloads } else { home }
}

#[derive(Default)]
pub struct Saves(HashMap<EventId, SaveState>);

impl Saves {
    pub fn state(&self, event: &EventId) -> Option<SaveState> {
        self.0.get(event).copied()
    }
    /// False when already running.
    pub fn begin(&mut self, event: &EventId) -> bool {
        self.0.insert(event.clone(), SaveState::Running) != Some(SaveState::Running)
    }
    pub fn finish(&mut self, event: &EventId, result: Result<(), MediaError>) {
        match result {
            Ok(()) => self.0.remove(event),
            Err(error) => self.0.insert(event.clone(), SaveState::Failed(error)),
        };
    }
    /// The user stopped it (or closed the picker): back to idle.
    pub fn cancel(&mut self, event: &EventId) {
        self.0.remove(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_confirmed_live_undeleted_attachments_offer_save() {
        let server = TimelineId::Server(EventId::new("$e:fixture.invalid"));
        assert!(offers(true, &server, false, false, true));
        assert!(!offers(true, &server, false, false, false), "text only");
        assert!(!offers(false, &server, false, false, true), "demo");
        assert!(!offers(true, &server, true, false, true), "deleted");
        assert!(!offers(true, &server, false, true, true), "sending");
        let pending = TimelineId::Pending(matrix_domain::TransactionId::new("t"));
        assert!(!offers(true, &pending, false, false, true));
    }

    #[test]
    fn state_runs_clears_on_success_keeps_the_error_and_ignores_a_second_start() {
        let (mut saves, id) = (Saves::default(), EventId::new("$e"));
        assert!(saves.begin(&id));
        assert!(!saves.begin(&id));
        saves.finish(&id, Err(MediaError::Failed));
        assert_eq!(
            saves.state(&id),
            Some(SaveState::Failed(MediaError::Failed))
        );
        assert!(saves.begin(&id), "retry");
        saves.cancel(&id);
        assert_eq!(saves.state(&id), None);
        saves.begin(&id);
        saves.finish(&id, Ok(()));
        assert_eq!(saves.state(&id), None);
    }

    #[test]
    fn only_a_declared_size_over_the_cap_is_refused_up_front() {
        let cap = matrix_engine::media::MAX_SAVE_BYTES as u64;
        assert!(over_cap(Some(cap + 1)));
        assert!(!over_cap(Some(cap)) && !over_cap(None));
    }

    #[test]
    fn over_the_cap_says_so_and_is_not_retried() {
        assert_eq!(failure_label(MediaError::TooLarge), "File troppo grande");
        assert_eq!(failure_label(MediaError::Failed), "Download non riuscito");
        assert!(!retryable(MediaError::TooLarge) && retryable(MediaError::Failed));
    }
}
