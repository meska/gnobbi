#![cfg(all(feature = "live", unix))]
use matrix_desktop::{FormStatus, SignInForm};
use matrix_engine::signin::{SignInCommand, SignInEvent, SignInMode, SignInPrompt};
#[test]
fn captured_prompt_source_cannot_submit_after_identical_prompt_replacement() {
    let mut form = SignInForm::default();
    let prompt = SignInPrompt {
        mode: SignInMode::Password,
        error: None,
    };
    form.apply(SignInEvent::Prompt(prompt));
    let old = form.source().unwrap();
    form.apply(SignInEvent::Prompt(prompt));
    form.password.replace(None, "fixture-password");
    form.phrase
        .replace(None, "local fixture phrase long enough");
    assert!(form.submit_source(old).is_none());
    assert!(!form.password.display_text().is_empty());
    let current = form.source().unwrap();
    assert_ne!(old, current);
    assert!(matches!(
        form.submit_source(current),
        Some(SignInCommand::Password { .. })
    ));
    assert!(form.submit_source(current).is_none());
}

#[test]
fn prompt_source_does_not_cross_form_instances_or_mode_roundtrips() {
    let mut first = SignInForm::default();
    let mut second = SignInForm::default();
    let password = SignInPrompt {
        mode: SignInMode::Password,
        error: None,
    };
    first.apply(SignInEvent::Prompt(password));
    second.apply(SignInEvent::Prompt(password));
    let old = first.source().unwrap();
    assert_ne!(Some(old), second.source());
    assert!(second.submit_source(old).is_none());
    first.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Unlock,
        error: None,
    }));
    assert!(first.submit_source(old).is_none());
    first.apply(SignInEvent::Prompt(password));
    assert!(first.submit_source(old).is_none());
    assert!(matches!(first.status(), FormStatus::Prompt(_)));
}

#[test]
fn validation_retry_retires_consumed_source_without_consuming_new_secrets() {
    let mut form = SignInForm::default();
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Password,
        error: None,
    }));
    let old = form.source().unwrap();
    assert!(form.submit_source(old).is_none());
    assert_ne!(form.source(), Some(old));
    form.password.replace(None, "fixture-password");
    form.phrase
        .replace(None, "local fixture phrase long enough");
    assert!(form.submit_source(old).is_none());
    assert!(!form.password.display_text().is_empty());
    let current = form.source().unwrap();
    assert!(form.submit_source(current).is_some());
}

#[test]
fn preedit_refusal_keeps_source_until_terminal_events_retire_it() {
    use matrix_engine::signin::SignInError;
    for event in [
        SignInEvent::Failed(SignInError::NoSession),
        SignInEvent::Authenticated(matrix_domain::AccountId::new(1)),
    ] {
        let mut form = SignInForm::default();
        assert_eq!(form.source(), None);
        form.apply(SignInEvent::Prompt(SignInPrompt {
            mode: SignInMode::Password,
            error: None,
        }));
        let source = form.source().unwrap();
        form.password.mark(None, "fixture", None);
        assert!(form.submit_source(source).is_none());
        assert_eq!(form.source(), Some(source));
        form.apply(event);
        assert_eq!(form.source(), None);
        assert!(form.submit_source(source).is_none());
    }
}

// Boundary probe of existing Secret validation and prompt retry behavior.
#[test]
fn form_refuses_submission_during_protected_ime_preedit() {
    let mut form = SignInForm::default();
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Password,
        error: None,
    }));
    form.password.mark(None, "preedit", None);
    form.phrase
        .replace(None, "local fixture phrase long enough");
    assert!(form.submit().is_none());
    assert_eq!(form.password.marked_utf16(), Some(0..7));
    assert!(matches!(form.status(), FormStatus::Prompt(_)));
}
#[test]
fn form_empty_secret_returns_a_fixed_validation_error_before_queueing() {
    use matrix_engine::signin::SignInError;
    for missing_password in [true, false] {
        let mut form = SignInForm::default();
        if missing_password {
            form.phrase.replace(None, "local unlock phrase");
        } else {
            form.password.replace(None, "fixture-password");
        }
        form.apply(SignInEvent::Prompt(SignInPrompt {
            mode: SignInMode::Password,
            error: None,
        }));
        assert!(form.submit().is_none());
        assert_eq!(
            form.status(),
            FormStatus::Prompt(SignInPrompt {
                mode: SignInMode::Password,
                error: Some(SignInError::InvalidSecret)
            })
        );
        assert_eq!(form.password.display_text(), "");
        assert_eq!(form.phrase.display_text(), "");
    }
}

#[test]
fn form_setup_failure_clears_secrets_and_cannot_be_submitted() {
    use matrix_engine::signin::SignInError;
    let mut form = SignInForm::default();
    form.password.replace(None, "fixture-password");
    form.phrase.replace(None, "local unlock phrase");
    form.apply(SignInEvent::Failed(SignInError::ProfileBusy));
    assert_eq!(form.status(), FormStatus::Failed(SignInError::ProfileBusy));
    assert_eq!(form.password.display_text(), "");
    assert_eq!(form.phrase.display_text(), "");
    assert!(form.submit().is_none());
}

#[test]
fn form_success_clears_secret_buffers_and_prevents_another_submission() {
    use matrix_domain::AccountId;
    let mut form = SignInForm::default();
    form.password.replace(None, "fixture-password");
    form.phrase.replace(None, "local unlock phrase");
    form.apply(SignInEvent::Authenticated(AccountId::new(2)));
    assert_eq!(form.status(), FormStatus::Authenticated(AccountId::new(2)));
    assert_eq!(form.password.display_text(), "");
    assert_eq!(form.phrase.display_text(), "");
    assert!(form.submit().is_none());
}

#[test]
fn form_unlock_does_not_require_or_submit_a_server_password() {
    let mut form = SignInForm::default();
    form.password.replace(None, "must not be submitted");
    form.phrase.replace(None, "restore unlock phrase");
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Unlock,
        error: None,
    }));
    assert!(matches!(form.submit(), Some(SignInCommand::Unlock { .. })));
    assert_eq!(form.status(), FormStatus::Busy);
    assert_eq!(form.password.display_text(), "");
    assert_eq!(form.phrase.display_text(), "");
}

#[test]
fn form_consumes_credentials_once_only_after_a_backend_prompt() {
    let mut form = SignInForm::default();
    form.homeserver = "https://fixture.invalid".into();
    form.user = "@alice:fixture.invalid".into();
    form.password.replace(None, "fixture-password");
    form.phrase.replace(None, "local unlock phrase");
    assert!(form.submit().is_none());
    assert_eq!(form.status(), FormStatus::Opening);
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Password,
        error: None,
    }));
    assert!(matches!(
        form.submit(),
        Some(SignInCommand::Password { .. })
    ));
    assert_eq!(form.status(), FormStatus::Busy);
    assert_eq!(form.password.display_text(), "");
    assert_eq!(form.phrase.display_text(), "");
    assert!(form.submit().is_none());
    assert_eq!(form.homeserver, "https://fixture.invalid");
    assert_eq!(form.user, "@alice:fixture.invalid");
}
