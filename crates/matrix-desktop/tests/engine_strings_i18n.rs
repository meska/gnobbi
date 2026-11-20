//! Engine/domain texts shown in the UI are English catalog keys: translated at the call site (or by the
//! domain translator for notification bodies). One test per path, in the same process (global language).
use matrix_desktop::i18n::{self, Preference, t};
use matrix_domain::notifications::notification_body;
use matrix_engine::operations::{OperationError, space_error_line};
use matrix_engine::profile::ProfileError;

#[test]
fn engine_lines_and_notification_bodies_follow_the_language() {
    use ProfileError::*;
    let errors = [NameEmpty, NameTooLong, NotAllowed, FileUnreadable, FileTooLarge, FileInvalid, Network, Rejected];
    let lines = [OperationError::Forbidden, OperationError::Unavailable, OperationError::Transport].map(space_error_line);
    i18n::apply(Preference::Fixed("en"), None);
    assert_eq!(notification_body(None, true, false), "New messages");
    i18n::apply(Preference::Fixed("it"), None);
    for line in errors.map(ProfileError::message).into_iter().chain(lines) {
        assert_ne!(t(line), line, "missing it row: {line}");
    }
    assert_eq!(notification_body(None, true, false), "Nuovi messaggi");
    assert_eq!(notification_body(None, true, true), "1 nuovo messaggio");
    i18n::apply(Preference::Fixed("en"), None);
    assert_eq!(notification_body(None, true, false), "New messages");
}
