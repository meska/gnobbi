//! Public prompt metadata only. Protected editor content never enters this projection.
use crate::{FormStatus, SignInForm, SignInSource};
use accesskit::{Action, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use matrix_engine::signin::SignInMode;

pub use crate::{DEFER_LABEL, RECOVERY_KEY_LABEL, RECOVERY_PENDING, RECOVERY_WARNING, RECOVER_LABEL, RETRY_LABEL};
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
}
impl Control {
    pub const ALL: [Self; 7] = [
        Self::Homeserver,
        Self::User,
        Self::Password,
        Self::Phrase,
        Self::Submit,
        Self::Recover,
        Self::Defer,
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
    pub bounds: [Option<accesskit::Rect>; 7],
    pub vault: bool,
    pub recovery: Option<matrix_engine::signin::RecoveryState>,
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
            signin: Some(signin),
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
            bounds: [None; 7],
            vault: form.vault,
            recovery: form.recovery(),
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
            Control::Recover | Control::Phrase if self.recovery.is_some() => {
                self.recovery != Some(RecoveryState::Unsupported)
            }
            _ if self.recovery.is_some() => false,
            Control::Submit => true,
            c => crate::visible_fields(self.mode, self.vault).contains(&(c as usize)),
        }
    }
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label(if self.recovery.is_some() {
            "Dime — Cronologia cifrata"
        } else if self.mode != SignInMode::Password {
            "Dime — Sblocca"
        } else {
            "Dime — Accedi"
        });
        root.set_description(if let Some(state) = self.recovery {
            if state == RecoveryState::Pending {
                RECOVERY_PENDING
            } else {
                crate::signin_form::recovery_message(state).unwrap_or(RECOVERY_WARNING)
            }
        } else {
            self.status.error_message().unwrap_or(match self.status {
            FormStatus::Opening => "Apertura profilo…",
            FormStatus::Busy => "Attendere…",
            _ => "Matrix diretto · nessun gateway",
        })
        });
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
                Control::Submit | Control::Recover | Control::Defer => Role::Button,
                _ => Role::TextInput,
            });
            node.set_label(match control {
                Control::Homeserver => "Homeserver HTTPS",
                Control::User => "ID Matrix completo",
                Control::Password => "Password Matrix",
                Control::Phrase if self.recovery.is_some() => RECOVERY_KEY_LABEL,
                Control::Phrase => "Frase locale · almeno 16 caratteri",
                Control::Submit if self.mode != SignInMode::Password => "Sblocca",
                Control::Submit => "Accedi",
                Control::Recover if matches!(self.recovery, Some(RecoveryState::Failed(_))) => RETRY_LABEL,
                Control::Recover => RECOVER_LABEL,
                Control::Defer => DEFER_LABEL,
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
