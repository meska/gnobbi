//! UI-only sign-in state; storage and authentication stay on the backend.
use crate::LoginInput;
use matrix_engine::signin::{Secret, SignInCommand, SignInEvent, SignInMode, SignInPrompt};
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
        use matrix_engine::signin::SignInError::*;
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
    status: FormStatus,
}
impl Default for SignInForm {
    fn default() -> Self {
        Self {
            homeserver: String::new(),
            user: String::new(),
            password: LoginInput::secret(),
            phrase: LoginInput::secret(),
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
    pub fn submit_source(&mut self, source: SignInSource) -> Option<SignInCommand> {
        if self.source != Some(source) {
            return None;
        }
        self.submit()
    }
    pub fn apply(&mut self, event: SignInEvent) {
        self.source = matches!(event, SignInEvent::Prompt(_)).then(SignInSource::new);
        match event {
            SignInEvent::Prompt(prompt) => self.status = FormStatus::Prompt(prompt),
            SignInEvent::Authenticated(account) => {
                self.password.take();
                self.phrase.take();
                self.status = FormStatus::Authenticated(account);
            }
            // Recovery step is rendered by the sign-in view, not by this form.
            SignInEvent::Recovery(_) => {}
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
            let phrase = Secret::new(std::mem::take(&mut *phrase))?;
            Ok(match prompt.mode {
                SignInMode::Password => SignInCommand::Password {
                    homeserver: self.homeserver.clone(),
                    user: self.user.clone(),
                    password: Secret::new(std::mem::take(&mut *password))?,
                    phrase,
                },
                SignInMode::Unlock => SignInCommand::Unlock { phrase },
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
