//! Device overview and explicit, snapshot-scoped SAS decisions.
use super::ROOT;
use accesskit::{Action, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use matrix_domain::AccountId;
use matrix_engine::verification::{VerificationAction, VerificationDevice, VerificationSnapshot};

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
    flow_ids: [NodeId; 4],
    next_flow: u64,
}
impl Default for DeviceNodes {
    fn default() -> Self {
        Self {
            ids: Default::default(),
            next: 1_u64 << 55,
            panel_key: None,
            flow_ids: [NodeId(0); 4],
            next_flow: 1_u64 << 56,
        }
    }
}
impl DeviceNodes {
    /// Scope decision handles before publishing or resolving assistive actions.
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
    Focus {
        account: AccountId,
        control: DeviceControl,
    },
    Decide {
        snapshot: Box<VerificationSnapshot>,
        action: VerificationAction,
    },
    Begin {
        account: AccountId,
        device: VerificationDevice,
        previous: Option<Box<VerificationSnapshot>>,
    },
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
                        DeviceControl::Device(id) => DeviceAction::Begin {
                            account: panel.account,
                            device: panel.devices.iter().find(|d| d.id == id)?.device.clone(),
                            previous: panel.snapshot.clone().map(Box::new),
                        },
                        _ => DeviceAction::Decide {
                            snapshot: Box::new(panel.snapshot.clone()?),
                            action: control.decision()?,
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
            (Self::Open(panel), DeviceAction::Decide { snapshot, action }) => {
                panel.account == snapshot.account_id
                    && panel.snapshot.as_ref() == Some(snapshot.as_ref())
                    && panel
                        .controls()
                        .iter()
                        .any(|c| c.decision() == Some(*action))
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
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceControl {
    #[default]
    Close,
    Refresh,
    Confirm,
    Mismatch,
    Cancel,
    Comparison,
    Device(NodeId),
}
impl DeviceControl {
    pub fn id(self) -> NodeId {
        match self {
            Self::Device(id) => id,
            _ => NodeId(41 + self.index().unwrap() as u64),
        }
    }
    pub fn index(self) -> Option<usize> {
        Some(match self {
            Self::Close => 0,
            Self::Refresh => 1,
            Self::Confirm => 2,
            Self::Mismatch => 3,
            Self::Cancel => 4,
            Self::Comparison => 5,
            Self::Device(_) => return None,
        })
    }
    fn decision(self) -> Option<VerificationAction> {
        match self {
            Self::Confirm => Some(VerificationAction::Confirm),
            Self::Mismatch => Some(VerificationAction::Mismatch),
            Self::Cancel => Some(VerificationAction::Cancel),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct DeviceOverview {
    pub account: AccountId,
    pub focus: DeviceControl,
    pub bounds: [Option<accesskit::Rect>; 6],
    labels: Vec<String>,
    snapshot: Option<VerificationSnapshot>,
    decisions: [bool; 3],
    comparison: Option<String>,
    devices: Vec<SemanticDevice>,
    can_begin: bool,
    flow_ids: [NodeId; 4],
}
impl DeviceOverview {
    pub fn project(
        account: AccountId,
        model: &crate::DeviceVerification,
        focus: DeviceControl,
        bounds: [Option<accesskit::Rect>; 6],
    ) -> Self {
        let mut labels: Vec<String> = model
            .own_device()
            .map(|id| format!("Questo dispositivo · {id}"))
            .into_iter()
            .collect();
        labels.push("Altri dispositivi".into());
        if model.devices().is_empty() {
            labels.push("Nessun altro dispositivo disponibile".into());
        }
        labels.extend(model.devices().iter().map(|device| {
            format!(
                "{} · {}",
                device.device_id,
                if device.verified {
                    "Verificato"
                } else {
                    "Non verificato"
                }
            )
        }));
        labels.push(model.status_label().into());
        if let Some(snapshot) = model.snapshot() {
            labels.push(format!("Account: {}", snapshot.user_id));
            labels.push(format!("Dispositivo: {}", snapshot.device_id));
        }
        labels.push(model.trust_label().into());
        let comparison = model
            .comparison()
            .map(|(a, b, c)| format!("Numeri SAS: {a}, {b}, {c}"));
        let mut panel = Self {
            account,
            focus,
            bounds,
            labels,
            snapshot: model.snapshot().cloned(),
            decisions: [
                VerificationAction::Confirm,
                VerificationAction::Mismatch,
                VerificationAction::Cancel,
            ]
            .map(|a| model.can_act(a)),
            comparison,
            devices: vec![],
            can_begin: model.can_begin(),
            flow_ids: [NodeId(43), NodeId(44), NodeId(45), NodeId(46)],
        };
        if !matches!(panel.focus, DeviceControl::Device(_))
            && !panel.controls().contains(&panel.focus)
        {
            panel.focus = DeviceControl::Close;
        }
        panel
    }

    pub fn with_devices(mut self, devices: Vec<SemanticDevice>) -> Self {
        self.devices = devices
            .into_iter()
            .filter(|d| d.account == self.account)
            .collect();
        if !self.controls().contains(&self.focus) {
            self.focus = DeviceControl::Close;
        }
        self
    }

    pub fn controls(&self) -> Vec<DeviceControl> {
        let mut controls = vec![DeviceControl::Close, DeviceControl::Refresh];
        if self.can_begin {
            // Verified rows are read-only labels: re-verification is not offered.
            controls.extend(
                self.devices
                    .iter()
                    .filter(|d| !d.device.verified)
                    .map(|d| DeviceControl::Device(d.id)),
            );
        }
        if self.comparison.is_some() {
            controls.push(DeviceControl::Comparison);
        }
        for (index, control) in [
            DeviceControl::Confirm,
            DeviceControl::Mismatch,
            DeviceControl::Cancel,
        ]
        .into_iter()
        .enumerate()
        {
            if self.decisions[index] {
                controls.push(control);
            }
        }
        controls
    }

    pub fn control_id(&self, control: DeviceControl) -> NodeId {
        match control.index() {
            Some(index @ 2..=5) => self.flow_ids[index - 2],
            _ => control.id(),
        }
    }

    pub fn next_control(&self, reverse: bool) -> DeviceControl {
        let controls = self.controls();
        let index = controls.iter().position(|c| *c == self.focus).unwrap_or(0);
        controls[(index + if reverse { controls.len() - 1 } else { 1 }) % controls.len()]
    }

    pub fn tree(&self) -> TreeUpdate {
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
                DeviceControl::Confirm => "Coincidono",
                DeviceControl::Mismatch => "Non coincidono",
                DeviceControl::Cancel => "Annulla verifica",
                DeviceControl::Comparison => self.comparison.as_deref().unwrap_or_default(),
                DeviceControl::Device(id) => {
                    let device = &self.devices.iter().find(|d| d.id == id).unwrap().device;
                    device_label = format!(
                        "Avvia verifica con {} · {}",
                        device.device_id,
                        if device.verified {
                            "Verificato"
                        } else {
                            "Non verificato"
                        }
                    );
                    &device_label
                }
            };
            let comparison = control == DeviceControl::Comparison;
            let mut node = Node::new(if comparison {
                Role::Label
            } else {
                Role::Button
            });
            if comparison {
                node.set_value(label);
            } else {
                node.set_label(label);
            }
            node.add_action(Action::Focus);
            if !comparison {
                node.add_action(Action::Click);
            }
            let bounds = match control {
                DeviceControl::Device(id) => self
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| d.bounds),
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
