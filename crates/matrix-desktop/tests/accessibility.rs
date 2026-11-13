#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

#[test]
fn search_exports_query_and_actual_focus_without_exposing_hidden_controls() {
    use matrix_desktop::accessibility::{NavigationSnapshot, ROOT, SEARCH};
    let snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: true,
        query: "caffè".into(),
        search_focused: true,
        search_bounds: Some(accesskit::Rect::new(16., 64., 304., 104.)),
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    };
    let tree = snapshot.tree();
    assert_eq!(tree.focus, SEARCH);
    let search = &tree.nodes.iter().find(|(id, _)| *id == SEARCH).unwrap().1;
    assert_eq!(search.role(), accesskit::Role::SearchInput);
    assert_eq!(search.label(), Some("Cerca conversazioni"));
    assert_eq!(search.value(), Some("caffè"));
    assert_eq!(search.bounds(), snapshot.search_bounds);
    assert!(search.supports_action(accesskit::Action::Focus));
    assert_eq!(tree.nodes[0].1.label(), Some("Gnobbi"));
    let hidden = NavigationSnapshot {
        title: "Alice (@alice:example.org)".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        visible: false,
        ..snapshot
    }
    .tree();
    // the window node reads the same string as the native title
    assert_eq!(
        hidden.nodes[0].1.label(),
        Some("Alice (@alice:example.org)")
    );
    assert_eq!(hidden.focus, ROOT);
    assert_eq!(hidden.nodes.len(), 1);
    assert!(hidden.nodes[0].1.children().is_empty());
}
#[test]
fn search_text_run_exposes_unicode_query_with_append_only_caret() {
    use matrix_desktop::accessibility::{NavigationSnapshot, SEARCH};
    for query in ["caffè 🙂", ""] {
        let tree = NavigationSnapshot {
            title: "Gnobbi".into(),
            new_chat: Default::default(),
            room_search: Default::default(),
            #[cfg(all(feature = "live", unix))]
            signin: None,
            openers: vec![],
            back: None,
            drawer: None,
            scrollbars: vec![],
            operation_panel: None,
            spaces: None,
            navigation_revision: 0,
            icon_choice: None,
            new_room: None,
            devices: Default::default(),
            settings: None,
            about: None,
            details: Default::default(),
            notifications: Default::default(),
            dark: false,
            visible: true,
            query: query.into(),
            search_focused: true,
            search_bounds: None,
            rooms: vec![],
            history: None,
            list_focused: false,
            composer: None,
        }
        .tree();
        let search = &tree.nodes.iter().find(|(id, _)| *id == SEARCH).unwrap().1;
        assert_eq!(
            search.children().len(),
            1,
            "AT-SPI Text needs a real text run"
        );
        let run_id = search.children()[0];
        let run = &tree.nodes.iter().find(|(id, _)| *id == run_id).unwrap().1;
        assert_eq!(run.role(), accesskit::Role::TextRun);
        assert_eq!(run.value(), Some(query));
        assert_eq!(
            run.character_lengths(),
            query
                .chars()
                .map(|c| c.len_utf8() as u8)
                .collect::<Vec<_>>()
        );
        let selection = search.text_selection().unwrap();
        assert_eq!(selection.anchor, selection.focus);
        assert_eq!(selection.focus.node, run_id);
        assert_eq!(selection.focus.character_index, query.chars().count());
    }
}

#[test]
fn history_projection_is_bounded_scoped_and_keeps_event_identity_on_prepend() {
    use matrix_desktop::{DemoHistory, accessibility::HistoryNodes};
    let scope = matrix_domain::RoomScope {
        account_id: matrix_domain::AccountId::new(21),
        room_id: matrix_domain::RoomId::new("!history:fixture.invalid"),
    };
    let mut history = DemoHistory::new(scope.clone(), 10000);
    let mut nodes = HistoryNodes::default();
    let first = nodes.project(&history, 0, false);
    assert_eq!(first.messages.len(), 20);
    assert_eq!(first.total, 10000);
    assert_eq!(first.start, 0);
    assert!(first.older.is_none());
    assert_eq!(first.newer.as_ref(), Some(&history.events()[20].id));
    assert_eq!(first.messages[0].event, history.events()[0].id);
    assert_eq!(first.messages[0].text, history.events()[0].text);
    let stable = first.messages[0].id;
    history.prepend_older(25);
    assert_eq!(nodes.project(&history, 25, true).messages[0].id, stable);
    let last = nodes.project(&history, usize::MAX, true);
    assert_eq!(last.messages.len(), 1);
    assert_eq!(last.start, 10024);
    assert!(last.newer.is_none());
    assert!(last.older.is_some());
    let other = DemoHistory::new(
        matrix_domain::RoomScope {
            account_id: matrix_domain::AccountId::new(22),
            ..scope
        },
        10000,
    );
    assert_ne!(nodes.project(&other, 0, false).messages[0].id, stable);
    let empty = nodes.project(
        &DemoHistory::new(other.events()[0].id.scope.clone(), 0),
        0,
        false,
    );
    assert!(empty.messages.is_empty());
    assert!(empty.first.is_none());
    assert!(empty.last.is_none());
}

#[test]
fn history_tree_exports_scoped_focus_actions_and_logical_positions() {
    use matrix_desktop::{
        DemoHistory,
        accessibility::{HistoryNodes, NavigationAction, NavigationSnapshot},
    };
    let history = DemoHistory::new(
        matrix_domain::RoomScope {
            account_id: matrix_domain::AccountId::new(21),
            room_id: matrix_domain::RoomId::new("!history:fixture.invalid"),
        },
        10000,
    );
    let projected = HistoryNodes::default().project(&history, 20, true);
    let message = projected.messages[0].clone();
    let snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        list_focused: false,
        composer: None,
        history: Some(projected),
    };
    let tree = snapshot.tree();
    assert_eq!(tree.focus, message.id);
    let item = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == message.id)
        .unwrap()
        .1;
    assert_eq!(item.role(), accesskit::Role::ListItem);
    assert_eq!(item.label(), Some(message.text.as_str()));
    assert_eq!(item.position_in_set(), Some(20));
    assert_eq!(item.size_of_set(), Some(10000));
    let request = accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_tree: accesskit::TreeId::ROOT,
        target_node: message.id,
        data: None,
    };
    assert!(
        matches!(snapshot.action(&request), Some(NavigationAction::FocusMessage(id)) if id == message.event)
    );
    let hidden = NavigationSnapshot {
        new_chat: Default::default(),
        room_search: Default::default(),
        history: None,
        ..snapshot
    };
    assert!(hidden.action(&request).is_none());
    assert!(!hidden.tree().nodes.iter().any(|(id, _)| *id == message.id));
}

#[test]
fn history_navigation_targets_real_scoped_events_and_omits_unavailable_boundaries() {
    use matrix_desktop::{
        DemoHistory,
        accessibility::{HistoryNodes, NavigationAction, NavigationSnapshot},
    };
    let history = DemoHistory::new(
        matrix_domain::RoomScope {
            account_id: matrix_domain::AccountId::new(21),
            room_id: matrix_domain::RoomId::new("!history:fixture.invalid"),
        },
        100,
    );
    let mut snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        list_focused: false,
        composer: None,
        history: Some(HistoryNodes::default().project(&history, 20, false)),
    };
    for (label, target) in [
        ("Primo messaggio", 0),
        ("Messaggi precedenti", 0),
        ("Messaggi successivi", 40),
        ("Ultimo messaggio", 99),
    ] {
        let tree = snapshot.tree();
        let (id, button) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .expect("missing history control");
        assert_eq!(button.role(), accesskit::Role::Button);
        let request = accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: *id,
            data: None,
        };
        assert!(
            matches!(snapshot.action(&request), Some(NavigationAction::FocusMessage(id)) if id == history.events()[target].id)
        );
    }
    snapshot.history = Some(HistoryNodes::default().project(&history, 0, false));
    assert!(
        !snapshot
            .tree()
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Primo messaggio")
                || n.label() == Some("Messaggi precedenti"))
    );
    snapshot.history = None;
    assert!(
        !snapshot
            .tree()
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Ultimo messaggio"))
    );
}

#[test]
fn room_nodes_keep_scoped_identity_across_filtering_and_reject_hidden_targets() {
    use matrix_desktop::{Sidebar, accessibility::RoomNodes};
    let mut sidebar = Sidebar::demo(matrix_domain::AccountId::new(7));
    let mut registry = RoomNodes::default();
    let initial = registry.project(&sidebar);
    let second = initial[1].id;
    let scope = initial[1].scope.clone();
    sidebar.set_query("Design desktop");
    let filtered = registry.project(&sidebar);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, second);
    assert_eq!(registry.resolve(second), Some(&scope));
    assert!(registry.resolve(initial[0].id).is_none());
    sidebar.set_query("");
    assert_eq!(registry.project(&sidebar)[1].id, second);
}

#[test]
fn selected_room_is_the_semantic_focus_when_the_room_list_owns_keyboard_focus() {
    use matrix_desktop::{
        Sidebar,
        accessibility::{NavigationSnapshot, ROOM_LIST, RoomNodes},
    };
    let sidebar = Sidebar::demo(matrix_domain::AccountId::new(9));
    let rooms = RoomNodes::default().project(&sidebar);
    let selected = rooms[0].id;
    let snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: true,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms,
        history: None,
        list_focused: true,
        composer: None,
    };
    let tree = snapshot.tree();
    assert_eq!(tree.focus, selected);
    let list = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == ROOM_LIST)
        .unwrap()
        .1;
    assert_eq!(list.role(), accesskit::Role::ListBox);
    assert_eq!(list.children().len(), 300);
    let row = &tree.nodes.iter().find(|(id, _)| *id == selected).unwrap().1;
    assert_eq!(row.role(), accesskit::Role::ListBoxOption);
    assert_eq!(row.is_selected(), Some(true));
    // The fixture row has a last-message time, which depends on the machine clock and locale.
    assert!(row.label().is_some_and(|l| l.starts_with("Laboratorio UI, ultimo messaggio ")), "{:?}", row.label());
    assert!(row.supports_action(accesskit::Action::Click));
}

#[test]
fn composer_exports_multiline_role_draft_and_focus_independently_of_sidebar_visibility() {
    use matrix_desktop::accessibility::{COMPOSER, ComposerSnapshot, NavigationSnapshot};
    let snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: Some(ComposerSnapshot {
            send_source: Some(1),
            send_focused: false,
            icon_focused: None,
            live: true,
            send_bounds: None,
            attachment: None,
            mentions: None,
            recording: None,
            emoji_open: false,
            picker_pinned: false,
            picker_selected: None,
            picker_close_focused: false,
            text: "caffè 🙂\nseconda riga".into(),
            anchor: 0,
            cursor: 0,
            focused: true,
            bounds: Some(accesskit::Rect::new(344., 660., 1000., 700.)),
            scope: matrix_domain::RoomScope {
                account_id: matrix_domain::AccountId::new(9),
                room_id: matrix_domain::RoomId::new("!fixture:local.invalid"),
            },
            composing: false,
        }),
    };
    let tree = snapshot.tree();
    let editor = &tree.nodes.iter().find(|(id, _)| *id == COMPOSER).unwrap().1;
    assert_eq!(editor.role(), accesskit::Role::MultilineTextInput);
    assert_eq!(editor.label(), Some("Scrivi un messaggio"));
    assert_eq!(editor.value(), Some("caffè 🙂\nseconda riga"));
    assert_eq!(
        editor.bounds(),
        Some(accesskit::Rect::new(344., 660., 1000., 700.))
    );
    assert!(editor.supports_action(accesskit::Action::Focus));
    assert_eq!(tree.focus, COMPOSER);
    assert!(tree.nodes[0].1.children().contains(&COMPOSER));
    let hidden = NavigationSnapshot {
        new_chat: Default::default(),
        room_search: Default::default(),
        composer: None,
        ..snapshot.clone()
    }
    .tree();
    assert!(!hidden.nodes.iter().any(|(id, _)| *id == COMPOSER));
    use matrix_desktop::accessibility::{RECORD, RECORD_CANCEL, RECORD_SEND, RECORDING};
    let label_of = |tree: &accesskit::TreeUpdate, id| {
        tree.nodes
            .iter()
            .find(|(n, _)| *n == id)
            .map(|(_, node)| node.label().unwrap().to_owned())
    };
    assert_eq!(label_of(&tree, RECORD), None, "text to send: Send takes Voice's place");
    let mut blank = snapshot.clone();
    blank.composer.as_mut().unwrap().text.clear();
    assert_eq!(
        label_of(&blank.tree(), RECORD).as_deref(),
        Some("Registra nota vocale")
    );
    assert_eq!(label_of(&tree, RECORDING), None);
    let mut recording = snapshot.clone();
    recording.composer.as_mut().unwrap().recording = Some(7);
    let tree = recording.tree();
    assert_eq!(label_of(&tree, RECORD), None);
    assert_eq!(
        label_of(&tree, RECORDING).as_deref(),
        Some("Registrazione in corso, 7 secondi")
    );
    assert_eq!(
        label_of(&tree, RECORD_CANCEL).as_deref(),
        Some("Annulla registrazione")
    );
    assert_eq!(
        label_of(&tree, RECORD_SEND).as_deref(),
        Some("Invia nota vocale")
    );
    use matrix_desktop::accessibility::MENTIONS;
    assert_eq!(label_of(&tree, MENTIONS), None);
    let mut mentioning = snapshot.clone();
    mentioning.composer.as_mut().unwrap().mentions =
        Some((vec!["alice".into(), "Bruno Verdi".into()], 1));
    assert_eq!(
        label_of(&mentioning.tree(), MENTIONS).as_deref(),
        Some("Menzioni: alice; Bruno Verdi, selezionato")
    );
    use matrix_desktop::accessibility::{
        EMOJI, EMOJI_CLOSE, EMOJI_ITEM, EMOJI_PANEL, EMOJI_PICKER, EMOJI_TAB,
    };
    let expanded = |tree: &accesskit::TreeUpdate| {
        tree.nodes.iter().find(|(id, _)| *id == EMOJI).unwrap().1.is_expanded()
    };
    assert_eq!(label_of(&tree, EMOJI).as_deref(), Some("Emoji"));
    assert_eq!(expanded(&tree), Some(false));
    assert_eq!(label_of(&tree, EMOJI_PICKER), None);
    let mut picking = snapshot.clone();
    picking.composer.as_mut().unwrap().emoji_open = true;
    let tree = picking.tree();
    assert_eq!(label_of(&tree, EMOJI).as_deref(), Some("Emoji"), "the label does not change with the state");
    assert_eq!(expanded(&tree), Some(true));
    assert_eq!(label_of(&tree, EMOJI_PICKER).as_deref(), Some("Selettore emoji"));
    assert_eq!(label_of(&tree, EMOJI_CLOSE), None, "the popup has no Close");
    let node = |id| &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
    assert_eq!(node(EMOJI_PICKER).role(), accesskit::Role::Group);
    assert_eq!(node(EMOJI_TAB).role(), accesskit::Role::Tab);
    assert_eq!(node(EMOJI_TAB).is_selected(), Some(true));
    let picker = node(EMOJI_PANEL);
    assert_eq!(picker.role(), accesskit::Role::TabPanel);
    let labels: std::collections::BTreeSet<String> = picker
        .children()
        .iter()
        .map(|id| {
            let (_, n) = tree.nodes.iter().find(|(i, _)| i == id).unwrap();
            assert_eq!(n.role(), accesskit::Role::Button);
            n.label().unwrap().to_owned()
        })
        .collect();
    assert_eq!(picker.children().len(), 32);
    assert_eq!(labels.len(), 32);
    assert!(labels.iter().all(|l| !l.is_empty()));
    let mut drawer = picking.clone();
    {
        let c = drawer.composer.as_mut().unwrap();
        c.picker_pinned = true;
        c.picker_selected = Some(3);
        c.focused = false;
    }
    let tree = drawer.tree();
    assert_eq!(label_of(&tree, EMOJI_CLOSE).as_deref(), Some("Chiudi"));
    assert_eq!(tree.focus, accesskit::NodeId(EMOJI_ITEM + 3));
    {
        let c = drawer.composer.as_mut().unwrap();
        c.picker_selected = None;
        c.picker_close_focused = true;
    }
    assert_eq!(drawer.tree().focus, EMOJI_CLOSE);
}

#[test]
fn composer_text_runs_preserve_unicode_hard_lines_and_reversed_selection() {
    use matrix_desktop::accessibility::{COMPOSER, ComposerSnapshot, NavigationSnapshot};
    let tree = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: Some(ComposerSnapshot {
            send_source: Some(1),
            send_focused: false,
            icon_focused: None,
            live: true,
            send_bounds: None,
            attachment: None,
            mentions: None,
            recording: None,
            emoji_open: false,
            picker_pinned: false,
            picker_selected: None,
            picker_close_focused: false,
            text: "caffè 🙂\nseconda riga\n".into(),
            anchor: "caffè 🙂\nseconda riga\n".len(),
            cursor: "caffè ".len(),
            focused: true,
            bounds: None,
            scope: matrix_domain::RoomScope {
                account_id: matrix_domain::AccountId::new(9),
                room_id: matrix_domain::RoomId::new("!fixture:local.invalid"),
            },
            composing: false,
        }),
    }
    .tree();
    let editor = &tree.nodes.iter().find(|(id, _)| *id == COMPOSER).unwrap().1;
    assert_eq!(
        editor.children().len(),
        3,
        "hard lines including trailing empty line"
    );
    let runs: Vec<_> = editor
        .children()
        .iter()
        .map(|id| &tree.nodes.iter().find(|(n, _)| n == id).unwrap().1)
        .collect();
    assert_eq!(runs[0].role(), accesskit::Role::TextRun);
    assert_eq!(runs[0].value(), Some("caffè 🙂\n"));
    assert_eq!(runs[0].character_lengths(), &[1, 1, 1, 1, 2, 1, 4, 1]);
    assert_eq!(runs[1].value(), Some("seconda riga\n"));
    assert_eq!(runs[2].value(), Some(""));
    let selection = editor.text_selection().unwrap();
    assert_eq!(selection.anchor.node, editor.children()[2]);
    assert_eq!(selection.anchor.character_index, 0);
    assert_eq!(selection.focus.node, editor.children()[0]);
    assert_eq!(selection.focus.character_index, 6);
}

#[test]
fn send_control_is_actionable_only_for_a_committed_nonblank_draft() {
    use matrix_desktop::accessibility::{ComposerSnapshot, NavigationSnapshot, SEND};
    let mut snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: Some(ComposerSnapshot {
            send_source: Some(1),
            send_focused: false,
            icon_focused: None,
            live: true,
            send_bounds: None,
            attachment: None,
            mentions: None,
            recording: None,
            emoji_open: false,
            picker_pinned: false,
            picker_selected: None,
            picker_close_focused: false,
            text: "hello".into(),
            anchor: 5,
            cursor: 5,
            focused: true,
            bounds: None,
            scope: matrix_domain::RoomScope {
                account_id: matrix_domain::AccountId::new(9),
                room_id: matrix_domain::RoomId::new("!fixture:local.invalid"),
            },
            composing: false,
        }),
    };
    let button = |snapshot: &NavigationSnapshot| {
        snapshot
            .tree()
            .nodes
            .into_iter()
            .find(|(id, _)| *id == SEND)
            .map(|(_, n)| n)
    };
    // Voice and Send are one slot: exactly one of the two nodes exists at a time.
    let voice = |snapshot: &NavigationSnapshot| {
        snapshot.tree().nodes.iter().any(|(id, _)| *id == matrix_desktop::accessibility::RECORD)
    };
    let send = button(&snapshot).unwrap();
    assert_eq!(send.role(), accesskit::Role::Button);
    assert_eq!(send.label(), Some("Invia messaggio"));
    assert!(!send.is_disabled());
    assert_eq!(send.description(), None, "live Send is unchanged");
    assert!(send.supports_action(accesskit::Action::Click));
    assert!(!voice(&snapshot));
    snapshot.composer.as_mut().unwrap().live = false;
    let demo = button(&snapshot).unwrap();
    assert_eq!(demo.label(), Some("Invia messaggio"));
    assert_eq!(demo.description(), Some("demo"), "demo Send says «demo» without visible help text");
    snapshot.composer.as_mut().unwrap().live = true;
    snapshot.composer.as_mut().unwrap().composing = true;
    assert!(button(&snapshot).is_none() && voice(&snapshot));
    snapshot.composer.as_mut().unwrap().composing = false;
    snapshot.composer.as_mut().unwrap().text = " \t\n\u{a0}".into();
    assert!(button(&snapshot).is_none() && voice(&snapshot));
    snapshot.composer.as_mut().unwrap().text = "hello".into();
    snapshot.composer.as_mut().unwrap().recording = Some(3);
    assert!(button(&snapshot).is_none() && !voice(&snapshot), "recording: Send never replaces the recording controls");
    {
        use matrix_desktop::accessibility::NavigationAction;
        let on_send = |snapshot: &NavigationSnapshot, action| {
            snapshot.action(&accesskit::ActionRequest {
                action,
                target_tree: accesskit::TreeId::ROOT,
                target_node: SEND,
                data: None,
            })
        };
        assert!(on_send(&snapshot, accesskit::Action::Click).is_none(), "recording: no Send click");
        assert!(on_send(&snapshot, accesskit::Action::Focus).is_none(), "recording: no Send focus");
        snapshot.composer.as_mut().unwrap().recording = None;
        assert!(matches!(on_send(&snapshot, accesskit::Action::Click), Some(NavigationAction::SubmitComposer { .. })));
    }
    // Focus never points at a node that is not in the tree (accesskit_consumer panics on it).
    let c = snapshot.composer.as_mut().unwrap();
    c.text = String::new();
    c.send_focused = true;
    assert_ne!(snapshot.tree().focus, SEND);
    snapshot.composer = None;
    assert!(!snapshot.tree().nodes.iter().any(|(id, _)| *id == SEND));
}

#[test]
fn semantic_submit_captures_scope_and_body_and_rejects_hidden_or_uncommitted_drafts() {
    use matrix_desktop::accessibility::{
        ComposerSnapshot, NavigationAction, NavigationSnapshot, SEND,
    };
    let scope = matrix_domain::RoomScope {
        account_id: matrix_domain::AccountId::new(17),
        room_id: matrix_domain::RoomId::new("!fixture:local.invalid"),
    };
    let mut snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: Some(ComposerSnapshot {
            send_source: Some(1),
            send_focused: false,
            icon_focused: None,
            live: true,
            send_bounds: None,
            attachment: None,
            mentions: None,
            recording: None,
            emoji_open: false,
            picker_pinned: false,
            picker_selected: None,
            picker_close_focused: false,
            scope: scope.clone(),
            composing: false,
            text: "draft 🙂".into(),
            anchor: 0,
            cursor: 0,
            focused: false,
            bounds: None,
        }),
    };
    let request = accesskit::ActionRequest {
        action: accesskit::Action::Click,
        target_tree: accesskit::TreeId::ROOT,
        target_node: SEND,
        data: None,
    };
    let Some(NavigationAction::SubmitComposer {
        scope: target,
        text,
        source,
    }) = snapshot.action(&request)
    else {
        panic!("missing scoped submit command")
    };
    assert_eq!(target, scope);
    assert_eq!(text, "draft 🙂");
    assert!(
        snapshot
            .composer
            .as_ref()
            .unwrap()
            .matches_submission(&target, &text, source)
    );
    let other_account = matrix_domain::RoomScope {
        account_id: matrix_domain::AccountId::new(18),
        ..scope.clone()
    };
    assert!(
        !snapshot
            .composer
            .as_ref()
            .unwrap()
            .matches_submission(&other_account, &text, source)
    );
    let other_room = matrix_domain::RoomScope {
        room_id: matrix_domain::RoomId::new("!other:local.invalid"),
        ..scope.clone()
    };
    assert!(
        !snapshot
            .composer
            .as_ref()
            .unwrap()
            .matches_submission(&other_room, &text, source)
    );
    snapshot.composer.as_mut().unwrap().text = "changed".into();
    assert!(
        !snapshot
            .composer
            .as_ref()
            .unwrap()
            .matches_submission(&target, &text, source)
    );
    assert_eq!(
        text, "draft 🙂",
        "queued command must own its expected body"
    );
    snapshot.composer.as_mut().unwrap().composing = true;
    assert!(snapshot.action(&request).is_none());
    snapshot.composer.as_mut().unwrap().composing = false;
    snapshot.composer.as_mut().unwrap().text.clear();
    assert!(snapshot.action(&request).is_none());
    snapshot.composer = None;
    assert!(snapshot.action(&request).is_none());
    let malformed = accesskit::ActionRequest {
        data: Some(accesskit::ActionData::Value("ignored".into())),
        ..request
    };
    assert!(snapshot.action(&malformed).is_none());
}
