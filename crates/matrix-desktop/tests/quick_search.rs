use matrix_desktop::AppDirs;
use matrix_desktop::quick_search::*;
use std::time::Instant;

fn room(id: &str, name: &str, activity: u64) -> Entry {
    Entry {
        id: id.into(),
        name: name.into(),
        activity,
        ..Default::default()
    }
}
fn names(index: &Index, q: &str) -> Vec<String> {
    index.search(q, 50).into_iter().map(|h| h.name).collect()
}
fn rec(id: &str) -> Recent {
    Recent {
        id: id.into(),
        name: id.into(),
        avatar: None,
    }
}
fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("gnobbi-qs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs {
        config: root.join("cfg"),
        data: root.join("data"),
        cache: root.join("cache"),
    }
}

#[test]
fn relevance_exact_prefix_word_substring() {
    let index = Index::new([
        room("!1", "Il Mondo", 1),
        room("!2", "Mondo", 1),
        room("!3", "Mondovì", 1),
        room("!4", "Immondo", 9),
    ]);
    assert_eq!(
        names(&index, "mondo"),
        ["Mondo", "Mondovì", "Il Mondo", "Immondo"]
    );
}

#[test]
fn accents_and_case_are_ignored() {
    let index = Index::new([room("!1", "Caffè Città", 1)]);
    assert_eq!(names(&index, "CAFFE"), ["Caffè Città"]);
    assert_eq!(names(&index, "città"), ["Caffè Città"]);
    assert_eq!(names(&index, "citta"), ["Caffè Città"]);
}

#[test]
fn matches_alias_ids_and_dm_contact() {
    let mut dm = room("!dm:s", "Chat", 1);
    dm.contact = Some("Giulia Rossi".into());
    dm.user_id = Some("@giulia:s".into());
    let mut aliased = room("!a:s", "Squadra", 1);
    aliased.aliases = vec!["#team:s".into()];
    let index = Index::new([dm, aliased]);
    assert_eq!(names(&index, "rossi"), ["Chat"]);
    assert_eq!(names(&index, "@giulia:s"), ["Chat"]);
    assert_eq!(names(&index, "#team"), ["Squadra"]);
    assert_eq!(names(&index, "!a:s"), ["Squadra"]);
}

#[test]
fn ties_go_to_recent_activity_then_name_and_empty_query_is_empty() {
    let index = Index::new([
        room("!1", "Bb", 1),
        room("!2", "Ba", 5),
        room("!3", "Bc", 5),
    ]);
    assert_eq!(names(&index, "b"), ["Ba", "Bc", "Bb"]);
    assert!(index.search("  ", 10).is_empty());
    assert_eq!(index.search("b", 2).len(), 2);
}

#[test]
fn secondary_label_is_bridge_else_matrix_id() {
    let mut a = room("!a:s", "A", 1);
    a.bridge = Some("WhatsApp".into());
    let mut b = room("!b:s", "B", 1);
    b.user_id = Some("@b:s".into());
    b.avatar = Some("mxc://s/av".into());
    let index = Index::new([a, b, room("!c:s", "C", 1)]);
    let hit = |q: &str| index.search(q, 1).remove(0);
    assert_eq!(hit("a").secondary, "WhatsApp");
    assert_eq!(hit("b").secondary, "@b:s");
    assert_eq!(hit("b").avatar.as_deref(), Some("mxc://s/av"));
    assert_eq!(hit("c").secondary, "!c:s");
    assert_eq!(index.section(), Section::Local);
}

#[test]
fn upsert_replaces_and_remove_drops() {
    let mut index = Index::new([room("!1", "Vecchio", 1)]);
    index.upsert(room("!1", "Nuovo", 1));
    assert_eq!(
        (
            index.len(),
            names(&index, "nuovo").len(),
            names(&index, "vecchio").len()
        ),
        (1, 1, 0)
    );
    index.remove("!1");
    assert!(index.is_empty());
}

#[test]
fn recents_add_dedup_limit_clear() {
    let mut r = Recents::default();
    for i in 0..12 {
        r.opened(rec(&format!("!{i}")));
    }
    assert_eq!(r.seen().len(), SEEN_LIMIT);
    assert_eq!(r.seen()[0].id, "!11");
    r.opened(rec("!5"));
    assert_eq!(r.seen()[0].id, "!5");
    assert_eq!(r.seen().iter().filter(|x| x.id == "!5").count(), 1);
    for i in 0..20 {
        r.picked(rec(&format!("@{i}")));
    }
    r.picked(rec("@19"));
    assert_eq!(r.searched().len(), SEARCHED_LIMIT);
    r.clear_searched();
    assert!(r.searched().is_empty());
    assert_eq!(r.seen().len(), SEEN_LIMIT);
}

#[test]
fn persistence_is_per_account_and_reloads() {
    let d = dirs("persist");
    assert_eq!(RecentsStore::load(&d).get(1), Recents::default());
    let mut store = RecentsStore::default();
    let mut a = Recents::default();
    a.opened(Recent {
        id: "!a".into(),
        name: "A".into(),
        avatar: Some("mxc://s/x".into()),
    });
    a.picked(rec("@u"));
    let mut b = Recents::default();
    b.opened(rec("!b"));
    store.set(1, a.clone());
    store.set(2, b.clone());
    store.save(&d).unwrap();
    let loaded = RecentsStore::load(&d);
    assert_eq!(loaded.get(1), a);
    assert_eq!(loaded.get(2), b);
    assert_eq!(loaded.get(3), Recents::default());
}

#[test]
fn corrupt_file_gives_empty() {
    let d = dirs("corrupt");
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("quick-search.json"), "{nope").unwrap();
    assert_eq!(RecentsStore::load(&d), RecentsStore::default());
}

/// Run with `--nocapture` to see the timings.
#[test]
fn timing_on_synthetic_rooms() {
    for n in [1000usize, 5000] {
        let t = Instant::now();
        let index = Index::new((0..n).map(|i| {
            let mut e = room(
                &format!("!r{i}:server.example"),
                &format!("Stanza progetto {i} Città"),
                i as u64,
            );
            e.aliases = vec![format!("#alias-{i}:server.example")];
            e
        }));
        let build = t.elapsed();
        let t = Instant::now();
        let runs = 20;
        for q in ["s", "stanza pro", "citta", "zzz"] {
            for _ in 0..runs {
                std::hint::black_box(index.search(q, 20));
            }
        }
        let per = t.elapsed() / (4 * runs);
        println!("rooms={n} build={build:?} search_avg={per:?}");
        assert!(per.as_millis() < 50, "search too slow: {per:?}");
    }
}

#[test]
fn removing_an_account_forgets_its_recents_and_rail_layout_only() {
    let d = dirs("remove");
    let mut store = RecentsStore::default();
    let mut recents = Recents::default();
    recents.opened(rec("!a:x"));
    store.set(1, recents.clone());
    store.set(2, recents.clone());
    store.save(&d).unwrap();
    let mut store = RecentsStore::load(&d);
    store.remove(1);
    store.save(&d).unwrap();
    let store = RecentsStore::load(&d);
    assert_eq!(store.get(1), Recents::default());
    assert_eq!(store.get(2), recents);

    let mut layouts = matrix_desktop::RailLayouts::default();
    let layout = matrix_desktop::RailLayout::default();
    layouts.set(1, layout.clone());
    layouts.remove(1);
    assert_eq!(layouts, matrix_desktop::RailLayouts::default());
}
