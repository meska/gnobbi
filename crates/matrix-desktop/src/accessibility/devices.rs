//! Device overview and explicit, snapshot-scoped SAS decisions.
use super::ROOT;
use accesskit::{Action, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use matrix_domain::AccountId;
use crate::{PopupBody, PopupButton, PopupView};
use matrix_engine::verification::{VerificationDevice, VerificationSnapshot};

#[derive(Clone)]
pub struct SemanticDevice {
    pub id: NodeId,
    pub account: AccountId,
    pub device: VerificationDevice,
    pub bounds: Option<accesskit::Rect>,
}
pub struct DeviceNodes {
    ids: std::collections::HashMap<(AccountId, String, String), NodeId>,
    next: u64,
    panel_key: Option<(AccountId, Option<VerificationSnapshot>)>,
    flow_ids: [NodeId; 9],
    next_flow: u64,
}
impl Default for DeviceNodes {
    fn default() -> Self {
        Self {
            ids: Default::default(),
            next: 1_u64 << 55,
            panel_key: None,
            flow_ids: [NodeId(0); 9],
            next_flow: 1_u64 << 56,
        }
    }
}
impl DeviceNodes {
    /// Scope popup button handles before publishing or resolving assistive actions.
    /// Never recycle a prior handle, even if an older snapshot is seen again.
    pub fn scope_panel(&mut self, mut panel: DeviceOverview) -> DeviceOverview {
        let key = (panel.account, panel.snapshot.clone());
        if self.panel_key.as_ref() != Some(&key) {
            for id in &mut self.flow_ids {
                *id = NodeId(self.next_flow);
                self.next_flow = self
                    .next_flow
                    .checked_add(1)
                    .expect("SAS semantic ID exhaustion");
                assert!(
                    self.next_flow < 1_u64 << 57,
                    "SAS semantic ID range exhausted"
                );
            }
            self.panel_key = Some(key);
        }
        panel.flow_ids = self.flow_ids;
        panel
    }
    pub fn project(
        &mut self,
        account: AccountId,
        model: &crate::DeviceVerification,
    ) -> Vec<SemanticDevice> {
        model
            .devices()
            .iter()
            .map(|device| {
                let key = (account, device.user_id.clone(), device.device_id.clone());
                let id = *self.ids.entry(key).or_insert_with(|| {
                    let id = NodeId(self.next);
                    self.next = self
                        .next
                        .checked_add(1)
                        .expect("device semantic ID exhaustion");
                    assert!(
                        self.next < 1_u64 << 56,
                        "device semantic ID range exhausted"
                    );
                    id
                });
                SemanticDevice {
                    id,
                    account,
                    device: device.clone(),
                    bounds: None,
                }
            })
            .collect()
    }
}

pub const OPEN: NodeId = NodeId(40);
#[derive(Clone, Default)]
pub enum DeviceView {
    #[default]
    Unavailable,
    Closed(AccountId),
    Open(Box<DeviceOverview>),
}
#[derive(Clone, PartialEq, Eq)]
pub enum DeviceAction {
    Open(AccountId),
    Close(AccountId),
    Refresh(AccountId),
    /// «Disconnetti inattivi…» next to «Aggiorna dispositivi».
    OpenInactive(AccountId),
    /// The threshold selector of the open «inattivi» dialog: next value.
    CycleThreshold(AccountId),
    Focus {
        account: AccountId,
        control: DeviceControl,
    },
    /// A button of the verification popup, valid only for the snapshot it was published with.
    Popup {
        account: AccountId,
        button: PopupButton,
        snapshot: Option<Box<VerificationSnapshot>>,
    },
    Begin {
        account: AccountId,
        device: VerificationDevice,
        previous: Option<Box<VerificationSnapshot>>,
    },
    /// «Disconnetti» on a row: opens the password dialog (same path as the pointer click).
    OpenDelete {
        account: AccountId,
        device: VerificationDevice,
    },
    /// «Rinomina» on «Questo dispositivo»: opens the name dialog.
    OpenRename(AccountId),
    /// Annulla / confirm of whichever dialog (Disconnetti or Rinomina) is open.
    CancelDelete(AccountId),
    SubmitDelete(AccountId),
}
impl std::fmt::Debug for DeviceAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceAction(<private>)")
    }
}
impl DeviceView {
    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<DeviceAction> {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        match self {
            Self::Closed(account)
                if request.target_node == OPEN && request.action == Action::Click =>
            {
                Some(DeviceAction::Open(*account))
            }
            Self::Open(panel) => {
                let control = panel
                    .controls()
                    .into_iter()
                    .find(|c| panel.control_id(*c) == request.target_node)?;
                match request.action {
                    Action::Focus => Some(DeviceAction::Focus {
                        account: panel.account,
                        control,
                    }),
                    Action::Click => Some(match control {
                        DeviceControl::Close => DeviceAction::Close(panel.account),
                        DeviceControl::Refresh => DeviceAction::Refresh(panel.account),
                        DeviceControl::Inactive => DeviceAction::OpenInactive(panel.account),
                        DeviceControl::DialogThreshold => DeviceAction::CycleThreshold(panel.account),
                        DeviceControl::Delete(id) => DeviceAction::OpenDelete {
                            account: panel.account,
                            device: panel.devices.iter().find(|d| d.id == id)?.device.clone(),
                        },
                        DeviceControl::Rename => DeviceAction::OpenRename(panel.account),
                        DeviceControl::DialogCancel => DeviceAction::CancelDelete(panel.account),
                        DeviceControl::DialogConfirm => DeviceAction::SubmitDelete(panel.account),
                        DeviceControl::DialogField => return None,
                        DeviceControl::Device(id) => DeviceAction::Begin {
                            account: panel.account,
                            device: panel.devices.iter().find(|d| d.id == id)?.device.clone(),
                            previous: panel.snapshot.clone().map(Box::new),
                        },
                        DeviceControl::Popup(button) => DeviceAction::Popup {
                            account: panel.account,
                            button,
                            snapshot: panel.snapshot.clone().map(Box::new),
                        },
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    pub fn allows(&self, action: &DeviceAction) -> bool {
        match (self, action) {
            (Self::Closed(current), DeviceAction::Open(account)) => current == account,
            (Self::Open(panel), DeviceAction::Close(account) | DeviceAction::Refresh(account)) => {
                panel.account == *account
            }
            (Self::Open(panel), DeviceAction::Focus { account, control }) => {
                panel.account == *account && panel.controls().contains(control)
            }
            (
                Self::Open(panel),
                DeviceAction::Popup {
                    account,
                    button,
                    snapshot,
                },
            ) => {
                panel.account == *account
                    && panel.snapshot.as_ref() == snapshot.as_deref()
                    && panel.controls().contains(&DeviceControl::Popup(*button))
            }
            (
                Self::Open(panel),
                DeviceAction::Begin {
                    account,
                    device,
                    previous,
                },
            ) => {
                panel.account == *account
                    && panel.can_begin
                    && !device.verified
                    && panel.snapshot.as_ref() == previous.as_deref()
                    && panel
                        .devices
                        .iter()
                        .any(|d| d.account == *account && d.device == *device)
            }
            (Self::Open(panel), DeviceAction::OpenInactive(a)) => {
                panel.account == *a && panel.dialog.is_none() && panel.can_inactive
            }
            (Self::Open(panel), DeviceAction::CycleThreshold(a)) => {
                panel.account == *a && panel.dialog_inactive.is_some()
            }
            (Self::Open(panel), DeviceAction::OpenRename(a)) => {
                panel.account == *a && panel.dialog.is_none() && panel.can_rename
            }
            (Self::Open(panel), DeviceAction::OpenDelete { account, device }) => {
                panel.account == *account
                    && panel.dialog.is_none()
                    && panel
                        .devices
                        .iter()
                        .any(|d| d.device == *device && panel.deletable(d))
            }
            (Self::Open(panel), DeviceAction::CancelDelete(a) | DeviceAction::SubmitDelete(a)) => {
                panel.account == *a && panel.dialog.is_some()
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceControl {
    #[default]
    Close,
    Refresh,
    /// «Disconnetti inattivi…».
    Inactive,
    /// Threshold selector inside the «inattivi» dialog.
    DialogThreshold,
    Device(NodeId),
    /// «Disconnetti» of the row whose device handle is the payload.
    Delete(NodeId),
    /// «Rinomina» of this session's device.
    Rename,
    DialogField,
    DialogCancel,
    DialogConfirm,
    /// A button of the verification popup (its X included).
    Popup(PopupButton),
}
impl DeviceControl {
    pub fn id(self) -> NodeId {
        match self {
            Self::Device(id) => id,
            Self::Delete(id) => NodeId(id.0 | 1 << 58),
            Self::Rename => NodeId(50),
            Self::Inactive => NodeId(51),
            Self::DialogThreshold => NodeId(52),
            Self::DialogField => NodeId(47),
            Self::DialogCancel => NodeId(48),
            Self::DialogConfirm => NodeId(49),
            Self::Popup(b) => NodeId(60 + b.index() as u64),
            _ => NodeId(41 + self.index().unwrap() as u64),
        }
    }
    pub fn index(self) -> Option<usize> {
        Some(match self {
            Self::Close => 0,
            Self::Refresh => 1,
            Self::Device(_) | Self::Delete(_) | Self::Rename | Self::Inactive | Self::DialogThreshold => return None,
            Self::DialogField | Self::DialogCancel | Self::DialogConfirm | Self::Popup(_) => return None,
        })
    }
}

#[derive(Clone)]
struct InactiveDialogView {
    threshold: &'static str,
    counter: String,
    unknown: Option<String>,
    lines: Vec<String>,
}

#[derive(Clone)]
pub struct DeviceOverview {
    pub account: AccountId,
    pub focus: DeviceControl,
    pub bounds: [Option<accesskit::Rect>; 2],
    labels: Vec<String>,
    snapshot: Option<VerificationSnapshot>,
    /// The open verification popup: it alone is reachable while it is up, over the panel or over any screen.
    popup: Option<PopupView>,
    devices: Vec<SemanticDevice>,
    can_begin: bool,
    own_device: Option<String>,
    can_rename: bool,
    /// Open «Disconnetti» or «Rinomina» dialog: title and last outcome. The password is never here.
    dialog: Option<(String, Option<&'static str>)>,
    dialog_rename: bool,
    can_inactive: bool,
    /// «Disconnetti inattivi…» dialog: selector text, counter, unknown-activity line, one label per device.
    dialog_inactive: Option<InactiveDialogView>,
    flow_ids: [NodeId; 9],
}
impl DeviceOverview {
    pub fn project(
        account: AccountId,
        model: &crate::DeviceVerification,
        focus: DeviceControl,
        bounds: [Option<accesskit::Rect>; 2],
    ) -> Self {
        let mut labels: Vec<String> = model
            .own_device()
            .map(|id| match model.own() {
                Some(own) => format!(
                    "Questo dispositivo · {}",
                    crate::device_label(own)
                ),
                None => format!("Questo dispositivo · {id}"),
            })
            .into_iter()
            .collect();
        labels.push("Altri dispositivi".into());
        if model.devices().is_empty() {
            labels.push("Nessun altro dispositivo disponibile".into());
        }
        labels.extend(model.devices().iter().map(|device| {
            crate::device_label(device)
        }));
        let popup = model.popup_view();
        let mut panel = Self {
            account,
            focus: match model.popup_focused() {
                Some(button) if popup.is_some() => DeviceControl::Popup(button),
                _ => focus,
            },
            bounds,
            labels,
            snapshot: model.snapshot().cloned(),
            popup,
            devices: vec![],
            can_begin: model.can_begin(),
            own_device: model.own_device().map(str::to_owned),
            can_rename: model.own().is_some(),
            dialog: model
                .delete_dialog()
                .map(|d| (d.name.clone(), d.error))
                .or_else(|| model.rename_dialog().map(|d| ("Rinomina".to_owned(), d.error))),
            dialog_rename: model.rename_dialog().is_some(),
            can_inactive: model.has_inactive(crate::time_fmt::now_ms()),
            dialog_inactive: model.delete_dialog().and_then(|d| {
                Some(InactiveDialogView {
                    threshold: d.threshold?.label(),
                    counter: d.counter(),
                    unknown: d.unknown_note(),
                    lines: model.delete_targets().into_iter().map(crate::device_label).collect(),
                })
            }),
            flow_ids: PopupButton::ALL.map(|b| DeviceControl::Popup(b).id()),
        };
        if !matches!(panel.focus, DeviceControl::Device(_) | DeviceControl::Delete(_))
            && !panel.controls().contains(&panel.focus)
        {
            panel.focus = panel.controls()[0];
        }
        panel
    }

    pub fn with_devices(mut self, devices: Vec<SemanticDevice>) -> Self {
        self.devices = devices
            .into_iter()
            .filter(|d| d.account == self.account)
            .collect();
        if !self.controls().contains(&self.focus) {
            self.focus = self.controls()[0];
        }
        self
    }

    fn deletable(&self, d: &SemanticDevice) -> bool {
        self.own_device.as_deref() != Some(&d.device.device_id)
    }

    pub fn controls(&self) -> Vec<DeviceControl> {
        if let Some(popup) = &self.popup {
            return popup
                .buttons
                .iter()
                .copied()
                .chain([PopupButton::Close])
                .map(DeviceControl::Popup)
                .collect();
        }
        if self.dialog.is_some() {
            let mut controls = vec![DeviceControl::DialogField];
            controls.extend(self.dialog_inactive.as_ref().map(|_| DeviceControl::DialogThreshold));
            controls.extend([DeviceControl::DialogCancel, DeviceControl::DialogConfirm]);
            return controls;
        }
        let mut controls = vec![DeviceControl::Close];
        if self.can_rename {
            controls.push(DeviceControl::Rename);
        }
        controls.push(DeviceControl::Refresh);
        if self.can_inactive {
            controls.push(DeviceControl::Inactive);
        }
        for d in &self.devices {
            // Verified rows are read-only labels: re-verification is not offered.
            if self.can_begin && !d.device.verified {
                controls.push(DeviceControl::Device(d.id));
            }
            if self.deletable(d) {
                controls.push(DeviceControl::Delete(d.id));
            }
        }
        controls
    }

    pub fn control_id(&self, control: DeviceControl) -> NodeId {
        match control {
            DeviceControl::Popup(b) => self.flow_ids[b.index()],
            _ => control.id(),
        }
    }

    pub fn next_control(&self, reverse: bool) -> DeviceControl {
        let controls = self.controls();
        let index = controls.iter().position(|c| *c == self.focus).unwrap_or(0);
        controls[(index + if reverse { controls.len() - 1 } else { 1 }) % controls.len()]
    }

    fn dialog_tree(&self, name: &str, error: Option<&str>) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Verifica dispositivi");
        let mut dialog = Node::new(Role::Dialog);
        dialog.set_label(name);
        let mut nodes = vec![];
        if let Some(error) = error {
            let id = NodeId(1 << 59);
            let mut node = Node::new(Role::Label);
            node.set_value(error);
            dialog.push_child(id);
            nodes.push((id, node));
        }
        if let Some(v) = &self.dialog_inactive {
            let mut label = |id: NodeId, text: &str, nodes: &mut Vec<(NodeId, Node)>| {
                let mut node = Node::new(Role::Label);
                node.set_value(text);
                dialog.push_child(id);
                nodes.push((id, node));
            };
            label(NodeId((1 << 59) + 2), &v.counter, &mut nodes);
            if let Some(u) = &v.unknown {
                label(NodeId((1 << 59) + 3), u, &mut nodes);
            }
            for (i, line) in v.lines.iter().enumerate() {
                label(NodeId((1 << 59) + 16 + i as u64), line, &mut nodes);
            }
        }
        for control in self.controls() {
            let threshold_label;
            let (role, label) = match control {
                DeviceControl::DialogThreshold => {
                    threshold_label = format!("Soglia inattività: {}", self.dialog_inactive.as_ref().map_or("", |v| v.threshold));
                    (Role::Button, threshold_label.as_str())
                }
                DeviceControl::DialogField if self.dialog_rename => (Role::TextInput, "Nome"),
                DeviceControl::DialogField => (Role::PasswordInput, "Password"),
                DeviceControl::DialogCancel => (Role::Button, "Annulla"),
                _ if self.dialog_rename => (Role::Button, "Salva"),
                _ => (Role::Button, "Disconnetti"),
            };
            // No value on the field: the typed password is never published.
            let mut node = Node::new(role);
            node.set_label(label);
            node.add_action(Action::Focus);
            if control != DeviceControl::DialogField {
                node.add_action(Action::Click);
            }
            dialog.push_child(control.id());
            nodes.push((control.id(), node));
        }
        let dialog_id = NodeId((1 << 59) + 1);
        root.push_child(dialog_id);
        nodes.insert(0, (dialog_id, dialog));
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: self.control_id(self.focus),
        }
    }

    /// The popup as a dialog named after its title: emoji (SDK names) or numbers, outcome, then its buttons.
    fn popup_tree(&self, view: &PopupView) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Verifica");
        let mut dialog = Node::new(Role::Dialog);
        dialog.set_label(view.title);
        let texts: Vec<String> = match &view.body {
            PopupBody::Waiting => vec!["In attesa dell’altro dispositivo".into()],
            PopupBody::Empty => vec![],
            PopupBody::Request(who) => vec![who.clone()],
            PopupBody::Outcome(text) => vec![(*text).into()],
            PopupBody::Decimals((a, b, c)) => vec![format!("Numeri SAS: {a}, {b}, {c}")],
            PopupBody::Emoji(emoji) => emoji.iter().map(|(_, name)| name.clone()).collect(),
        };
        let mut nodes = vec![];
        for (i, text) in texts.into_iter().enumerate() {
            let id = NodeId((1 << 59) + 2 + i as u64);
            let mut node = Node::new(Role::Label);
            node.set_value(text);
            dialog.push_child(id);
            nodes.push((id, node));
        }
        for control in self.controls() {
            let DeviceControl::Popup(button) = control else { continue };
            let mut node = Node::new(Role::Button);
            node.set_label(button.label());
            node.add_action(Action::Focus);
            node.add_action(Action::Click);
            let id = self.control_id(control);
            dialog.push_child(id);
            nodes.push((id, node));
        }
        let dialog_id = NodeId((1 << 59) + 1);
        root.push_child(dialog_id);
        nodes.insert(0, (dialog_id, dialog));
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: self.control_id(self.focus),
        }
    }

    pub fn tree(&self) -> TreeUpdate {
        if let Some(view) = &self.popup {
            return self.popup_tree(view);
        }
        if let Some((name, error)) = &self.dialog {
            return self.dialog_tree(name, *error);
        }
        let mut root = Node::new(Role::Window);
        root.set_label("Verifica dispositivi");
        let mut nodes = vec![];
        for (index, label) in self.labels.iter().enumerate() {
            let id = NodeId((1_u64 << 60) + index as u64);
            let mut node = Node::new(Role::Label);
            node.set_value(label.clone());
            root.push_child(id);
            nodes.push((id, node));
        }
        for control in self.controls() {
            let device_label;
            let label = match control {
                DeviceControl::Close => "Chiudi verifica dispositivi",
                DeviceControl::Refresh => "Aggiorna dispositivi",
                DeviceControl::Inactive => "Disconnetti inattivi",
                DeviceControl::Rename => "Rinomina",
                DeviceControl::Device(id) => {
                    let device = &self.devices.iter().find(|d| d.id == id).unwrap().device;
                    device_label = format!("Avvia verifica con {}", crate::device_label(device));
                    &device_label
                }
                DeviceControl::Delete(id) => {
                    let device = &self.devices.iter().find(|d| d.id == id).unwrap().device;
                    device_label = format!("Disconnetti {}", crate::device_title(device));
                    &device_label
                }
                DeviceControl::DialogField
                | DeviceControl::DialogThreshold
                | DeviceControl::DialogCancel
                | DeviceControl::DialogConfirm
                | DeviceControl::Popup(_) => unreachable!("dialog and popup have their own tree"),
            };
            let mut node = Node::new(Role::Button);
            node.set_label(label);
            node.add_action(Action::Focus);
            node.add_action(Action::Click);
            let bounds = match control {
                DeviceControl::Device(id) | DeviceControl::Delete(id) => self
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| d.bounds),
                DeviceControl::Rename | DeviceControl::Inactive => None,
                _ => self.bounds[control.index().unwrap()],
            };
            if let Some(bounds) = bounds {
                node.set_bounds(bounds);
            }
            root.push_child(self.control_id(control));
            nodes.push((self.control_id(control), node));
        }
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: self.control_id(self.focus),
        }
    }
}
