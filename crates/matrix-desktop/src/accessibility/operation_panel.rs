//! Permission-scoped inline message controls with per-opening semantic IDs.
use super::ROOT;
use accesskit::{Action, Node, NodeId, Rect, Role, TreeUpdate};
use matrix_domain::ScopedTimelineId;

#[derive(Clone)]
pub struct PanelSnapshot {
    pub preview_available: bool,
    pub media_status: Option<String>,
    pub preview_focused: bool,
    pub preview_bounds: Option<Rect>,
    pub confirming: bool,
    pub redact_focused: bool,
    pub redact_bounds: Option<Rect>,
    pub redact_available: bool,
    pub target: ScopedTimelineId,
    pub tag: matrix_engine::RequestTag,
    pub focused: bool,
    pub bounds: Option<Rect>,
    pub notice: String,
    pub close: NodeId,
    pub reply_available: bool,
    pub reply_focused: bool,
    pub reply_bounds: Option<Rect>,
    pub edit_available: bool,
    pub edit_focused: bool,
    pub edit_bounds: Option<Rect>,
    pub retry_request: Option<matrix_engine::operations::OperationRequest>,
    pub retry_focused: bool,
    pub retry_bounds: Option<Rect>,
    pub react_available: bool,
    pub react_focused: bool,
    pub react_bounds: Option<Rect>,
}
#[derive(Clone, Copy)]
pub enum PanelIntent {
    Preview,
    FocusPreview,
    BeginRedact,
    FocusRedact,
    FocusConfirmRedact,
    ConfirmRedact,
    CancelRedact,
    Retry,
    FocusRetry,
    React,
    FocusReact,
    Edit,
    FocusEdit,
    Reply,
    FocusReply,
    Focus,
    Close,
    Composer,
    Message,
}
pub struct PanelAction {
    pub retry_request: Option<matrix_engine::operations::OperationRequest>,
    pub target: ScopedTimelineId,
    pub tag: matrix_engine::RequestTag,
    pub intent: PanelIntent,
}
impl PanelSnapshot {
    pub fn key(&self, key: &str, shift: bool, opener_focused: bool) -> Option<PanelAction> {
        if self.tag.is_cancelled() {
            return None;
        }
        let intent = if self.confirming {
            match (key, shift) {
                ("escape", false) if self.focused || self.redact_focused => {
                    PanelIntent::CancelRedact
                }
                ("space" | "enter", false) if self.focused => PanelIntent::CancelRedact,
                ("space" | "enter", false) if self.redact_focused && self.redact_available => {
                    PanelIntent::ConfirmRedact
                }
                ("tab", false) if self.focused && self.redact_available => {
                    PanelIntent::FocusConfirmRedact
                }
                ("tab", true) if self.redact_focused => PanelIntent::Focus,
                ("tab", _) if self.focused || self.redact_focused => PanelIntent::Composer,
                _ => return None,
            }
        } else {
            match (key, shift, self.focused) {
                ("tab", true, true) if self.preview_available => PanelIntent::FocusPreview,
                ("tab", false, false) if self.preview_available && self.preview_focused => {
                    PanelIntent::Focus
                }
                ("tab", true, false) if self.preview_available && self.preview_focused => {
                    PanelIntent::Message
                }
                ("space" | "enter", false, false)
                    if self.preview_available && self.preview_focused =>
                {
                    PanelIntent::Preview
                }
                ("escape", false, false) if self.preview_available && self.preview_focused => {
                    PanelIntent::Close
                }
                ("space" | "enter", false, false)
                    if self.redact_focused && self.redact_available =>
                {
                    PanelIntent::BeginRedact
                }
                ("escape", false, false) if self.redact_focused && self.redact_available => {
                    PanelIntent::Close
                }
                ("tab", false, false) if self.redact_focused && self.redact_available => {
                    PanelIntent::Composer
                }
                ("tab", true, false) if self.redact_focused && self.redact_available => {
                    if self.react_available {
                        PanelIntent::FocusReact
                    } else if self.edit_available {
                        PanelIntent::FocusEdit
                    } else if self.reply_available {
                        PanelIntent::FocusReply
                    } else {
                        PanelIntent::Focus
                    }
                }
                ("tab", false, false)
                    if self.redact_available
                        && ((self.react_focused && self.react_available)
                            || (self.edit_focused
                                && self.edit_available
                                && !self.react_available)
                            || (self.reply_focused
                                && self.reply_available
                                && !self.edit_available
                                && !self.react_available)) =>
                {
                    PanelIntent::FocusRedact
                }
                ("tab", false, true)
                    if self.redact_available
                        && !self.reply_available
                        && !self.edit_available
                        && !self.react_available
                        && self.retry_request.is_none() =>
                {
                    PanelIntent::FocusRedact
                }
                ("tab", false, true) if self.retry_request.is_some() => PanelIntent::FocusRetry,
                ("tab", false, false) if self.retry_focused && self.retry_request.is_some() => {
                    PanelIntent::Composer
                }
                ("tab", true, false) if self.retry_focused && self.retry_request.is_some() => {
                    PanelIntent::Focus
                }
                ("space" | "enter", false, false)
                    if self.retry_focused && self.retry_request.is_some() =>
                {
                    PanelIntent::Retry
                }
                ("escape", false, false) if self.retry_focused && self.retry_request.is_some() => {
                    PanelIntent::Close
                }
                ("tab", false, true) if self.reply_available => PanelIntent::FocusReply,
                ("tab", false, true) if self.edit_available => PanelIntent::FocusEdit,
                ("tab", false, true) if self.react_available => PanelIntent::FocusReact,
                ("tab", false, false) if self.react_focused && self.react_available => {
                    PanelIntent::Composer
                }
                ("tab", true, false) if self.react_focused && self.react_available => {
                    if self.edit_available {
                        PanelIntent::FocusEdit
                    } else if self.reply_available {
                        PanelIntent::FocusReply
                    } else {
                        PanelIntent::Focus
                    }
                }
                ("space" | "enter", false, false) if self.react_focused && self.react_available => {
                    PanelIntent::React
                }
                ("escape", false, false) if self.react_focused && self.react_available => {
                    PanelIntent::Close
                }
                ("tab", false, false)
                    if self.edit_focused && self.edit_available && self.react_available =>
                {
                    PanelIntent::FocusReact
                }
                ("tab", false, false) if self.edit_focused && self.edit_available => {
                    PanelIntent::Composer
                }
                ("tab", true, false) if self.edit_focused && self.edit_available => {
                    if self.reply_available {
                        PanelIntent::FocusReply
                    } else {
                        PanelIntent::Focus
                    }
                }
                ("space" | "enter", false, false) if self.edit_focused && self.edit_available => {
                    PanelIntent::Edit
                }
                ("escape", false, false) if self.edit_focused && self.edit_available => {
                    PanelIntent::Close
                }
                ("tab", false, false)
                    if self.reply_focused && self.reply_available && self.edit_available =>
                {
                    PanelIntent::FocusEdit
                }
                ("tab", false, false) if self.reply_focused && self.reply_available => {
                    if self.react_available {
                        PanelIntent::FocusReact
                    } else {
                        PanelIntent::Composer
                    }
                }
                ("tab", true, false) if self.reply_focused && self.reply_available => {
                    PanelIntent::Focus
                }
                ("space" | "enter", false, false) if self.reply_focused && self.reply_available => {
                    PanelIntent::Reply
                }
                ("escape", false, false) if self.reply_focused && self.reply_available => {
                    PanelIntent::Close
                }
                ("tab", false, false) if opener_focused && self.preview_available => {
                    PanelIntent::FocusPreview
                }
                ("tab", false, false) if opener_focused => PanelIntent::Focus,
                ("tab", false, true) => PanelIntent::Composer,
                ("tab", true, true) => PanelIntent::Message,
                ("space" | "enter" | "escape", false, true) => PanelIntent::Close,
                _ => return None,
            }
        };
        Some(PanelAction {
            retry_request: self.retry_request.clone(),
            target: self.target.clone(),
            tag: self.tag.clone(),
            intent,
        })
    }
    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<PanelAction> {
        if request.target_tree != accesskit::TreeId::ROOT
            || request.data.is_some()
            || self.tag.is_cancelled()
        {
            return None;
        }
        let intent = match (request.target_node, request.action) {
            (id, Action::Focus)
                if self.redact_available && self.confirming && id == self.confirm_redact_id() =>
            {
                PanelIntent::FocusConfirmRedact
            }
            (id, Action::Focus)
                if self.redact_available && !self.confirming && id == self.redact_id() =>
            {
                PanelIntent::FocusRedact
            }
            (id, Action::Click) if self.confirming && id == self.close => PanelIntent::CancelRedact,
            (id, Action::Focus) if self.confirming && id == self.close => PanelIntent::Focus,
            (id, Action::Click)
                if self.redact_available && self.confirming && id == self.confirm_redact_id() =>
            {
                PanelIntent::ConfirmRedact
            }
            (id, Action::Click)
                if self.redact_available && !self.confirming && id == self.redact_id() =>
            {
                PanelIntent::BeginRedact
            }
            _ if self.confirming => return None,
            (id, Action::Focus) if self.retry_request.is_some() && id == self.retry_id() => {
                PanelIntent::FocusRetry
            }
            (id, Action::Click) if self.retry_request.is_some() && id == self.retry_id() => {
                PanelIntent::Retry
            }
            (id, Action::Focus) if self.react_available && id == self.react_id() => {
                PanelIntent::FocusReact
            }
            (id, Action::Click) if self.react_available && id == self.react_id() => {
                PanelIntent::React
            }
            (id, Action::Click) if self.edit_available && id == self.edit_id() => PanelIntent::Edit,
            (id, Action::Focus) if self.edit_available && id == self.edit_id() => {
                PanelIntent::FocusEdit
            }
            (id, Action::Focus) if self.reply_available && id == self.reply_id() => {
                PanelIntent::FocusReply
            }
            (id, Action::Click) if self.reply_available && id == self.reply_id() => {
                PanelIntent::Reply
            }
            (id, Action::Click) if self.preview_available && id == self.preview_id() => {
                PanelIntent::Preview
            }
            (id, Action::Focus) if self.preview_available && id == self.preview_id() => {
                PanelIntent::FocusPreview
            }
            (id, Action::Click) if id == self.close => PanelIntent::Close,
            (id, Action::Focus) if id == self.close => PanelIntent::Focus,
            _ => return None,
        };
        Some(PanelAction {
            retry_request: self.retry_request.clone(),
            target: self.target.clone(),
            tag: self.tag.clone(),
            intent,
        })
    }
    pub fn allows(&self, action: &PanelAction) -> bool {
        !self.tag.is_cancelled()
            && !action.tag.is_cancelled()
            && self.target == action.target
            && self.tag.scope() == action.tag.scope()
            && self.tag.generation() == action.tag.generation()
            && (match action.intent {
                PanelIntent::BeginRedact | PanelIntent::FocusRedact => {
                    self.redact_available && !self.confirming
                }
                PanelIntent::ConfirmRedact | PanelIntent::FocusConfirmRedact => {
                    self.redact_available && self.confirming
                }
                PanelIntent::CancelRedact => self.confirming,
                PanelIntent::Focus | PanelIntent::Composer => true,
                _ => !self.confirming,
            })
            && (!matches!(action.intent, PanelIntent::Retry | PanelIntent::FocusRetry)
                || self
                    .retry_request
                    .as_ref()
                    .zip(action.retry_request.as_ref())
                    .is_some_and(|(current, queued)| {
                        current.scope == queued.scope
                            && current.target == queued.target
                            && current.transaction_id == queued.transaction_id
                            && current.operation == queued.operation
                    }))
            && (!matches!(action.intent, PanelIntent::Reply | PanelIntent::FocusReply)
                || self.reply_available)
            && (!matches!(action.intent, PanelIntent::Edit | PanelIntent::FocusEdit)
                || self.edit_available)
            && (!matches!(action.intent, PanelIntent::React | PanelIntent::FocusReact)
                || self.react_available)
            && (!matches!(
                action.intent,
                PanelIntent::Preview | PanelIntent::FocusPreview
            ) || self.preview_available)
    }
    pub fn project(
        operations: &crate::LiveOperations,
        focused: bool,
        bounds: Option<Rect>,
    ) -> Option<Self> {
        let target = operations.selection()?.clone();
        let tag = operations.selection_tag()?.clone();
        if tag.is_cancelled() || tag.generation() >= (1_u64 << 51) {
            return None;
        }

        Some(Self {
            preview_available: matches!(target.id, matrix_domain::TimelineId::Server(_)),
            media_status: None,
            preview_focused: false,
            preview_bounds: None,
            confirming: false,
            redact_focused: false,
            redact_bounds: None,
            redact_available: operations
                .details()
                .is_some_and(|details| details.permissions.redact)
                && !operations.is_pending(),
            target,
            retry_request: operations.retry_request().cloned(),
            retry_focused: false,
            retry_bounds: None,
            // Eight IDs per opening stay inside [2^54, 2^55); never reuse a
            // previous opening's child for a new selection or SAS/device node.
            close: NodeId((1_u64 << 54) + tag.generation() * 8),
            tag,
            focused,
            bounds,
            notice: operations.notice(false).into(),
            reply_available: operations
                .details()
                .is_some_and(|details| details.permissions.reply)
                && !operations.is_pending(),
            reply_focused: false,
            reply_bounds: None,
            edit_available: operations
                .details()
                .is_some_and(|details| details.permissions.edit)
                && !operations.is_pending(),
            edit_focused: false,
            edit_bounds: None,
            react_available: operations
                .details()
                .is_some_and(|details| details.permissions.react)
                && !operations.is_pending(),
            react_focused: false,
            react_bounds: None,
        })
    }
    pub fn preview_id(&self) -> NodeId {
        // Separate range: preserve the existing eight IDs per panel opening.
        NodeId((1_u64 << 57) + self.tag.generation() * 2)
    }
    pub fn redact_id(&self) -> NodeId {
        NodeId(self.close.0 + 6)
    }
    pub fn confirm_redact_id(&self) -> NodeId {
        NodeId(self.close.0 + 7)
    }
    pub fn reply_id(&self) -> NodeId {
        NodeId(self.close.0 + 2)
    }
    pub fn edit_id(&self) -> NodeId {
        NodeId(self.close.0 + 3)
    }
    pub fn retry_id(&self) -> NodeId {
        NodeId(self.close.0 + 5)
    }
    pub fn react_id(&self) -> NodeId {
        NodeId(self.close.0 + 4)
    }
    pub fn append_to(&self, tree: &mut TreeUpdate) {
        if self.confirming {
            let mut cancel = Node::new(Role::Button);
            cancel.set_label("Annulla eliminazione messaggio");
            cancel.set_description(self.notice.clone());
            cancel.add_action(Action::Click);
            cancel.add_action(Action::Focus);
            if let Some(bounds) = self.bounds {
                cancel.set_bounds(bounds);
            }
            let root = &mut tree
                .nodes
                .iter_mut()
                .find(|(id, _)| *id == ROOT)
                .expect("navigation root")
                .1;
            root.push_child(self.close);
            if self.redact_available {
                root.push_child(self.confirm_redact_id());
            }
            tree.nodes.push((self.close, cancel));
            if self.redact_available {
                let mut confirm = Node::new(Role::Button);
                confirm.set_label("Conferma eliminazione messaggio");
                confirm.set_description(self.notice.clone());
                confirm.add_action(Action::Click);
                confirm.add_action(Action::Focus);
                if let Some(bounds) = self.redact_bounds {
                    confirm.set_bounds(bounds);
                }
                tree.nodes.push((self.confirm_redact_id(), confirm));
            }
            if self.focused {
                tree.focus = self.close;
            } else if self.redact_focused && self.redact_available {
                tree.focus = self.confirm_redact_id();
            }
            return;
        }
        let mut close = Node::new(Role::Button);
        close.set_label("Chiudi azioni messaggio");
        close.set_description(self.notice.clone());
        close.add_action(Action::Click);
        close.add_action(Action::Focus);
        if let Some(bounds) = self.bounds {
            close.set_bounds(bounds);
        }
        let notice_id = NodeId(self.close.0 + 1);
        let mut notice = Node::new(Role::Label);
        notice.set_value(self.notice.clone());
        let root = &mut tree
            .nodes
            .iter_mut()
            .find(|(id, _)| *id == ROOT)
            .expect("navigation root")
            .1;
        root.push_child(notice_id);
        root.push_child(self.close);
        if self.preview_available {
            root.push_child(self.preview_id());
            if self.media_status.is_some() {
                root.push_child(NodeId(self.preview_id().0 + 1));
            }
        }
        if self.redact_available {
            root.push_child(self.redact_id());
        }
        if self.reply_available {
            root.push_child(self.reply_id());
        }
        if self.edit_available {
            root.push_child(self.edit_id());
        }
        if self.react_available {
            root.push_child(self.react_id());
        }
        if self.retry_request.is_some() {
            root.push_child(self.retry_id());
        }
        if self.retry_request.is_some() {
            let mut retry = Node::new(Role::Button);
            retry.set_label("Riprova operazione messaggio");
            retry.set_description(self.notice.clone());
            retry.add_action(Action::Click);
            retry.add_action(Action::Focus);
            if let Some(bounds) = self.retry_bounds {
                retry.set_bounds(bounds);
            }
            tree.nodes.push((self.retry_id(), retry));
        }
        if self.react_available {
            let mut react = Node::new(Role::Button);
            react.set_label("Aggiungi o togli reazione +1");
            react.set_description(self.notice.clone());
            react.add_action(Action::Click);
            react.add_action(Action::Focus);
            if let Some(bounds) = self.react_bounds {
                react.set_bounds(bounds);
            }
            tree.nodes.push((self.react_id(), react));
        }
        if self.reply_available {
            let mut reply = Node::new(Role::Button);
            reply.set_label("Rispondi al messaggio");
            reply.set_description(self.notice.clone());
            reply.add_action(Action::Click);
            reply.add_action(Action::Focus);
            if let Some(bounds) = self.reply_bounds {
                reply.set_bounds(bounds);
            }
            tree.nodes.push((self.reply_id(), reply));
        }
        tree.nodes.push((notice_id, notice));
        if self.edit_available {
            let mut edit = Node::new(Role::Button);
            edit.set_label("Modifica il messaggio");
            edit.set_description(self.notice.clone());
            edit.add_action(Action::Click);
            edit.add_action(Action::Focus);
            if let Some(bounds) = self.edit_bounds {
                edit.set_bounds(bounds);
            }
            tree.nodes.push((self.edit_id(), edit));
        }
        if self.preview_available {
            let mut preview = Node::new(Role::Button);
            preview.set_label("Anteprima PNG");
            preview.set_description(format!("{} Carica solo il messaggio selezionato; PNG valido ≤4 MiB / 2048 px, anteprima in memoria ≤320×240. Nessun salvataggio su disco.", self.media_status.as_deref().unwrap_or("Anteprima non richiesta.")));
            preview.add_action(Action::Click);
            preview.add_action(Action::Focus);
            if let Some(bounds) = self.preview_bounds {
                preview.set_bounds(bounds);
            }
            tree.nodes.push((self.preview_id(), preview));
            if let Some(status) = &self.media_status {
                let mut node = Node::new(Role::Status);
                node.set_label(status.clone());
                tree.nodes.push((NodeId(self.preview_id().0 + 1), node));
            }
        }
        tree.nodes.push((self.close, close));
        if self.redact_available {
            let mut redact = Node::new(Role::Button);
            redact.set_label("Elimina il messaggio");
            redact.set_description(self.notice.clone());
            redact.add_action(Action::Click);
            redact.add_action(Action::Focus);
            if let Some(bounds) = self.redact_bounds {
                redact.set_bounds(bounds);
            }
            tree.nodes.push((self.redact_id(), redact));
        }
        if self.focused {
            tree.focus = self.close;
        } else if self.preview_available && self.preview_focused {
            tree.focus = self.preview_id();
        } else if self.reply_available && self.reply_focused {
            tree.focus = self.reply_id();
        } else if self.edit_available && self.edit_focused {
            tree.focus = self.edit_id();
        } else if self.retry_request.is_some() && self.retry_focused {
            tree.focus = self.retry_id();
        } else if self.react_available && self.react_focused {
            tree.focus = self.react_id();
        } else if self.redact_available && self.redact_focused {
            tree.focus = self.redact_id();
        }
    }
}
