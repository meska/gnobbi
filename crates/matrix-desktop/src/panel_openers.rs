//! The three existing root panel buttons; not a global navigation framework.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Info,
    Devices,
    Notifications,
}
impl Control {
    pub const ALL: [Self; 3] = [Self::Info, Self::Devices, Self::Notifications];
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "Apri informazioni stanza",
            Self::Devices => "Apri verifica dispositivi",
            Self::Notifications => "Apri notifiche locali",
        }
    }
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn id(self) -> accesskit::NodeId {
        use crate::accessibility::{details, devices, notifications};
        match self {
            Self::Info => details::OPEN,
            Self::Devices => devices::OPEN,
            Self::Notifications => notifications::OPEN,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Intent {
    pub control: Control,
    pub source: u64,
    pub focus_only: bool,
}
#[cfg(all(feature = "linux-a11y", target_os = "linux"))]
#[derive(Clone)]
pub struct Snapshot {
    pub control: Control,
    pub source: u64,
    pub focused: bool,
    pub bounds: Option<accesskit::Rect>,
}
