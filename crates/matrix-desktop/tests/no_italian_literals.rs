//! User-visible text in the migrated files is an English key (`t("…")`, `plural(…)`), never an Italian
//! literal: the Italian text lives in the catalog, where `cargo i18n` sees it. Fails when one comes back.
use std::path::Path;

/// Files whose texts moved to the catalog. Add a file here when it is migrated.
const FILES: &[&str] = &[
    "add_server.rs",
    "audio_player.rs",
    "bin/gnobbi/devices.rs",
    "directory_rooms.rs",
    "history.rs",
    "key_export.rs",
    "link_requests.rs",
    "quick_search.rs",
    "row_text.rs",
    "search_list.rs",
    "sidebar.rs",
    "signin_form.rs",
    "update/helper.rs",
    "update/install.rs",
    "update/mod.rs",
    "verification.rs",
    "voice_recorder.rs",
];

/// Italian-only words (an English one never appears here) next to the accented letters below.
const ITALIAN: &[&str] = &[
    "non", "della", "delle", "dello", "degli", "nel", "nella", "nelle", "sono", "più", "già", "questo", "questa", "questi", "serve", "riprova", "riprovare", "nessun", "nessuna",
    "errore", "chiave", "chiavi", "stanza", "stanze", "messaggio", "messaggi", "aggiornamento", "aggiornato", "continua", "senza", "inserisci", "controlla", "troppi", "tutte", "tutti",
    "lette", "letta", "esporta", "impossibile", "membro", "membri", "risposta", "risposte", "ricerca", "risultati", "risultato", "il", "lo", "gli", "una", "uno", "per", "con", "che",
    "dal", "dei", "del", "scrivi", "ammessi", "indirizzi", "porta", "nome", "utente", "credenziali", "accesso", "sblocco", "profilo", "frase", "dispositivo", "dispositivi", "cartella",
    "verifica", "verificato", "annulla", "chiudi", "apri", "salva", "copia",
];

/// Demo fixtures of the offline demo history: sample content, not UI text.
const DEMO: &[&str] = &["Una fixture offline", "Un caffè in città", "Prima riga esplicita", "[Anteprima locale", "Link demo", "Evento {", "Storico {", "Ricezione simulata"];

fn is_italian(text: &str) -> bool {
    text.chars().any(|c| "àèéìòùÀÈÉÌÒÙ".contains(c)) || text.split(|c: char| !c.is_alphabetic()).any(|w| ITALIAN.contains(&w.to_lowercase().as_str()))
}

/// String literals of one line (naive: escapes are honoured, raw strings and `'"'` are not literals here).
fn literals(line: &str) -> Vec<String> {
    let line = line.replace("'\"'", "''");
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => match current.take() {
                Some(done) => out.push(done),
                None => current = Some(String::new()),
            },
            '\\' if current.is_some() => {
                let s = current.as_mut().unwrap();
                s.push(c);
                s.extend(chars.next());
            }
            '/' if current.is_none() && chars.as_str().starts_with('/') => break,
            c => {
                if let Some(s) = current.as_mut() {
                    s.push(c);
                }
            }
        }
    }
    out
}

/// The code of a source file: everything before its inline `#[cfg(test)] mod … {`.
fn code(source: &str) -> &str {
    let mut from = 0;
    while let Some(at) = source[from..].find("#[cfg(test)]") {
        let at = from + at;
        let rest = source[at + 12..].trim_start();
        if rest.starts_with("mod ") && rest.lines().next().is_some_and(|l| l.trim_end().ends_with('{')) {
            return &source[..at];
        }
        from = at + 12;
    }
    source
}

fn italian_literals(source: &str) -> Vec<String> {
    code(source)
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .flat_map(literals)
        .filter(|lit| is_italian(lit) && !DEMO.iter().any(|d| lit.starts_with(d)))
        .collect()
}

#[test]
fn detector_tells_italian_from_english() {
    assert!(is_italian("Chiave errata. Controlla e riprova."));
    assert!(is_italian("Sì"));
    assert!(!is_italian("Wrong key. Check it and try again."));
    assert!(!is_italian("The server does not respond"));
    let source = "fn f() { t(\"Wrong key\"); x(\"Chiave errata\"); // «Posizione»\n}\n#[cfg(test)]\nmod tests {\n    fn g() { \"Annulla\"; }\n}\n";
    assert_eq!(italian_literals(source), ["Chiave errata"]);
}

#[test]
fn migrated_files_have_no_italian_literals() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in FILES {
        let text = std::fs::read_to_string(src.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let found = italian_literals(&text);
        assert!(found.is_empty(), "{file}: Italian text outside the catalog, use t(\"English\") and add the it row: {found:?}");
    }
}
