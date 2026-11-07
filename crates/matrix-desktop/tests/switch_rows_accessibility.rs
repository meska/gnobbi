//! Switch rows (Notifiche / Stanza / Conteggio): accessible Toggled matches what the view draws, Click flips it,
//! a disabled row is drawn off and takes no Click. Plus a source scan: no ON/OFF, no «· N/M/P», no «Chiudi · Esc» anywhere.
//! The painted side (pixels, height, X, keys, ring) is src/switch_rows_test.rs (private display).

/// Default build too: runs without the a11y feature and without a display.
#[test]
fn no_on_off_letters_or_close_esc_text_in_any_view() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = 0;
    for entry in std::fs::read_dir(&dir).unwrap().chain(std::fs::read_dir(dir.join("accessibility")).unwrap()) {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") || path.to_string_lossy().ends_with("_test.rs") {
            continue;
        }
        files += 1;
        let src = std::fs::read_to_string(&path).unwrap();
        for bad in ["\"ON\"", "\"OFF\"", "\"· N\"", "\"· M\"", "\"· P\"", " · N\"", " · M\"", " · P\"", "Chiudi · Esc", "Chiudi · esc"] {
            assert!(!src.contains(bad), "{} contains {bad:?}", path.display());
        }
    }
    assert!(files > 40, "scan looked at {files} files only");
}

/// Default build too: Tab order of the panel, wrapping; «Stanza» is skipped while no room is selected.
#[test]
fn tab_order_wraps_and_skips_the_room_switch_without_a_room() {
    use matrix_desktop::notifications::NotificationControl::*;
    let walk = |room, back| {
        let mut c = Enabled;
        (0..4).map(|_| { c = c.next(room, back); c }).collect::<Vec<_>>()
    };
    assert_eq!(walk(true, false), [Mute, Preview, Close, Enabled]);
    assert_eq!(walk(true, true), [Close, Preview, Mute, Enabled]);
    assert_eq!(walk(false, false), [Preview, Close, Enabled, Preview]);
    assert_eq!(walk(false, true), [Close, Preview, Enabled, Close]);
}

/// Panel headers close with the shared X icon, never a visible «Chiudi» word.
#[test]
fn panel_headers_close_with_the_x_icon() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in ["verification_view.rs", "notification_view.rs", "room_details_view.rs", "thread_view.rs"] {
        let src = std::fs::read_to_string(dir.join(file)).unwrap();
        assert!(!src.contains(".child(\"Chiudi\")"), "{file}: visible «Chiudi» text");
        assert!(src.contains("Icon::Close"), "{file}: no close icon");
    }
}

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
mod tree {
    use accesskit::{Action, ActionRequest, Toggled, TreeId};
    use matrix_desktop::accessibility::notifications::{
        ENABLED, MUTE, NotificationAction, NotificationSnapshot, NotificationView, PREVIEW,
    };
    use matrix_domain::{AccountId, RoomId, RoomScope};

    fn view(enabled: bool, room_muted: Option<bool>, preview: bool) -> NotificationView {
        named(enabled, room_muted, preview, "Privacy")
    }

    fn named(enabled: bool, room_muted: Option<bool>, preview: bool, name: &str) -> NotificationView {
        NotificationView::Open(Box::new(NotificationSnapshot {
            bounds: [None; 4],
            focus: Default::default(),
            enabled,
            preview,
            selected: room_muted.map(|m| (RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:fixture.invalid") }, m)),
            room_name: name.into(),
            service_failed: false,
        }))
    }

    fn click(id: accesskit::NodeId) -> ActionRequest {
        ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: id, data: None }
    }

    /// (toggled, disabled, clickable) of a node.
    fn node(v: &NotificationView, id: accesskit::NodeId) -> (Option<Toggled>, bool, bool) {
        let tree = v.tree().unwrap();
        let n = &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
        (n.toggled(), n.is_disabled(), n.supports_action(Action::Click))
    }

    #[test]
    fn toggled_follows_the_drawn_switch_and_click_flips_it() {
        // (view, expected Toggled of Notifiche / Stanza / Conteggio, drawn on = the same booleans)
        let cases = [
            ("on", view(true, Some(false), true), [Toggled::True; 3]),
            ("off", view(false, Some(true), false), [Toggled::False; 3]),
            ("mixed", view(true, Some(true), false), [Toggled::True, Toggled::False, Toggled::False]),
        ];
        for (name, v, want) in cases {
            for (id, want) in [ENABLED, MUTE, PREVIEW].into_iter().zip(want) {
                assert_eq!(node(&v, id), (Some(want), false, true), "{name}: node {id:?}");
            }
            let on = |t: Toggled| t == Toggled::True;
            assert_eq!(v.action(&click(ENABLED)), Some(NotificationAction::SetEnabled(!on(want[0]))), "{name}: Click Notifiche");
            assert_eq!(v.action(&click(PREVIEW)), Some(NotificationAction::SetPreview(!on(want[2]))), "{name}: Click Conteggio");
            let Some(NotificationAction::SetMuted { muted, .. }) = v.action(&click(MUTE)) else { panic!("{name}: Click Stanza") };
            // Stanza is on when NOT muted: clicking an "on" row mutes it.
            assert_eq!(muted, on(want[1]), "{name}: Click Stanza");
        }
    }

    #[test]
    fn room_switch_without_a_room_is_drawn_off_disabled_and_ignores_click() {
        for enabled in [true, false] {
            let v = view(enabled, None, true);
            assert_eq!(node(&v, MUTE), (Some(Toggled::False), true, false));
            assert!(v.action(&click(MUTE)).is_none());
            // The other two stay live.
            assert_eq!(node(&v, ENABLED), (Some(if enabled { Toggled::True } else { Toggled::False }), false, true));
            assert_eq!(node(&v, PREVIEW), (Some(Toggled::True), false, true));
        }
    }

    fn mute_label(v: &NotificationView) -> String {
        let tree = v.tree().unwrap();
        tree.nodes.iter().find(|(i, _)| *i == MUTE).unwrap().1.label().unwrap().to_owned()
    }

    #[test]
    fn room_name_is_in_the_label_on_or_off_follows_the_room_and_is_absent_without_one() {
        for muted in [false, true] {
            assert_eq!(mute_label(&named(true, Some(muted), true, "Privacy")), "Notifiche della stanza Privacy");
            // Another room selected while the panel is open: the label follows.
            assert_eq!(mute_label(&named(true, Some(muted), true, "Famiglia")), "Notifiche della stanza Famiglia");
        }
        // No room: no trailing space, no name, node disabled.
        let v = named(true, None, true, "");
        assert_eq!(mute_label(&v), "Notifiche della stanza");
        assert!(node(&v, MUTE).1);
    }
}
