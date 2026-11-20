//! «Esporta chiavi…» and «Rigenera chiave di recupero…» in «Privacy e sicurezza»: password check,
//! one-line texts and the state of the sub-page. No UI types; the engine does the work.

/// Shortest export password, counted in characters. One constant to change.
pub const EXPORT_MIN_CHARS: usize = 12;
/// Longest password in bytes: the limit of the engine's `Secret::new`.
pub const EXPORT_MAX_BYTES: usize = 4096;
/// Declared delay after which a copied recovery key is wiped from the clipboard.
pub const CLIPBOARD_SECONDS: u64 = 60;
pub const EXPORT_ROW_LABEL: &str = "Esporta chiavi…";
pub const RESET_ROW_LABEL: &str = "Rigenera chiave di recupero…";
pub const WRONG_PHRASE: &str = "Frase di sblocco errata";
pub const RESET_WARNING: &str = "La chiave di recupero attuale smetterà di funzionare";
pub const RESET_RUNNING: &str = "Rigenerazione…";
pub const SAVE_WARNING: &str = "Salvala ora: non verrà più mostrata";
pub const COPY_KEY_LABEL: &str = "Copia chiave di recupero";
pub const SAVE_FILE_LABEL: &str = "Salva su file";
pub const KEPT_LABEL: &str = "L'ho conservata altrove";
pub const OFFER_TITLE: &str = "Chiave di recupero";
pub const OFFER_TEXT: &str = "Questo account non ha una chiave di recupero: senza, i messaggi cifrati si perdono con i dispositivi";
pub const ENTER_TEXT: &str = "Questo dispositivo non ha le chiavi di recupero dell'account: inserisci la chiave per leggere i messaggi cifrati vecchi";
pub const ENTER_KEY_LABEL: &str = "Inserisci chiave di recupero";
pub const RESTORE_LABEL: &str = "Ripristina";
pub const CANCEL_LABEL: &str = "Annulla";
pub const RESTORING_LABEL: &str = "Ripristino…";
pub const CREATE_KEY_LABEL: &str = "Crea chiave di recupero";
pub const NOT_NOW_LABEL: &str = "Non ora";
pub const NEVER_ASK_LABEL: &str = "Non chiedere più per questo account";
pub const KEY_FILE_NAME: &str = "gnobbi-chiave-di-recupero.txt";

/// The offer (create or enter) is shown once per session, unless suppressed for the account
/// or another key page is already open.
pub fn should_offer(kind: Option<OfferKind>, suppressed: bool, asked: bool, page_open: bool) -> Option<OfferKind> {
    kind.filter(|_| !suppressed && !asked && !page_open)
}

/// What the first-sign-in dialog proposes. `Create` only when the account has no secret storage at all
/// (creating on an account that has one would replace its key); `Enter` when the key exists but this
/// device lacks the secrets (SDK `Incomplete`: not disabled, keys not complete here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferKind {
    Create,
    Enter,
}

pub fn offer_kind(disabled: Option<bool>, complete: Option<bool>) -> Option<OfferKind> {
    match (disabled, complete) {
        (Some(true), _) => Some(OfferKind::Create),
        (Some(false), Some(false)) => Some(OfferKind::Enter),
        _ => None,
    }
}

/// Plain text of the key file: the key alone on its own line under a title.
pub fn key_file_text(key: &str) -> String {
    format!("Chiave di recupero Gnobbi\n{key}\n")
}

/// Writes the key file readable by the owner only; an existing file is replaced.
pub fn write_key_file(path: &std::path::Path, key: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    file.write_all(key_file_text(key).as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordError {
    TooShort,
    TooLong,
    Mismatch,
}
impl PasswordError {
    pub fn text(self) -> &'static str {
        match self {
            Self::TooShort => "Password troppo corta: almeno 12 caratteri",
            Self::TooLong => "Password troppo lunga: al massimo 4096 byte",
            Self::Mismatch => "Le password non coincidono",
        }
    }
}

/// Length first (so an empty pair says «troppo corta»), then the two typings byte for byte.
pub fn check_password(first: &str, second: &str) -> Result<(), PasswordError> {
    if first.chars().count() < EXPORT_MIN_CHARS {
        Err(PasswordError::TooShort)
    } else if first.len() > EXPORT_MAX_BYTES {
        Err(PasswordError::TooLong)
    } else if first != second {
        Err(PasswordError::Mismatch)
    } else {
        Ok(())
    }
}

pub fn exported_text(count: usize) -> String {
    match count {
        1 => "Esportata 1 chiave".into(),
        n => format!("Esportate {n} chiavi"),
    }
}

pub fn export_error_text(error: matrix_engine::key_export::KeyExportError) -> &'static str {
    use matrix_engine::key_export::KeyExportError::*;
    match error {
        WrongPhrase => WRONG_PHRASE,
        NoKeys => "Nessuna chiave da esportare su questo dispositivo",
        NotWritable => "Impossibile scrivere in questa posizione",
        WrongPassword | Deadline | Failed => "Esportazione non riuscita: riprova",
    }
}

pub fn reset_error_text(error: matrix_engine::recovery::RecoveryError) -> &'static str {
    use matrix_engine::recovery::RecoveryError::*;
    match error {
        WrongPhrase => WRONG_PHRASE,
        IdentityRequired => "Recupera prima le chiavi su questo dispositivo",
        AuthRequired => "Il server chiede una conferma non supportata qui",
        Transport | Deadline => "Server non raggiungibile: riprova",
        _ => "Rigenerazione non riuscita: riprova",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeysKind {
    Export,
    Reset,
}

/// A secret kept only so the fields can be refilled after a failed try; wiped on drop.
pub struct Held {
    pub password: zeroize::Zeroizing<String>,
    pub phrase: Option<zeroize::Zeroizing<String>>,
}

/// The open sub-page. `busy` covers the file picker and the command in flight.
pub struct KeysPage {
    pub kind: KeysKind,
    pub busy: bool,
    /// The one line under the button: (text, is_error).
    pub line: Option<(String, bool)>,
    pub held: Option<Held>,
    /// Set by the engine answer: the view refills (error) or clears (ok) the fields once.
    pub refill: bool,
    /// The new recovery key while it is shown; never read from anywhere else.
    pub secret: Option<matrix_engine::recovery::RecoverySecret>,
    /// The shown key was copied or saved once (or the user said it is kept elsewhere): «Fatto» unlocks.
    pub kept: bool,
}

impl KeysPage {
    pub fn new(kind: KeysKind) -> Self {
        Self {
            kind,
            busy: false,
            line: None,
            held: None,
            refill: false,
            secret: None,
            kept: false,
        }
    }
    /// The new key is on screen: the only way out is «L'ho salvata».
    pub fn shown(&self) -> bool {
        self.secret.is_some()
    }
    /// Esc and the header close the page, except while the new key is shown or being made
    /// (the old key is already void: closing would lose the new one).
    pub fn closable(&self) -> bool {
        !(self.kind == KeysKind::Reset && (self.busy || self.shown()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_rules() {
        assert_eq!(check_password("", ""), Err(PasswordError::TooShort));
        assert_eq!(
            check_password("abcdefghijk", "abcdefghijk"),
            Err(PasswordError::TooShort)
        );
        assert_eq!(check_password("abcdefghijkl", "abcdefghijkl"), Ok(()));
        assert_eq!(
            check_password("abcdefghijkl", "abcdefghijkL"),
            Err(PasswordError::Mismatch)
        );
        // 12 characters, 24 bytes: counted in characters.
        assert_eq!(check_password("àèìòùàèìòùàè", "àèìòùàèìòùàè"), Ok(()));
        assert_eq!(
            check_password("àèìòùàèìòù", "àèìòùàèìòù"),
            Err(PasswordError::TooShort)
        );
    }

    #[test]
    fn outcome_line_counts_keys() {
        assert_eq!(exported_text(1), "Esportata 1 chiave");
        assert_eq!(exported_text(7), "Esportate 7 chiavi");
    }

    #[test]
    fn reset_page_cannot_be_closed_while_running_or_shown() {
        let mut page = KeysPage::new(KeysKind::Reset);
        assert!(page.closable());
        page.busy = true;
        assert!(!page.closable());
        page.busy = false;
        {
            page.secret = Some(matrix_engine::recovery::RecoverySecret::fixture(
                "EsTx fixt",
            ));
            assert!(page.shown() && !page.closable());
            page.secret = None;
            assert!(page.closable());
        }
        let mut export = KeysPage::new(KeysKind::Export);
        export.busy = true;
        assert!(export.closable());
    }

    #[test]
    fn offer_kind_maps_the_sdk_state_and_is_asked_once() {
        use OfferKind::{Create, Enter};
        assert_eq!(offer_kind(Some(true), Some(false)), Some(Create), "Disabled");
        assert_eq!(offer_kind(Some(false), Some(false)), Some(Enter), "Incomplete");
        assert_eq!(offer_kind(Some(false), Some(true)), None, "Enabled");
        assert_eq!(offer_kind(None, Some(false)), None, "Unknown: wait");
        assert_eq!(offer_kind(Some(false), None), None, "completeness not known yet");
        assert_eq!(should_offer(Some(Enter), false, false, false), Some(Enter));
        assert_eq!(should_offer(Some(Create), true, false, false), None, "suppressed for the account");
        assert_eq!(should_offer(Some(Create), false, true, false), None, "once per session");
        assert_eq!(should_offer(Some(Enter), false, false, true), None, "another key page is open");
    }

    #[test]
    fn key_file_holds_the_key_and_is_private() {
        let path = std::env::temp_dir().join(format!("gnobbi-keyfile-{}.txt", std::process::id()));
        write_key_file(&path, "EsTx abcd").unwrap();
        write_key_file(&path, "EsTx abcd").unwrap(); // replaced, not appended
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Chiave di recupero Gnobbi\nEsTx abcd\n");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions()) & 0o777, 0o600);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn password_edge_cases() {
        // Empty second typing never beats the length rule; mismatch is exact (case, spaces, normalization).
        assert_eq!(check_password("abcdefghijkl", ""), Err(PasswordError::Mismatch));
        assert_eq!(check_password("", "abcdefghijkl"), Err(PasswordError::TooShort));
        assert_eq!(check_password("abcdefghijkl ", "abcdefghijkl"), Err(PasswordError::Mismatch));
        assert_eq!(check_password("            ", "            "), Ok(())); // 12 spaces: length only, by design
        assert_eq!(check_password(&"x".repeat(4096), &"x".repeat(4096)), Ok(()));
        assert_eq!(check_password(&"x".repeat(4097), &"x".repeat(4097)), Err(PasswordError::TooLong));
        assert_eq!(check_password(&"x".repeat(100_000), &"x".repeat(100_000)), Err(PasswordError::TooLong));
        assert_eq!(check_password("e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}", "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}"), Ok(()));
    }

    #[test]
    fn user_texts_never_echo_the_password() {
        let secret = "hunter2-hunter2";
        let mut texts: Vec<String> = [PasswordError::TooShort, PasswordError::TooLong, PasswordError::Mismatch]
            .iter()
            .map(|e| e.text().to_owned())
            .collect();
        texts.extend([exported_text(0), exported_text(3), WRONG_PHRASE.into(), RESET_WARNING.into()]);
        {
            use matrix_engine::key_export::KeyExportError::*;
            for e in [WrongPhrase, WrongPassword, NoKeys, NotWritable, Deadline, Failed] {
                texts.push(export_error_text(e).into());
            }
            assert_eq!(export_error_text(NoKeys), "Nessuna chiave da esportare su questo dispositivo");
        }
        for t in texts {
            assert!(!t.contains(secret) && !t.contains("BEGIN MEGOLM"), "{t}");
        }
        let held = Held { password: secret.to_owned().into(), phrase: None };
        let _ = held; // no Debug impl: the secret cannot reach a log through {:?}
    }
}
