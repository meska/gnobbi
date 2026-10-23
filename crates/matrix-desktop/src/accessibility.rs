//! Experimental Linux AT-SPI projection for theme, search and room navigation.
//! This is not complete screen-reader support for the messenger.

use accesskit::{Action, Node, NodeId, Role, Tree, TreeId, TreeUpdate};

pub mod compact;
pub mod details;
pub mod devices;
pub mod event_id_copy;
pub mod filters;
pub mod links;
pub mod notifications;
pub mod offline_delivery;
pub mod operation_panel;
pub mod pagination;
pub use crate::panel_openers;
pub mod scrollbars;
pub mod selection;
pub mod semantic_session;
#[cfg(all(feature = "live", unix))]
pub mod signin;
pub mod spaces;

pub const ROOT: NodeId = NodeId(1);
pub const THEME: NodeId = NodeId(2);
pub const SEARCH: NodeId = NodeId(3);
pub const ROOM_LIST: NodeId = NodeId(4);
pub const COMPOSER: NodeId = NodeId(5);
pub const SEND: NodeId = NodeId(6);

#[derive(Clone)]
pub struct ComposerSnapshot {
    pub send_source: Option<u64>,
    pub scope: matrix_domain::RoomScope,
    pub composing: bool,
    pub send_focused: bool,
    pub send_bounds: Option<accesskit::Rect>,
    pub text: String,
    /// UTF-8 byte positions from the editor, including selection direction.
    pub anchor: usize,
    pub cursor: usize,
    pub focused: bool,
    pub bounds: Option<accesskit::Rect>,
}

impl ComposerSnapshot {
    /// Revalidate the queued intent against the current editor, not a later draft.
    pub fn matches_submission(
        &self,
        scope: &matrix_domain::RoomScope,
        text: &str,
        source: u64,
    ) -> bool {
        self.send_source == Some(source)
            && self.scope == *scope
            && self.text == text
            && !self.composing
            && !text.trim().is_empty()
    }
}

#[derive(Clone)]
pub struct SemanticMessage {
    pub offline_delivery: Option<offline_delivery::Snapshot>,
    pub event_id_copy: Option<event_id_copy::Snapshot>,
    pub links: Vec<links::Snapshot>,
    pub selection: selection::Snapshot,
    pub actions_bounds: Option<accesskit::Rect>,
    pub actions_focused: bool,
    pub actions: Option<NodeId>,
    pub retry_bounds: Option<accesskit::Rect>,
    pub retry_focused: bool,
    pub retry: Option<NodeId>,
    pub bounds: Option<accesskit::Rect>,
    pub id: NodeId,
    pub event: matrix_domain::ScopedTimelineId,
    pub text: String,
    pub description: String,
}

#[derive(Clone, Default)]
pub struct HistorySnapshot {
    pub confirmation: Option<links::ConfirmationSnapshot>,
    pub server: Option<pagination::ServerPagination>,
    pub messages: Vec<SemanticMessage>,
    pub start: usize,
    pub total: usize,
    pub focused: bool,
    pub first: Option<matrix_domain::ScopedTimelineId>,
    pub last: Option<matrix_domain::ScopedTimelineId>,
    pub older: Option<matrix_domain::ScopedTimelineId>,
    pub newer: Option<matrix_domain::ScopedTimelineId>,
}

impl HistorySnapshot {
    /// Enrich a fresh projection with exact public unthreaded peer markers.
    pub fn apply_public_receipts(
        &mut self,
        scope: &matrix_domain::RoomScope,
        receipts: &[matrix_engine::receipts::PublicReadReceipt],
    ) {
        for message in &mut self.messages {
            if message.event.scope != *scope {
                continue;
            }
            let matrix_domain::TimelineId::Server(event) = &message.event.id else {
                continue;
            };
            if let Some(receipt) = receipts
                .iter()
                .find(|r| r.event_id == *event && r.readers > 0)
            {
                message.description = message.description.replace(
                    "Ricevuta di lettura non disponibile",
                    &format!(
                        "Ricevuta pubblica su questo evento: {} peer",
                        receipt.readers
                    ),
                );
            }
        }
    }

    pub fn mark_offline(&mut self) {
        for message in &mut self.messages {
            message
                .description
                .push_str(" · Demo offline · Nessuna consegna Matrix");
            message.retry = None;
            message.event_id_copy = None;
            message.actions = None;
        }
    }

    pub fn allows_actions(&self, target: &matrix_domain::ScopedTimelineId) -> bool {
        self.messages
            .iter()
            .any(|message| message.event == *target && message.actions.is_some())
    }

    pub fn actions_key(&self, key: &str, shift: bool) -> Option<NavigationAction> {
        if !self.focused {
            return None;
        }
        let message = self
            .messages
            .iter()
            .find(|message| message.actions_focused)
            .or_else(|| self.messages.first())?;
        message.actions?;
        match (key, shift, message.actions_focused) {
            ("tab", false, false) => {
                Some(NavigationAction::FocusMessageActions(message.event.clone()))
            }
            ("tab", true, true) => Some(NavigationAction::FocusMessage(message.event.clone())),
            ("tab", false, true) => Some(NavigationAction::FocusComposer),
            ("space" | "enter", false, true) => {
                Some(NavigationAction::OpenMessageActions(message.event.clone()))
            }
            _ => None,
        }
    }
    /// Route unmodified keyboard input within the focused message/retry pair.
    pub fn retry_key(&self, key: &str, shift: bool) -> Option<NavigationAction> {
        if !self.focused {
            return None;
        }
        let message = self
            .messages
            .iter()
            .find(|message| message.retry_focused)
            .or_else(|| self.messages.first())?;
        message.retry?;
        match (key, shift, message.retry_focused) {
            ("tab", false, false) => {
                Some(NavigationAction::FocusMessageRetry(message.event.clone()))
            }
            ("tab", true, true) => Some(NavigationAction::FocusMessage(message.event.clone())),
            ("tab", false, true) => Some(NavigationAction::FocusComposer),
            ("space" | "enter", false, true) => {
                Some(NavigationAction::RetryMessage(message.event.clone()))
            }
            _ => None,
        }
    }

    /// Revalidate a queued retry against the currently exposed failed message.
    pub fn allows_retry(&self, target: &matrix_domain::ScopedTimelineId) -> bool {
        self.messages
            .iter()
            .any(|message| message.event == *target && message.retry.is_some())
    }

    fn controls(
        &self,
    ) -> [(
        NodeId,
        &'static str,
        Option<&matrix_domain::ScopedTimelineId>,
    ); 4] {
        [
            (NodeId(10), "Primo messaggio", self.first.as_ref()),
            (NodeId(11), "Messaggi precedenti", self.older.as_ref()),
            (NodeId(12), "Messaggi successivi", self.newer.as_ref()),
            (NodeId(13), "Ultimo messaggio", self.last.as_ref()),
        ]
    }
}

pub struct HistoryNodes {
    ids: std::collections::HashMap<matrix_domain::ScopedTimelineId, NodeId>,
    next_id: u64,
}
impl Default for HistoryNodes {
    fn default() -> Self {
        Self {
            ids: Default::default(),
            next_id: 1_u64 << 62,
        }
    }
}
impl HistoryNodes {
    pub fn project(
        &mut self,
        history: &crate::DemoHistory,
        start: usize,
        focused: bool,
    ) -> HistorySnapshot {
        let events = history.events();
        if events.is_empty() {
            return HistorySnapshot::default();
        }
        let start = start.min(events.len() - 1);
        let end = start.saturating_add(20).min(events.len());
        let messages = events[start..end]
            .iter()
            .map(|event| {
                // The ACK changes protocol identity, not the focused message.
                let pending =
                    event
                        .transaction_id
                        .as_ref()
                        .map(|tx| matrix_domain::ScopedTimelineId {
                            scope: event.id.scope.clone(),
                            id: matrix_domain::TimelineId::Pending(tx.clone()),
                        });
                let retained = pending.as_ref().and_then(|key| self.ids.get(key)).copied();
                let id = retained.unwrap_or_else(|| {
                    *self.ids.entry(event.id.clone()).or_insert_with(|| {
                        let id = NodeId(self.next_id);
                        self.next_id = self
                            .next_id
                            .checked_add(8)
                            .expect("semantic history IDs exhausted");
                        id
                    })
                });
                self.ids.insert(event.id.clone(), id);
                if let Some(pending) = pending {
                    self.ids.insert(pending, id);
                }
                SemanticMessage {
                    offline_delivery: None,
                    event_id_copy: None,
                    links: Vec::new(),
                    selection: Default::default(),
                    actions_bounds: None,
                    actions_focused: false,
                    actions: (event.send_state.is_none()
                        && matches!(event.id.id, matrix_domain::TimelineId::Server(_)))
                    .then_some(NodeId(id.0 + 2)),
                    retry_bounds: None,
                    retry_focused: false,
                    retry: (event.send_state == Some(crate::DemoSendState::Failed)
                        && matches!(event.id.id, matrix_domain::TimelineId::Pending(_)))
                    .then_some(NodeId(id.0 + 1)),
                    bounds: None,
                    id,
                    event: event.id.clone(),
                    text: event.text.clone(),
                    description: format!(
                        "{} · {}{}{}{}{}",
                        event.time,
                        if event.outgoing {
                            "In uscita"
                        } else {
                            "In entrata"
                        },
                        match event.send_state {
                            Some(crate::DemoSendState::Pending) => " · In attesa",
                            Some(crate::DemoSendState::Failed) => " · Invio fallito",
                            None => "",
                        },
                        if event.outgoing && event.server_confirmed {
                            " · Confermato dal server · Ricevuta di lettura non disponibile"
                        } else {
                            ""
                        },
                        if event.annotation.is_empty() {
                            String::new()
                        } else {
                            format!(" · Reazione propria: {}", event.annotation)
                        },
                        if event.redacted {
                            " · Stato: eliminato"
                        } else {
                            ""
                        }
                    ),
                }
            })
            .collect();
        HistorySnapshot {
            confirmation: None,
            server: None,
            messages,
            start,
            total: events.len(),
            focused,
            first: (start > 0).then(|| events[0].id.clone()),
            last: (end < events.len()).then(|| events[events.len() - 1].id.clone()),
            older: (start > 0).then(|| events[start.saturating_sub(20)].id.clone()),
            newer: (end < events.len()).then(|| events[end].id.clone()),
        }
    }
}

#[derive(Clone)]
pub struct SemanticRoom {
    pub bounds: Option<accesskit::Rect>,
    pub focused: bool,
    pub revision: u64,
    pub id: NodeId,
    pub scope: matrix_domain::RoomScope,
    pub name: String,
    pub unread: u64,
    pub selected: bool,
}

pub struct RoomNodes {
    bounds: std::collections::HashMap<NodeId, (u64, accesskit::Rect)>,
    focused: Option<RoomIntent>,
    ids: std::collections::HashMap<matrix_domain::RoomScope, NodeId>,
    visible: std::collections::HashMap<NodeId, matrix_domain::RoomScope>,
    next_id: u64,
}

impl Default for RoomNodes {
    fn default() -> Self {
        Self {
            ids: Default::default(),
            visible: Default::default(),
            bounds: Default::default(),
            focused: None,
            next_id: 100,
        }
    }
}

impl RoomNodes {
    pub fn project(&mut self, sidebar: &crate::Sidebar) -> Vec<SemanticRoom> {
        self.ids
            .retain(|scope, _| sidebar.rooms().iter().any(|r| r.scope == *scope));
        self.visible.clear();
        sidebar
            .visible_indices()
            .iter()
            .map(|&index| {
                let room = &sidebar.rooms()[index];
                let id = *self.ids.entry(room.scope.clone()).or_insert_with(|| {
                    let id = NodeId(self.next_id);
                    self.next_id = self
                        .next_id
                        .checked_add(1)
                        .expect("semantic node IDs exhausted");
                    id
                });
                self.visible.insert(id, room.scope.clone());
                SemanticRoom {
                    bounds: self
                        .bounds
                        .get(&id)
                        .filter(|(revision, _)| *revision == sidebar.navigation_revision())
                        .map(|(_, bounds)| *bounds),
                    focused: self.focused.as_ref().is_some_and(|intent| {
                        intent.scope == room.scope
                            && intent.revision == sidebar.navigation_revision()
                    }),
                    revision: sidebar.navigation_revision(),
                    id,
                    scope: room.scope.clone(),
                    name: room.name.clone(),
                    unread: room.unread,
                    selected: sidebar.selected().is_some_and(|r| r.scope == room.scope),
                }
            })
            .collect()
    }

    pub fn resolve(&self, id: NodeId) -> Option<&matrix_domain::RoomScope> {
        self.visible.get(&id)
    }

    pub fn focus_action(&mut self, intent: &RoomIntent, sidebar: &crate::Sidebar) -> bool {
        if self.resolve_action(intent, sidebar).is_none() {
            return false;
        }
        self.focused = Some(intent.clone());
        true
    }

    pub fn focused(&self, sidebar: &crate::Sidebar) -> Option<&matrix_domain::RoomScope> {
        self.resolve_action(self.focused.as_ref()?, sidebar)
    }

    pub fn activation_key(&self, sidebar: &crate::Sidebar, key: &str) -> Option<RoomIntent> {
        if !matches!(key, "enter" | "space") {
            return None;
        }
        self.focused(sidebar)?;
        self.focused.clone()
    }

    pub fn set_bounds(
        &mut self,
        intent: &RoomIntent,
        sidebar: &crate::Sidebar,
        bounds: accesskit::Rect,
    ) -> bool {
        if self.resolve_action(intent, sidebar).is_none()
            || self.bounds.get(&intent.id) == Some(&(intent.revision, bounds))
        {
            return false;
        }
        self.bounds
            .retain(|_, (revision, _)| *revision == sidebar.navigation_revision());
        self.bounds.insert(intent.id, (intent.revision, bounds));
        true
    }

    pub fn resolve_action(
        &self,
        intent: &RoomIntent,
        sidebar: &crate::Sidebar,
    ) -> Option<&matrix_domain::RoomScope> {
        (intent.revision == sidebar.navigation_revision())
            .then(|| self.resolve(intent.id))
            .flatten()
            .filter(|scope| {
                **scope == intent.scope
                    && sidebar
                        .visible_indices()
                        .iter()
                        .any(|&i| sidebar.rooms()[i].scope == **scope)
            })
    }
}

#[derive(Clone)]
pub struct RoomIntent {
    pub id: NodeId,
    pub scope: matrix_domain::RoomScope,
    pub revision: u64,
}

/// UI-owned navigation state; hidden pages never leave actionable ghost nodes.
#[derive(Clone)]
pub struct NavigationSnapshot {
    pub openers: Vec<panel_openers::Snapshot>,
    #[cfg(all(feature = "live", unix))]
    pub signin: Option<signin::Snapshot>,
    pub back: Option<compact::BackSnapshot>,
    pub scrollbars: Vec<scrollbars::ScrollbarSnapshot>,
    pub operation_panel: Option<operation_panel::PanelSnapshot>,
    pub spaces: Option<spaces::SpacesSnapshot>,
    pub navigation_revision: u64,
    pub theme_focused: bool,
    pub theme_bounds: Option<accesskit::Rect>,
    pub unread_filter: Option<filters::UnreadFilter>,
    pub devices: devices::DeviceView,
    pub details: details::DetailsView,
    pub notifications: notifications::NotificationView,
    pub dark: bool,
    pub visible: bool,
    pub query: String,
    pub search_focused: bool,
    pub search_bounds: Option<accesskit::Rect>,
    pub rooms: Vec<SemanticRoom>,
    pub history: Option<HistorySnapshot>,
    pub list_focused: bool,
    pub composer: Option<ComposerSnapshot>,
}

impl NavigationSnapshot {
    /// Convert an AT request to an owned, bounded-queue command, not a UI callback.
    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<NavigationAction> {
        #[cfg(all(feature = "live", unix))]
        if let Some(signin) = &self.signin {
            return signin.action(request);
        }
        if request.target_tree == TreeId::ROOT
            && request.data.is_none()
            && matches!(request.action, Action::Click | Action::Focus)
            && let Some(control) = self.history.as_ref().and_then(|h| {
                h.messages
                    .iter()
                    .filter_map(|m| m.offline_delivery.as_ref())
                    .find(|s| s.id() == request.target_node)
            })
        {
            return Some(NavigationAction::OfflineDelivery(
                offline_delivery::Intent {
                    target: control.target.clone(),
                    focus_only: request.action == Action::Focus,
                },
            ));
        }
        if request.target_tree == TreeId::ROOT
            && request.data.is_none()
            && matches!(request.action, Action::Click | Action::Focus)
            && let Some(copy) = self.history.as_ref().and_then(|h| {
                h.messages
                    .iter()
                    .filter_map(|m| m.event_id_copy.as_ref())
                    .find(|c| c.id() == request.target_node)
            })
        {
            return Some(NavigationAction::CopyEventId(event_id_copy::Intent {
                target: copy.target.clone(),
                focus_only: request.action == Action::Focus,
            }));
        }
        if let Some(action) = self
            .history
            .as_ref()
            .and_then(|h| h.confirmation.as_ref())
            .and_then(|c| c.action(request))
        {
            return Some(NavigationAction::LinkConfirmation(action));
        }
        if request.target_tree == TreeId::ROOT
            && request.data.is_none()
            && matches!(request.action, Action::Click | Action::Focus)
            && let Some(link) = self.history.as_ref().and_then(|h| {
                h.messages
                    .iter()
                    .flat_map(|m| &m.links)
                    .find(|l| l.id() == request.target_node)
            })
        {
            return Some(NavigationAction::MessageLink(links::Intent {
                target: link.target.clone(),
                focus_only: request.action == Action::Focus,
            }));
        }
        if request.target_tree == TreeId::ROOT
            && request.data.is_none()
            && matches!(request.action, Action::Click | Action::Focus)
            && let Some(history) = &self.history
            && let Some((message, control)) = history.messages.iter().find_map(|m| {
                selection::Control::ALL
                    .into_iter()
                    .find(|c| {
                        (m.selection.selected || *c == selection::Control::Select)
                            && NodeId(m.id.0 + 3 + *c as u64) == request.target_node
                    })
                    .map(|c| (m, c))
            })
        {
            return Some(NavigationAction::MessageSelection(selection::Intent {
                generation: message.selection.generation,
                target: message.event.clone(),
                text: message.text.clone(),
                control,
                focus_only: request.action == Action::Focus,
            }));
        }
        for bar in &self.scrollbars {
            if self
                .tree()
                .nodes
                .iter()
                .any(|(id, node)| *id == bar.node().0 && node.role() == Role::ScrollBar)
                && let Some(action) = bar.action(request)
            {
                return Some(NavigationAction::Scrollbar(action));
            }
        }
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        if let Some(control) = panel_openers::Control::ALL
            .into_iter()
            .find(|c| c.id() == request.target_node)
        {
            if matches!(self.devices, devices::DeviceView::Open(_))
                || matches!(self.notifications, notifications::NotificationView::Open(_))
                || matches!(self.details, details::DetailsView::Open(_))
            {
                return None;
            }
            let opener = self.openers.iter().find(|s| s.control == control)?;
            return matches!(request.action, Action::Focus | Action::Click).then_some(
                NavigationAction::PanelOpener(panel_openers::Intent {
                    control,
                    source: opener.source,
                    focus_only: request.action == Action::Focus,
                }),
            );
        }
        if let Some(action) = self.devices.action(request) {
            return Some(NavigationAction::Devices(action));
        }
        if matches!(self.devices, devices::DeviceView::Open(_)) {
            return None;
        }
        if let Some(action) = self.notifications.action(request) {
            return Some(NavigationAction::Notifications(action));
        }
        if matches!(self.notifications, notifications::NotificationView::Open(_)) {
            return None;
        }
        if let Some(action) = self.details.action(request) {
            return Some(NavigationAction::Details(action));
        }
        if matches!(self.details, details::DetailsView::Open(_)) {
            return None;
        }
        if let Some(panel) = &self.operation_panel
            && let Some(action) = panel.action(request)
        {
            return Some(NavigationAction::OperationPanel(action));
        }
        if let Some(back) = &self.back
            && let Some(action) = back.action(request)
        {
            return Some(NavigationAction::CompactBack(action));
        }
        if self.visible
            && let Some(spaces) = &self.spaces
            && let Some(action) = spaces.action(request)
        {
            return Some(NavigationAction::Spaces(action));
        }
        if self.visible
            && let Some(filter) = &self.unread_filter
            && let Some(action) = filter.action(request)
        {
            return Some(NavigationAction::Filter(action));
        }
        if request.action == Action::Focus
            && let Some(server) = self.history.as_ref().and_then(|h| h.server.as_ref())
            && let Some((id, node)) = server.node()
            && request.target_node == id
            && node.supports_action(Action::Focus)
        {
            return Some(NavigationAction::FocusServerHistory(server.tag.clone()));
        }
        if request.action == Action::Focus
            && let Some(message) = self.history.as_ref().and_then(|history| {
                history
                    .messages
                    .iter()
                    .find(|m| m.actions == Some(request.target_node))
            })
        {
            return Some(NavigationAction::FocusMessageActions(message.event.clone()));
        }
        if request.action == Action::Focus
            && let Some(message) = self.history.as_ref().and_then(|history| {
                history
                    .messages
                    .iter()
                    .find(|m| m.retry == Some(request.target_node))
            })
        {
            return Some(NavigationAction::FocusMessageRetry(message.event.clone()));
        }
        if request.action == Action::Click
            && let Some(history) = &self.history
        {
            if let Some(message) = history
                .messages
                .iter()
                .find(|m| m.actions == Some(request.target_node))
            {
                return Some(NavigationAction::OpenMessageActions(message.event.clone()));
            }
            if let Some(message) = history
                .messages
                .iter()
                .find(|m| m.retry == Some(request.target_node))
            {
                return Some(NavigationAction::RetryMessage(message.event.clone()));
            }
            if let Some(server) = &history.server
                && let Some((id, node)) = server.node()
                && id == request.target_node
                && node.supports_action(Action::Click)
            {
                return Some(NavigationAction::LoadServerHistory(server.tag.clone()));
            }
            for (id, _, target) in history.controls() {
                if request.target_node == id {
                    return target.cloned().map(NavigationAction::FocusMessage);
                }
            }
        }
        match (request.target_node, request.action) {
            (id, Action::Focus)
                if self
                    .history
                    .as_ref()
                    .is_some_and(|history| history.messages.iter().any(|m| m.id == id)) =>
            {
                Some(NavigationAction::FocusMessage(
                    self.history
                        .as_ref()?
                        .messages
                        .iter()
                        .find(|m| m.id == id)?
                        .event
                        .clone(),
                ))
            }
            (SEND, Action::Focus) => {
                let composer = self.composer.as_ref()?;
                let source = composer.send_source?;
                (!composer.composing && !composer.text.trim().is_empty()).then(|| {
                    NavigationAction::FocusSend {
                        scope: composer.scope.clone(),
                        text: composer.text.clone(),
                        source,
                    }
                })
            }
            (SEND, Action::Click) => {
                let composer = self.composer.as_ref()?;
                let source = composer.send_source?;
                (!composer.composing && !composer.text.trim().is_empty()).then(|| {
                    NavigationAction::SubmitComposer {
                        scope: composer.scope.clone(),
                        text: composer.text.clone(),
                        source,
                    }
                })
            }
            (COMPOSER, Action::Focus) if self.composer.is_some() => {
                Some(NavigationAction::FocusComposer)
            }
            (THEME, Action::Click) if self.visible => {
                Some(NavigationAction::ToggleTheme(self.navigation_revision))
            }
            (THEME, Action::Focus) if self.visible => {
                Some(NavigationAction::FocusTheme(self.navigation_revision))
            }
            (SEARCH, Action::Focus) if self.visible => Some(NavigationAction::FocusSearch),
            (id, action @ (Action::Click | Action::Focus))
                if self.visible && self.rooms.iter().any(|room| room.id == id) =>
            {
                let room = self.rooms.iter().find(|room| room.id == id)?;
                let intent = RoomIntent {
                    id,
                    scope: room.scope.clone(),
                    revision: room.revision,
                };
                Some(if action == Action::Focus {
                    NavigationAction::FocusRoom(intent)
                } else {
                    NavigationAction::ActivateRoom(intent)
                })
            }
            _ => None,
        }
    }

    pub fn tree(&self) -> TreeUpdate {
        #[cfg(all(feature = "live", unix))]
        if let Some(signin) = &self.signin {
            return signin.tree();
        }
        if let devices::DeviceView::Open(panel) = &self.devices {
            return panel.tree();
        }
        if let Some(tree) = self.notifications.tree() {
            return tree;
        }
        if let details::DetailsView::Open(panel) = &self.details {
            return panel.tree();
        }
        let mut tree = theme_tree(self.dark);
        if self.visible {
            if let Some(bounds) = self.theme_bounds {
                tree.nodes[1].1.set_bounds(bounds);
            }
            if self.theme_focused {
                tree.focus = THEME;
            }
            let mut search = Node::new(Role::SearchInput);
            search.set_label("Cerca conversazioni");
            search.set_value(self.query.clone());
            let run_id = NodeId(7);
            let mut run = Node::new(Role::TextRun);
            run.set_value(self.query.clone());
            run.set_character_lengths(
                self.query
                    .chars()
                    .map(|c| c.len_utf8() as u8)
                    .collect::<Vec<_>>(),
            );
            search.push_child(run_id);
            let caret = accesskit::TextPosition {
                node: run_id,
                character_index: self.query.chars().count(),
            };
            search.set_text_selection(accesskit::TextSelection {
                anchor: caret,
                focus: caret,
            });
            tree.nodes.push((run_id, run));
            search.add_action(Action::Focus);
            if let Some(bounds) = self.search_bounds {
                search.set_bounds(bounds);
            }
            tree.nodes[0].1.push_child(SEARCH);
            tree.nodes.push((SEARCH, search));
            let mut list = Node::new(Role::ListBox);
            list.set_label("Conversazioni");
            list.set_children(self.rooms.iter().map(|room| room.id).collect::<Vec<_>>());
            tree.nodes[0].1.push_child(ROOM_LIST);
            tree.nodes.push((ROOM_LIST, list));
            for room in &self.rooms {
                let mut node = Node::new(Role::ListBoxOption);
                node.set_label(room.name.clone());
                node.set_description(match room.unread {
                    0 => "Nessun messaggio non letto".to_owned(),
                    1 => "1 messaggio non letto".to_owned(),
                    count => format!("{count} messaggi non letti"),
                });
                node.set_selected(room.selected);
                if let Some(bounds) = room.bounds {
                    node.set_bounds(bounds);
                }
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                tree.nodes.push((room.id, node));
            }
            if self.search_focused {
                tree.focus = SEARCH;
            } else if self.list_focused {
                tree.focus = self
                    .rooms
                    .iter()
                    .find(|room| room.focused)
                    .or_else(|| self.rooms.iter().find(|room| room.selected))
                    .map_or(ROOM_LIST, |room| room.id);
            }
        } else {
            tree.nodes.truncate(1);
            tree.nodes[0].1.set_children(Vec::<NodeId>::new());
        }
        if let Some(back) = &self.back {
            tree.nodes[0].1.push_child(back.id);
            tree.nodes.push((back.id, back.node()));
            if back.focused {
                tree.focus = back.id;
            }
        }
        if let Some(history) = &self.history {
            if let Some(server) = &history.server
                && let Some((id, node)) = server.node()
            {
                tree.nodes[0].1.push_child(id);
                tree.nodes.push((id, node));
                if server.focused {
                    tree.focus = id;
                }
            }
            let mut list = Node::new(Role::List);
            list.set_label("Storico messaggi");
            for (offset, message) in history.messages.iter().enumerate() {
                let mut node = Node::new(Role::ListItem);
                node.set_label(message.text.clone());
                node.set_description(message.description.clone());
                if let Some(bounds) = message.bounds {
                    node.set_bounds(bounds);
                }
                node.set_position_in_set(history.start + offset);
                node.set_size_of_set(history.total);
                node.add_action(Action::Focus);
                node.set_selected(message.selection.selected);
                for link in &message.links {
                    node.push_child(link.id());
                    tree.nodes.push((link.id(), link.node()));
                }
                if let Some(control) = &message.offline_delivery {
                    node.push_child(control.id());
                    tree.nodes.push((control.id(), control.node()));
                }
                if let Some(copy) = &message.event_id_copy {
                    node.push_child(copy.id());
                    tree.nodes.push((copy.id(), copy.node()));
                }
                if let Some(text) = &message.selection.text {
                    node.set_description(if text.is_empty() {
                        format!("{} · Selezione vuota", message.description)
                    } else {
                        format!("{} · Testo selezionato: {}", message.description, text)
                    });
                } else if message.selection.selected {
                    node.set_description(format!("{} · Testo selezionato", message.description));
                }
                for control in selection::Control::ALL {
                    if control != selection::Control::Select && !message.selection.selected {
                        continue;
                    }
                    let selection_id = NodeId(message.id.0 + 3 + control as u64);
                    let mut selection = Node::new(Role::Button);
                    selection.set_label(control.label());
                    selection.add_action(Action::Click);
                    selection.add_action(Action::Focus);
                    if let Some(bounds) = message.selection.bounds[control as usize] {
                        selection.set_bounds(bounds);
                    }
                    node.push_child(selection_id);
                    tree.nodes.push((selection_id, selection));
                }
                if let Some(id) = message.actions {
                    let mut button = Node::new(Role::Button);
                    button.set_label("Apri azioni messaggio");
                    button.add_action(Action::Click);
                    button.add_action(Action::Focus);
                    if let Some(bounds) = message.actions_bounds {
                        button.set_bounds(bounds);
                    }
                    node.push_child(id);
                    tree.nodes.push((id, button));
                }
                if let Some(id) = message.retry {
                    let mut retry = Node::new(Role::Button);
                    retry.set_label("Riprova invio messaggio");
                    retry.add_action(Action::Click);
                    retry.add_action(Action::Focus);
                    if let Some(bounds) = message.retry_bounds {
                        retry.set_bounds(bounds);
                    }
                    node.push_child(id);
                    tree.nodes.push((id, retry));
                }
                list.push_child(message.id);
                tree.nodes.push((message.id, node));
            }
            tree.nodes[0].1.push_child(NodeId(8));
            tree.nodes.push((NodeId(8), list));
            for (id, label, target) in history.controls() {
                if target.is_some() {
                    let mut button = Node::new(Role::Button);
                    button.set_label(label);
                    button.add_action(Action::Click);
                    tree.nodes[0].1.push_child(id);
                    tree.nodes.push((id, button));
                }
            }
            if history.focused
                && let Some(message) = history.messages.first()
            {
                tree.focus = message.id;
            }
            if let Some(id) = history
                .messages
                .iter()
                .find(|message| message.retry_focused)
                .and_then(|message| message.retry)
            {
                tree.focus = id;
            }
        }
        if let Some(id) = self.history.as_ref().and_then(|history| {
            history
                .messages
                .iter()
                .find(|m| m.actions_focused)
                .and_then(|m| m.actions)
        }) {
            tree.focus = id;
        }
        if let Some(id) = self.history.as_ref().filter(|h| h.focused).and_then(|h| {
            h.messages.iter().find_map(|m| {
                m.selection
                    .focused
                    .filter(|c| m.selection.selected || *c == selection::Control::Select)
                    .map(|c| NodeId(m.id.0 + 3 + c as u64))
            })
        }) {
            tree.focus = id;
        }
        if let Some(link) = self
            .history
            .as_ref()
            .filter(|h| h.focused)
            .and_then(|h| h.messages.iter().flat_map(|m| &m.links).find(|l| l.focused))
        {
            tree.focus = link.id();
        }
        if let Some(copy) = self.history.as_ref().filter(|h| h.focused).and_then(|h| {
            h.messages
                .iter()
                .filter_map(|m| m.event_id_copy.as_ref())
                .find(|c| c.focused)
        }) {
            tree.focus = copy.id();
        }
        if let Some(control) = self.history.as_ref().filter(|h| h.focused).and_then(|h| {
            h.messages
                .iter()
                .filter_map(|m| m.offline_delivery.as_ref())
                .find(|s| s.focused)
        }) {
            tree.focus = control.id();
        }
        if let Some(composer) = &self.composer {
            let mut editor = Node::new(Role::MultilineTextInput);
            editor.set_label("Scrivi un messaggio");
            editor.set_value(composer.text.clone());
            editor.add_action(Action::Focus);
            if let Some(bounds) = composer.bounds {
                editor.set_bounds(bounds);
            }
            let mut lines: Vec<&str> = composer.text.split_inclusive('\n').collect();
            if composer.text.is_empty() || composer.text.ends_with('\n') {
                lines.push("");
            }
            let mut start = 0;
            let mut anchor = None;
            let mut focus = None;
            for (index, line) in lines.iter().enumerate() {
                // Disjoint from fixed controls and monotonically allocated room IDs.
                let id = NodeId((1_u64 << 63) + index as u64);
                let mut run = Node::new(Role::TextRun);
                run.set_value(*line);
                // Matches the existing editor's Unicode-scalar selection granularity.
                run.set_character_lengths(
                    line.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>(),
                );
                for (offset, position) in [
                    (composer.anchor, &mut anchor),
                    (composer.cursor, &mut focus),
                ] {
                    if offset >= start && (offset < start + line.len() || index == lines.len() - 1)
                    {
                        *position = Some(accesskit::TextPosition {
                            node: id,
                            character_index: line
                                .char_indices()
                                .take_while(|(byte, _)| *byte < offset.saturating_sub(start))
                                .count(),
                        });
                    }
                }
                editor.push_child(id);
                tree.nodes.push((id, run));
                start += line.len();
            }
            if let (Some(anchor), Some(focus)) = (anchor, focus) {
                editor.set_text_selection(accesskit::TextSelection { anchor, focus });
            }
            tree.nodes[0].1.push_child(COMPOSER);
            tree.nodes.push((COMPOSER, editor));
            let mut send = Node::new(Role::Button);
            send.set_label("Invia messaggio");
            if let Some(bounds) = composer.send_bounds {
                send.set_bounds(bounds);
            }
            if composer.composing || composer.text.trim().is_empty() {
                send.set_disabled();
            } else {
                send.add_action(Action::Focus);
                send.add_action(Action::Click);
            }
            tree.nodes[0].1.push_child(SEND);
            tree.nodes.push((SEND, send));
            if composer.focused {
                tree.focus = COMPOSER;
            } else if composer.send_focused {
                tree.focus = SEND;
            }
        }
        for opener in &self.openers {
            let id = opener.control.id();
            let mut node = Node::new(Role::Button);
            node.set_label(opener.control.label());
            node.add_action(Action::Focus);
            node.add_action(Action::Click);
            if let Some(bounds) = opener.bounds {
                node.set_bounds(bounds);
            }
            tree.nodes[0].1.push_child(id);
            tree.nodes.push((id, node));
            if opener.focused {
                tree.focus = id;
            }
        }
        for bar in &self.scrollbars {
            if match bar.pane {
                scrollbars::Pane::Rooms(_) => self.visible,
                scrollbars::Pane::History(_) => self.history.is_some(),
            } {
                let (id, node) = bar.node();
                tree.nodes[0].1.push_child(id);
                tree.nodes.push((id, node));
            }
        }
        if self.visible
            && let Some(filter) = &self.unread_filter
        {
            let (id, node) = filter.node();
            tree.nodes[0].1.push_child(id);
            tree.nodes.push((id, node));
            if filter.focused {
                tree.focus = id;
            }
        }
        if self.visible
            && let Some(spaces) = &self.spaces
        {
            let nodes = spaces.nodes();
            if let Some(id) = spaces.focused
                && nodes.iter().any(|(node_id, _)| *node_id == id)
            {
                tree.focus = id;
            }
            for (id, node) in nodes {
                tree.nodes[0].1.push_child(id);
                tree.nodes.push((id, node));
            }
        }
        if let Some(panel) = &self.operation_panel {
            panel.append_to(&mut tree);
        }
        if let Some(confirmation) = self.history.as_ref().and_then(|h| h.confirmation.as_ref()) {
            confirmation.append_to(&mut tree);
        }
        tree
    }
}

/// Full tree for the first functional semantic slice, without message/account data.
pub fn theme_tree(dark: bool) -> TreeUpdate {
    let mut root = Node::new(Role::Window);
    root.set_label("Dime");
    root.set_children(vec![THEME]);
    let mut theme = Node::new(Role::Button);
    theme.set_label(if dark {
        "Attiva tema chiaro"
    } else {
        "Attiva tema scuro"
    });
    theme.add_action(Action::Click);
    theme.add_action(Action::Focus);
    TreeUpdate {
        nodes: vec![(ROOT, root), (THEME, theme)],
        tree: Some(Tree::new(ROOT)),
        tree_id: TreeId::ROOT,
        focus: ROOT,
    }
}

use std::sync::{Arc, Mutex, mpsc};

struct Activation(Arc<Mutex<semantic_session::SemanticSession>>);
impl accesskit::ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        Some(self.0.lock().unwrap().tree())
    }
}

pub enum NavigationAction {
    PanelOpener(panel_openers::Intent),
    #[cfg(all(feature = "live", unix))]
    SignIn(signin::Intent),
    CopyEventId(event_id_copy::Intent),
    OfflineDelivery(offline_delivery::Intent),
    CompactBack(compact::BackIntent),
    LinkConfirmation(links::ConfirmationIntent),
    MessageSelection(selection::Intent),
    MessageLink(links::Intent),
    Scrollbar(scrollbars::ScrollbarAction),
    OperationPanel(operation_panel::PanelAction),
    FocusMessageActions(matrix_domain::ScopedTimelineId),
    OpenMessageActions(matrix_domain::ScopedTimelineId),
    FocusMessageRetry(matrix_domain::ScopedTimelineId),
    RetryMessage(matrix_domain::ScopedTimelineId),
    Spaces(spaces::SpacesAction),
    Filter(filters::FilterAction),
    FocusServerHistory(matrix_engine::RequestTag),
    LoadServerHistory(matrix_engine::RequestTag),
    Devices(devices::DeviceAction),
    Details(details::DetailsAction),
    Notifications(notifications::NotificationAction),
    FocusMessage(matrix_domain::ScopedTimelineId),
    ToggleTheme(u64),
    FocusTheme(u64),
    FocusSearch,
    FocusComposer,
    FocusSend {
        source: u64,
        scope: matrix_domain::RoomScope,
        text: String,
    },
    SubmitComposer {
        source: u64,
        scope: matrix_domain::RoomScope,
        text: String,
    },
    ActivateRoom(RoomIntent),
    FocusRoom(RoomIntent),
}

struct Actions(
    mpsc::SyncSender<NavigationAction>,
    Arc<Mutex<semantic_session::SemanticSession>>,
);
impl accesskit::ActionHandler for Actions {
    fn do_action(&mut self, request: accesskit::ActionRequest) {
        let Some(command) = self.1.lock().unwrap().action(&request) else {
            return;
        };
        // No GPUI calls, blocking send, filesystem or network on the AT thread.
        let _ = self.0.try_send(command);
    }
}

struct Deactivation;
impl accesskit::DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {}
}

/// Owns the Linux platform adapter; action callbacks never capture UI entities.
/// Only this explicit feature initializes AccessKit's process-global executor.
pub struct LinuxAccessibility {
    adapter: accesskit_unix::Adapter,
    snapshot: Arc<Mutex<semantic_session::SemanticSession>>,
    actions: mpsc::Receiver<NavigationAction>,
}

impl LinuxAccessibility {
    pub fn new(snapshot: NavigationSnapshot) -> Self {
        let snapshot = Arc::new(Mutex::new(semantic_session::SemanticSession::new(snapshot)));
        let (sender, actions) = mpsc::sync_channel(8);
        Self {
            adapter: accesskit_unix::Adapter::new(
                Activation(snapshot.clone()),
                Actions(sender, snapshot.clone()),
                Deactivation,
            ),
            snapshot,
            actions,
        }
    }

    /// Publishes a UI-local projection under the lock, with no I/O or GPUI calls.
    pub fn update(&mut self, snapshot: NavigationSnapshot, focused: bool) {
        self.snapshot.lock().unwrap().update(snapshot);
        self.adapter.update_window_focus_state(focused);
        self.adapter
            .update_if_active(|| self.snapshot.lock().unwrap().tree());
    }

    /// A bounded foreground pump must call this, at most eight times per tick.
    pub fn take_action(&self) -> Option<NavigationAction> {
        self.actions.try_recv().ok()
    }
}
