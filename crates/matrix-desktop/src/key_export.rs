//! «Esporta chiavi…» and «Rigenera chiave di recupero…» in «Privacy e sicurezza»: password check,
//! one-line texts and the state of the sub-page. No GPUI types; the engine does the work.

/// Shortest export password, counted in characters. One constant to change.
pub const EXPORT_MIN_CHARS: usize = 12;
/// Declared delay after which a copied recovery key is wiped from the clipboard.
pub const CLIPBOARD_SECONDS: u64 = 60;
pub const EXPORT_ROW_LABEL: &str = "Esporta chiavi…";
pub const RESET_ROW_LABEL: &str = "Rigenera chiave di recupero…";
pub const WRONG_PHRASE: &str = "Frase di sblocco errata";
pub const RESET_WARNING: &str = "La chiave di recupero attuale smetterà di funzionare";
pub const RESET_RUNNING: &str = "Rigenerazione…";
pub const SAVE_WARNING: &str = "Salvala ora: non verrà più mostrata";
pub const COPY_KEY_LABEL: &str = "Copia chiave di recupero";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordError {
    TooShort,
    Mismatch,
}
impl PasswordError {
    pub fn text(self) -> &'static str {
        match self {
            Self::TooShort => "Password troppo corta: almeno 12 caratteri",
            Self::Mismatch => "Le password non coincidono",
        }
    }
}

/// Length first (so an empty pair says «troppo corta»), then the two typings byte for byte.
pub fn check_password(first: &str, second: &str) -> Result<(), PasswordError> {
    if first.chars().count() < EXPORT_MIN_CHARS {
        Err(PasswordError::TooShort)
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

#[cfg(feature = "live")]
pub fn export_error_text(error: matrix_engine::key_export::KeyExportError) -> &'static str {
    use matrix_engine::key_export::KeyExportError::*;
    match error {
        WrongPhrase => WRONG_PHRASE,
        NoKeys => "Nessuna chiave da esportare su questo dispositivo",
        NotWritable => "Impossibile scrivere in questa posizione",
        WrongPassword | Deadline | Failed => "Esportazione non riuscita: riprova",
    }
}

#[cfg(feature = "live")]
pub fn reset_error_text(error: matrix_engine::recovery::RecoveryError) -> &'static str {
    use matrix_engine::recovery::RecoveryError::*;
    match error {
        WrongPhrase => WRONG_PHRASE,
        IdentityRequired => "Recupera prima le chiavi su questo dispositivo",
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
    #[cfg(feature = "live")]
    pub secret: Option<matrix_engine::recovery::RecoverySecret>,
}

impl KeysPage {
    pub fn new(kind: KeysKind) -> Self {
        Self {
            kind,
            busy: false,
            line: None,
            held: None,
            refill: false,
            #[cfg(feature = "live")]
            secret: None,
        }
    }
    /// The new key is on screen: the only way out is «L'ho salvata».
    pub fn shown(&self) -> bool {
        #[cfg(feature = "live")]
        return self.secret.is_some();
        #[cfg(not(feature = "live"))]
        false
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
        #[cfg(feature = "live")]
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
    fn password_edge_cases() {
        // Empty second typing never beats the length rule; mismatch is exact (case, spaces, normalization).
        assert_eq!(check_password("abcdefghijkl", ""), Err(PasswordError::Mismatch));
        assert_eq!(check_password("", "abcdefghijkl"), Err(PasswordError::TooShort));
        assert_eq!(check_password("abcdefghijkl ", "abcdefghijkl"), Err(PasswordError::Mismatch));
        assert_eq!(check_password("            ", "            "), Ok(())); // 12 spaces: length only, by design
        assert_eq!(check_password(&"x".repeat(100_000), &"x".repeat(100_000)), Ok(()));
        assert_eq!(check_password("e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}", "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}"), Ok(()));
    }

    #[test]
    fn user_texts_never_echo_the_password() {
        let secret = "hunter2-hunter2";
        let mut texts: Vec<String> = [PasswordError::TooShort, PasswordError::Mismatch]
            .iter()
            .map(|e| e.text().to_owned())
            .collect();
        texts.extend([exported_text(0), exported_text(3), WRONG_PHRASE.into(), RESET_WARNING.into()]);
        #[cfg(feature = "live")]
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
