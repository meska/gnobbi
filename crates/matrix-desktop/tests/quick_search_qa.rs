//! QA of the quick-search core: edge cases not covered by quick_search.rs (very long names, literal special characters,
//! NFD/emoji/CJK, nameless DMs, whitespace, odd names in the saved file, «Svuota» vs «Visti», file mode).
//! Timing and per-account persistence are already in quick_search.rs.
use matrix_desktop::AppDirs;
use matrix_desktop::quick_search::*;

fn room(id: &str, name: &str, activity: u64) -> Entry {
    Entry {
        id: id.into(),
        name: name.into(),
        activity,
        ..Default::default()
    }
}

fn many(n: usize) -> Vec<Entry> {
    (0..n)
        .map(|i| {
            let mut e = room(
                &format!("!r{i}:matrix-desktop.test"),
                &format!("Stanza {i} Caffè Città Progetto Alfa{}", i % 97),
                i as u64,
            );
            if i % 5 == 0 {
                e.user_id = Some(format!("@utente{i}:matrix-desktop.test"));
                e.contact = Some(format!("Giulia Rossi {i}"));
            }
            e
        })
        .collect()
}

#[test]
fn very_long_name_matches_and_does_not_panic() {
    let long = "Lunghissimo ".repeat(2_000) + "ago";
    let index = Index::new([room("!l:s", &long, 1)]);
    let hits = index.search("ago", 50);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, long); // the UI truncates; the core keeps the full name
}

#[test]
fn special_characters_are_literal_not_patterns() {
    let index = Index::new([
        room("!1:s", "C++ (dev) [team] *all*", 1),
        room("!2:s", "a.b", 1),
        room("!3:s", "axb", 1),
        room("!4:s", "100% \\ \"quote\" 'x' <b>", 1),
    ]);
    let n = |q: &str| -> Vec<String> { index.search(q, 50).into_iter().map(|h| h.name).collect() };
    assert_eq!(n("c++"), ["C++ (dev) [team] *all*"]);
    assert_eq!(n("[team]"), ["C++ (dev) [team] *all*"]);
    assert_eq!(n("*all*"), ["C++ (dev) [team] *all*"]);
    assert_eq!(n("a.b"), ["a.b"]); // '.' is not a wildcard
    assert_eq!(n("100%"), ["100% \\ \"quote\" 'x' <b>"]);
    assert_eq!(n("\\"), ["100% \\ \"quote\" 'x' <b>"]);
}

#[test]
fn accents_decomposed_emoji_and_non_latin() {
    let index = Index::new([
        room("!1:s", "Cafe\u{301} Zu\u{308}rich", 1), // NFD input
        room("!2:s", "Ångström ß Œuvre", 1),
        room("!3:s", "🦀 Rust 🚀", 1),
        room("!4:s", "Привет мир", 1),
        room("!5:s", "東京 チーム", 1),
    ]);
    let n = |q: &str| index.search(q, 50).len();
    assert_eq!(n("cafe zurich"), 1);
    assert_eq!(n("CAFÉ"), 1);
    assert_eq!(n("angstrom"), 1);
    assert_eq!(n("œuvre"), 1);
    assert_eq!(n("🦀"), 1);
    assert_eq!(n("привет"), 1);
    assert_eq!(n("ПРИВЕТ МИР"), 1);
    assert_eq!(n("東京"), 1);
}

#[test]
fn nameless_dm_is_found_by_contact_or_user_id_and_never_by_blank() {
    let mut dm = room("!dm:s", "", 1);
    dm.user_id = Some("@giulia:s".into());
    dm.contact = Some("Giulia Rossi".into());
    let mut dm2 = room("!dm2:s", "", 1); // no name, no contact, only the Matrix ID
    dm2.user_id = Some("@anon:s".into());
    let index = Index::new([dm, dm2]);
    assert_eq!(index.search("rossi", 50).len(), 1);
    assert_eq!(index.search("anon", 50).len(), 1);
    // Whatever the UI draws as title for a nameless row, the secondary label must not be empty.
    for h in index.search("s", 50) {
        assert!(!h.secondary.is_empty());
    }
    assert!(index.search("   ", 50).is_empty());
}

#[test]
fn no_results_and_whitespace_only() {
    let index = Index::new(many(100));
    assert!(index.search("zzzzqqqq", 50).is_empty());
    assert!(index.search("", 50).is_empty());
    assert!(index.search("\t \n", 50).is_empty());
}

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("gnobbi-qsqa-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs {
        config: root.join("cfg"),
        data: root.join("data"),
        cache: root.join("cache"),
    }
}

fn rec(id: &str, name: &str) -> Recent {
    Recent {
        id: id.into(),
        name: name.into(),
        avatar: None,
    }
}

#[test]
fn recents_with_special_names_survive_a_restart() {
    let d = dirs("special");
    let mut store = RecentsStore::default();
    let mut r = store.get(7);
    let names = ["Caffè \"Città\" 🦀", "back\\slash", "", "a\u{0}b", "line\nbreak"];
    for (i, n) in names.iter().enumerate() {
        r.picked(rec(&format!("!{i}:s"), n));
    }
    store.set(7, r);
    store.save(&d).unwrap();
    let back = RecentsStore::load(&d).get(7);
    let got: Vec<_> = back.searched().iter().rev().map(|r| r.name.as_str()).collect();
    assert_eq!(got, names);
}

#[test]
fn clear_keeps_seen_and_survives_a_restart() {
    let d = dirs("clear");
    let mut store = RecentsStore::default();
    let mut r = store.get(1);
    r.opened(rec("!a:s", "A"));
    r.picked(rec("!b:s", "B"));
    r.clear_searched();
    store.set(1, r);
    store.save(&d).unwrap();
    let back = RecentsStore::load(&d).get(1);
    assert_eq!(back.seen().len(), 1);
    assert!(back.searched().is_empty());
}

#[test]
fn recents_file_is_owner_only() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let d = dirs("perm");
        let mut store = RecentsStore::default();
        store.set(1, Recents::default());
        store.save(&d).unwrap();
        let mode = std::fs::metadata(d.config.join("quick-search.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "mode {mode:o}");
    }
}
