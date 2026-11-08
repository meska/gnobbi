//!: the on/off controls converted to «fixed positive label + switch»: the label never follows
//! the state, the role is Switch with Toggled exposed, and the stored keys/defaults (hide_* = negative on disk) did not move.
use matrix_desktop::tray_menu::{NOTIFICATIONS, TrayItem, tray_menu};
use matrix_desktop::{AppDirs, Preferences, RoomPrefs};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn dirs(name: &str) -> AppDirs {
    let root = std::env::temp_dir().join(format!("fixed-switch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    AppDirs { config: root.join("cfg"), data: root.join("data"), cache: root.join("cache") }
}

fn scope(room: &str) -> RoomScope {
    RoomScope { account_id: AccountId::new(7), room_id: RoomId::new(room) }
}

fn json(d: &AppDirs, file: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(d.config.join(file)).unwrap()).unwrap()
}

#[test]
fn tray_label_is_fixed_and_the_check_mark_carries_the_state() {
    for (window, enabled) in [(true, true), (true, false), (false, true), (false, false)] {
        let menu = tray_menu(window, enabled, &[]);
        let TrayItem::Entry { label, accessible_name, checked, .. } = &menu[1] else { panic!("entry expected") };
        assert_eq!((label.as_str(), accessible_name.as_str()), (NOTIFICATIONS, NOTIFICATIONS));
        assert_eq!(*checked, enabled, "window_visible={window}");
        // The window entry stays a plain action (no check mark): only «Notifiche» became a switch.
        assert!(matches!(&menu[0], TrayItem::Entry { checked: false, .. }));
    }
}

#[test]
fn stored_keys_and_defaults_are_unchanged_by_the_positive_ui() {
    // Global: «Messaggi eliminati» is hidden by default → the switch (!hide_redacted) starts OFF; key stays `hide_deleted`.
    let d = dirs("keys");
    let p = Preferences::default();
    assert!(p.hide_redacted && p.read_receipts && !p.close_to_tray && !p.debug);
    p.save(&d).unwrap();
    let v = json(&d, "settings.json");
    assert_eq!(v["hide_deleted"], true);
    assert!(v.get("show_deleted").is_none() && v.get("hide_redacted").is_none(), "no new/renamed key: {v}");
    for k in ["read_receipts", "close_to_tray", "debug"] {
        assert!(v.get(k).is_some(), "{k} missing");
    }
    // Flipping the switch ON (show deleted) is stored as hide_deleted=false, and reloads the same.
    Preferences { hide_redacted: false, ..p }.save(&d).unwrap();
    assert_eq!(json(&d, "settings.json")["hide_deleted"], false);
    assert!(!Preferences::load(&d).hide_redacted);

    // Per room: file name, key names and shapes (list for status, map for redacted) are the ones of before the conversion.
    let (a, b) = (scope("!a:test"), scope("!b:test"));
    let mut r = RoomPrefs::default();
    assert!(!r.hide_status(&a), "status rows shown by default → switch ON");
    assert_eq!(r.hide_redacted(&a), None, "no room choice → global default applies");
    r.toggle_hide_status(&a);
    r.set_hide_redacted(&b, false);
    r.save(&d).unwrap();
    let v = json(&d, "room-prefs.json");
    assert_eq!(v["hide_status"], serde_json::json!(["7|!a:test"]));
    assert_eq!(v["hide_redacted"], serde_json::json!({ "7|!b:test": false }));
    assert_eq!(v.as_object().unwrap().len(), 2);
}

#[test]
fn room_scope_is_not_global_and_rapid_toggling_ends_where_parity_says() {
    let d = dirs("scope");
    let (a, b) = (scope("!a:test"), scope("!b:test"));
    let mut r = RoomPrefs::default();
    for _ in 0..1001 {
        r.toggle_hide_status(&a);
    }
    r.save(&d).unwrap();
    let r = RoomPrefs::load(&d);
    assert!(r.hide_status(&a) && !r.hide_status(&b), "odd toggles hide room a only");
    // Room choice never touches the global preference.
    assert!(Preferences::load(&d).hide_redacted);
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
mod a11y {
    use accesskit::{Action, ActionRequest, Role, Toggled, TreeId};
    use matrix_desktop::accessibility::new_chat::{NewChatAction, NewChatView, Snapshot};
    use matrix_desktop::accessibility::settings::{LABELS, SettingsAction, SettingsSnapshot};

    fn click(node: accesskit::NodeId) -> ActionRequest {
        ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: node, data: None }
    }

    #[test]
    fn settings_switches_keep_their_labels_for_every_state_and_expose_it() {
        let mut seen = vec![];
        for bits in 0..16u8 {
            let on = [0, 1, 2, 3].map(|i| bits >> i & 1 == 1);
            let tree = SettingsSnapshot { focus: 0, on }.tree();
            let sw: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Switch).collect();
            let labels: Vec<_> = sw.iter().map(|(_, n)| n.label().unwrap().to_owned()).collect();
            assert_eq!(labels, LABELS, "state {bits:04b}");
            for (i, (id, n)) in sw.iter().enumerate() {
                assert_eq!(n.toggled(), Some(if on[i] { Toggled::True } else { Toggled::False }));
                // Click/Space/Enter all arrive as Click: explicit target = the opposite of what is shown.
                let want = SettingsSnapshot { focus: 0, on }.action(&click(*id));
                assert_eq!(want, Some(SettingsAction::Set(i, !on[i])));
            }
            seen.push(labels);
        }
        assert!(seen.windows(2).all(|w| w[0] == w[1]));
    }

    #[test]
    fn new_chat_scope_switch_has_one_label_and_follows_the_state() {
        let mk = |all| {
            NewChatView::Open(Box::new(Snapshot { text: String::new(), entries: vec![], active: 0, status: None, all }))
        };
        for all in [false, true] {
            let tree = mk(Some(all)).tree().unwrap();
            let (id, n) = tree.nodes.iter().find(|(_, n)| n.role() == Role::Switch).expect("switch");
            assert_eq!(n.label(), Some("Cerca anche su altre istanze"), "all={all}");
            assert_eq!(n.toggled(), Some(if all { Toggled::True } else { Toggled::False }));
            assert_eq!(mk(Some(all)).action(&click(*id)), Some(NewChatAction::ToggleAll));
        }
        // Server without cross-instance search: no switch at all, and the old «Stessa istanza|Tutti» segments are gone.
        let tree = mk(None).tree().unwrap();
        assert!(tree.nodes.iter().all(|(_, n)| n.role() != Role::Switch));
        assert!(tree.nodes.iter().all(|(_, n)| !matches!(n.label(), Some("Stessa istanza" | "Tutti"))));
    }
}
