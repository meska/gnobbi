use matrix_desktop::{AppDirs, NavigationEntry, RailLayout, RailLayouts};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("rail-layout-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    AppDirs {
        config: root.join("cfg"),
        data: root.join("data"),
        cache: root.join("cache"),
    }
}

fn entries() -> Vec<NavigationEntry> {
    let mk = |id: Option<&str>| NavigationEntry {
        scope: id.map(|id| RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new(id),
        }),
        name: id.unwrap_or("All").into(),
        unread: 0,
        mentions: 0,
        selected: false,
        avatar: None,
    };
    vec![mk(None), mk(Some("a")), mk(Some("b")), mk(Some("c"))]
}

fn shown(layout: &RailLayout) -> Vec<(String, bool)> {
    layout
        .arrange(entries())
        .into_iter()
        .map(|(e, h)| (e.name, h))
        .collect()
}

fn names(v: &[(&str, bool)]) -> Vec<(String, bool)> {
    v.iter().map(|(n, h)| (n.to_string(), *h)).collect()
}

fn current(layout: &RailLayout) -> Vec<String> {
    layout
        .arrange(entries())
        .into_iter()
        .filter_map(|(e, _)| e.scope.map(|s| s.room_id.as_str().to_owned()))
        .collect()
}

#[test]
fn default_keeps_server_order_and_tutte_first() {
    assert_eq!(
        shown(&RailLayout::default()),
        names(&[("All", false), ("a", false), ("b", false), ("c", false)])
    );
}

#[test]
fn move_and_hide_survive_a_restart() {
    let d = dirs("restart");
    let mut layout = RailLayout::default();
    let cur = current(&layout);
    assert!(layout.shift(&cur, "c", -1));
    let cur = current(&layout);
    assert!(layout.shift(&cur, "c", -1));
    layout.toggle_hidden("b");
    let mut all = RailLayouts::default();
    all.set(7, layout.clone());
    all.save(&d).unwrap();
    // "restart": a new load from disk
    let again = RailLayouts::load(&d).get(7);
    assert_eq!(
        shown(&again),
        names(&[("All", false), ("c", false), ("a", false), ("b", true)])
    );
    assert_eq!(RailLayouts::load(&d).get(8), RailLayout::default()); // other account untouched
}

#[test]
fn edges_do_not_move_and_toggle_shows_again() {
    let mut layout = RailLayout::default();
    let cur = current(&layout);
    assert!(!layout.shift(&cur, "a", -1));
    assert!(!layout.shift(&cur, "c", 1));
    assert!(!layout.shift(&cur, "zz", 1));
    layout.toggle_hidden("a");
    layout.toggle_hidden("a");
    assert_eq!(shown(&layout), shown(&RailLayout::default()));
}

#[test]
fn new_and_vanished_spaces_are_harmless() {
    let mut layout = RailLayout::default();
    let cur = current(&layout);
    layout.shift(&cur, "c", -1); // order a, c, b
    let mut few = entries();
    few.remove(1); // "a" gone
    let out: Vec<_> = layout
        .arrange(few)
        .into_iter()
        .map(|(e, _)| e.name)
        .collect();
    assert_eq!(out, ["All", "c", "b"]);
}

fn load_raw(name: &str, raw: &[u8]) -> RailLayouts {
    let d = dirs(name);
    std::fs::create_dir_all(&d.config).unwrap();
    std::fs::write(d.config.join("rail-layout.json"), raw).unwrap();
    RailLayouts::load(&d)
}

#[test]
fn corrupt_files_fall_back_to_default_order() {
    for (i, raw) in [&b"{not json"[..], b"[]", b"42", b"", b"null", b"\xff\xfe"]
        .iter()
        .enumerate()
    {
        let l = load_raw(&format!("bad{i}"), raw);
        assert_eq!(l, RailLayouts::default(), "{raw:?}");
        assert_eq!(shown(&l.get(1)), shown(&RailLayout::default()));
    }
}

#[test]
fn invalid_fields_reset_only_that_field() {
    let l = load_raw(
        "fields",
        br#"{"1":{"order":["c",3],"hidden":["a"]},"2":{"order":["b","b"],"hidden":"x"},"3":{"order":["c","a"],"hidden":[null]},"x":{},"4":5}"#,
    );
    assert_eq!(
        shown(&l.get(1)),
        names(&[("All", false), ("a", true), ("b", false), ("c", false)])
    ); // bad order only
    assert_eq!(shown(&l.get(2)), shown(&RailLayout::default())); // duplicate and wrong type
    assert_eq!(
        shown(&l.get(3)),
        names(&[("All", false), ("c", false), ("a", false), ("b", false)])
    ); // bad hidden only
    assert_eq!(shown(&l.get(4)), shown(&RailLayout::default()));
}

#[test]
fn only_hidden_spaces_never_remove_tutte() {
    let mut layout = RailLayout::default();
    for id in ["a", "b", "c"] {
        layout.toggle_hidden(id);
    }
    let out = shown(&layout);
    assert_eq!(out[0], ("All".to_string(), false));
    assert!(out[1..].iter().all(|(_, h)| *h));
}

fn order(layout: &RailLayout) -> Vec<String> {
    current(layout)
}

#[test]
fn move_to_start_end_and_same_position() {
    let mut l = RailLayout::default();
    let cur = current(&l);
    assert!(l.move_to(&cur, "c", 0));
    assert_eq!(order(&l), ["c", "a", "b"]);
    let cur = current(&l);
    assert!(l.move_to(&cur, "c", 2));
    assert_eq!(order(&l), ["a", "b", "c"]);
    let before = l.clone();
    assert!(!l.move_to(&current(&l), "b", 1));
    assert_eq!(l, before);
}

#[test]
fn move_to_counts_hidden_spaces_and_keeps_them_hidden() {
    let mut l = RailLayout::default();
    l.toggle_hidden("b");
    // a, b(hidden), c: dropping "a" after the hidden one lands at index 2
    assert!(l.move_to(&current(&l), "a", 2));
    assert_eq!(
        shown(&l),
        names(&[("All", false), ("b", true), ("c", false), ("a", false)])
    );
    assert!(l.move_to(&current(&l), "a", 1));
    assert_eq!(
        shown(&l),
        names(&[("All", false), ("b", true), ("a", false), ("c", false)])
    );
}

#[test]
fn move_to_invalid_indices_change_nothing() {
    let mut l = RailLayout::default();
    let before = l.clone();
    let cur = current(&l);
    assert!(!l.move_to(&cur, "a", 3));
    assert!(!l.move_to(&cur, "a", usize::MAX));
    assert!(!l.move_to(&cur, "zz", 0));
    assert!(!l.move_to(&[], "a", 0));
    assert_eq!(l, before);
}
