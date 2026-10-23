//! Message-body commands; queued requests retain the exact source text and scope.
use matrix_domain::ScopedTimelineId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Select,
    Copy,
    Cancel,
}

impl Control {
    pub const ALL: [Self; 3] = [Self::Select, Self::Copy, Self::Cancel];
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Seleziona testo messaggio",
            Self::Copy => "Copia testo selezionato",
            Self::Cancel => "Annulla selezione testo",
        }
    }
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub generation: u64,
    pub selected: bool,
    /// None means inactive; an empty string is a collapsed current-body caret.
    pub text: Option<String>,
    pub focused: Option<Control>,
    pub bounds: [Option<accesskit::Rect>; 3],
}

impl super::HistorySnapshot {
    pub fn selection_key(&self, key: &str, shift: bool) -> Option<super::NavigationAction> {
        if !self.focused {
            return None;
        }
        let message = self
            .messages
            .iter()
            .find(|m| m.selection.focused.is_some())
            .or_else(|| self.messages.first())?;
        let (control, focus_only) = match (key, shift, message.selection.focused) {
            ("f8", false, _) => (Control::Select, false),
            ("escape", false, _) if message.selection.selected => (Control::Cancel, false),
            ("space" | "enter", false, Some(c)) => (c, false),
            ("tab", true, Some(Control::Select)) => {
                return Some(super::NavigationAction::FocusMessage(message.event.clone()));
            }
            ("tab", false, Some(Control::Cancel)) => {
                return Some(super::NavigationAction::FocusComposer);
            }
            ("tab", false, Some(Control::Select)) if !message.selection.selected => {
                return Some(super::NavigationAction::FocusComposer);
            }
            ("tab", true, Some(Control::Copy)) => (Control::Select, true),
            ("tab", false, Some(Control::Copy)) => (Control::Cancel, true),
            ("tab", false, Some(Control::Select)) | ("tab", true, Some(Control::Cancel)) => {
                (Control::Copy, true)
            }
            _ => return None,
        };
        Some(super::NavigationAction::MessageSelection(Intent {
            generation: message.selection.generation,
            target: message.event.clone(),
            text: message.text.clone(),
            control,
            focus_only,
        }))
    }
}

#[derive(Clone)]
pub struct Intent {
    pub generation: u64,
    pub target: ScopedTimelineId,
    pub text: String,
    pub control: Control,
    pub focus_only: bool,
}

impl Intent {
    pub fn is_current(&self, selection: &crate::TextSelection) -> bool {
        self.generation == selection.generation()
            && (self.control == Control::Select
                || selection.range(&self.target, &self.text).is_some())
    }
}
