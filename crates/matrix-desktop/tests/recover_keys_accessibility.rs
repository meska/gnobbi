#![cfg(all(feature = "linux-a11y", feature = "live", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Live, Role, TreeId};
use matrix_desktop::{
    DemoHistory,
    accessibility::{HistoryNodes, HistorySnapshot, NavigationAction, NavigationSnapshot},
};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};

fn snapshot(history: HistorySnapshot) -> NavigationSnapshot {
    NavigationSnapshot {
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
        unread_filter: None,
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
        history: Some(history),
        list_focused: false,
        composer: None,
    }
}

/// One undecryptable row and one readable row.
fn history() -> DemoHistory {
    let mut history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!r:test"),
        },
        0,
    );
    let event = |id: &str, utd: bool| matrix_engine::history::HistoryEvent {
        event_id: EventId::new(id),
        body: if utd {
            "Impossibile decifrare".into()
        } else {
            "leggibile".into()
        },
        outgoing: false,
        redacted: false,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status: false,
        utd,
        attachment_name: None,
        attachment_size: None,
        media: None,
        mentions: Vec::new(),
        highlighted: false,
        poll: None,
    };
    history.prepend_server(vec![event("$ok:test", false), event("$utd:test", true)]);
    history
}

fn labelled<'a>(
    tree: &'a accesskit::TreeUpdate,
    label: &str,
) -> Vec<&'a (accesskit::NodeId, accesskit::Node)> {
    tree.nodes
        .iter()
        .filter(|(_, n)| n.label() == Some(label))
        .collect()
}

#[test]
fn only_an_undecryptable_bubble_offers_the_control_and_only_while_incomplete() {
    let history = history();
    let utd = history.events()[1].id.clone();
    let mut projected = HistoryNodes::default().project(&history, 0, true);
    assert!(projected.messages[0].recover.is_none() && projected.messages[1].recover.is_none());

    projected.offer_recover_keys(false, None, None);
    assert!(
        labelled(&snapshot(projected.clone()).tree(), "Recupera chiavi").is_empty(),
        "keys_complete: no control anywhere"
    );

    projected.offer_recover_keys(true, None, None);
    projected.messages[1]
        .recover
        .as_mut()
        .unwrap()
        .control_bounds = Some(accesskit::Rect::new(1., 2., 80., 20.));
    assert!(
        projected.messages[0].recover.is_none(),
        "a readable row never offers it"
    );
    let current = snapshot(projected.clone());
    let tree = current.tree();
    let controls = labelled(&tree, "Recupera chiavi");
    assert_eq!(
        controls.len(),
        1,
        "one control, on the undecryptable bubble"
    );
    let (id, control) = controls[0];
    assert_eq!(control.role(), Role::Button);
    assert!(control.supports_action(Action::Click));
    assert_eq!(
        control.bounds(),
        Some(accesskit::Rect::new(1., 2., 80., 20.))
    );
    assert!(
        labelled(&tree, "Chiave di recupero Matrix").is_empty(),
        "no field until it is opened"
    );
    let parent = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Impossibile decifrare"))
        .unwrap();
    assert!(parent.1.children().contains(id));
    let click = ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *id,
        action: Action::Click,
        data: None,
    };
    assert!(matches!(current.action(&click), Some(NavigationAction::RecoverKeys(t)) if t == utd));
    assert!(current.history.as_ref().unwrap().allows_recover_keys(&utd));

    projected.mark_offline();
    assert!(
        labelled(&snapshot(projected).tree(), "Recupera chiavi").is_empty(),
        "the offline demo has no recovery"
    );
}

#[test]
fn the_open_field_has_its_label_focus_and_the_pending_state_is_announced() {
    let history = history();
    let utd = history.events()[1].id.clone();
    let mut projected = HistoryNodes::default().project(&history, 0, true);
    projected.offer_recover_keys(true, Some(&utd), Some(matrix_desktop::RECOVERY_PENDING));
    {
        let r = projected.messages[1].recover.as_mut().unwrap();
        r.field_bounds = Some(accesskit::Rect::new(3., 30., 200., 60.));
        r.field_focused = true;
    }
    let current = snapshot(projected.clone());
    let tree = current.tree();
    let fields = labelled(&tree, "Chiave di recupero Matrix");
    assert_eq!(fields.len(), 1);
    let (field_id, field) = fields[0];
    assert_eq!(
        field.role(),
        Role::PasswordInput,
        "the key is a secret field"
    );
    assert!(field.value().is_none(), "the typed key is never exported");
    assert_eq!(
        field.bounds(),
        Some(accesskit::Rect::new(3., 30., 200., 60.))
    );
    assert_eq!(
        tree.focus, *field_id,
        "the screen reader follows the key focus into the field"
    );
    let status = labelled(&tree, matrix_desktop::RECOVERY_PENDING);
    assert_eq!(status.len(), 1, "Pending is a node of its own");
    assert_eq!(status[0].1.live(), Some(Live::Polite));
    let focus = ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *field_id,
        action: Action::Focus,
        data: None,
    };
    assert!(matches!(current.action(&focus), Some(NavigationAction::RecoverKeys(t)) if t == utd));

    // A failure replaces Pending by the one-line message; another bubble never shows the field.
    projected.offer_recover_keys(
        true,
        Some(&utd),
        Some("Chiave di recupero Matrix non valida. Controlla e riprova."),
    );
    let tree = snapshot(projected.clone()).tree();
    assert_eq!(
        labelled(
            &tree,
            "Chiave di recupero Matrix non valida. Controlla e riprova."
        )
        .len(),
        1
    );
    assert!(labelled(&tree, matrix_desktop::RECOVERY_PENDING).is_empty());
    projected.offer_recover_keys(true, Some(&history.events()[0].id), None);
    assert!(
        labelled(
            &snapshot(projected.clone()).tree(),
            "Chiave di recupero Matrix"
        )
        .is_empty()
    );

    // Recovery completes: control, field and status vanish together.
    projected.offer_recover_keys(false, Some(&utd), Some("x"));
    let tree = snapshot(projected).tree();
    assert!(
        labelled(&tree, "Recupera chiavi").is_empty()
            && labelled(&tree, "Chiave di recupero Matrix").is_empty()
    );
}

#[test]
fn node_ids_of_the_new_controls_collide_with_nothing() {
    let history = history();
    let utd = history.events()[1].id.clone();
    let mut projected = HistoryNodes::default().project(&history, 0, true);
    projected.offer_recover_keys(true, Some(&utd), Some("riga"));
    let tree = snapshot(projected).tree();
    let mut ids: Vec<_> = tree.nodes.iter().map(|(id, _)| id.0).collect();
    let all = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all);
}

#[test]
fn keys_complete_false_offers_the_action_on_the_bubble_true_withdraws_it() {
    use matrix_engine::verification::VerificationEvent::KeysComplete;
    let account = AccountId::new(1);
    let history = history();
    let mut verification = matrix_desktop::DeviceVerification::new(account);
    let controls = |verification: &matrix_desktop::DeviceVerification| {
        let mut projected = HistoryNodes::default().project(&history, 0, true);
        projected.offer_recover_keys(verification.offers_recover_keys(true), None, None);
        labelled(&snapshot(projected).tree(), "Recupera chiavi").len()
    };
    assert_eq!(controls(&verification), 0, "unknown until the engine says");
    verification.apply(KeysComplete {
        account_id: account,
        complete: false,
    });
    assert_eq!(controls(&verification), 1);
    verification.apply(KeysComplete {
        account_id: account,
        complete: true,
    });
    assert_eq!(controls(&verification), 0);
}
