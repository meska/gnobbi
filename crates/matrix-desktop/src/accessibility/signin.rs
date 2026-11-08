//! Public prompt metadata only. Protected editor content never enters this projection.
use crate::{FormStatus, SignInForm, SignInSource};
use accesskit::{Action, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use matrix_engine::signin::SignInMode;

use crate::SetupPhase;
pub use crate::{
    COPY_LABEL, DEFER_LABEL, RECOVER_LABEL, RECOVERY_KEY_LABEL, RECOVERY_PENDING, RECOVERY_WARNING,
    RETRY_LABEL, SAVED_LABEL, SETUP_KEY_MASKED, SETUP_LABEL, SETUP_RETRY_LABEL,
};
use matrix_engine::signin::RecoveryState;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Homeserver,
    User,
    Password,
    Phrase,
    Submit,
    /// Post-login step only.
    Recover,
    Defer,
    /// Copies the generated recovery key; the key itself is never in the tree.
    Copy,
}
impl Control {
    pub const ALL: [Self; 8] = [
        Self::Homeserver,
        Self::User,
        Self::Password,
        Self::Phrase,
        Self::Submit,
        Self::Recover,
        Self::Defer,
        Self::Copy,
    ];
    pub fn id(self) -> NodeId {
        NodeId(100 + self as u64)
    }
}
#[derive(Clone)]
pub struct Snapshot {
    pub source: Option<SignInSource>,
    pub status: FormStatus,
    pub mode: SignInMode,
    pub public: [String; 2],
    pub bounds: [Option<accesskit::Rect>; 8],
    pub vault: bool,
    pub recovery: Option<matrix_engine::signin::RecoveryState>,
    pub setup: Option<SetupPhase>,
    pub setup_offered: bool,
    pub focused: Option<Control>,
}
#[derive(Clone, Copy)]
pub struct Intent {
    pub source: SignInSource,
    pub control: Control,
    pub focus_only: bool,
}
impl super::NavigationSnapshot {
    pub fn for_signin(signin: Snapshot) -> Self {
        Self {
            title: crate::window::TITLE.into(),
            new_chat: Default::default(),
            room_search: Default::default(),
            signin: Some(signin),
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
            history: None,
            list_focused: false,
            composer: None,
        }
    }
}
impl Snapshot {
    pub fn new(form: &SignInForm, mode: SignInMode) -> Self {
        Self {
            source: form.source(),
            status: form.status(),
            mode,
            public: [form.homeserver.clone(), form.user.clone()],
            bounds: [None; 8],
            vault: form.vault,
            recovery: form.recovery(),
            setup: form.setup_phase(),
            setup_offered: form.setup_offered(),
            focused: None,
        }
    }
    pub fn action(&self, request: &ActionRequest) -> Option<super::NavigationAction> {
        if request.target_tree != TreeId::ROOT
            || request.data.is_some()
            || !matches!(request.action, Action::Click | Action::Focus)
        {
            return None;
        }
        let control = Control::ALL
            .into_iter()
            .find(|c| c.id() == request.target_node)?;
        if !self.shows(control) {
            return None;
        }
        Some(super::NavigationAction::SignIn(Intent {
            source: self.source?,
            control,
            focus_only: request.action == Action::Focus,
        }))
    }
    fn shows(&self, control: Control) -> bool {
        match control {
            Control::Defer => self.recovery.is_some(),
            Control::Copy => self.setup == Some(SetupPhase::KeyShown),
            Control::Recover if self.recovery.is_some() => {
                self.recovery != Some(RecoveryState::Unsupported)
                    || self.setup_offered
                    || self.setup == Some(SetupPhase::Pending)
            }
            Control::Phrase if self.recovery.is_some() => {
                self.recovery != Some(RecoveryState::Unsupported)
            }
            _ if self.setup == Some(SetupPhase::KeyShown) => false,
            _ if self.recovery.is_some() => false,
            Control::Submit => true,
            c => crate::visible_fields(self.mode, self.vault).contains(&(c as usize)),
        }
    }
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label(crate::window::title_for(Some((
            crate::window::signin_section(
                self.recovery.is_some(),
                self.mode != SignInMode::Password,
            ),
            false,
        ))));
        let description = if let Some(state) = self.recovery {
            Some(if self.setup == Some(SetupPhase::KeyShown) {
                SETUP_KEY_MASKED
            } else if let Some(message) = self.setup.and_then(crate::signin_form::setup_message) {
                message
            } else if state == RecoveryState::Pending {
                RECOVERY_PENDING
            } else {
                crate::signin_form::recovery_message(state).unwrap_or(RECOVERY_WARNING)
            })
        } else {
            self.status.error_message().or(match self.status {
                FormStatus::Opening => Some("Apertura profilo…"),
                FormStatus::Busy => Some("Attendere…"),
                _ => None,
            })
        };
        if let Some(description) = description {
            root.set_description(description);
        }
        let mut tree = TreeUpdate {
            nodes: vec![(super::ROOT, root)],
            tree: Some(Tree::new(super::ROOT)),
            tree_id: TreeId::ROOT,
            focus: super::ROOT,
        };
        for control in Control::ALL.into_iter().filter(|c| self.shows(*c)) {
            let index = control as usize;
            let mut node = Node::new(match control {
                Control::Password | Control::Phrase => Role::PasswordInput,
                Control::Submit | Control::Recover | Control::Defer | Control::Copy => Role::Button,
                _ => Role::TextInput,
            });
            node.set_label(match control {
                Control::Homeserver => "Homeserver HTTPS",
                Control::User => "ID Matrix completo",
                Control::Password => "Password Matrix",
                Control::Phrase if self.recovery.is_some() => RECOVERY_KEY_LABEL,
                Control::Phrase => "Frase locale · almeno 16 caratteri",
                Control::Submit if self.mode == SignInMode::UnlockVault => "Riprova",
                Control::Submit if self.mode != SignInMode::Password => "Sblocca",
                Control::Submit => "Accedi",
                Control::Recover if self.recovery == Some(RecoveryState::Unsupported) => {
                    if matches!(self.setup, Some(SetupPhase::Failed(_))) {
                        SETUP_RETRY_LABEL
                    } else {
                        SETUP_LABEL
                    }
                }
                Control::Recover if matches!(self.recovery, Some(RecoveryState::Failed(_))) => {
                    RETRY_LABEL
                }
                Control::Recover => RECOVER_LABEL,
                Control::Defer if self.setup == Some(SetupPhase::KeyShown) => SAVED_LABEL,
                Control::Defer => DEFER_LABEL,
                Control::Copy => COPY_LABEL,
            });
            if index < 2 {
                node.set_value(self.public[index].clone());
            }
            if self.source.is_some() {
                node.add_action(Action::Focus);
                node.add_action(Action::Click);
            } else {
                node.set_disabled();
            }
            if let Some(bounds) = self.bounds[index] {
                node.set_bounds(bounds);
            }
            tree.nodes[0].1.push_child(control.id());
            tree.nodes.push((control.id(), node));
        }
        if self.source.is_some()
            && let Some(focused) = self.focused
            && tree.nodes.iter().any(|(id, _)| *id == focused.id())
        {
            tree.focus = focused.id();
        }
        tree
    }
}
