//! Translation coverage: `cargo i18n` lists, per language (empty catalogs only as a count), the strings used in the source
//! (`t("…")`, `t_ctx("ctx", "…")`, `plural(n, "…", "…")`) that the catalog lacks. Never fails the build.
use matrix_desktop::i18n::{LANGUAGES, parse};
use std::collections::BTreeSet;
use std::path::Path;

/// `(context, source)` for every literal call in `text`. Only direct string literals count.
fn used(text: &str) -> BTreeSet<(String, String)> {
    let mut found = BTreeSet::new();
    for (name, args) in [("t", 1), ("t_ctx", 2)] {
        let needle = format!("{name}(");
        for (at, _) in text.match_indices(&needle) {
            // `at` must start the identifier (`.t(`, `format(` and `at(` are other functions).
            if text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            let mut rest = &text[at + needle.len()..];
            let mut lits = Vec::new();
            for i in 0..args {
                if i > 0 {
                    let Some(r) = rest.trim_start().strip_prefix(',') else { break };
                    rest = r;
                }
                let Some((lit, r)) = literal(rest.trim_start()) else { break };
                lits.push(lit);
                rest = r;
            }
            match (name, lits.as_slice()) {
                ("t", [s]) => found.insert((String::new(), s.clone())),
                ("t_ctx", [c, s]) => found.insert((c.clone(), s.clone())),
                _ => false,
            };
        }
    }
    // plural(n, "one", "other"): the key is the `one` form.
    for (at, _) in text.match_indices("plural(") {
        if text[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let rest = &text[at + 7..];
        if let Some((_, after)) = rest.split_once(',')
            && let Some((one, _)) = literal(after.trim_start())
        {
            found.insert(("@plural".into(), one));
        }
    }
    found
}

/// A Rust string literal at the start of `s`, unescaped, with the rest after it.
fn literal(s: &str) -> Option<(String, &str)> {
    let mut chars = s.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 2..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

fn sources(dir: &Path, into: &mut BTreeSet<(String, String)>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("i18n.rs") && !path.to_string_lossy().ends_with("tests.rs") {
            into.extend(used(&std::fs::read_to_string(&path).unwrap_or_default()));
        }
    }
}

fn keys_used() -> BTreeSet<(String, String)> {
    let mut keys = BTreeSet::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut keys);
    keys
}

#[test]
fn finds_literal_calls_only() {
    let code = r#"ui.label(t("Cancel")); x.t("no"); t_ctx("menu", "View"); format("no"); plural(n, "{n} file", "{n} files"); t(var); t("a\"b\n")"#;
    let keys: Vec<_> = used(code).into_iter().collect();
    let want = [("", "Cancel"), ("", "a\"b\n"), ("@plural", "{n} file"), ("menu", "View")].map(|(c, s)| (c.to_string(), s.to_string()));
    assert_eq!(keys, want);
}

/// `cargo i18n`
#[test]
#[ignore = "report; run with `cargo i18n`"]
fn translation_gaps() {
    let keys = keys_used();
    println!("{} strings used in the source", keys.len());
    for l in LANGUAGES.iter().filter(|l| l.code != "en") {
        let (catalog, errors) = parse(l.source, l.plural_forms);
        let missing: Vec<_> = keys.iter().filter(|(c, s)| !catalog.get(c).is_some_and(|m| m.contains_key(s))).collect();
        println!("\n[{}] {} missing, {} malformed rows", l.code, missing.len(), errors.len());
        for e in errors {
            println!("  row error: {e}");
        }
        if catalog.is_empty() {
            println!("  (no translations yet: not listed)");
            continue;
        }
        for (c, s) in missing {
            println!("  {}\t{}", c, s.replace('\t', "\\t").replace('\n', "\\n"));
        }
    }
}
