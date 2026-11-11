//! AT object paths are valid only for one continuous period of visibility.
//! Reusing a removed object can leave screen-reader caches permanently defunct.
use super::{NavigationAction, NavigationSnapshot};
use accesskit::{ActionRequest, NodeId, TreeUpdate};
use std::collections::{HashMap, HashSet};

pub struct SemanticSession {
    snapshot: NavigationSnapshot,
    current: HashMap<NodeId, NodeId>,
    next: u64,
    tree: TreeUpdate,
    install: super::install_offer::Snapshot,
}

impl SemanticSession {
    pub fn new(snapshot: NavigationSnapshot) -> Self {
        let tree = snapshot.tree();
        let mut result = Self {
            snapshot: snapshot.clone(),
            current: HashMap::new(),
            next: 1,
            tree,
            install: Default::default(),
        };
        result.update(snapshot);
        result
    }

    pub fn update(&mut self, snapshot: NavigationSnapshot) {
        #[cfg(all(feature = "live", unix))]
        if self.snapshot.signin.as_ref().map(|s| (s.source, s.mode))
            != snapshot.signin.as_ref().map(|s| (s.source, s.mode))
        {
            for control in super::signin::Control::ALL {
                self.current.remove(&control.id());
            }
        }
        if self
            .snapshot
            .composer
            .as_ref()
            .map(|c| (&c.scope, &c.text, c.composing, c.send_source))
            != snapshot
                .composer
                .as_ref()
                .map(|c| (&c.scope, &c.text, c.composing, c.send_source))
        {
            self.current.remove(&super::SEND);
        }
        if self.snapshot.navigation_revision != snapshot.navigation_revision {
            for i in 0..10 {
                self.current.remove(&NodeId(50 + i));
            }
        }
        for old in &self.snapshot.openers {
            if !snapshot
                .openers
                .iter()
                .any(|new| new.control == old.control && new.source == old.source)
            {
                self.current.remove(&old.control.id());
            }
        }
        for old in &self.snapshot.rooms {
            if !snapshot.rooms.iter().any(|new| {
                new.id == old.id && new.scope == old.scope && new.revision == old.revision
            }) {
                self.current.remove(&old.id);
            }
        }
        if let Some(old) = &self.snapshot.spaces
            && !snapshot
                .spaces
                .as_ref()
                .is_some_and(|new| new.revision == old.revision && new.account == old.account)
        {
            self.current.remove(&super::spaces::TOGGLE);
            for choice in &old.choices {
                self.current.remove(&choice.id);
            }
        }
        // A scrollbar represents one scoped conversation or filtered room set,
        // not a permanent capability to manipulate whichever pane comes next.
        for old in &self.snapshot.scrollbars {
            let id = old.node().0;
            if !snapshot
                .scrollbars
                .iter()
                .any(|bar| bar.node().0 == id && bar.pane == old.pane)
            {
                self.current.remove(&id);
            }
        }
        if let Some(history) = &self.snapshot.history {
            for old in &history.messages {
                if !snapshot.history.as_ref().is_some_and(|h| {
                    h.messages.iter().any(|m| {
                        m.event == old.event
                            && m.text == old.text
                            && m.selection.generation == old.selection.generation
                    })
                }) {
                    for control in super::selection::Control::ALL {
                        self.current.remove(&NodeId(old.id.0 + 3 + control as u64));
                    }
                }
            }
        }
        let mut tree = snapshot.tree();
        let visible: HashSet<_> = tree.nodes.iter().map(|(id, _)| *id).collect();
        self.current.retain(|id, _| visible.contains(id));
        for (id, _) in &tree.nodes {
            self.current.entry(*id).or_insert_with(|| {
                let allocated = self.next;
                self.next = self
                    .next
                    .checked_add(1)
                    .expect("semantic ID space exhausted");
                NodeId(allocated)
            });
        }
        // NavigationSnapshot currently emits only child and text-selection
        // NodeId relations. Extend this translation before adding any other
        // AccessKit relation (labelled_by, active_descendant, etc.) to it.
        let physical = |id: NodeId| self.current[&id];
        for (id, node) in &mut tree.nodes {
            *id = physical(*id);
            node.set_children(
                node.children()
                    .iter()
                    .copied()
                    .map(physical)
                    .collect::<Vec<_>>(),
            );
            if let Some(mut selection) = node.text_selection().cloned() {
                selection.anchor.node = physical(selection.anchor.node);
                selection.focus.node = physical(selection.focus.node);
                node.set_text_selection(selection);
            }
        }
        if let Some(metadata) = &mut tree.tree {
            metadata.root = physical(metadata.root);
        }
        tree.focus = physical(tree.focus);
        self.tree = tree;
        self.snapshot = snapshot;
    }

    /// The proposal and Settings row live outside the navigation snapshot.
    pub fn set_install_offer(&mut self, install: super::install_offer::Snapshot) {
        self.install = install;
    }

    pub fn tree(&self) -> TreeUpdate {
        if let Some(tree) = self.install.tree() {
            return tree;
        }
        let mut tree = self.tree.clone();
        self.install.add_settings_row(&mut tree);
        tree
    }

    pub fn action(&self, request: &ActionRequest) -> Option<NavigationAction> {
        if let Some(choice) = self.install.action(request) {
            return Some(NavigationAction::InstallOffer(choice));
        }
        if self.install.dialog.is_some() {
            return None;
        }
        let logical = self.current.iter().find_map(|(logical, physical)| {
            (*physical == request.target_node).then_some(*logical)
        })?;
        let mut request = request.clone();
        request.target_node = logical;
        self.snapshot.action(&request)
    }
}
