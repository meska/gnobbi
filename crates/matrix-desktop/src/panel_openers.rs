//! The three existing root panel buttons; not a global navigation framework.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Info,
    Devices,
    Notifications,
    /// Magnifier left of Info: asks to open the room search (placeholder until the search task).
    Search,
}
impl Control {
    // Search first: it sits left of Info in the header and in the accessibility tree.
    pub const ALL: [Self; 4] = [Self::Search, Self::Info, Self::Devices, Self::Notifications];
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "Apri informazioni stanza",
            Self::Devices => "Apri verifica dispositivi",
            Self::Notifications => "Apri notifiche locali",
            Self::Search => "Cerca nei messaggi",
        }
    }
    #[cfg(all(feature = "linux-a11y", target_os = "linux"))]
    pub fn id(self) -> accesskit::NodeId {
        use crate::accessibility::{details, devices, notifications};
        match self {
            Self::Info => details::OPEN,
            Self::Devices => devices::OPEN,
            Self::Notifications => notifications::OPEN,
            Self::Search => details::SEARCH,
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
