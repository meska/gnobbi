#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
//! Accessibility tree of the member detail: one named button per action, ‹ and ✕, the confirmation, the focus.
use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::ProfileAct as A;
use matrix_desktop::Sidebar;
use matrix_desktop::accessibility::details::{BACK, DetailsAction, DetailsControl, DetailsSnapshot, DetailsView};
use matrix_domain::AccountId;

const ALL: [A; 16] = [
    A::CopyId,
    A::Message,
    A::Link,
    A::Quote,
    A::ReadReceipt,
    A::Verify,
    A::Power(0),
    A::Power(50),
    A::Power(100),
    A::RemoveMessages,
    A::Kick,
    A::Ban,
    A::Ignore,
    A::Unignore,
    A::Confirm,
    A::Cancel,
];

fn profile_panel(acts: Vec<A>) -> DetailsSnapshot {
    let sidebar = Sidebar::demo(AccountId::new(18));
    let room = sidebar.selected().unwrap();
    DetailsSnapshot {
        scope: room.scope.clone(),
        name: room.name.clone(),
        info: room.info.clone(),
        people: vec![("Profilo: Ada, @ada:fixture.invalid".into(), None)],
        back: true,
        acts,
        focus: DetailsControl::default(),
        profile_focus: 0,
        bounds: [None; 6],
        hide_status: false,
        hide_redacted: false,
    }
}

fn click(view: &DetailsView, id: accesskit::NodeId) -> Option<DetailsAction> {
    view.action(&ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: id, data: None })
}

#[test]
fn every_action_is_one_uniquely_named_button_that_routes_to_its_act() {
    let panel = profile_panel(ALL.to_vec());
    let tree = panel.tree();
    let scope = panel.scope.clone();
    let view = DetailsView::Open(Box::new(panel));
    let mut names = std::collections::BTreeSet::new();
    for act in ALL {
        let found: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Button && n.label() == Some(act.label())).collect();
        // Power(_) levels have distinct names; every other act appears once.
        assert_eq!(found.len(), 1, "exactly one button named «{}»", act.label());
        let (id, node) = found[0];
        assert!(node.supports_action(Action::Click), "«{}» is clickable", act.label());
        assert_eq!(click(&view, *id), Some(DetailsAction::Act { scope: scope.clone(), act }), "«{}» routes to its act", act.label());
        assert!(names.insert(act.label()), "name «{}» is unique", act.label());
    }
    assert_eq!(names.len(), ALL.len());
}

#[test]
fn back_and_close_are_named_buttons_and_back_exists_only_inside_a_profile() {
    let panel = profile_panel(vec![A::CopyId]);
    let tree = panel.tree();
    let named = |label: &str| tree.nodes.iter().find(|(_, n)| n.role() == Role::Button && n.label() == Some(label));
    let (id, back) = named("Membri").expect("‹ is announced as «Membri»");
    assert_eq!(*id, BACK);
    assert!(back.supports_action(Action::Click));
    let (id, close) = named("Chiudi informazioni stanza").expect("✕ is announced");
    assert_eq!(*id, DetailsControl::Close.id());
    assert!(close.supports_action(Action::Click) && close.supports_action(Action::Focus));
    let view = DetailsView::Open(Box::new(panel.clone()));
    assert_eq!(click(&view, BACK), Some(DetailsAction::Members(panel.scope.clone())), "‹ leads back to the member list");
    assert_eq!(click(&view, DetailsControl::Close.id()), Some(DetailsAction::Close(panel.scope.clone())), "✕ closes");
    let mut list = panel;
    list.back = false;
    list.acts = vec![];
    assert!(!list.tree().nodes.iter().any(|(id, _)| *id == BACK), "the member list has no ‹");
}

#[test]
fn confirmation_exposes_only_its_two_answers() {
    let panel = profile_panel(vec![A::Confirm, A::Cancel]);
    let buttons: Vec<_> = panel
        .tree()
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Button)
        .filter_map(|(_, n)| n.label().map(str::to_owned))
        .collect();
    for wanted in ["Conferma", "Annulla", "Membri", "Chiudi informazioni stanza"] {
        assert!(buttons.iter().any(|b| b == wanted), "{wanted}");
    }
    assert!(!buttons.iter().any(|b| ["Espelli", "Banna", "Ignora"].contains(&b.as_str())));
}

#[test]
fn profile_tree_focus_starts_on_the_first_control_and_acts_can_take_focus() {
    let panel = profile_panel(vec![A::CopyId, A::Message, A::Ignore]);
    let tree = panel.tree();
    // The keyboard ring starts on ‹ (members_view.rs profile_focus = 0); the tree must say the same.
    assert_eq!(tree.focus, BACK, "initial focus is ‹, the first control of the ring");
    for act in [A::CopyId, A::Message, A::Ignore] {
        let (_, node) = tree.nodes.iter().find(|(_, n)| n.label() == Some(act.label())).unwrap();
        assert!(node.supports_action(Action::Focus), "«{}» can take focus from a screen reader", act.label());
    }
}
