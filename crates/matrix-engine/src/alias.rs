//! Validazione pura della parte locale di un alias di stanza (`#local:server`).

use std::fmt;

pub const MAX_LOCAL_LEN: usize = 64;
/// Limite Matrix per l'alias completo (byte).
pub const MAX_ALIAS_LEN: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasError {
    Empty,
    TooLong,
    InvalidChar(char),
}

impl fmt::Display for AliasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Nome non valido")
    }
}

impl std::error::Error for AliasError {}

/// Normalizza (maiuscole ASCII -> minuscole) e valida. Ammessi: a-z 0-9 `-._=/`.
pub fn normalize_alias_local(input: &str) -> Result<String, AliasError> {
    let local = input.to_ascii_lowercase();
    if local.is_empty() {
        return Err(AliasError::Empty);
    }
    if let Some(c) = local
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || "-._=/".contains(*c)))
    {
        return Err(AliasError::InvalidChar(c));
    }
    if local.len() > MAX_LOCAL_LEN {
        return Err(AliasError::TooLong);
    }
    Ok(local)
}

/// Alias completo `#local:server` per l'anteprima; valida anche la lunghezza totale.
pub fn compose_alias(local: &str, server: &str) -> Result<String, AliasError> {
    let alias = format!("#{}:{}", normalize_alias_local(local)?, server);
    if alias.len() > MAX_ALIAS_LEN {
        return Err(AliasError::TooLong);
    }
    Ok(alias)
}

/// Principale + alternativi di `m.room.canonical_alias`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Canonical {
    pub main: Option<String>,
    pub alts: Vec<String>,
}

impl Canonical {
    pub fn contains(&self, alias: &str) -> bool {
        self.main.as_deref() == Some(alias) || self.alts.iter().any(|a| a == alias)
    }

    /// Stato dopo l'aggiunta. `make_canonical` None: principale solo se non ce n'è uno.
    /// Il vecchio principale passa tra gli alternativi.
    pub fn with_added(&self, alias: &str, make_canonical: Option<bool>) -> Self {
        let make = make_canonical.unwrap_or(self.main.is_none());
        if make { self.with_main(alias) } else if self.contains(alias) { self.clone() } else {
            let mut next = self.clone();
            next.alts.push(alias.to_owned());
            next
        }
    }

    /// Stato dopo la rimozione; se era il principale subentra il primo alternativo, o nessuno.
    pub fn without(&self, alias: &str) -> Self {
        let mut next = self.clone();
        next.alts.retain(|a| a != alias);
        if next.main.as_deref() == Some(alias) {
            next.main = (!next.alts.is_empty()).then(|| next.alts.remove(0));
        }
        next
    }

    /// Cambia il principale con uno degli alias esistenti; None se l'alias non è tra quelli.
    pub fn with_canonical(&self, alias: &str) -> Option<Self> {
        self.contains(alias).then(|| self.with_main(alias))
    }

    fn with_main(&self, alias: &str) -> Self {
        let mut alts: Vec<String> = self.alts.iter().filter(|a| *a != alias).cloned().collect();
        if let Some(old) = self.main.as_ref().filter(|m| *m != alias) {
            alts.insert(0, old.clone());
        }
        Self { main: Some(alias.to_owned()), alts }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasOpError {
    WrongScope,
    Unavailable,
    /// M_ROOM_IN_USE
    AliasInUse,
    Forbidden,
    /// M_INVALID_PARAM o nome rifiutato dalla validazione locale
    InvalidName,
    /// Il nostro server non risponde.
    Network,
    ServerUnreachable,
    /// L'alias esiste sul server ma lo stato non l'ha preso e l'annullamento è fallito:
    /// riprovando `add_room_alias` con lo stesso nome si completa.
    Orphaned { alias: String },
}

/// Alias di una stanza come li vede l'utente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomAliases {
    pub canonical: Canonical,
    /// Alias del nostro server che puntano alla stanza (anche non pubblicati nello stato).
    pub local: Vec<String>,
    /// Livello di permesso per `m.room.canonical_alias`; false se sconosciuto.
    pub can_edit_aliases: bool,
    /// Il server dell'account: il dominio fisso del dialogo «Aggiungi indirizzo».
    pub server: String,
}

/// Una richiesta dell'interfaccia; ogni risposta porta l'elenco riletto dal server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AliasAction {
    Load,
    Add { local: String, make_canonical: Option<bool> },
    Remove { alias: String },
    SetCanonical { alias: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid() {
        assert_eq!(normalize_alias_local("my-room_1.a=b/c").unwrap(), "my-room_1.a=b/c");
    }

    #[test]
    fn uppercase_is_lowercased() {
        assert_eq!(normalize_alias_local("MyRoom").unwrap(), "myroom");
    }

    #[test]
    fn rejects_bad_chars() {
        assert_eq!(normalize_alias_local("a b"), Err(AliasError::InvalidChar(' ')));
        assert_eq!(normalize_alias_local("#a"), Err(AliasError::InvalidChar('#')));
        assert_eq!(normalize_alias_local("a:b"), Err(AliasError::InvalidChar(':')));
        assert_eq!(normalize_alias_local("caffè"), Err(AliasError::InvalidChar('è')));
        assert_eq!(normalize_alias_local("日本"), Err(AliasError::InvalidChar('日')));
    }

    #[test]
    fn empty() {
        assert_eq!(normalize_alias_local(""), Err(AliasError::Empty));
    }

    #[test]
    fn length_limit() {
        assert!(normalize_alias_local(&"a".repeat(64)).is_ok());
        assert_eq!(normalize_alias_local(&"a".repeat(65)), Err(AliasError::TooLong));
    }

    #[test]
    fn compose() {
        assert_eq!(compose_alias("Foo", "example.org").unwrap(), "#foo:example.org");
        assert_eq!(compose_alias("a", &"s".repeat(260)), Err(AliasError::TooLong));
        assert_eq!(AliasError::Empty.to_string(), "Nome non valido");
    }

    fn c(main: Option<&str>, alts: &[&str]) -> Canonical {
        Canonical { main: main.map(Into::into), alts: alts.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn first_alias_becomes_main_by_default() {
        assert_eq!(c(None, &[]).with_added("#a:x", None), c(Some("#a:x"), &[]));
        assert_eq!(c(None, &[]).with_added("#a:x", Some(false)), c(None, &["#a:x"]));
        // con un principale già presente: alternativo, salvo richiesta esplicita
        assert_eq!(c(Some("#a:x"), &[]).with_added("#b:x", None), c(Some("#a:x"), &["#b:x"]));
        assert_eq!(c(Some("#a:x"), &[]).with_added("#b:x", Some(true)), c(Some("#b:x"), &["#a:x"]));
    }

    #[test]
    fn removing_main_promotes_next_alt_or_none() {
        assert_eq!(c(Some("#a:x"), &["#b:x", "#c:x"]).without("#a:x"), c(Some("#b:x"), &["#c:x"]));
        assert_eq!(c(Some("#a:x"), &[]).without("#a:x"), c(None, &[]));
        assert_eq!(c(Some("#a:x"), &["#b:x"]).without("#b:x"), c(Some("#a:x"), &[]));
    }

    #[test]
    fn set_canonical_only_among_existing() {
        let s = c(Some("#a:x"), &["#b:x", "#c:x"]);
        assert_eq!(s.with_canonical("#c:x"), Some(c(Some("#c:x"), &["#a:x", "#b:x"])));
        assert_eq!(s.with_canonical("#z:x"), None);
    }
}
