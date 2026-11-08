//! When to propose adding the launcher entry, and what each choice does. No I/O here: the check
//! and the installation are injected, so the rules are testable without a display.
use crate::desktop_entry::Outcome;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// «Aggiungi al menu».
    Add,
    /// «Non ora»: asked again at the next launch, nothing saved.
    Later,
    /// «Non chiedere più»: saved in the preferences.
    Never,
    /// «Sostituisci» in the second step: overwrite the different `~/Applications/gnobbi.AppImage`.
    Replace,
}

#[derive(Default)]
pub struct State {
    checked: bool,
    pub open: bool,
    /// Second step in the same window: «Sostituire gnobbi.AppImage?» «Sostituisci» / «Annulla».
    pub confirm: bool,
    /// Bumped each time the dialog or Settings shows the control, so AT object ids are not reused.
    pub generation: u64,
    /// Last installation error, one line.
    pub error: Option<String>,
}

impl State {
    /// Shown only while open and the chat does not cover the window (`compact_chat`).
    pub fn visible(&self, compact_chat: bool) -> bool {
        self.open && !compact_chat
    }

    /// Evaluates `due` at most once per launch, and only when signed in, synced and the user is
    /// not typing; otherwise the check is retried on the next call.
    pub fn poll(&mut self, ready: bool, typing: bool, due: impl FnOnce() -> bool) {
        if self.checked || !ready || typing {
            return;
        }
        self.checked = true;
        if due() {
            self.open = true;
            self.generation += 1;
        }
    }

    /// Returns true when `dont_ask` changed and the preferences must be saved. «Aggiungi al menu»
    /// clears the opt-out (also from Settings); a failed installation keeps the dialog open, and a
    /// different existing copy opens the confirmation step. `install` gets `replace`.
    pub fn choose(
        &mut self,
        choice: Choice,
        dont_ask: &mut bool,
        install: impl FnOnce(bool) -> Result<Outcome, String>,
    ) -> bool {
        let target = match choice {
            Choice::Later => None,
            Choice::Never => Some(true),
            Choice::Add | Choice::Replace => match install(choice == Choice::Replace) {
                Ok(Outcome::Done) => Some(false),
                Ok(Outcome::NeedsConfirm) => {
                    self.open = true;
                    self.confirm = true;
                    self.generation += 1;
                    self.error = None;
                    return false;
                }
                Err(e) => {
                    self.error = Some(e);
                    return false;
                }
            },
        };
        self.open = false;
        self.confirm = false;
        self.error = None;
        target.is_some_and(|v| std::mem::replace(dont_ask, v) != v)
    }
}
