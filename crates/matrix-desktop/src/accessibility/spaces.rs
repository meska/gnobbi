//! Session-only filter over locally joined Matrix Spaces, never discovery/join.
use accesskit::{Action, Node, NodeId, Rect, Role, Toggled};
use matrix_domain::{AccountId, RoomScope};
use std::collections::HashMap;

pub const TOGGLE: NodeId = NodeId(60);
pub const ALL: NodeId = NodeId(61);

#[derive(Clone)]
pub struct SpaceChoice {
    pub id: NodeId,
    pub scope: Option<RoomScope>,
    pub name: String,
}

#[derive(Clone)]
pub struct SpacesSnapshot {
    pub revision: u64,
    pub persistent: bool,
    pub account: AccountId,
    pub open: bool,
    pub active: Option<RoomScope>,
    pub focused: Option<NodeId>,
    pub bounds: HashMap<NodeId, Rect>,
    pub choices: Vec<SpaceChoice>,
}

pub struct SpaceNodes {
    ids: HashMap<RoomScope, NodeId>,
    next_id: u64,
}
impl Default for SpaceNodes {
    fn default() -> Self {
        Self {
            ids: HashMap::new(),
            next_id: 1 << 57,
        }
    }
}
impl SpaceNodes {
    pub fn rail_choices(&mut self, sidebar: &crate::Sidebar) -> Vec<SpaceChoice> {
        let mut choices = self.choices(sidebar);
        if choices.is_empty() {
            choices.push(SpaceChoice {
                id: ALL,
                scope: None,
                name: String::new(),
            });
        }
        for (choice, entry) in choices.iter_mut().zip(sidebar.navigation_entries()) {
            choice.name = if entry.scope.is_none() {
                format!("Tutte le conversazioni · {} non letti", entry.unread)
            } else {
                format!("Spazio Matrix: {} · {} non letti", entry.name, entry.unread)
            };
        }
        choices
    }

    pub fn choices(&mut self, sidebar: &crate::Sidebar) -> Vec<SpaceChoice> {
        self.ids
            .retain(|scope, _| sidebar.spaces().any(|room| room.scope == *scope));
        if sidebar.spaces().next().is_none() {
            return vec![];
        }
        let mut choices = vec![SpaceChoice {
            id: ALL,
            scope: None,
            name: "Tutte le stanze".into(),
        }];
        for room in sidebar.spaces() {
            let id = *self.ids.entry(room.scope.clone()).or_insert_with(|| {
                assert!(self.next_id < (1 << 58), "semantic Space IDs exhausted");
                let id = NodeId(self.next_id);
                self.next_id += 1;
                id
            });
            choices.push(SpaceChoice {
                id,
                scope: Some(room.scope.clone()),
                name: room.name.clone(),
            });
        }
        choices
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpacesAction {
    Focus {
        revision: u64,
        account: AccountId,
        id: NodeId,
    },
    SetOpen {
        revision: u64,
        account: AccountId,
        open: bool,
    },
    Choose {
        revision: u64,
        account: AccountId,
        id: NodeId,
        scope: Option<RoomScope>,
        previous: Option<RoomScope>,
    },
}

impl SpacesSnapshot {
    /// None means leave the filter, never trap Tab within a menu.
    pub fn next_focus(&self, reverse: bool) -> Option<NodeId> {
        let ids: Vec<_> = std::iter::once(TOGGLE)
            .filter(|_| !self.persistent)
            .chain(
                self.choices
                    .iter()
                    .filter(|_| self.open)
                    .map(|choice| choice.id),
            )
            .collect();
        let index = ids.iter().position(|id| Some(*id) == self.focused)?;
        if reverse {
            index.checked_sub(1).map(|index| ids[index])
        } else {
            ids.get(index + 1).copied()
        }
    }
    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<SpacesAction> {
        if request.target_tree != accesskit::TreeId::ROOT || request.data.is_some() {
            return None;
        }
        let id = request.target_node;
        let known = (id == TOGGLE && !self.persistent)
            || (self.open && self.choices.iter().any(|choice| choice.id == id));
        if !known {
            return None;
        }
        match request.action {
            Action::Focus => Some(SpacesAction::Focus {
                revision: self.revision,
                account: self.account,
                id,
            }),
            Action::Click if id == TOGGLE => Some(SpacesAction::SetOpen {
                revision: self.revision,
                account: self.account,
                open: !self.open,
            }),
            Action::Click => Some(SpacesAction::Choose {
                revision: self.revision,
                account: self.account,
                id,
                scope: self
                    .choices
                    .iter()
                    .find(|choice| choice.id == id)?
                    .scope
                    .clone(),
                previous: self.active.clone(),
            }),
            _ => None,
        }
    }

    pub fn allows(&self, action: &SpacesAction) -> bool {
        let revision = match action {
            SpacesAction::Focus { revision, .. }
            | SpacesAction::SetOpen { revision, .. }
            | SpacesAction::Choose { revision, .. } => *revision,
        };
        if revision != self.revision {
            return false;
        }
        match action {
            SpacesAction::Focus { account, id, .. } => {
                *account == self.account
                    && ((*id == TOGGLE && !self.persistent)
                        || (self.open && self.choices.iter().any(|choice| choice.id == *id)))
            }
            SpacesAction::SetOpen { account, open, .. } => {
                !self.persistent && *account == self.account && *open != self.open
            }
            SpacesAction::Choose {
                account,
                id,
                scope,
                previous,
                ..
            } => {
                *account == self.account
                    && self.open
                    && self.active == *previous
                    && self
                        .choices
                        .iter()
                        .any(|choice| choice.id == *id && choice.scope == *scope)
            }
        }
    }

    pub fn nodes(&self) -> Vec<(NodeId, Node)> {
        let label = self
            .choices
            .iter()
            .find(|choice| choice.scope == self.active)
            .map_or("Tutte le stanze", |choice| choice.name.as_str());
        let mut toggle = Node::new(Role::Button);
        toggle.set_label(format!("Spazi: {label}"));
        toggle.set_expanded(self.open);
        // The pinned AT-SPI adapter does not export expanded/collapsed flags.
        toggle.set_description(if self.open {
            "Filtro degli Spazi joined · Aperto"
        } else {
            "Filtro degli Spazi joined · Chiuso"
        });
        toggle.add_action(Action::Click);
        toggle.add_action(Action::Focus);
        if let Some(bounds) = self.bounds.get(&TOGGLE) {
            toggle.set_bounds(*bounds);
        }
        let mut nodes = if self.persistent {
            vec![]
        } else {
            vec![(TOGGLE, toggle)]
        };
        if self.open {
            for choice in &self.choices {
                let mut node = Node::new(Role::RadioButton);
                node.set_label(choice.name.clone());
                node.set_toggled(if choice.scope == self.active {
                    Toggled::True
                } else {
                    Toggled::False
                });
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                if let Some(bounds) = self.bounds.get(&choice.id) {
                    node.set_bounds(*bounds);
                }
                nodes.push((choice.id, node));
            }
        }
        nodes
    }
}
