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

#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
mod tree {
    use accesskit::{Action, ActionRequest, Toggled, TreeId};
    use matrix_desktop::accessibility::notifications::{
        ENABLED, MUTE, NotificationAction, NotificationSnapshot, NotificationView, PREVIEW,
    };
    use matrix_domain::{AccountId, RoomId, RoomScope};

    fn view(enabled: bool, room_muted: Option<bool>, preview: bool) -> NotificationView {
        NotificationView::Open(Box::new(NotificationSnapshot {
            bounds: [None; 4],
            focus: Default::default(),
            enabled,
            preview,
            selected: room_muted.map(|m| (RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:fixture.invalid") }, m)),
            room_name: "Privacy".into(),
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
}
