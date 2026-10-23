#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use matrix_desktop::accessibility::{THEME, theme_tree};

#[test]
fn search_exports_query_and_actual_focus_without_exposing_hidden_controls() {
    use matrix_desktop::accessibility::{NavigationSnapshot, ROOT, SEARCH};
    let snapshot = NavigationSnapshot {
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
    let hidden = NavigationSnapshot {
        visible: false,
        ..snapshot
    }
    .tree();
    assert_eq!(hidden.focus, ROOT);
    assert_eq!(hidden.nodes.len(), 1);
    assert!(hidden.nodes[0].1.children().is_empty());
}
#[test]
fn search_text_run_exposes_unicode_query_with_append_only_caret() {
    use matrix_desktop::accessibility::{NavigationSnapshot, SEARCH};
    for query in ["caffè 🙂", ""] {
        let tree = NavigationSnapshot {
            #[cfg(all(feature = "live", unix))]
            signin: None,
            openers: vec![],
            back: None,
            scrollbars: vec![],
            operation_panel: None,
            spaces: None,
            navigation_revision: 0,
            theme_focused: false,
            theme_bounds: None,
            unread_filter: None,
            devices: Default::default(),
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
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
    assert_eq!(row.label(), Some("Laboratorio UI"));
    assert!(row.supports_action(accesskit::Action::Click));
}

#[test]
fn composer_exports_multiline_role_draft_and_focus_independently_of_sidebar_visibility() {
    use matrix_desktop::accessibility::{COMPOSER, ComposerSnapshot, NavigationSnapshot};
    let snapshot = NavigationSnapshot {
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
            send_bounds: None,
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
        composer: None,
        ..snapshot
    }
    .tree();
    assert!(!hidden.nodes.iter().any(|(id, _)| *id == COMPOSER));
}

#[test]
fn composer_text_runs_preserve_unicode_hard_lines_and_reversed_selection() {
    use matrix_desktop::accessibility::{COMPOSER, ComposerSnapshot, NavigationSnapshot};
    let tree = NavigationSnapshot {
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
            send_bounds: None,
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
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
            send_bounds: None,
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
            .unwrap()
            .1
    };
    assert_eq!(button(&snapshot).role(), accesskit::Role::Button);
    assert_eq!(button(&snapshot).label(), Some("Invia messaggio"));
    assert!(!button(&snapshot).is_disabled());
    assert!(button(&snapshot).supports_action(accesskit::Action::Click));
    snapshot.composer.as_mut().unwrap().composing = true;
    assert!(button(&snapshot).is_disabled());
    assert!(!button(&snapshot).supports_action(accesskit::Action::Click));
    snapshot.composer.as_mut().unwrap().composing = false;
    snapshot.composer.as_mut().unwrap().text = " \n".into();
    assert!(button(&snapshot).is_disabled());
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
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
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
            send_bounds: None,
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

#[test]
fn theme_control_exports_its_current_action_label_and_role() {
    let update = theme_tree(false);
    let (id, button) = update.nodes.iter().find(|(id, _)| *id == THEME).unwrap();
    assert_eq!(*id, THEME);
    assert_eq!(button.role(), accesskit::Role::Button);
    assert_eq!(button.label(), Some("Attiva tema scuro"));
    assert!(button.supports_action(accesskit::Action::Click));
    let dark = theme_tree(true);
    let (_, button) = dark.nodes.iter().find(|(id, _)| *id == THEME).unwrap();
    assert_eq!(button.label(), Some("Attiva tema chiaro"));
    let root = update.tree.unwrap().root;
    assert!(
        update
            .nodes
            .iter()
            .any(|(id, node)| *id == root && node.children().contains(&THEME))
    );
}

#[test]
fn theme_focus_is_actionable_visible_and_has_observed_geometry() {
    use matrix_desktop::accessibility::{NavigationAction, NavigationSnapshot, ROOT};
    let bounds = accesskit::Rect::new(250., 10., 300., 42.);
    let mut snapshot = NavigationSnapshot {
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: true,
        theme_bounds: Some(bounds),
        unread_filter: None,
        devices: Default::default(),
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: true,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    };
    let request = accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_node: THEME,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    assert!(matches!(
        snapshot.action(&request),
        Some(NavigationAction::FocusTheme(0))
    ));
    let tree = snapshot.tree();
    assert_eq!(tree.focus, THEME);
    let node = &tree.nodes.iter().find(|(id, _)| *id == THEME).unwrap().1;
    assert_eq!(node.bounds(), Some(bounds));
    assert!(node.supports_action(accesskit::Action::Focus));
    snapshot.visible = false;
    assert!(snapshot.action(&request).is_none());
    assert_eq!(snapshot.tree().focus, ROOT);
    assert!(!snapshot.tree().nodes.iter().any(|(id, _)| *id == THEME));
}
