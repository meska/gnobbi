#![cfg(all(feature = "live", feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::{
    SignInForm,
    accessibility::{
        NavigationAction, NavigationSnapshot,
        signin::{Control, Snapshot},
    },
};
use matrix_engine::signin::{SignInEvent, SignInMode, SignInPrompt};

fn navigation(signin: Snapshot) -> NavigationSnapshot {
    let mut snapshot = NavigationSnapshot::for_signin(signin);
    snapshot.query = "hidden chat query".into();
    snapshot
}
fn prompt(form: &mut SignInForm) {
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Password,
        error: None,
    }));
}
fn request(id: accesskit::NodeId, action: Action) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    }
}
#[test]
fn unlock_projection_contains_only_protected_phrase_and_unlock_button() {
    let mut form = SignInForm::default();
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Unlock,
        error: None,
    }));
    form.homeserver = "hidden homeserver sentinel".into();
    form.user = "hidden user sentinel".into();
    form.password.replace(None, "hidden password sentinel");
    form.phrase.replace(None, "protected unlock sentinel");
    let mut snapshot = Snapshot::new(&form, SignInMode::Unlock);
    snapshot.bounds[3] = Some(accesskit::Rect::new(20., 120., 300., 156.));
    snapshot.focused = Some(Control::Phrase);
    let nav = navigation(snapshot);
    let tree = nav.tree();
    assert_eq!(tree.nodes.len(), 3);
    assert_eq!(tree.nodes[0].1.label(), Some("Gnobbi — Sblocca"));
    let phrase = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == Control::Phrase.id())
        .unwrap()
        .1;
    assert_eq!(phrase.role(), Role::PasswordInput);
    assert_eq!(phrase.label(), Some("Frase locale · almeno 16 caratteri"));
    assert!(phrase.value().is_none());
    assert!(phrase.children().is_empty());
    assert_eq!(phrase.bounds(), nav.signin.as_ref().unwrap().bounds[3]);
    assert_eq!(tree.focus, Control::Phrase.id());
    let submit = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == Control::Submit.id())
        .unwrap()
        .1;
    assert_eq!(submit.role(), Role::Button);
    assert_eq!(submit.label(), Some("Sblocca"));
    let dump = format!("{tree:?}");
    for forbidden in [
        "hidden homeserver sentinel",
        "hidden user sentinel",
        "hidden password sentinel",
        "protected unlock sentinel",
        "hidden chat query",
    ] {
        assert!(!dump.contains(forbidden));
    }
}

#[test]
fn unlock_actions_only_dispatch_current_visible_controls() {
    use matrix_desktop::accessibility::semantic_session::SemanticSession;
    let mut form = SignInForm::default();
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Unlock,
        error: None,
    }));
    let nav = navigation(Snapshot::new(&form, SignInMode::Unlock));
    for control in Control::ALL {
        for action in [Action::Focus, Action::Click] {
            assert_eq!(
                nav.action(&request(control.id(), action)).is_some(),
                matches!(control, Control::Phrase | Control::Submit)
            );
        }
    }
    let mut session = SemanticSession::new(nav);
    let old: Vec<_> = session
        .tree()
        .nodes
        .into_iter()
        .skip(1)
        .map(|(id, _)| request(id, Action::Focus))
        .collect();
    assert_eq!(old.len(), 2);
    for req in &old {
        assert!(session.action(req).is_some());
    }
    for mode in [SignInMode::Unlock, SignInMode::Password, SignInMode::Unlock] {
        form.apply(SignInEvent::Prompt(SignInPrompt { mode, error: None }));
        session.update(navigation(Snapshot::new(&form, mode)));
        for req in &old {
            assert!(session.action(req).is_none());
        }
    }
    form.phrase.replace(None, "fixture protected unlock phrase");
    assert!(form.submit().is_some());
    let busy = navigation(Snapshot::new(&form, SignInMode::Unlock));
    assert_eq!(busy.tree().nodes[0].1.description(), Some("Attendere…"));
    assert!(
        busy.tree()
            .nodes
            .iter()
            .skip(1)
            .all(|(_, n)| n.is_disabled())
    );
    for control in Control::ALL {
        assert!(busy.action(&request(control.id(), Action::Click)).is_none());
    }
}

#[test]
fn unlock_local_error_explains_phrase_without_requesting_hidden_password() {
    let mut form = SignInForm::default();
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Unlock,
        error: None,
    }));
    assert!(form.submit().is_none());
    assert_eq!(
        form.status().error_message(),
        Some("Inserisci la frase locale del profilo.")
    );
    let snapshot = Snapshot::new(&form, SignInMode::Unlock);
    let tree = navigation(snapshot).tree();
    assert_eq!(
        tree.nodes[0].1.description(),
        Some("Inserisci la frase locale del profilo.")
    );
}

#[test]
fn unlock_projection_ignores_focus_of_hidden_password_controls() {
    let mut form = SignInForm::default();
    form.apply(SignInEvent::Prompt(SignInPrompt {
        mode: SignInMode::Unlock,
        error: None,
    }));
    for control in [Control::Homeserver, Control::User, Control::Password] {
        let mut snapshot = Snapshot::new(&form, SignInMode::Unlock);
        snapshot.focused = Some(control);
        let tree = navigation(snapshot).tree();
        assert_eq!(tree.focus, matrix_desktop::accessibility::ROOT);
    }
}

#[test]
fn semantic_session_retires_identical_replacement_prompt_handles() {
    use matrix_desktop::accessibility::semantic_session::SemanticSession;
    let mut form = SignInForm::default();
    prompt(&mut form);
    let mut session = SemanticSession::new(navigation(Snapshot::new(&form, SignInMode::Password)));
    let tree = session.tree();
    let old = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Password Matrix"))
        .unwrap()
        .0;
    assert!(session.action(&request(old, Action::Focus)).is_some());
    prompt(&mut form);
    session.update(navigation(Snapshot::new(&form, SignInMode::Password)));
    assert!(
        session.action(&request(old, Action::Focus)).is_none(),
        "a native handle must not bind to a replacement prompt"
    );
}

#[test]
fn password_error_projection_explains_the_same_fixed_validation_error() {
    use matrix_engine::signin::SignInError;
    let mut form = SignInForm::default();
    for (error, message) in [
        (
            SignInError::InvalidHomeserver,
            "Inserisci un endpoint homeserver HTTPS valido.",
        ),
        (
            SignInError::InvalidUserId,
            "Inserisci l’ID Matrix completo: @nome:server.",
        ),
        (
            SignInError::InvalidSecret,
            "Password richiesta; frase locale di almeno 16 caratteri.",
        ),
        (
            SignInError::UnsupportedLogin,
            "Questo server non offre login password. SSO/OIDC non supportati.",
        ),
        (
            SignInError::Discovery,
            "Server non raggiungibile o certificato TLS non valido.",
        ),
        (
            SignInError::ProfileBusy,
            "Profilo già aperto in un’altra istanza. Chiudila e riavvia.",
        ),
    ] {
        form.apply(SignInEvent::Prompt(SignInPrompt {
            mode: SignInMode::Password,
            error: Some(error),
        }));
        let tree = navigation(Snapshot::new(&form, SignInMode::Password)).tree();
        assert_eq!(tree.nodes[0].1.description(), Some(message));
    }
}

#[test]
fn password_projection_protects_both_secrets_and_hides_chat() {
    let mut form = SignInForm::default();
    prompt(&mut form);
    form.homeserver = "https://fixture.test".into();
    form.user = "@alice:fixture.test".into();
    form.password.replace(None, "fixture password sentinel");
    form.phrase.replace(None, "fixture local phrase sentinel");
    let mut snapshot = Snapshot::new(&form, SignInMode::Password);
    snapshot.bounds[2] = Some(accesskit::Rect::new(20., 120., 300., 156.));
    snapshot.focused = Some(Control::Password);
    let nav = navigation(snapshot);
    let tree = nav.tree();
    for (control, role, label) in [
        (Control::Homeserver, Role::TextInput, "Homeserver HTTPS"),
        (Control::User, Role::TextInput, "ID Matrix completo"),
        (Control::Password, Role::PasswordInput, "Password Matrix"),
        (
            Control::Phrase,
            Role::PasswordInput,
            "Frase locale · almeno 16 caratteri",
        ),
        (Control::Submit, Role::Button, "Accedi"),
    ] {
        let node = &tree
            .nodes
            .iter()
            .find(|(id, _)| *id == control.id())
            .unwrap()
            .1;
        assert_eq!(node.role(), role);
        assert_eq!(node.label(), Some(label));
        assert!(node.supports_action(Action::Focus));
        if matches!(control, Control::Password | Control::Phrase) {
            assert!(node.value().is_none());
            assert!(node.children().is_empty());
        }
    }
    assert_eq!(tree.focus, Control::Password.id());
    assert_eq!(
        tree.nodes
            .iter()
            .find(|(id, _)| *id == Control::Password.id())
            .unwrap()
            .1
            .bounds(),
        nav.signin.as_ref().unwrap().bounds[2]
    );
    let dump = format!("{tree:?}");
    for forbidden in [
        "fixture password sentinel",
        "fixture local phrase sentinel",
        "hidden chat query",
    ] {
        assert!(!dump.contains(forbidden));
    }
    assert!(matches!(
        nav.action(&request(Control::Password.id(), Action::Focus)),
        Some(NavigationAction::SignIn(_))
    ));
}

fn node(tree: &accesskit::TreeUpdate, control: Control) -> Option<&accesskit::Node> {
    tree.nodes
        .iter()
        .find(|(id, _)| *id == control.id())
        .map(|(_, n)| n)
}
fn vault_form(mode: SignInMode) -> SignInForm {
    let mut form = SignInForm::default();
    form.vault = true;
    form.apply(SignInEvent::Prompt(SignInPrompt { mode, error: None }));
    form
}

#[test]
fn vault_password_projection_has_no_local_phrase_node() {
    let form = vault_form(SignInMode::Password);
    let nav = navigation(Snapshot::new(&form, SignInMode::Password));
    let tree = nav.tree();
    assert!(node(&tree, Control::Phrase).is_none());
    for control in [Control::Homeserver, Control::User, Control::Password, Control::Submit] {
        assert!(node(&tree, control).is_some());
        assert!(nav.action(&request(control.id(), Action::Click)).is_some());
    }
    assert!(nav.action(&request(Control::Phrase.id(), Action::Click)).is_none());
    assert!(!format!("{tree:?}").contains("Frase locale"));
}

#[test]
fn vault_unlock_projection_is_a_single_retry_button() {
    let form = vault_form(SignInMode::UnlockVault);
    let nav = navigation(Snapshot::new(&form, SignInMode::UnlockVault));
    let tree = nav.tree();
    assert_eq!(tree.nodes.len(), 2);
    assert_eq!(node(&tree, Control::Submit).unwrap().label(), Some("Riprova"));
    assert!(nav.action(&request(Control::Password.id(), Action::Click)).is_none());
    assert!(nav.action(&request(Control::Submit.id(), Action::Click)).is_some());
}

#[test]
fn recovery_step_projects_key_field_recover_and_defer_without_secrets() {
    use matrix_engine::signin::RecoveryState;
    let mut form = vault_form(SignInMode::Password);
    form.password.replace(None, "hidden password sentinel");
    form.apply(SignInEvent::Authenticated(matrix_domain::AccountId::new(1)));
    form.apply(SignInEvent::Recovery(RecoveryState::Offered));
    form.phrase.replace(None, "hidden recovery key sentinel");
    let nav = navigation(Snapshot::new(&form, SignInMode::Password));
    let tree = nav.tree();
    assert_eq!(tree.nodes[0].1.label(), Some("Gnobbi — Cronologia cifrata"));
    assert!(tree.nodes[0].1.description().unwrap().contains("non sarà leggibile"));
    assert_eq!(tree.nodes.len(), 4);
    let key = node(&tree, Control::Phrase).unwrap();
    assert_eq!(key.role(), Role::PasswordInput);
    assert_eq!(key.label(), Some("Chiave di recupero Matrix"));
    assert!(key.value().is_none());
    let recover = node(&tree, Control::Recover).unwrap();
    assert_eq!(recover.role(), Role::Button);
    assert_eq!(recover.label(), Some("Recupera cronologia cifrata"));
    assert!(!recover.is_disabled());
    for control in [Control::Recover, Control::Phrase, Control::Defer] {
        assert!(matches!(
            nav.action(&request(control.id(), Action::Click)),
            Some(NavigationAction::SignIn(i)) if i.control == control
        ));
    }
    let defer = node(&tree, Control::Defer).unwrap();
    assert_eq!(defer.label(), Some("Continua senza recupero"));
    for control in [Control::Homeserver, Control::User, Control::Password, Control::Submit] {
        assert!(nav.action(&request(control.id(), Action::Click)).is_none());
    }
    let dump = format!("{tree:?}");
    assert!(!dump.contains("hidden password sentinel") && !dump.contains("hidden recovery key sentinel"));
}

#[test]
fn recovery_states_project_pending_error_retry_and_unsupported() {
    use matrix_engine::signin::{RecoveryFailure, RecoveryState};
    let project = |state| {
        let mut form = vault_form(SignInMode::Password);
        form.apply(SignInEvent::Authenticated(matrix_domain::AccountId::new(1)));
        form.apply(SignInEvent::Recovery(RecoveryState::Offered));
        form.apply(SignInEvent::Recovery(state));
        let nav = navigation(Snapshot::new(&form, SignInMode::Password));
        let tree = nav.tree();
        (nav, tree)
    };
    let (nav, tree) = project(RecoveryState::Pending);
    assert!(tree.nodes[0].1.description().unwrap().contains("in corso"));
    assert!(tree.nodes.iter().skip(1).all(|(_, n)| n.is_disabled()));
    assert!(nav.action(&request(Control::Defer.id(), Action::Click)).is_none());

    let (nav, tree) = project(RecoveryState::Failed(RecoveryFailure::InvalidKey));
    assert!(tree.nodes[0].1.description().unwrap().contains("non valida"));
    assert_eq!(node(&tree, Control::Recover).unwrap().label(), Some("Riprova recupero"));
    assert!(nav.action(&request(Control::Recover.id(), Action::Click)).is_some());

    let (nav, tree) = project(RecoveryState::Unsupported);
    // No secret storage: no key field, but the distinct setup offer next to the skip.
    assert_eq!(tree.nodes.len(), 3);
    assert!(node(&tree, Control::Phrase).is_none());
    assert_eq!(node(&tree, Control::Recover).unwrap().label(), Some(matrix_desktop::SETUP_LABEL));
    assert!(nav.action(&request(Control::Defer.id(), Action::Click)).is_some());
    assert!(nav.action(&request(Control::Recover.id(), Action::Click)).is_some());
}

#[test]
fn recovery_setup_projects_offer_masked_key_copy_and_confirm_without_the_key() {
    use matrix_engine::recovery::RecoverySecret;
    use matrix_engine::signin::{RecoveryState, SetupEvent, SetupFailure};
    const KEY: &str = "SENTINEL recovery KEY 4242";
    let mut form = vault_form(SignInMode::Password);
    form.apply(SignInEvent::Authenticated(matrix_domain::AccountId::new(1)));
    form.apply(SignInEvent::Recovery(RecoveryState::Unsupported));
    form.apply_setup(SetupEvent::Pending);
    let tree = navigation(Snapshot::new(&form, SignInMode::Password)).tree();
    assert!(tree.nodes[0].1.description().unwrap().contains("in corso"));
    assert!(tree.nodes.iter().skip(1).all(|(_, n)| n.is_disabled()));

    form.apply_setup(SetupEvent::Failed(SetupFailure::Network));
    let nav = navigation(Snapshot::new(&form, SignInMode::Password));
    let tree = nav.tree();
    assert!(tree.nodes[0].1.description().unwrap().contains("non raggiungibile"));
    assert_eq!(node(&tree, Control::Recover).unwrap().label(), Some(matrix_desktop::SETUP_RETRY_LABEL));
    assert!(nav.action(&request(Control::Recover.id(), Action::Click)).is_some());

    form.apply_setup(SetupEvent::Failed(SetupFailure::Partial));
    let nav = navigation(Snapshot::new(&form, SignInMode::Password));
    let tree = nav.tree();
    assert!(node(&tree, Control::Recover).is_none(), "a permanent refusal offers no retry");
    assert!(nav.action(&request(Control::Recover.id(), Action::Click)).is_none());
    assert!(nav.action(&request(Control::Defer.id(), Action::Click)).is_some());

    form.apply_setup(SetupEvent::Ready(RecoverySecret::fixture(KEY)));
    let nav = navigation(Snapshot::new(&form, SignInMode::Password));
    let tree = nav.tree();
    assert_eq!(tree.nodes[0].1.description(), Some(matrix_desktop::SETUP_KEY_MASKED));
    assert_eq!(node(&tree, Control::Copy).unwrap().label(), Some(matrix_desktop::COPY_LABEL));
    assert_eq!(node(&tree, Control::Defer).unwrap().label(), Some(matrix_desktop::SAVED_LABEL));
    assert!(node(&tree, Control::Recover).is_none() && node(&tree, Control::Phrase).is_none());
    for control in [Control::Copy, Control::Defer] {
        assert!(matches!(
            nav.action(&request(control.id(), Action::Click)),
            Some(NavigationAction::SignIn(i)) if i.control == control
        ));
    }
    let dump = format!("{tree:?}");
    assert!(!dump.contains("SENTINEL") && !dump.contains("4242"), "the key never reaches the a11y tree");
}
