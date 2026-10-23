//! UI-only sign-in state; storage and authentication stay on the backend.
use crate::LoginInput;
use matrix_engine::signin::{
    RecoveryState, Secret, SignInCommand, SignInEvent, SignInMode, SignInPrompt,
};
pub const RECOVERY_WARNING: &str = "Senza recupero la cronologia cifrata precedente non sarà leggibile su questo dispositivo.";
pub const RECOVER_LABEL: &str = "Recupera cronologia cifrata (non ancora disponibile)";
pub const DEFER_LABEL: &str = "Continua senza recupero";
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
            InvalidHomeserver => "Inserisci un endpoint homeserver HTTPS valido.",
            InvalidUserId => "Inserisci l’ID Matrix completo: @nome:server.",
            InvalidSecret if matches!(self, Self::Prompt(prompt) if prompt.mode == SignInMode::Unlock) => {
                "Inserisci la frase locale del profilo."
            }
            InvalidSecret => "Password richiesta; frase locale di almeno 16 caratteri.",
            UnsupportedLogin => "Questo server non offre login password. SSO/OIDC non supportati.",
            Discovery | InvalidTrustRoot => {
                "Server non raggiungibile o certificato TLS non valido."
            }
            Authentication(matrix_engine::auth::AuthError::CredentialsRejected) => {
                "Credenziali non accettate. Riprova."
            }
            Authentication(_) => {
                "Accesso/sblocco non riuscito. Controlla le credenziali e riprova."
            }
            Vault(Unavailable | Locked) => {
                "Portachiavi di sistema non disponibile o bloccato. Sbloccalo e riprova."
            }
            Vault(_) => "La chiave del profilo non è nel portachiavi di sistema. Il profilo non è stato modificato.",
            ProfileBusy => "Profilo già aperto in un’altra istanza. Chiudila e riavvia.",
            _ => "Profilo locale non disponibile. Chiudi e verifica il percorso del profilo.",
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
    status: FormStatus,
}
impl Default for SignInForm {
    fn default() -> Self {
        Self {
            homeserver: String::new(),
            user: String::new(),
            password: LoginInput::secret(),
            phrase: LoginInput::secret(),
            vault: false,
            recovery: None,
            status: FormStatus::Opening,
            source: None,
        }
    }
}
impl SignInForm {
    pub fn status(&self) -> FormStatus {
        self.status
    }
    pub fn source(&self) -> Option<SignInSource> {
        self.source
    }
    pub fn recovery(&self) -> Option<RecoveryState> {
        self.recovery
    }
    pub fn error_message(&self) -> Option<&'static str> {
        match self.status {
            FormStatus::Prompt(SignInPrompt {
                mode: SignInMode::Password,
                error: Some(matrix_engine::signin::SignInError::InvalidSecret),
            }) if self.vault => Some("Inserisci la password Matrix."),
            status => status.error_message(),
        }
    }
    /// Skip the optional recovery step; stale or repeated activations do nothing.
    pub fn defer_source(&mut self, source: SignInSource) -> Option<SignInCommand> {
        if self.source != Some(source) || self.recovery.is_none() {
            return None;
        }
        self.source = None;
        Some(SignInCommand::DeferRecovery)
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
            SignInEvent::Recovery(state) => self.recovery = Some(state),
            SignInEvent::Failed(error) => {
                self.password.take();
                self.phrase.take();
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
