//! UI-only sign-in state; storage and authentication stay on the backend.
use crate::LoginInput;
use crate::i18n::t;
use matrix_engine::signin::{
    RecoveryFailure, RecoveryState, Secret, SetupEvent, SetupFailure, SignInCommand, SignInEvent,
    SignInMode, SignInPrompt,
};
pub(crate) fn recovery_message(state: RecoveryState) -> Option<&'static str> {
    Some(match state {
        RecoveryState::Failed(RecoveryFailure::InvalidKey) => {
            t("Wrong key. Check it and try again.")
        }
        RecoveryState::Failed(RecoveryFailure::Network) => {
            t("Server unreachable. Try again later or continue without recovery.")
        }
        RecoveryState::Failed(RecoveryFailure::Timeout) => {
            t("Recovery took too long. Try again or continue without recovery.")
        }
        RecoveryState::Failed(RecoveryFailure::BackupUnavailable) => {
            t("No key backup on the server. Continue without recovery.")
        }
        RecoveryState::Failed(RecoveryFailure::Other) => {
            t("Recovery failed. Try again or continue without recovery.")
        }
        RecoveryState::Unsupported => {
            t("This account has no Matrix secret storage: no recovery key to enter.")
        }
        _ => return None,
    })
}
pub(crate) fn setup_message(phase: SetupPhase) -> Option<&'static str> {
    Some(match phase {
        SetupPhase::Pending => t("Creating the recovery and backup…"),
        SetupPhase::KeyShown => return None,
        SetupPhase::Unsupported => {
            t("The server offers no key backup: no recovery to create. Continue without recovery.")
        }
        SetupPhase::Failed(SetupFailure::AlreadyExists) => {
            t("This account already has a recovery: enter the existing recovery key.")
        }
        SetupPhase::Failed(SetupFailure::Partial) => {
            t("This account's recovery is only half set up: Gnobbi does not change it. Continue without recovery.")
        }
        SetupPhase::Failed(SetupFailure::IdentityRequired) => {
            t("This account already has a cryptographic identity this device does not hold: Gnobbi does not replace it. Continue without recovery.")
        }
        SetupPhase::Failed(SetupFailure::AuthRequired) => {
            t("The server asks for an extra confirmation that Gnobbi does not support. Continue without recovery.")
        }
        SetupPhase::Failed(SetupFailure::Network) => {
            t("Server unreachable: the recovery was not created. Try again or continue without recovery.")
        }
        SetupPhase::Failed(SetupFailure::Timeout) => {
            t("Creation took too long. Try again or continue without recovery.")
        }
        SetupPhase::Failed(SetupFailure::Other) => {
            t("Creating the recovery failed. Try again or continue without recovery.")
        }
    })
}
/// Payload-free state of a first-time recovery setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupPhase {
    Pending,
    /// The generated key is on screen and waits for the explicit confirmation.
    KeyShown,
    Failed(SetupFailure),
    Unsupported,
}
/// Editor indices (homeserver, user, password, phrase) shown for a flow.
pub fn visible_fields(mode: SignInMode, vault: bool) -> &'static [usize] {
    match (mode, vault) {
        (SignInMode::Password, true) => &[0, 1, 2],
        (SignInMode::Password, false) => &[0, 1, 2, 3],
        (SignInMode::Unlock, _) => &[3],
        (SignInMode::UnlockVault, _) => &[],
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormStatus {
    Opening,
    Prompt(SignInPrompt),
    Busy,
    Authenticated(matrix_domain::AccountId),
    Failed(matrix_engine::signin::SignInError),
}
impl FormStatus {
    pub fn error_message(self) -> Option<&'static str> {
        let error = match self {
            Self::Prompt(prompt) => prompt.error?,
            Self::Failed(error) => error,
            _ => return None,
        };
        use matrix_engine::signin::{
            SignInError::*,
            VaultError::{Locked, Unavailable},
        };
        Some(match error {
            InvalidHomeserver => t("Enter a server name or a valid HTTPS address."),
            InvalidUserId => t("Enter a valid username or Matrix ID."),
            InvalidSecret if matches!(self, Self::Prompt(prompt) if prompt.mode == SignInMode::Unlock) => {
                t("Enter the profile's local passphrase.")
            }
            InvalidSecret => t("Password required; local passphrase of at least 16 characters."),
            UnsupportedLogin => t("This server offers no password login. SSO/OIDC are not supported."),
            NoSlidingSync => t("This server does not support sliding sync"),
            ServerStatus => {
                t("The server answered with an error: check the address (e.g. https://matrix.example.org).")
            }
            Tls | InvalidTrustRoot => t("Invalid server certificate (TLS)."),
            Discovery => t("Server unreachable or not answering."),
            Authentication(matrix_engine::auth::AuthError::CredentialsRejected) => {
                t("Credentials not accepted. Try again.")
            }
            Authentication(matrix_engine::auth::AuthError::RateLimited) => {
                t("Too many attempts: try again in a moment.")
            }
            Authentication(_) => {
                t("Sign-in/unlock failed. Check the credentials and try again.")
            }
            Vault(Unavailable | Locked) => {
                t("System keychain unavailable or locked. Unlock it and try again.")
            }
            Vault(_) => {
                t("The profile key is not in the system keychain. The profile was not changed.")
            }
            ProfileBusy => t("Profile already open in another instance. Close it and restart."),
            Storage | InvalidStorePath => t("Local profile unavailable. Close and check the profile path."),
            // Unclassified: the detail goes to the log (see `apply`), not to a misleading specific message.
            _ => t("Unexpected error. Details are in the log."),
        })
    }
}
/// Opaque identity of one actionable prompt; contains no credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignInSource(u64);
impl SignInSource {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("sign-in source IDs exhausted"),
        )
    }
}
pub struct SignInForm {
    source: Option<SignInSource>,
    pub homeserver: String,
    pub user: String,
    pub password: LoginInput,
    pub phrase: LoginInput,
    /// The OS vault protects new profiles: no local phrase is asked.
    pub vault: bool,
    recovery: Option<RecoveryState>,
    setup: Option<SetupPhase>,
    /// Held only while it is on screen; zeroized on drop.
    key: Option<matrix_engine::recovery::RecoverySecret>,
    status: FormStatus,
}
impl Default for SignInForm {
    fn default() -> Self {
        Self {
            homeserver: "https://".to_owned(),
            user: String::new(),
            password: LoginInput::secret(),
            phrase: LoginInput::secret(),
            vault: false,
            recovery: None,
            setup: None,
            key: None,
            status: FormStatus::Opening,
            source: None,
        }
    }
}
impl SignInForm {
    pub fn server_entered(server: &str) -> bool {
        let server = server.trim();
        !server.is_empty() && !server.eq_ignore_ascii_case("https://")
    }

    pub fn status(&self) -> FormStatus {
        self.status
    }
    pub fn source(&self) -> Option<SignInSource> {
        self.source
    }
    pub fn recovery(&self) -> Option<RecoveryState> {
        self.recovery
    }
    pub fn setup_phase(&self) -> Option<SetupPhase> {
        self.setup
    }
    /// The generated recovery key, only while the confirmation is pending.
    pub fn recovery_key(&self) -> Option<&str> {
        self.key.as_ref().map(|key| key.expose())
    }
    /// Creating a recovery is offered only when the account has none, and retried only after transient failures.
    pub fn setup_offered(&self) -> bool {
        self.recovery == Some(RecoveryState::Unsupported)
            && matches!(
                self.setup,
                None | Some(SetupPhase::Failed(
                    SetupFailure::Network | SetupFailure::Timeout | SetupFailure::Other
                ))
            )
    }
    pub fn setup_source(&mut self, source: SignInSource) -> Option<SignInCommand> {
        if self.source != Some(source) || !self.setup_offered() {
            return None;
        }
        self.source = None;
        self.setup = Some(SetupPhase::Pending);
        Some(SignInCommand::SetupRecovery)
    }
    /// The only way past a shown key; the key leaves the form here.
    pub fn confirm_saved_source(&mut self, source: SignInSource) -> Option<SignInCommand> {
        if self.source != Some(source) || self.key.is_none() {
            return None;
        }
        self.key = None;
        self.setup = None;
        self.source = None;
        Some(SignInCommand::DeferRecovery)
    }
    pub fn apply_setup(&mut self, event: SetupEvent) {
        self.source = (!matches!(event, SetupEvent::Pending)).then(SignInSource::new);
        self.key = None;
        self.setup = Some(match event {
            SetupEvent::Pending => SetupPhase::Pending,
            SetupEvent::Ready(key) => {
                self.key = Some(key);
                SetupPhase::KeyShown
            }
            SetupEvent::Unsupported => SetupPhase::Unsupported,
            SetupEvent::Failed(failure) => {
                if failure == SetupFailure::AlreadyExists {
                    self.recovery = Some(RecoveryState::Offered);
                }
                SetupPhase::Failed(failure)
            }
        });
    }
    pub fn error_message(&self) -> Option<&'static str> {
        if self.setup == Some(SetupPhase::KeyShown) {
            return None;
        }
        if let Some(message) = self.setup.and_then(setup_message) {
            return Some(message);
        }
        if let Some(message) = self.recovery.and_then(recovery_message) {
            return Some(message);
        }
        match self.status {
            FormStatus::Prompt(SignInPrompt {
                mode: SignInMode::Password,
                error: Some(matrix_engine::signin::SignInError::InvalidSecret),
            }) if self.vault => Some(t("Enter the Matrix password.")),
            status => status.error_message(),
        }
    }
    /// Skip the optional recovery step; stale or repeated activations do nothing.
    pub fn defer_source(&mut self, source: SignInSource) -> Option<SignInCommand> {
        if self.source != Some(source) || self.recovery.is_none() || self.key.is_some() {
            return None;
        }
        self.source = None;
        Some(SignInCommand::DeferRecovery)
    }
    /// Send the recovery key typed into the phrase editor; the key never stays in the form.
    pub fn recover_source(&mut self, source: SignInSource) -> Option<SignInCommand> {
        if self.source != Some(source)
            || !matches!(
                self.recovery,
                Some(RecoveryState::Offered | RecoveryState::Failed(_))
            )
            || self.phrase.marked_utf16().is_some()
        {
            return None;
        }
        let mut key = self.phrase.take();
        match Secret::new(std::mem::take(&mut *key)) {
            Ok(key) => {
                self.source = None;
                self.setup = None;
                self.recovery = Some(RecoveryState::Pending);
                Some(SignInCommand::EnterRecovery(key))
            }
            Err(_) => {
                self.source = Some(SignInSource::new());
                self.recovery = Some(RecoveryState::Failed(RecoveryFailure::InvalidKey));
                None
            }
        }
    }
    pub fn submit_source(&mut self, source: SignInSource) -> Option<SignInCommand> {
        if self.source != Some(source) {
            return None;
        }
        self.submit()
    }
    pub fn apply(&mut self, event: SignInEvent) {
        self.source = matches!(
            event,
            SignInEvent::Prompt(_)
                | SignInEvent::Recovery(
                    RecoveryState::Offered | RecoveryState::Failed(_) | RecoveryState::Unsupported
                )
        )
        .then(SignInSource::new);
        match event {
            SignInEvent::Prompt(prompt) => self.status = FormStatus::Prompt(prompt),
            SignInEvent::Authenticated(account) => {
                self.password.take();
                self.phrase.take();
                self.status = FormStatus::Authenticated(account);
            }
            SignInEvent::Recovery(state) => {
                self.setup = None;
                self.key = None;
                self.recovery = Some(state);
            }
            SignInEvent::Failed(error) => {
                self.password.take();
                self.phrase.take();
                eprintln!("Gnobbi: sign-in/unlock failed: {error:?}");
                self.status = FormStatus::Failed(error);
            }
        }
    }
    pub fn submit(&mut self) -> Option<SignInCommand> {
        let FormStatus::Prompt(prompt) = self.status else {
            return None;
        };
        if self.password.marked_utf16().is_some() || self.phrase.marked_utf16().is_some() {
            return None;
        }
        self.source = None;
        let mut password = self.password.take();
        let mut phrase = self.phrase.take();
        let result = (|| {
            if prompt.mode == SignInMode::UnlockVault {
                return Ok(SignInCommand::UnlockVault);
            }
            if prompt.mode == SignInMode::Password && self.vault {
                return Ok(SignInCommand::VaultPassword {
                    homeserver: self.homeserver.clone(),
                    user: self.user.clone(),
                    password: Secret::new(std::mem::take(&mut *password))?,
                });
            }
            let phrase = Secret::new(std::mem::take(&mut *phrase))?;
            Ok(match prompt.mode {
                SignInMode::Password => SignInCommand::Password {
                    homeserver: self.homeserver.clone(),
                    user: self.user.clone(),
                    password: Secret::new(std::mem::take(&mut *password))?,
                    phrase,
                },
                SignInMode::Unlock | SignInMode::UnlockVault => SignInCommand::Unlock { phrase },
            })
        })();
        match result {
            Ok(command) => {
                self.status = FormStatus::Busy;
                Some(command)
            }
            Err(error) => {
                self.source = Some(SignInSource::new());
                self.status = FormStatus::Prompt(SignInPrompt {
                    error: Some(error),
                    ..prompt
                });
                None
            }
        }
    }
}

#[cfg(test)]
mod brand_tests {
    use super::*;

    #[test]
    fn user_facing_notes_say_gnobbi_never_dime() {
        for failure in [
            SetupFailure::Partial,
            SetupFailure::IdentityRequired,
            SetupFailure::AuthRequired,
        ] {
            let text = setup_message(SetupPhase::Failed(failure)).unwrap();
            assert!(!text.contains("Dime"), "{text}");
        }
    }
}

#[cfg(test)]
mod rate_limit_tests {
    use super::*;

    #[test]
    fn rate_limited_login_has_its_own_message() {
        let status = FormStatus::Failed(matrix_engine::signin::SignInError::Authentication(
            matrix_engine::auth::AuthError::RateLimited,
        ));
        assert_eq!(status.error_message(), Some(t("Too many attempts: try again in a moment.")));
    }
}

#[cfg(test)]
mod classification_tests {
    use super::*;
    use matrix_engine::signin::SignInError as E;

    fn message(error: E) -> Option<&'static str> {
        FormStatus::Failed(error).error_message()
    }

    #[test]
    fn only_tls_errors_get_the_tls_message() {
        let tls = message(E::Tls);
        assert!(tls.unwrap().contains("TLS"));
        for other in [E::Discovery, E::NoSession, E::AttemptLimit, E::ProtectionMismatch, E::AlreadySignedIn, E::Storage] {
            assert_ne!(message(other), tls, "{other:?}");
        }
        assert_eq!(message(E::NoSession), Some(t("Unexpected error. Details are in the log.")));
    }
}
