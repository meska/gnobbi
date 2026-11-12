//! Read-only room sheet. Semantic actions never mutate homeserver state.
use super::{ROOT, links};
use accesskit::{Action, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};
use matrix_domain::RoomScope;

pub const OPEN: NodeId = NodeId(30);
pub const SEARCH: NodeId = NodeId(33);
#[derive(Clone, Default)]
pub enum DetailsView {
    #[default]
    Unavailable,
    Closed(RoomScope),
    Open(Box<DetailsSnapshot>),
}
#[derive(Debug, PartialEq)]
pub enum DetailsAction {
    Open(RoomScope),
    Close(RoomScope),
    Copy(RoomScope),
    HideStatus(RoomScope),
    HideRedacted(RoomScope),
    Edit(RoomScope),
    /// Member row: read that user's profile.
    Profile {
        scope: RoomScope,
        user_id: String,
    },
    /// «Membri» link of a profile: back to the member list.
    Members(RoomScope),
    /// Action of the opened profile (message, link, quote, copy id).
    Act {
        scope: RoomScope,
        act: ProfileAct,
    },
    Focus {
        scope: RoomScope,
        control: DetailsControl,
    },
    /// Focus/Click on a link of «Argomento»: a click only asks for the confirmation.
    Link {
        scope: RoomScope,
        instance: u64,
        focus_only: bool,
    },
    /// Cancel/Open control of the topic link confirmation (`generation` names the painted request).
    LinkConfirmation {
        scope: RoomScope,
        generation: u64,
        control: links::ConfirmationControl,
        focus_only: bool,
    },
    /// Screen-reader focus on a control of the opened profile (index in the keyboard ring: ‹, ✕, acts).
    FocusRing { scope: RoomScope, index: usize },
}
impl DetailsView {
    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<DetailsAction> {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        if let Self::Open(panel) = self {
            if let Some(link) = panel.links.iter().find(|l| l.id() == request.target_node)
                && matches!(request.action, Action::Focus | Action::Click)
            {
                return Some(DetailsAction::Link {
                    scope: panel.scope.clone(),
                    instance: link.target.instance(),
                    focus_only: request.action == Action::Focus,
                });
            }
            if let Some(intent) = panel.confirmation.as_ref().and_then(|c| c.action(request)) {
                return Some(DetailsAction::LinkConfirmation {
                    scope: panel.scope.clone(),
                    generation: intent.request.generation(),
                    control: intent.control,
                    focus_only: intent.focus_only,
                });
            }
        }
        match self {
            Self::Closed(scope)
                if request.target_node == OPEN && request.action == Action::Click =>
            {
                Some(DetailsAction::Open(scope.clone()))
            }
            Self::Open(panel) if panel.back && request.action == Action::Focus => {
                let index = panel.ring_index(request.target_node)?;
                Some(DetailsAction::FocusRing { scope: panel.scope.clone(), index })
            }
            Self::Open(panel) if request.action == Action::Click && request.target_node == BACK => {
                panel.back.then(|| DetailsAction::Members(panel.scope.clone()))
            }
            Self::Open(panel)
                if request.action == Action::Click
                    && panel.acts.iter().enumerate().any(|(i, _)| panel.act_id(i) == request.target_node) =>
            {
                let i = (0..panel.acts.len()).find(|&i| panel.act_id(i) == request.target_node)?;
                Some(DetailsAction::Act { scope: panel.scope.clone(), act: panel.acts[i] })
            }
            Self::Open(panel)
                if request.action == Action::Click && panel.person(request.target_node).is_some() =>
            {
                Some(DetailsAction::Profile {
                    scope: panel.scope.clone(),
                    user_id: panel.person(request.target_node)?.to_owned(),
                })
            }
            Self::Open(panel) => {
                let control = DetailsControl::ALL
                    .into_iter()
                    .find(|c| c.id() == request.target_node)?;
                match request.action {
                    Action::Focus => Some(DetailsAction::Focus {
                        scope: panel.scope.clone(),
                        control,
                    }),
                    Action::Click => Some(match control {
                        DetailsControl::Topic => return None,
                        DetailsControl::Close => DetailsAction::Close(panel.scope.clone()),
                        DetailsControl::Copy => DetailsAction::Copy(panel.scope.clone()),
                        DetailsControl::HideStatus => DetailsAction::HideStatus(panel.scope.clone()),
                        DetailsControl::HideRedacted => {
                            DetailsAction::HideRedacted(panel.scope.clone())
                        }
                        DetailsControl::Edit => DetailsAction::Edit(panel.scope.clone()),
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    pub fn allows(&self, action: &DetailsAction) -> bool {
        match (self, action) {
            (Self::Closed(current), DetailsAction::Open(scope)) => current == scope,
            (
                Self::Open(panel),
                DetailsAction::Close(scope)
                | DetailsAction::Copy(scope)
                | DetailsAction::HideStatus(scope)
                | DetailsAction::HideRedacted(scope)
                | DetailsAction::Edit(scope)
                | DetailsAction::Profile { scope, .. }
                | DetailsAction::Members(scope)
                | DetailsAction::Act { scope, .. }
                | DetailsAction::Focus { scope, .. }
                | DetailsAction::Link { scope, .. }
                | DetailsAction::LinkConfirmation { scope, .. }
                | DetailsAction::FocusRing { scope, .. },
            ) => panel.scope == *scope,
            _ => false,
        }
    }
}

/// «Membri» link shown while a profile is open (ids 33.. are the labels).
pub const BACK: NodeId = NodeId(1 << 40);
/// Labels before the people in the sheet: name, kind, count, topic, encryption, room id.
const FIXED_LABELS: u64 = 6;

pub use crate::ProfileAct;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailsControl {
    #[default]
    Close,
    /// «Argomento»: scrollable area, a tab stop of its own (no click action).
    Topic,
    Copy,
    HideStatus,
    HideRedacted,
    Edit,
}
impl DetailsControl {
    /// Focus order = visual order.
    pub const ALL: [Self; 6] = [Self::Close, Self::Topic, Self::Copy, Self::HideStatus, Self::HideRedacted, Self::Edit];
    pub fn id(self) -> NodeId {
        match self {
            Self::Close => NodeId(31),
            Self::Copy => NodeId(32),
            Self::Topic => NodeId((1 << 40) + 4),
            // Above BACK: member labels take every id from 33 up.
            Self::HideStatus => NodeId((1 << 40) + 1),
            Self::HideRedacted => NodeId((1 << 40) + 2),
            Self::Edit => NodeId((1 << 40) + 3),
        }
    }
}
#[derive(Clone)]
pub struct DetailsSnapshot {
    pub scope: RoomScope,
    pub name: String,
    pub info: matrix_engine::RoomInfo,
    /// Members of the room (with the user id a click opens), or the opened profile.
    pub people: Vec<(String, Option<String>)>,
    /// A profile is shown (or loading): «Membri» leads back to the list.
    pub back: bool,
    /// Actions of the opened profile (empty for the room sheet).
    pub acts: Vec<ProfileAct>,
    pub focus: DetailsControl,
    /// Keyboard focus inside an opened profile: index in the ring ‹, ✕, then the acts.
    pub profile_focus: usize,
    pub bounds: [Option<accesskit::Rect>; 6],
    /// Current values: the toggles' accessible names follow the visible labels.
    pub hide_status: bool,
    pub hide_redacted: bool,
    /// Links of the topic text, in text order; children of the «Argomento» node.
    pub links: Vec<links::Snapshot>,
    /// Pending open-in-browser confirmation of a topic link.
    pub confirmation: Option<links::ConfirmationSnapshot>,
}
impl DetailsSnapshot {
    /// User id behind a member row's node.
    fn person(&self, id: NodeId) -> Option<&str> {
        let index = id.0.checked_sub(33 + FIXED_LABELS)? as usize;
        self.people.get(index)?.1.as_deref()
    }
    /// Node of the `index`th profile action (after the people labels).
    fn act_id(&self, index: usize) -> NodeId {
        NodeId(33 + FIXED_LABELS + self.people.len() as u64 + index as u64)
    }
    /// Node of the `index`th control of the profile's keyboard ring.
    fn ring_node(&self, index: usize) -> NodeId {
        match index {
            0 => BACK,
            1 => DetailsControl::Close.id(),
            i => self.act_id(i - 2),
        }
    }
    fn ring_index(&self, id: NodeId) -> Option<usize> {
        (0..2 + self.acts.len()).find(|&i| self.ring_node(i) == id)
    }
    pub fn next_focus(&self, reverse: bool) -> DetailsControl {
        let n = DetailsControl::ALL.len();
        let mut i = self.focus as usize;
        loop {
            i = (i + if reverse { n - 1 } else { 1 }) % n;
            if DetailsControl::ALL[i] != DetailsControl::Topic || self.has_topic() {
                return DetailsControl::ALL[i];
            }
        }
    }
    /// Same test as the view: the «Argomento» box exists only for a non-blank topic.
    fn has_topic(&self) -> bool {
        self.info.topic.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
    /// A focus left on the vanished «Argomento» box falls back to «Copia ID».
    fn focus_node(&self) -> NodeId {
        if self.focus == DetailsControl::Topic && !self.has_topic() {
            DetailsControl::Copy.id()
        } else {
            self.focus.id()
        }
    }
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Complementary);
        root.set_label("Informazioni stanza");
        let mut nodes = vec![];
        for (index, label) in [
            self.name.clone(),
            if self.info.is_space {
                "Spazio Matrix".into()
            } else {
                "Stanza Matrix".into()
            },
            format!(
                "{} {}",
                self.info.joined_members,
                if self.info.joined_members == 1 {
                    "membro"
                } else {
                    "membri"
                }
            ),
            format!(
                "Argomento: {}",
                self.info.topic.as_deref().unwrap_or("Nessun argomento")
            ),
            match self.info.encrypted {
                Some(true) => "Cifratura attiva",
                Some(false) => "Cifratura non attiva",
                None => "Cifratura non confermata dallo stato SDK",
            }
            .into(),
            self.scope.room_id.as_str().to_owned(),
        ]
        .into_iter()
        .chain(self.people.iter().map(|(label, _)| label.clone()))
        .enumerate()
        {
            let id = NodeId(33 + index as u64);
            let actionable = id.0 >= 33 + FIXED_LABELS && self.person(id).is_some();
            let mut node = Node::new(if actionable { Role::Button } else { Role::Label });
            if actionable {
                node.set_label(label);
                node.add_action(Action::Click);
            } else {
                node.set_value(label);
            }
            root.push_child(id);
            nodes.push((id, node));
        }
        for (index, (control, label)) in [
            (DetailsControl::Close, "Chiudi informazioni stanza"),
            (DetailsControl::Topic, "Argomento"),
            (DetailsControl::Copy, "Copia ID stanza"),
            (DetailsControl::HideStatus, "Messaggi di stato"),
            (DetailsControl::HideRedacted, "Messaggi eliminati"),
            (DetailsControl::Edit, "Modifica"),
        ]
        .into_iter()
        .enumerate()
        .filter(|(_, (control, _))| *control != DetailsControl::Topic || self.has_topic())
        {
            let on = match control {
                DetailsControl::HideStatus => Some(!self.hide_status),
                DetailsControl::HideRedacted => Some(!self.hide_redacted),
                _ => None,
            };
            let topic = control == DetailsControl::Topic;
            let mut node = Node::new(if on.is_some() {
                Role::Switch
            } else if topic {
                Role::ScrollView
            } else {
                Role::Button
            });
            node.set_label(label);
            if let Some(on) = on {
                node.set_toggled(if on { Toggled::True } else { Toggled::False });
            }
            if !topic {
                node.add_action(Action::Click);
            }
            node.add_action(Action::Focus);
            if let Some(bounds) = self.bounds[index] {
                node.set_bounds(bounds);
            }
            if topic {
                for link in &self.links {
                    node.push_child(link.id());
                    nodes.push((link.id(), link.node()));
                }
            }
            root.push_child(control.id());
            nodes.push((control.id(), node));
        }
        for (i, act) in self.acts.iter().enumerate() {
            let mut node = Node::new(Role::Button);
            node.set_label(act.label());
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            root.push_child(self.act_id(i));
            nodes.push((self.act_id(i), node));
        }
        if self.back {
            let mut node = Node::new(Role::Button);
            node.set_label("Membri");
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            root.push_child(BACK);
            nodes.push((BACK, node));
        }
        nodes.insert(0, (ROOT, root));
        let mut tree = TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: if self.back { self.ring_node(self.profile_focus) } else { self.focus_node() },
        };
        if let Some(confirmation) = &self.confirmation {
            confirmation.append_to(&mut tree);
        }
        tree
    }
}
