//! When to propose adding the launcher entry, and what each choice does. No I/O here: the check
//! and the installation are injected, so the rules are testable without a display.
use crate::desktop_entry::Outcome;

pub const NOTICE_DONE: &str = "Aggiunto al menu";
pub const NOTICE_PRESENT: &str = "Già presente nel menu";

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
    /// Set by the caller before `choose(Add)`: the entry was already valid (so the outcome is «già presente»).
    pub already: bool,
    /// Outcome shown in the Settings row after a successful «Aggiungi al menu».
    pub notice: Option<&'static str>,
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
        self.notice = None;
        let target = match choice {
            Choice::Later => None,
            Choice::Never => Some(true),
            Choice::Add | Choice::Replace => match install(choice == Choice::Replace) {
                Ok(Outcome::Done) => {
                    self.notice = Some(if self.already { NOTICE_PRESENT } else { NOTICE_DONE });
                    Some(false)
                }
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

#[cfg(test)]
mod notice_tests {
    use super::*;

    fn run(already: bool, r: Result<Outcome, String>) -> State {
        let mut s = State { already, ..Default::default() };
        s.choose(Choice::Add, &mut false, |_| r);
        s
    }

    #[test]
    fn add_reports_done_present_confirm_and_error() {
        assert_eq!(run(false, Ok(Outcome::Done)).notice, Some(NOTICE_DONE)); // missing/stale
        assert_eq!(run(true, Ok(Outcome::Done)).notice, Some(NOTICE_PRESENT)); // valid: never silent
        let c = run(false, Ok(Outcome::NeedsConfirm));
        assert!(c.confirm && c.notice.is_none());
        let e = run(false, Err("boom".into()));
        assert_eq!((e.error.as_deref(), e.notice), (Some("boom"), None));
    }
}
