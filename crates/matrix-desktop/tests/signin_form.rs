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

fn vault_form(mode: SignInMode) -> SignInForm {
    let mut form = SignInForm::default();
    form.vault = true;
    form.apply(SignInEvent::Prompt(SignInPrompt { mode, error: None }));
    form
}

#[test]
fn vault_form_submits_a_new_profile_with_only_the_server_password() {
    let mut form = vault_form(SignInMode::Password);
    form.homeserver = "https://fixture.invalid".into();
    form.user = "@alice:fixture.invalid".into();
    form.password.replace(None, "fixture-password");
    assert!(matches!(
        form.submit(),
        Some(SignInCommand::VaultPassword { .. })
    ));
    assert_eq!(form.status(), FormStatus::Busy);
    assert_eq!(form.password.display_text(), "");
}

#[test]
fn vault_form_without_a_password_shows_a_password_only_message() {
    use matrix_engine::signin::SignInError;
    let mut form = vault_form(SignInMode::Password);
    form.phrase.replace(None, "stray phrase must not count");
    assert!(form.submit().is_none());
    assert_eq!(
        form.status(),
        FormStatus::Prompt(SignInPrompt {
            mode: SignInMode::Password,
            error: Some(SignInError::InvalidSecret)
        })
    );
    assert_eq!(form.error_message(), Some("Inserisci la password Matrix."));
}

#[test]
fn vault_unlock_needs_no_secret_and_sends_the_vault_command() {
    let mut form = vault_form(SignInMode::UnlockVault);
    assert!(matches!(form.submit(), Some(SignInCommand::UnlockVault)));
    assert_eq!(form.status(), FormStatus::Busy);
}

#[test]
fn recovery_step_can_be_deferred_once_by_the_current_source_only() {
    use matrix_engine::signin::RecoveryState;
    let mut form = vault_form(SignInMode::Password);
    form.apply(SignInEvent::Authenticated(matrix_domain::AccountId::new(1)));
    assert_eq!(form.source(), None);
    form.apply(SignInEvent::Recovery(RecoveryState::Offered));
    assert_eq!(form.recovery(), Some(RecoveryState::Offered));
    let source = form.source().unwrap();
    form.apply(SignInEvent::Recovery(RecoveryState::Unsupported));
    assert!(form.defer_source(source).is_none());
    let current = form.source().unwrap();
    assert!(matches!(
        form.defer_source(current),
        Some(SignInCommand::DeferRecovery)
    ));
    assert!(form.defer_source(current).is_none());
    assert_eq!(form.source(), None);
}

#[test]
fn visible_fields_follow_the_flow() {
    use matrix_desktop::visible_fields;
    assert_eq!(visible_fields(SignInMode::Password, true), &[0, 1, 2]);
    assert_eq!(visible_fields(SignInMode::Password, false), &[0, 1, 2, 3]);
    assert_eq!(visible_fields(SignInMode::Unlock, true), &[3]);
    assert_eq!(
        visible_fields(SignInMode::UnlockVault, true),
        &[] as &[usize]
    );
}

#[test]
fn vault_failures_get_their_own_actionable_messages() {
    use matrix_engine::signin::{SignInError, VaultError::*};
    let message = |error| {
        FormStatus::Failed(SignInError::Vault(error))
            .error_message()
            .unwrap()
    };
    for error in [Unavailable, Locked] {
        let text = message(error);
        assert!(
            text.to_lowercase().contains("portachiavi"),
            "{error:?}: {text}"
        );
        assert!(!text.contains("percorso del profilo"), "{error:?}: {text}");
    }
    for error in [Missing, InvalidItem, AlreadyExists] {
        assert!(message(error).contains("chiave del profilo"), "{error:?}");
    }
    assert_ne!(message(Unavailable), message(Missing));
}

fn offered_form() -> SignInForm {
    use matrix_engine::signin::RecoveryState;
    let mut form = SignInForm::default();
    form.vault = true;
    form.apply(SignInEvent::Authenticated(matrix_domain::AccountId::new(1)));
    form.apply(SignInEvent::Recovery(RecoveryState::Offered));
    form
}

#[test]
fn recovery_key_is_sent_once_and_the_step_waits_in_pending() {
    use matrix_engine::signin::RecoveryState;
    let mut form = offered_form();
    form.phrase.replace(None, "EsTu fixture key words");
    let source = form.source().unwrap();
    assert!(matches!(
        form.recover_source(source),
        Some(SignInCommand::EnterRecovery(_))
    ));
    assert_eq!(form.recovery(), Some(RecoveryState::Pending));
    assert_eq!(form.source(), None);
    assert!(
        form.phrase.display_text().is_empty(),
        "the key leaves the editor"
    );
    assert!(form.recover_source(source).is_none());
}

#[test]
fn empty_recovery_key_fails_locally_and_allows_a_retry() {
    use matrix_engine::signin::{RecoveryFailure, RecoveryState};
    let mut form = offered_form();
    let source = form.source().unwrap();
    assert!(form.recover_source(source).is_none());
    assert_eq!(
        form.recovery(),
        Some(RecoveryState::Failed(RecoveryFailure::InvalidKey))
    );
    assert!(form.error_message().unwrap().contains("Chiave errata"));
    let retry = form.source().unwrap();
    assert_ne!(retry, source);
    form.phrase.replace(None, "second attempt key");
    assert!(matches!(
        form.recover_source(retry),
        Some(SignInCommand::EnterRecovery(_))
    ));
}

#[test]
fn every_recovery_failure_has_its_own_message_and_keeps_skip_and_retry() {
    use matrix_engine::signin::{RecoveryFailure::*, RecoveryState};
    let mut seen = std::collections::HashSet::new();
    for failure in [InvalidKey, Network, Timeout, BackupUnavailable, Other] {
        let mut form = offered_form();
        form.apply(SignInEvent::Recovery(RecoveryState::Failed(failure)));
        let message = form.error_message().expect("typed message");
        assert!(seen.insert(message), "{failure:?} shares its message");
        let source = form.source().expect("retry and skip stay possible");
        assert!(matches!(
            form.defer_source(source),
            Some(SignInCommand::DeferRecovery)
        ));
    }
    let mut form = offered_form();
    form.apply(SignInEvent::Recovery(RecoveryState::Unsupported));
    assert!(form.error_message().unwrap().contains("archivio segreto"));
    let source = form.source().unwrap();
    form.phrase.replace(None, "ignored");
    assert!(
        form.recover_source(source).is_none(),
        "no key route without secret storage"
    );
    assert_eq!(form.recovery(), Some(RecoveryState::Unsupported));
}

#[test]
fn recovery_wording_names_the_matrix_key_and_the_backup_limit() {
    assert!(matrix_desktop::RECOVERY_NOTE.contains("chiave di recupero Matrix"));
    assert!(matrix_desktop::RECOVERY_NOTE.contains("non è la frase locale"));
    assert!(matrix_desktop::RECOVERY_NOTE.contains("non tutti i vecchi messaggi"));
    assert_eq!(matrix_desktop::RECOVER_LABEL, "Recupera cronologia cifrata");
}

mod recovery_setup {
    use matrix_desktop::SetupPhase;
    use matrix_engine::recovery::RecoverySecret;
    use matrix_engine::signin::{
        RecoveryFailure, RecoveryState, SetupEvent, SetupFailure, SignInCommand, SignInEvent,
    };
    const KEY: &str = "EsTc fixture KEY 0000 1111 2222";

    fn absent_form() -> matrix_desktop::SignInForm {
        let mut form = matrix_desktop::SignInForm::default();
        form.vault = true;
        form.apply(SignInEvent::Authenticated(matrix_domain::AccountId::new(1)));
        form.apply(SignInEvent::Recovery(RecoveryState::Offered));
        form.apply(SignInEvent::Recovery(RecoveryState::Unsupported));
        form
    }
    fn shown_form() -> matrix_desktop::SignInForm {
        let mut form = absent_form();
        let source = form.source().unwrap();
        assert!(matches!(
            form.setup_source(source),
            Some(SignInCommand::SetupRecovery)
        ));
        form.apply_setup(SetupEvent::Pending);
        form.apply_setup(SetupEvent::Ready(RecoverySecret::fixture(KEY)));
        form
    }

    #[test]
    fn setup_is_offered_only_when_the_account_has_no_secret_storage() {
        for state in [
            RecoveryState::Offered,
            RecoveryState::Failed(RecoveryFailure::InvalidKey),
            RecoveryState::Failed(RecoveryFailure::BackupUnavailable),
        ] {
            let mut form = absent_form();
            form.apply(SignInEvent::Recovery(RecoveryState::Offered));
            form.apply(SignInEvent::Recovery(state));
            let source = form.source().unwrap();
            assert!(!form.setup_offered(), "{state:?}");
            assert!(form.setup_source(source).is_none(), "{state:?}");
        }
        let mut form = absent_form();
        assert!(form.setup_offered());
        let source = form.source().unwrap();
        assert!(matches!(
            form.setup_source(source),
            Some(SignInCommand::SetupRecovery)
        ));
        assert!(
            form.setup_source(source).is_none(),
            "one activation, one command"
        );
        form.apply_setup(SetupEvent::Pending);
        assert_eq!(form.setup_phase(), Some(SetupPhase::Pending));
        assert_eq!(
            form.source(),
            None,
            "nothing is actionable while the setup runs"
        );
    }

    #[test]
    fn the_key_is_shown_once_and_continuing_needs_the_explicit_confirmation() {
        let mut form = shown_form();
        assert_eq!(form.setup_phase(), Some(SetupPhase::KeyShown));
        assert_eq!(form.recovery_key(), Some(KEY));
        assert_eq!(
            form.error_message(),
            None,
            "no stale message next to a shown key"
        );
        let source = form.source().unwrap();
        assert!(
            form.defer_source(source).is_none(),
            "skipping is not the confirmation"
        );
        assert!(
            form.setup_source(source).is_none(),
            "no second key while one is shown"
        );
        assert_eq!(form.recovery_key(), Some(KEY));
        assert!(matches!(
            form.confirm_saved_source(source),
            Some(SignInCommand::DeferRecovery)
        ));
        assert_eq!(
            form.recovery_key(),
            None,
            "the key leaves the form on confirmation"
        );
        assert!(form.confirm_saved_source(source).is_none());
        assert_eq!(form.source(), None);
    }

    #[test]
    fn a_stale_source_cannot_confirm_and_confirming_needs_a_shown_key() {
        let mut form = absent_form();
        let source = form.source().unwrap();
        assert!(
            form.confirm_saved_source(source).is_none(),
            "no key, nothing to confirm"
        );
        let mut form = shown_form();
        let old = form.source().unwrap();
        form.apply_setup(SetupEvent::Ready(RecoverySecret::fixture(KEY)));
        assert!(form.confirm_saved_source(old).is_none());
        assert_eq!(form.recovery_key(), Some(KEY));
    }

    #[test]
    fn transient_failures_allow_retry_and_permanent_ones_only_continue_without() {
        for failure in [
            SetupFailure::Network,
            SetupFailure::Timeout,
            SetupFailure::Other,
        ] {
            let mut form = absent_form();
            let source = form.source().unwrap();
            form.setup_source(source).unwrap();
            form.apply_setup(SetupEvent::Failed(failure));
            assert_eq!(form.setup_phase(), Some(SetupPhase::Failed(failure)));
            assert!(form.error_message().is_some());
            assert!(form.setup_offered(), "{failure:?} can be retried");
            let source = form.source().unwrap();
            assert!(matches!(
                form.setup_source(source),
                Some(SignInCommand::SetupRecovery)
            ));
        }
        let mut messages = std::collections::HashSet::new();
        for failure in [
            SetupFailure::Partial,
            SetupFailure::IdentityRequired,
            SetupFailure::AuthRequired,
        ] {
            let mut form = absent_form();
            let source = form.source().unwrap();
            form.setup_source(source).unwrap();
            form.apply_setup(SetupEvent::Failed(failure));
            assert!(
                messages.insert(form.error_message().unwrap()),
                "{failure:?}"
            );
            assert!(
                !form.setup_offered(),
                "{failure:?} cannot be fixed by retrying"
            );
            let source = form.source().unwrap();
            assert!(form.setup_source(source).is_none());
            assert!(matches!(
                form.defer_source(source),
                Some(SignInCommand::DeferRecovery)
            ));
        }
        let mut form = absent_form();
        form.apply_setup(SetupEvent::Unsupported);
        assert!(!form.setup_offered());
        assert!(form.error_message().unwrap().contains("backup"));
    }

    #[test]
    fn an_existing_recovery_turns_the_offer_into_the_restore_route() {
        let mut form = absent_form();
        let source = form.source().unwrap();
        form.setup_source(source).unwrap();
        form.apply_setup(SetupEvent::Failed(SetupFailure::AlreadyExists));
        assert_eq!(form.recovery(), Some(RecoveryState::Offered));
        assert!(!form.setup_offered());
        assert!(form.error_message().unwrap().contains("già"));
        let source = form.source().unwrap();
        form.phrase.replace(None, "existing recovery key fixture");
        assert!(matches!(
            form.recover_source(source),
            Some(SignInCommand::EnterRecovery(_))
        ));
    }

    #[test]
    fn labels_keep_setup_distinct_from_restore() {
        assert_ne!(matrix_desktop::SETUP_LABEL, matrix_desktop::RECOVER_LABEL);
        assert!(matrix_desktop::SETUP_LABEL.contains("Crea"));
        assert!(matrix_desktop::SAVED_LABEL.contains("salvato"));
        assert!(matrix_desktop::SETUP_KEY_NOTE.contains("una sola volta"));
    }
}
