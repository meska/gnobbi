//! UI-only sign-in state; storage and authentication stay on the backend.
use crate::LoginInput;
use matrix_engine::signin::{
    RecoveryFailure, RecoveryState, Secret, SetupEvent, SetupFailure, SignInCommand, SignInEvent, SignInMode, SignInPrompt,
};
pub const RECOVERY_WARNING: &str = "Senza recupero la cronologia cifrata precedente non sarà leggibile su questo dispositivo.";
pub const RECOVER_LABEL: &str = "Recupera cronologia cifrata";
pub const RETRY_LABEL: &str = "Riprova recupero";
pub const RECOVERY_KEY_LABEL: &str = "Chiave di recupero Matrix";
pub const RECOVERY_NOTE: &str = "Inserisci la chiave di recupero Matrix del tuo account (non è la frase locale del profilo). Il backup può non contenere tutti i messaggi: non tutti i vecchi messaggi sono garantiti.";
pub const RECOVERY_PENDING: &str = "Verifica della chiave e download delle chiavi dal backup in corso…";
pub(crate) fn recovery_message(state: RecoveryState) -> Option<&'static str> {
    Some(match state {
        RecoveryState::Failed(RecoveryFailure::InvalidKey) => "Chiave di recupero Matrix non valida. Controlla e riprova.",
        RecoveryState::Failed(RecoveryFailure::Network) => "Server non raggiungibile. Riprova più tardi o continua senza recupero.",
        RecoveryState::Failed(RecoveryFailure::Timeout) => "Il recupero ha impiegato troppo tempo. Riprova o continua senza recupero.",
        RecoveryState::Failed(RecoveryFailure::BackupUnavailable) => "Il server non ha un backup delle chiavi utilizzabile. Continua senza recupero.",
        RecoveryState::Failed(RecoveryFailure::Other) => "Recupero non riuscito. Riprova o continua senza recupero.",
        RecoveryState::Unsupported => "Questo account non ha un archivio segreto Matrix: nessuna chiave di recupero da inserire.",
        _ => return None,
    })
}
pub const SETUP_LABEL: &str = "Crea chiave di recupero";
pub const SETUP_NOTE: &str = "Questo account non ha ancora un recupero Matrix. Puoi crearne uno nuovo: Gnobbi genera una chiave di recupero e un backup delle chiavi di cifratura.";
pub const SETUP_PENDING: &str = "Creazione del recupero e del backup in corso…";
pub const SETUP_KEY_LABEL: &str = "Chiave di recupero Matrix";
pub const SETUP_KEY_NOTE: &str = "Questa chiave viene mostrata una sola volta e Gnobbi non la conserva. Salvala in un gestore di password o in un luogo sicuro: senza di essa non potrai ripristinare la cronologia cifrata.";
pub const SETUP_KEY_MASKED: &str = "Chiave di recupero mostrata a schermo, una sola volta. Non è letta dal lettore di schermo.";
pub const SETUP_RETRY_LABEL: &str = "Riprova a creare la chiave";
pub const COPY_LABEL: &str = "Copia chiave";
pub const SAVED_LABEL: &str = "Ho salvato la chiave: continua";
pub(crate) fn setup_message(phase: SetupPhase) -> Option<&'static str> {
    Some(match phase {
        SetupPhase::Pending => SETUP_PENDING,
        SetupPhase::KeyShown => return None,
        SetupPhase::Unsupported => "Il server non offre il backup delle chiavi: nessun recupero da creare. Continua senza recupero.",
        SetupPhase::Failed(SetupFailure::AlreadyExists) => "Questo account ha già un recupero: inserisci la chiave di recupero esistente.",
        SetupPhase::Failed(SetupFailure::Partial) => "Il recupero di questo account è configurato solo a metà: Gnobbi non lo modifica. Continua senza recupero.",
        SetupPhase::Failed(SetupFailure::IdentityRequired) => "Questo account ha già un’identità crittografica che questo dispositivo non possiede: Gnobbi non la sostituisce. Continua senza recupero.",
        SetupPhase::Failed(SetupFailure::AuthRequired) => "Il server chiede una conferma aggiuntiva che Gnobbi non supporta. Continua senza recupero.",
        SetupPhase::Failed(SetupFailure::Network) => "Server non raggiungibile: il recupero non è stato creato. Riprova o continua senza recupero.",
        SetupPhase::Failed(SetupFailure::Timeout) => "La creazione ha impiegato troppo tempo. Riprova o continua senza recupero.",
        SetupPhase::Failed(SetupFailure::Other) => "Creazione del recupero non riuscita. Riprova o continua senza recupero.",
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
            NoSlidingSync => "Questo server non supporta lo sliding sync",
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
    setup: Option<SetupPhase>,
    /// Held only while it is on screen; zeroized on drop.
    key: Option<matrix_engine::recovery::RecoverySecret>,
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
            setup: None,
            key: None,
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
            }) if self.vault => Some("Inserisci la password Matrix."),
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
        for text in [SETUP_NOTE, SETUP_KEY_NOTE] {
            assert!(text.contains("Gnobbi") && !text.contains("Dime"), "{text}");
        }
        for failure in [SetupFailure::Partial, SetupFailure::IdentityRequired, SetupFailure::AuthRequired] {
            let text = setup_message(SetupPhase::Failed(failure)).unwrap();
            assert!(!text.contains("Dime"), "{text}");
        }
    }
}
