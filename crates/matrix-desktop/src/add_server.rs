//! Adding a server to the public-room directory: name normalization and the add state machine.
//! Pure logic, no GPUI, no network; the probe itself is done by the engine (`server_probed`).
use matrix_engine::search::SearchError;
use std::net::Ipv4Addr;

/// Why a typed name is not a usable server name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    NotHttps,
    Path,
    Credentials,
    InvalidPort,
    InvalidIp,
    Invalid,
}

impl NameError {
    pub fn text(self) -> &'static str {
        match self {
            Self::Empty => "Scrivi il nome del server",
            Self::NotHttps => "Sono ammessi solo indirizzi https",
            Self::Path => "Scrivi solo il nome del server, senza percorso",
            Self::Credentials => "Il nome del server non deve contenere utente o password",
            Self::InvalidPort => "Porta non valida",
            Self::InvalidIp => "Indirizzo IP non valido",
            Self::Invalid => "Nome del server non valido",
        }
    }
}

/// Canonical `host` or `host:port`, lowercase. Accepts `matrix.org`, `https://matrix.org/`, spaces.
// shortcut: IPv6 literals and non-ASCII (IDN) names are refused as invalid; add if ever needed.
pub fn normalize_server(input: &str) -> Result<String, NameError> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return Err(NameError::Empty);
    }
    let rest = match s.split_once("://") {
        Some(("https", r)) => r,
        Some(_) => return Err(NameError::NotHttps),
        None => &s,
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains('@') {
        return Err(NameError::Credentials);
    }
    if rest.contains(['/', '?', '#']) {
        return Err(NameError::Path);
    }
    if rest.starts_with('[') {
        return Err(NameError::Invalid);
    }
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    if let Some(p) = port {
        match p.parse::<u16>() {
            Ok(n) if n > 0 && p.bytes().all(|b| b.is_ascii_digit()) => {}
            _ => return Err(NameError::InvalidPort),
        }
    }
    if host.is_empty() {
        return Err(NameError::Invalid);
    }
    // all-digits-and-dots is meant as an IPv4 address and must parse as one
    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return host.parse::<Ipv4Addr>().map(|_| rest.to_owned()).map_err(|_| NameError::InvalidIp);
    }
    let label_ok = |l: &str| {
        (1..=63).contains(&l.len())
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    if host.len() <= 253 && host.split('.').all(label_ok) {
        Ok(rest.to_owned())
    } else {
        Err(NameError::Invalid)
    }
}

/// Why adding failed; one line each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddError {
    NoResponse,
    NotMatrix,
    DirectoryHidden,
    Tls,
    InvalidName(NameError),
    Present,
}

impl AddError {
    pub fn text(self) -> &'static str {
        match self {
            Self::NoResponse => "Il server non risponde",
            Self::NotMatrix => "Non è un server Matrix",
            Self::DirectoryHidden => "Il server non mostra le sue stanze",
            Self::Tls => "Certificato del server non valido",
            Self::InvalidName(e) => e.text(),
            Self::Present => "Già presente",
        }
    }
}

// The engine cannot tell "not Matrix" or TLS apart yet: those variants come from a richer probe.
impl From<SearchError> for AddError {
    fn from(e: SearchError) -> Self {
        match e {
            SearchError::InvalidServer => Self::InvalidName(NameError::Invalid),
            SearchError::Refused => Self::DirectoryHidden,
            SearchError::Unreachable | SearchError::Timeout | SearchError::Network => Self::NoResponse,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddState {
    Idle,
    /// Canonical name being probed.
    Probing(String),
    Ok(String),
    Error(AddError),
}

impl AddState {
    /// «Aggiungi» is enabled for a non-blank field unless a probe is in flight.
    pub fn can_submit(&self, text: &str) -> bool {
        !text.trim().is_empty() && !matches!(self, Self::Probing(_))
    }

    /// Enter / «Aggiungi». `Some(name)` = start probing it; `None` = nothing to probe
    /// (blank, probe in flight, or invalid name: state becomes `Error`).
    pub fn submit(&mut self, text: &str) -> Option<String> {
        if !self.can_submit(text) {
            return None;
        }
        match normalize_server(text) {
            Ok(name) => {
                *self = Self::Probing(name.clone());
                Some(name)
            }
            Err(e) => {
                *self = Self::Error(AddError::InvalidName(e));
                None
            }
        }
    }

    /// `server_probed` answer. Stale answers (other server, or not probing anymore) are ignored.
    /// Returns true when the server was accepted.
    pub fn probed(&mut self, server: &str, result: Result<(), AddError>) -> bool {
        if !matches!(self, Self::Probing(s) if s == server) {
            return false;
        }
        *self = match result {
            Ok(()) => Self::Ok(server.to_owned()),
            Err(e) => Self::Error(e),
        };
        result.is_ok()
    }

    /// The text changed: a shown error or result goes away; a probe in flight keeps running.
    pub fn edited(&mut self) {
        if !matches!(self, Self::Probing(_)) {
            *self = Self::Idle;
        }
    }

    /// Esc: back to Idle; a late answer is then ignored.
    pub fn cancel(&mut self) {
        *self = Self::Idle;
    }

    pub fn error(&self) -> Option<AddError> {
        match self {
            Self::Error(e) => Some(*e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_forms_become_canonical() {
        for (i, o) in [
            ("matrix.org", "matrix.org"),
            ("https://matrix.org", "matrix.org"),
            ("matrix.org/", "matrix.org"),
            ("  HTTPS://Matrix.ORG/  ", "matrix.org"),
            ("example.com:8448", "example.com:8448"),
            ("https://a-b.example.com:443/", "a-b.example.com:443"),
            ("198.51.100.10", "198.51.100.10"),
            ("198.51.100.10:8448", "198.51.100.10:8448"),
            ("localhost", "localhost"),
        ] {
            assert_eq!(normalize_server(i).as_deref(), Ok(o), "{i}");
        }
    }

    #[test]
    fn rejections_are_typed() {
        use NameError::*;
        for (i, e) in [
            ("", Empty),
            ("   ", Empty),
            ("http://matrix.org", NotHttps),
            ("ftp://matrix.org", NotHttps),
            ("matrix.org/path", Path),
            ("https://matrix.org/_matrix", Path),
            ("matrix.org?x=1", Path),
            ("matrix.org#a", Path),
            ("user@matrix.org", Credentials),
            ("https://user:pw@matrix.org", Credentials),
            ("matrix.org:0", InvalidPort),
            ("matrix.org:99999", InvalidPort),
            ("matrix.org:abc", InvalidPort),
            ("matrix.org:", InvalidPort),
            ("matrix.org:+80", InvalidPort),
            ("999.1.1.1", InvalidIp),
            ("1.2.3", InvalidIp),
            ("1.2.3.4.5", InvalidIp),
            (":8448", Invalid),
            ("-a.org", Invalid),
            ("a..org", Invalid),
            ("a.org.", Invalid),
            ("a b.org", Invalid),
            ("münchen.de", Invalid),
            ("[::1]", Invalid),
            ("https://", Invalid),
        ] {
            assert_eq!(normalize_server(i), Err(e), "{i:?}");
            assert!(!e.text().is_empty());
        }
    }

    #[test]
    fn happy_path_idle_probing_ok() {
        let mut s = AddState::Idle;
        assert!(!s.can_submit("  "));
        assert!(s.can_submit("x.org"));
        assert_eq!(s.submit(" X.org/ "), Some("x.org".into()));
        assert_eq!(s, AddState::Probing("x.org".into()));
        assert!(!s.can_submit("x.org"), "disabled while probing");
        assert_eq!(s.submit("y.org"), None);
        assert!(!s.probed("y.org", Ok(())), "stale answer");
        assert!(s.probed("x.org", Ok(())));
        assert_eq!(s, AddState::Ok("x.org".into()));
    }

    #[test]
    fn errors_edit_and_escape() {
        let mut s = AddState::Idle;
        assert_eq!(s.submit("http://x.org"), None);
        assert_eq!(s.error(), Some(AddError::InvalidName(NameError::NotHttps)));
        s.edited();
        assert_eq!(s, AddState::Idle);
        assert_eq!(s.submit("x.org"), Some("x.org".into()));
        s.edited();
        assert_eq!(s, AddState::Probing("x.org".into()), "edit does not stop the probe");
        for e in [AddError::NoResponse, AddError::NotMatrix, AddError::DirectoryHidden, AddError::Tls] {
            let mut s = AddState::Probing("x.org".into());
            assert!(!s.probed("x.org", Err(e)));
            assert_eq!(s.error(), Some(e));
            assert!(s.can_submit("x.org"), "retry allowed");
        }
        let mut s = AddState::Probing("x.org".into());
        s.cancel();
        assert_eq!(s, AddState::Idle);
        assert!(!s.probed("x.org", Ok(())), "late answer after Esc");
    }

    #[test]
    fn search_errors_map_to_reasons() {
        assert_eq!(AddError::from(SearchError::Refused), AddError::DirectoryHidden);
        assert_eq!(AddError::from(SearchError::Timeout), AddError::NoResponse);
        assert_eq!(AddError::from(SearchError::InvalidServer), AddError::InvalidName(NameError::Invalid));
    }
}
