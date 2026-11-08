//! The account drawer as a modal semantic page: account rows (switch + remove), the other
//! drawer entries and the removal confirmation. Like the other panels it replaces the page tree.
use super::ROOT;
use accesskit::{Action, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use matrix_domain::AccountId;

const CLOSE: NodeId = NodeId(2);
const ITEM_BASE: u64 = 3; // + DrawerItem as u64
const CONFIRM: NodeId = NodeId(20);
const CANCEL: NodeId = NodeId(21);
const PROMPT: NodeId = NodeId(22);
const ACCOUNT_BASE: u64 = 1 << 20; // + 2 * account ID (switch), + 1 (remove)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawerItem {
    Settings,
    Devices,
    Notifications,
    AddAccount,
    NewChat,
    NewRoom,
    About,
}
impl DrawerItem {
    const ALL: [(Self, &'static str); 7] = [
        (Self::Settings, "Impostazioni"),
        (Self::Devices, "Dispositivi"),
        (Self::Notifications, "Notifiche"),
        (Self::AddAccount, "Aggiungi account"),
        (Self::NewChat, "Nuova chat"),
        (Self::NewRoom, "Nuova stanza"),
        (Self::About, "Informazioni su Gnobbi"),
    ];
}

#[derive(Clone)]
pub struct DrawerRow {
    pub id: AccountId,
    pub name: String,
    pub user_id: String,
    pub unread: u64,
    pub current: bool,
    /// False for the shown account and for one whose session is gone.
    pub switchable: bool,
}

#[derive(Clone, Default)]
pub struct DrawerSnapshot {
    pub rows: Vec<DrawerRow>,
    /// Account whose removal waits for the user's confirmation (name shown in the prompt).
    pub confirm: Option<(AccountId, String)>,
    /// Entries other than the account rows exist only while signed in.
    pub connected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawerAction {
    Switch(AccountId),
    AskRemove(AccountId),
    ConfirmRemove,
    CancelRemove,
    Item(DrawerItem),
    Close,
}

fn switch_node(id: AccountId) -> NodeId {
    NodeId(ACCOUNT_BASE + 2 * id.as_u64())
}
fn remove_node(id: AccountId) -> NodeId {
    NodeId(ACCOUNT_BASE + 2 * id.as_u64() + 1)
}

impl DrawerRow {
    /// Name, Matrix ID, unread count and whether it is the shown account, in one label.
    pub fn label(&self) -> String {
        let mut label = if self.name == self.user_id {
            self.name.clone()
        } else {
            format!("{}, {}", self.name, self.user_id)
        };
        match self.unread {
            0 => {}
            1 => label.push_str(", 1 messaggio non letto"),
            n => label.push_str(&format!(", {n} messaggi non letti")),
        }
        if self.current {
            label.push_str(", account attivo");
        }
        label
    }
}

fn button(label: impl Into<String>, enabled: bool) -> Node {
    let mut node = Node::new(Role::Button);
    node.set_label(label.into());
    if enabled {
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
    } else {
        node.set_disabled();
    }
    node
}

impl DrawerSnapshot {
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Menu account");
        let mut nodes = vec![];
        let mut add = |root: &mut Node, id: NodeId, node: Node| {
            root.push_child(id);
            nodes.push((id, node));
        };
        if let Some((_, name)) = &self.confirm {
            let mut prompt = Node::new(Role::Label);
            prompt.set_value(format!(
                "Rimuovere l'account {name}? Esce dalla sessione e cancella i dati locali."
            ));
            add(&mut root, PROMPT, prompt);
            add(&mut root, CANCEL, button("Annulla", true));
            add(&mut root, CONFIRM, button("Rimuovi account", true));
        } else {
            add(&mut root, CLOSE, button("Chiudi menu", true));
            for row in &self.rows {
                add(
                    &mut root,
                    switch_node(row.id),
                    button(row.label(), row.switchable),
                );
                add(
                    &mut root,
                    remove_node(row.id),
                    button(format!("Rimuovi account {}", row.name), true),
                );
            }
            for (i, (item, label)) in DrawerItem::ALL.into_iter().enumerate() {
                if self.connected || matches!(item, DrawerItem::Settings | DrawerItem::About) {
                    add(&mut root, NodeId(ITEM_BASE + i as u64), button(label, true));
                }
            }
        }
        nodes.insert(0, (ROOT, root));
        let focus = nodes.get(1).map_or(ROOT, |(id, _)| *id);
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    pub fn action(&self, request: &ActionRequest) -> Option<DrawerAction> {
        if request.target_tree != TreeId::ROOT
            || request.data.is_some()
            || request.action != Action::Click
        {
            return None;
        }
        let target = request.target_node;
        let action = if self.confirm.is_some() {
            match target {
                CONFIRM => DrawerAction::ConfirmRemove,
                CANCEL => DrawerAction::CancelRemove,
                _ => return None,
            }
        } else if target == CLOSE {
            DrawerAction::Close
        } else if let Some(row) = self.rows.iter().find(|r| switch_node(r.id) == target) {
            DrawerAction::Switch(row.id)
        } else if let Some(row) = self.rows.iter().find(|r| remove_node(r.id) == target) {
            DrawerAction::AskRemove(row.id)
        } else {
            let (item, _) = DrawerItem::ALL
                .into_iter()
                .enumerate()
                .find(|(i, _)| NodeId(ITEM_BASE + *i as u64) == target)
                .map(|(_, entry)| entry)?;
            DrawerAction::Item(item)
        };
        self.allows(&action).then_some(action)
    }

    /// Revalidate a queued action against the current drawer.
    pub fn allows(&self, action: &DrawerAction) -> bool {
        match action {
            DrawerAction::ConfirmRemove | DrawerAction::CancelRemove => self.confirm.is_some(),
            _ if self.confirm.is_some() => false,
            DrawerAction::Switch(id) => self.rows.iter().any(|r| r.id == *id && r.switchable),
            DrawerAction::AskRemove(id) => self.rows.iter().any(|r| r.id == *id),
            DrawerAction::Item(item) => self.connected || matches!(item, DrawerItem::Settings | DrawerItem::About),
            DrawerAction::Close => true,
        }
    }
}
