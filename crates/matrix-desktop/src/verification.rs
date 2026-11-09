//! UI-only projection; SAS and device identities never enter diagnostics.
use matrix_domain::AccountId;
use matrix_engine::verification::{
    VerificationAction, VerificationCommand, VerificationEvent, VerificationSnapshot,
    VerificationState,
};

/// Row title as in Element: the display name, else the ID.
pub fn device_title(d: &matrix_engine::verification::VerificationDevice) -> &str {
    d.display_name.as_deref().unwrap_or(&d.device_id)
}

/// «Verificato · Ultima attività 17:11 · 192.0.2.17 · ID»; only the fields present
/// (the ID is the title when there is no name). `time` formats a ms timestamp.
pub fn device_detail(
    d: &matrix_engine::verification::VerificationDevice,
    time: impl Fn(u64) -> String,
) -> String {
    let mut parts = vec![if d.verified { "Verificato" } else { "Non verificato" }.to_owned()];
    parts.extend(d.last_seen_ts.map(|t| format!("Ultima attività {}", time(t))));
    parts.extend(d.last_seen_ip.clone());
    if d.display_name.is_some() {
        parts.push(d.device_id.clone());
    }
    parts.join(" · ")
}

/// Screen-reader text: the same data as the row, «title · detail».
pub fn device_label(d: &matrix_engine::verification::VerificationDevice) -> String {
    format!("{} · {}", device_title(d), device_detail(d, crate::time_fmt::day_or_time))
}

/// Open «Disconnetti» confirmation: the device to sign out and the one-line outcome of the last try.
pub struct DeleteDialog {
    pub device_id: String,
    pub name: String,
    pub busy: bool,
    pub error: Option<&'static str>,
}

/// Open «Rinomina» dialog for this session's device: the one-line outcome of the last try.
pub struct RenameDialog {
    pub busy: bool,
    pub error: Option<&'static str>,
}

/// A button of the verification popup. `Close` is the X at the top right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupButton {
    Accept,
    Decline,
    Cancel,
    Match,
    NoMatch,
    Yes,
    No,
    Ok,
    Close,
}
impl PopupButton {
    pub const ALL: [PopupButton; 9] = [
        Self::Accept,
        Self::Decline,
        Self::Cancel,
        Self::Match,
        Self::NoMatch,
        Self::Yes,
        Self::No,
        Self::Ok,
        Self::Close,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Accept => "Accetta",
            Self::Decline => "Rifiuta",
            Self::Cancel => "Annulla",
            Self::Match => "Corrispondono",
            Self::NoMatch => "Non corrispondono",
            Self::Yes => "Sì",
            Self::No => "No",
            Self::Ok => "OK",
            Self::Close => "Chiudi",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|b| *b == self).unwrap()
    }
}

/// What the popup shows besides its title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopupBody {
    Waiting,
    /// Title and buttons only («Annullare la verifica?»).
    Empty,
    /// «user · device» of the peer that asked.
    Request(String),
    /// Seven (symbol, SDK name) pairs.
    Emoji(Vec<(String, String)>),
    /// The peer negotiated only decimals.
    Decimals((u16, u16, u16)),
    Outcome(&'static str),
}

/// Everything the popup draws, derived from the SDK snapshot; never from our own trust logic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupView {
    pub title: &'static str,
    pub body: PopupBody,
    /// Action buttons in tab order; the X (`PopupButton::Close`) always comes last.
    pub buttons: Vec<PopupButton>,
}

pub struct DeviceVerification {
    popup: bool,
    /// «Annullare la verifica?» asked inside the popup while comparing.
    confirming: bool,
    /// Index into `popup_view().buttons` + the X.
    popup_focus: usize,
    /// The popup was closed before the SDK answered: cancel the flow as soon as it exists.
    abandoned: bool,
    /// Flow the user closed: a late snapshot of it must not reopen the popup.
    closed_flow: Option<matrix_engine::verification::VerificationTarget>,
    delete: Option<DeleteDialog>,
    rename: Option<RenameDialog>,
    account: AccountId,
    snapshot: Option<VerificationSnapshot>,
    pending: Option<VerificationCommand>,
    release: Option<matrix_engine::verification::VerificationTarget>,
    deciding: bool,
    devices: Vec<matrix_engine::verification::VerificationDevice>,
    failed: bool,
    profile: Option<(String, Option<String>)>,
    own_device: Option<String>,
    own: Option<matrix_engine::verification::VerificationDevice>,
    profile_avatar: Option<String>,
    /// Key recovery complete (backup on, cross-signing keys held); None until the engine says.
    keys_complete: Option<bool>,
    #[cfg(feature = "live")]
    key_recovery: Option<matrix_engine::signin::RecoveryState>,
}
impl DeviceVerification {
    pub fn new(account: AccountId) -> Self {
        Self {
            popup: false,
            confirming: false,
            popup_focus: 0,
            abandoned: false,
            closed_flow: None,
            delete: None,
            rename: None,
            account,
            snapshot: None,
            pending: None,
            release: None,
            deciding: false,
            devices: Vec::new(),
            failed: false,
            profile: None,
            own_device: None,
            own: None,
            profile_avatar: None,
            keys_complete: None,
            #[cfg(feature = "live")]
            key_recovery: None,
        }
    }
    /// (user id, display name) from the SDK; None until the first sync starts.
    /// `mxc` of the account's own picture, if the server has one.
    pub fn profile_avatar(&self) -> Option<&str> {
        self.profile_avatar.as_deref()
    }
    pub fn profile(&self) -> Option<&(String, Option<String>)> {
        self.profile.as_ref()
    }
    /// This session's device ID (not in `devices()`); None until the profile arrives.
    pub fn own_device(&self) -> Option<&str> {
        self.own_device.as_deref()
    }
    /// This session's device with name, state and last activity; None until the first list arrives.
    pub fn own(&self) -> Option<&matrix_engine::verification::VerificationDevice> {
        self.own.as_ref()
    }
    pub fn devices(&self) -> &[matrix_engine::verification::VerificationDevice] {
        &self.devices
    }
    pub fn delete_dialog(&self) -> Option<&DeleteDialog> {
        self.delete.as_ref()
    }
    /// Opens the confirmation for `devices()[index]`; never for this session's own device.
    pub fn open_delete(&mut self, index: usize) -> bool {
        let Some(d) = self.devices.get(index) else { return false };
        if self.own_device.as_deref() == Some(&d.device_id) || self.delete.is_some() || self.rename.is_some() {
            return false;
        }
        self.delete = Some(DeleteDialog {
            device_id: d.device_id.clone(),
            name: device_title(d).to_owned(),
            busy: false,
            error: None,
        });
        true
    }
    pub fn cancel_delete(&mut self) {
        self.delete = None;
    }
    pub fn rename_dialog(&self) -> Option<&RenameDialog> {
        self.rename.as_ref()
    }
    /// Opens «Rinomina» for this session's device (known once the first list arrived).
    pub fn open_rename(&mut self) -> bool {
        if self.own.is_none() || self.delete.is_some() || self.rename.is_some() {
            return false;
        }
        self.rename = Some(RenameDialog { busy: false, error: None });
        true
    }
    pub fn cancel_rename(&mut self) {
        self.rename = None;
    }
    /// Queues the rename; the engine validates the name and answers with the fresh own device or an error.
    pub fn submit_rename(&mut self, name: &str) -> bool {
        let Some(dialog) = self.rename.as_mut().filter(|d| !d.busy) else { return false };
        if self.pending.is_some() {
            return false;
        }
        dialog.busy = true;
        dialog.error = None;
        self.pending = Some(VerificationCommand::RenameOwnDevice {
            account_id: self.account,
            display_name: name.to_owned(),
        });
        true
    }
    /// Queues the sign-out with the password typed now; the password lives only in the command.
    pub fn submit_delete(&mut self, mut password: zeroize::Zeroizing<String>) -> bool {
        let Some(dialog) = self.delete.as_mut().filter(|d| !d.busy) else { return false };
        if password.is_empty() || self.pending.is_some() {
            return false;
        }
        dialog.busy = true;
        dialog.error = None;
        self.pending = Some(VerificationCommand::DeleteDevice {
            account_id: self.account,
            device_id: dialog.device_id.clone(),
            password: matrix_engine::verification::DevicePassword::new(std::mem::take(&mut *password)),
        });
        true
    }
    /// «Recupera chiavi» is offered on an undecryptable row only while the recovery is known to be incomplete.
    pub fn offers_recover_keys(&self, utd: bool) -> bool {
        utd && self.keys_complete == Some(false)
    }
    #[cfg(feature = "live")]
    /// State of the last «Recupera chiavi» attempt, for the key field.
    pub fn key_recovery(&self) -> Option<matrix_engine::signin::RecoveryState> {
        self.key_recovery
    }
    #[cfg(feature = "live")]
    /// One-line failure shown by the key field: the same wording as the sign-in step.
    pub fn key_recovery_message(&self) -> Option<&'static str> {
        self.key_recovery.and_then(crate::signin_form::recovery_message)
    }
    #[cfg(feature = "live")]
    /// Queues the recovery with the key typed now; the key lives only in the command (same checks as sign-in).
    pub fn recover_keys(&mut self, mut key: zeroize::Zeroizing<String>) -> bool {
        use matrix_engine::signin::{RecoveryFailure, RecoveryState, Secret};
        if self.pending.is_some() || self.key_recovery == Some(RecoveryState::Pending) {
            return false;
        }
        let Ok(key) = Secret::new(std::mem::take(&mut *key)) else {
            self.key_recovery = Some(RecoveryState::Failed(RecoveryFailure::InvalidKey));
            return false;
        };
        self.key_recovery = Some(RecoveryState::Pending);
        self.pending = Some(VerificationCommand::RecoverKeys { account_id: self.account, key });
        true
    }
    pub fn can_begin(&self) -> bool {
        self.pending.is_none()
            && self.release.is_none()
            && !self.popup
            && !self.deciding
            && (self.failed
                || self.snapshot.as_ref().is_none_or(|s| {
                    matches!(
                        s.state,
                        VerificationState::Verified
                            | VerificationState::Cancelled
                            | VerificationState::Mismatched
                            | VerificationState::Failed
                    )
                }))
    }
    pub fn begin(&mut self, index: usize) -> bool {
        if !self.can_begin() {
            return false;
        }
        // A verified device is not offered for re-verification: no unexpected SAS.
        let Some(device) = self.devices.get(index).filter(|d| !d.verified) else {
            return false;
        };
        self.pending = Some(VerificationCommand::Begin {
            account_id: self.account,
            user_id: device.user_id.clone(),
            device_id: device.device_id.clone(),
        });
        self.start_popup();
        true
    }
    fn start_popup(&mut self) {
        self.snapshot = None;
        self.failed = false;
        self.deciding = true;
        self.popup = true;
        self.confirming = false;
        self.abandoned = false;
        self.popup_focus = 0;
    }
    /// «Verifica utente»: ask the whole user, not one device.
    pub fn begin_user(&mut self, user_id: String) -> bool {
        if !self.can_begin() {
            return false;
        }
        self.pending = Some(VerificationCommand::BeginUser { account_id: self.account, user_id });
        self.start_popup();
        true
    }
    pub fn pending(&self) -> Option<&VerificationCommand> {
        self.pending.as_ref()
    }
    pub fn apply(&mut self, event: VerificationEvent) {
        match event {
            VerificationEvent::Profile {
                account_id,
                user_id,
                display_name,
                device_id,
                avatar,
            } if account_id == self.account => {
                self.profile = Some((user_id, display_name));
                self.own_device = device_id;
                self.profile_avatar = avatar;
            }
            VerificationEvent::Snapshot(snapshot) if snapshot.account_id == self.account => {
                if self.popup && self.snapshot.as_ref().is_some_and(|s| s.user_id != snapshot.user_id || s.flow_id != snapshot.flow_id) {
                    return;
                }
                if !self.popup && self.closed_flow.as_ref() == Some(&snapshot.target()) && is_final(snapshot.state) {
                    self.release = Some(snapshot.target());
                }
                // A request from another device or user opens the popup; one the user already closed does not.
                if snapshot.state == VerificationState::Requested
                    && !self.popup
                    && self.closed_flow.as_ref() != Some(&snapshot.target())
                {
                    self.popup = true;
                }
                if self.snapshot.as_ref().is_none_or(|s| s.state != snapshot.state) {
                    self.confirming = false;
                    self.popup_focus = 0;
                }
                self.snapshot = Some(snapshot);
                self.deciding = false;
                self.failed = false;
                if std::mem::take(&mut self.abandoned) && !self.popup && self.snapshot.as_ref().is_some_and(|s| !is_final(s.state)) {
                    self.closed_flow = self.snapshot.as_ref().map(|s| s.target());
                    self.act(VerificationAction::Cancel);
                }
            }
            VerificationEvent::Devices {
                account_id,
                devices,
            } if account_id == self.account => {
                // The engine sends a fresh list only after a successful sign-out.
                if self.delete.as_ref().is_some_and(|d| d.busy && !devices.iter().any(|x| x.device_id == d.device_id)) {
                    self.delete = None;
                }
                self.devices = devices
            }
            VerificationEvent::DeviceDeleted { account_id, device_id } if account_id == self.account => {
                self.devices.retain(|x| x.device_id != device_id);
                if self.delete.as_ref().is_some_and(|d| d.device_id == device_id) {
                    self.delete = None;
                }
            }
            VerificationEvent::DeviceDeleteFailed { account_id, device_id, error }
                if account_id == self.account =>
            {
                use matrix_engine::verification::DeviceDeleteError::*;
                if let Some(d) = self.delete.as_mut().filter(|d| d.device_id == device_id) {
                    d.busy = false;
                    d.error = Some(match error {
                        Unavailable => "Non disponibile con questo account",
                        WrongPassword => "Password non corretta",
                        _ => "Non riuscito. Riprova.",
                    });
                }
            }
            VerificationEvent::OwnDevice { account_id, device } if account_id == self.account => {
                self.own = Some(device);
                // The engine sends the own device after a rename only when it succeeded.
                if self.rename.as_ref().is_some_and(|d| d.busy) {
                    self.rename = None;
                }
            }
            VerificationEvent::Error { account_id, error } if account_id == self.account && self.rename.as_ref().is_some_and(|d| d.busy) => {
                // A failed rename is not a failed verification: the dialog says what happened.
                let d = self.rename.as_mut().unwrap();
                d.busy = false;
                d.error = Some(match error {
                    matrix_engine::verification::VerificationError::InvalidName => "Nome non valido",
                    _ => "Non riuscito. Riprova.",
                });
            }
            #[cfg(feature = "live")]
            VerificationEvent::KeysRecovery { account_id, state } if account_id == self.account => {
                self.key_recovery = Some(state);
            }
            VerificationEvent::KeysComplete { account_id, complete } if account_id == self.account => {
                self.keys_complete = Some(complete);
                #[cfg(feature = "live")]
                if complete {
                    self.key_recovery = None;
                }
            }
            VerificationEvent::Error { account_id, .. } if account_id == self.account => {
                self.failed = true;
                self.pending = None;
                self.deciding = false;
                self.delete = None;
                self.abandoned = false;
                self.confirming = false;
            }
            _ => (),
        }
    }
    pub fn snapshot(&self) -> Option<&VerificationSnapshot> {
        self.snapshot.as_ref()
    }
    pub fn refresh(&mut self) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(VerificationCommand::List {
            account_id: self.account,
        });
        true
    }
    pub fn can_act(&self, action: VerificationAction) -> bool {
        if self.failed || self.pending.is_some() || self.deciding {
            return false;
        }
        use VerificationState::*;
        let Some(snapshot) = self.snapshot.as_ref() else {
            return false;
        };
        match action {
            VerificationAction::Confirm | VerificationAction::Mismatch => {
                matches!(snapshot.state, Compare { .. })
            }
            VerificationAction::Cancel => !is_final(snapshot.state),
            VerificationAction::Accept => snapshot.state == Requested,
            _ => false,
        }
    }
    pub fn act(&mut self, action: VerificationAction) -> bool {
        if !self.can_act(action) {
            return false;
        }
        self.pending = Some(VerificationCommand::Act {
            target: self.snapshot.as_ref().unwrap().target(),
            action,
        });
        self.deciding = true;
        true
    }
    pub fn popup_open(&self) -> bool {
        self.popup
    }
    fn comparing(&self) -> bool {
        !self.failed
            && !self.deciding
            && self.snapshot.as_ref().is_some_and(|s| matches!(s.state, VerificationState::Compare { .. }))
    }
    /// The popup's content for the current SDK state; `None` while closed.
    pub fn popup_view(&self) -> Option<PopupView> {
        use PopupButton::*;
        use VerificationState::*;
        if !self.popup {
            return None;
        }
        let view = |title, body, buttons| Some(PopupView { title, body, buttons });
        if self.failed {
            return view("Verifica", PopupBody::Outcome("Verifica non riuscita"), vec![Ok]);
        }
        let Some(s) = self.snapshot.as_ref() else {
            return view("Verifica", PopupBody::Waiting, vec![Cancel]);
        };
        match s.state {
            Requested => view(
                "Richiesta di verifica",
                PopupBody::Request(format!("{} · {}", s.user_id, s.device_id)),
                vec![Accept, Decline],
            ),
            Compare { .. } if self.confirming => view("Annullare la verifica?", PopupBody::Empty, vec![Yes, No]),
            Compare { decimals } if !self.deciding => view(
                "Verifica",
                match s.sas_emoji.as_ref().filter(|e| !e.is_empty()) {
                    Some(emoji) => PopupBody::Emoji(emoji.clone()),
                    None => PopupBody::Decimals(decimals),
                },
                vec![Match, NoMatch],
            ),
            Verified => view("Verifica", PopupBody::Outcome("Verificato"), vec![Ok]),
            Cancelled => view("Verifica", PopupBody::Outcome("Verifica annullata"), vec![Ok]),
            Mismatched => view("Verifica", PopupBody::Outcome("Gli emoji non corrispondono"), vec![Ok]),
            Failed => view("Verifica", PopupBody::Outcome("Verifica non riuscita"), vec![Ok]),
            _ => view("Verifica", PopupBody::Waiting, vec![Cancel]),
        }
    }
    /// Tab order of the popup: its buttons, then the X.
    pub fn popup_controls(&self) -> Vec<PopupButton> {
        self.popup_view()
            .map(|v| v.buttons.into_iter().chain([PopupButton::Close]).collect())
            .unwrap_or_default()
    }
    pub fn popup_focused(&self) -> Option<PopupButton> {
        self.popup_controls().get(self.popup_focus).copied()
    }
    pub fn popup_set_focus(&mut self, button: PopupButton) {
        if let Some(i) = self.popup_controls().iter().position(|b| *b == button) {
            self.popup_focus = i;
        }
    }
    /// Tab / Shift+Tab: the focus never leaves the popup.
    pub fn popup_tab(&mut self, reverse: bool) {
        let n = self.popup_controls().len();
        if n > 0 {
            self.popup_focus = (self.popup_focus + if reverse { n - 1 } else { 1 }) % n;
        }
    }
    /// X, Esc and a click outside: ask first while comparing, otherwise leave (cancelling a live flow).
    pub fn popup_close(&mut self) {
        if !self.popup {
            return;
        }
        if self.confirming {
            self.confirming = false;
        } else if self.comparing() {
            self.confirming = true;
        } else {
            self.close_now();
            return;
        }
        self.popup_focus = 0;
    }
    /// Closes the popup; a flow still alive behind it is cancelled through the SDK, never left running.
    fn close_now(&mut self) {
        let live = !self.failed && self.snapshot.as_ref().map_or(self.deciding, |s| !is_final(s.state));
        self.closed_flow = self.snapshot.as_ref().map(|s| s.target());
        if let Some(snapshot) = self.snapshot.as_ref().filter(|s| self.failed || is_final(s.state)) {
            self.release = Some(snapshot.target());
        }
        self.popup = false;
        self.confirming = false;
        if live && !self.act(VerificationAction::Cancel) {
            self.abandoned = true;
        }
    }
    /// A popup button pressed by mouse, keyboard or assistive technology; false if it does not apply now.
    pub fn popup_press(&mut self, button: PopupButton) -> bool {
        if !self.popup_controls().contains(&button) {
            return false;
        }
        use PopupButton::*;
        match button {
            Close => self.popup_close(),
            Accept => return self.act(VerificationAction::Accept),
            Match => return self.act(VerificationAction::Confirm),
            NoMatch => return self.act(VerificationAction::Mismatch),
            Decline | Cancel | Yes => self.close_now(),
            No => {
                self.confirming = false;
                self.popup_focus = 0;
            }
            Ok => self.close_now(),
        }
        true
    }
    pub fn flush(&mut self, sender: &mut matrix_engine::Sender<VerificationCommand>) {
        if let Some(target) = self.release.take() {
            match sender.try_send(VerificationCommand::Release { target }) {
                Ok(()) => (),
                Err(matrix_engine::TrySendError::Full(VerificationCommand::Release { target }) | matrix_engine::TrySendError::Busy(VerificationCommand::Release { target })) => {
                    self.release = Some(target);
                    return;
                }
                Err(_) => { self.failed = true; return; }
            }
        }
        if let Some(command) = self.pending.take() {
            match sender.try_send(command) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(command)
                    | matrix_engine::TrySendError::Busy(command),
                ) => self.pending = Some(command),
                Err(matrix_engine::TrySendError::Closed(_)) => {
                    self.deciding = false;
                    self.failed = true;
                }
            }
        }
    }
}

fn is_final(state: VerificationState) -> bool {
    matches!(
        state,
        VerificationState::Verified | VerificationState::Cancelled | VerificationState::Mismatched | VerificationState::Failed
    )
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn terminal_popup_keeps_its_flow_when_another_snapshot_arrives() {
        let account = AccountId::new(1);
        let mut v = DeviceVerification::new(account);
        let snapshot = |id: &str, state| VerificationSnapshot {
            account_id: account, user_id: "@a:x".into(), device_id: "DEV".into(),
            flow_id: id.into(), state, device_verified: false, sas_emoji: None,
        };
        v.apply(VerificationEvent::Snapshot(snapshot("first", VerificationState::Requested)));
        v.apply(VerificationEvent::Snapshot(snapshot("first", VerificationState::Cancelled)));
        v.apply(VerificationEvent::Snapshot(snapshot("second", VerificationState::Requested)));
        assert_eq!(v.snapshot().unwrap().flow_id, "first");
        v.popup_close();
        assert!(!v.popup_open());
        assert!(v.pending().is_none(), "Esc must not cancel the second flow");
    }

    #[test]
    fn closing_terminal_flow_retries_release_and_distinguishes_senders() {
        use matrix_engine::verification::VerificationCommand;
        let account = AccountId::new(1);
        let mut v = DeviceVerification::new(account);
        let snap = |user: &str, state| VerificationSnapshot {
            account_id: account, user_id: user.into(), device_id: "DEV".into(),
            flow_id: "same".into(), state, device_verified: false, sas_emoji: None,
        };
        v.apply(VerificationEvent::Snapshot(snap("@a:x", VerificationState::Requested)));
        v.apply(VerificationEvent::Snapshot(snap("@a:x", VerificationState::Cancelled)));
        v.popup_close();
        let (mut tx, mut rx) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
        tx.try_send(VerificationCommand::List { account_id: account }).unwrap_or_else(|_| panic!("queue"));
        v.flush(&mut tx);
        assert!(rx.try_recv().is_ok());
        v.flush(&mut tx);
        assert!(matches!(rx.try_recv(), Ok(VerificationCommand::Release { target }) if target.user_id == "@a:x" && target.flow_id == "same"));
        v.apply(VerificationEvent::Snapshot(snap("@a:x", VerificationState::Requested)));
        assert!(!v.popup_open());
        v.apply(VerificationEvent::Snapshot(snap("@b:x", VerificationState::Requested)));
        assert!(v.popup_open());
    }

    #[test]
    fn profile_is_kept_only_for_own_account() {
        let mut v = DeviceVerification::new(AccountId::new(1));
        let ev = |n, name: Option<&str>| VerificationEvent::Profile {
            account_id: AccountId::new(n),
            user_id: "@a:x".into(),
            display_name: name.map(Into::into),
            device_id: Some("OWNDEV".into()),
            avatar: Some("mxc://x/a".into()),
        };
        v.apply(ev(2, Some("Altro")));
        assert!(v.profile().is_none());
        v.apply(ev(1, Some("Ada")));
        assert_eq!(
            v.profile(),
            Some(&("@a:x".to_string(), Some("Ada".to_string())))
        );
        assert_eq!(v.own_device(), Some("OWNDEV"));
        assert_eq!(v.profile_avatar(), Some("mxc://x/a"));
        // The picture is removed on the server: the next profile read has none.
        v.apply(VerificationEvent::Profile {
            account_id: AccountId::new(1),
            user_id: "@a:x".into(),
            display_name: None,
            device_id: None,
            avatar: None,
        });
        assert_eq!(v.profile_avatar(), None);
    }

    fn dev(name: Option<&str>, verified: bool, ts: Option<u64>, ip: Option<&str>) -> matrix_engine::verification::VerificationDevice {
        matrix_engine::verification::VerificationDevice {
            user_id: "@a:x".into(),
            device_id: "ABCDEF".into(),
            verified,
            display_name: name.map(Into::into),
            last_seen_ts: ts,
            last_seen_ip: ip.map(Into::into),
        }
    }

    #[test]
    fn device_line_shows_only_present_fields() {
        let t = |ms: u64| format!("T{ms}");
        let full = dev(Some("Gnobbi (macOS)"), true, Some(7), Some("192.0.2.17"));
        assert_eq!(device_title(&full), "Gnobbi (macOS)");
        assert_eq!(device_detail(&full, t), "Verificato · Ultima attività T7 · 192.0.2.17 · ABCDEF");
        let bare = dev(None, false, None, None);
        assert_eq!(device_title(&bare), "ABCDEF");
        assert_eq!(device_detail(&bare, t), "Non verificato");
        assert_eq!(device_detail(&dev(Some("N"), false, None, Some("::1")), t), "Non verificato · ::1 · ABCDEF");
    }

    #[test]
    fn screen_reader_label_has_the_row_data() {
        assert_eq!(device_label(&dev(None, true, None, None)), "ABCDEF · Verificato");
        let l = device_label(&dev(Some("Mac"), false, Some(1), Some("203.0.113.1")));
        assert!(l.starts_with("Mac · Non verificato · Ultima attività ") && l.ends_with(" · 203.0.113.1 · ABCDEF"), "{l}");
    }

    #[test]
    fn delete_flow_outcomes() {
        use matrix_engine::verification::DeviceDeleteError;
        let a = AccountId::new(1);
        let mut v = DeviceVerification::new(a);
        let list = |ids: &[&str]| VerificationEvent::Devices {
            account_id: a,
            devices: ids.iter().map(|i| { let mut d = dev(Some("Vecchio"), true, None, None); d.device_id = (*i).into(); d }).collect(),
        };
        v.apply(VerificationEvent::Profile { account_id: a, user_id: "@a:x".into(), display_name: None, device_id: Some("OWN".into()), avatar: None });
        v.apply(list(&["OWN", "OLD"]));
        assert!(!v.open_delete(0), "never the current device");
        assert!(v.open_delete(1));
        assert_eq!(v.delete_dialog().unwrap().name, "Vecchio");
        let pw = || zeroize::Zeroizing::new("pw".to_owned());
        // wrong password: one line, dialog stays, retry possible
        assert!(v.submit_delete(pw()));
        assert!(matches!(v.pending, Some(VerificationCommand::DeleteDevice { .. })));
        v.pending = None;
        let fail = |error| VerificationEvent::DeviceDeleteFailed { account_id: a, device_id: "OLD".into(), error };
        v.apply(fail(DeviceDeleteError::WrongPassword));
        assert_eq!(v.delete_dialog().unwrap().error, Some("Password non corretta"));
        v.apply(fail(DeviceDeleteError::Unavailable));
        assert_eq!(v.delete_dialog().unwrap().error, Some("Non disponibile con questo account"));
        // success but the re-read failed: engine says DeviceDeleted; dialog closes, row goes, no failure
        assert!(v.submit_delete(pw()));
        v.pending = None;
        v.apply(VerificationEvent::DeviceDeleted { account_id: a, device_id: "OLD".into() });
        assert!(v.delete_dialog().is_none() && !v.failed);
        assert!(v.devices.iter().all(|d| d.device_id != "OLD"));
        v.apply(list(&["OWN", "OLD"]));
        assert!(v.open_delete(1));
        // success: the refreshed list no longer has the device
        assert!(v.submit_delete(pw()));
        v.pending = None;
        v.apply(list(&["OWN", "OLD"])); // unrelated refresh while busy keeps the dialog
        assert!(v.delete_dialog().is_some());
        v.apply(list(&["OWN"]));
        assert!(v.delete_dialog().is_none());
        // cancel
        v.apply(list(&["OWN", "OLD"]));
        assert!(v.open_delete(1));
        v.cancel_delete();
        assert!(v.delete_dialog().is_none());
    }

    #[test]
    fn rename_own_device_dialog() {
        use matrix_engine::verification::VerificationError;
        let a = AccountId::new(1);
        let mut v = DeviceVerification::new(a);
        assert!(!v.open_rename(), "own device not known yet");
        let own = |name: &str| VerificationEvent::OwnDevice { account_id: a, device: dev(Some(name), true, None, None) };
        v.apply(own("Gnobbi (macOS)"));
        assert!(v.open_rename());
        assert!(!v.open_rename(), "one dialog at a time");
        // cancel
        v.cancel_rename();
        assert!(v.rename_dialog().is_none());
        // invalid name: engine answers Error; dialog stays with one line, no verification failure
        assert!(v.open_rename());
        assert!(v.submit_rename("  "));
        assert!(matches!(&v.pending, Some(VerificationCommand::RenameOwnDevice { display_name, .. }) if display_name == "  "));
        assert!(!v.submit_rename("x"), "busy");
        v.pending = None;
        v.apply(VerificationEvent::Error { account_id: a, error: VerificationError::InvalidName });
        assert_eq!(v.rename_dialog().unwrap().error, Some("Nome non valido"));
        assert!(!v.rename_dialog().unwrap().busy && !v.failed);
        // retry succeeds: the fresh own device closes the dialog and carries the new name
        assert!(v.submit_rename("Portatile"));
        v.pending = None;
        v.apply(VerificationEvent::Error { account_id: a, error: VerificationError::Transport });
        assert_eq!(v.rename_dialog().unwrap().error, Some("Non riuscito. Riprova."));
        assert!(v.submit_rename("Portatile"));
        v.pending = None;
        v.apply(own("Portatile"));
        assert!(v.rename_dialog().is_none());
        assert_eq!(device_title(v.own().unwrap()), "Portatile");
        // an unrelated own-device refresh never opens or closes anything
        assert!(v.open_rename());
        v.apply(own("Portatile"));
        assert!(v.rename_dialog().is_some());
    }

    #[test]
    fn last_activity_uses_the_shared_local_time_format() {
        use chrono::FixedOffset;
        const THU_2330: u64 = 1_768_519_800_000; // 2026-01-15 23:30 UTC
        const DAY: u64 = 86_400_000;
        let cet = FixedOffset::east_opt(3600).unwrap();
        let d = dev(Some("N"), true, Some(THU_2330), None);
        let p = crate::time_fmt::Patterns::fallback();
        let at = |now| device_detail(&d, |t| crate::time_fmt::day_or_time_in(t, now, &cet, &p));
        // 23:30 UTC is already Friday 16 Jan 00:30 in CET: the local day decides, not UTC.
        assert_eq!(at(THU_2330 + 60_000), "Verificato · Ultima attività 00:30 · ABCDEF");
        assert_eq!(at(THU_2330 + 3 * DAY), "Verificato · Ultima attività Ven 00:30 · ABCDEF");
        assert_eq!(at(THU_2330 + 30 * DAY), "Verificato · Ultima attività 16/01/26 00:30 · ABCDEF");
    }

    #[test]
    fn delete_acts_on_the_active_account_only() {
        use matrix_engine::verification::DeviceDeleteError;
        let (a, b) = (AccountId::new(1), AccountId::new(2));
        let mut v = DeviceVerification::new(a);
        let list = |acc, id: &str| VerificationEvent::Devices { account_id: acc, devices: vec![{ let mut d = dev(Some("X"), true, None, None); d.device_id = id.into(); d }] };
        v.apply(list(b, "OTHERACC"));
        assert!(v.devices().is_empty(), "another account's list is ignored");
        v.apply(list(a, "MINE"));
        assert!(v.open_delete(0) && v.submit_delete(zeroize::Zeroizing::new("pw".into())));
        match v.pending.take() {
            Some(VerificationCommand::DeleteDevice { account_id, device_id, .. }) => assert_eq!((account_id, device_id.as_str()), (a, "MINE")),
            _ => panic!("expected DeleteDevice"),
        }
        // A failure reported for another account or another device leaves this dialog untouched.
        v.apply(VerificationEvent::DeviceDeleteFailed { account_id: b, device_id: "MINE".into(), error: DeviceDeleteError::WrongPassword });
        v.apply(VerificationEvent::DeviceDeleteFailed { account_id: a, device_id: "ELSE".into(), error: DeviceDeleteError::WrongPassword });
        assert_eq!(v.delete_dialog().unwrap().error, None);
        // Every error text is a single line; empty password and a second submit are refused.
        for e in [DeviceDeleteError::WrongPassword, DeviceDeleteError::Unavailable, DeviceDeleteError::Transport, DeviceDeleteError::Rejected] {
            v.apply(VerificationEvent::DeviceDeleteFailed { account_id: a, device_id: "MINE".into(), error: e });
            let text = v.delete_dialog().unwrap().error.unwrap();
            assert!(!text.contains('\n') && !text.is_empty(), "{text:?}");
        }
        assert!(!v.submit_delete(zeroize::Zeroizing::new(String::new())), "empty password");
        assert!(v.submit_delete(zeroize::Zeroizing::new("x".into())));
        assert!(!v.submit_delete(zeroize::Zeroizing::new("x".into())), "no double submit while busy");
    }

    #[test]
    fn own_device_comes_from_its_own_event() {
        let mut v = DeviceVerification::new(AccountId::new(1));
        let ev = |n| VerificationEvent::OwnDevice { account_id: AccountId::new(n), device: dev(None, true, None, None) };
        v.apply(ev(2));
        assert!(v.own().is_none());
        v.apply(ev(1));
        assert!(v.own().is_some_and(|d| d.verified));
        assert!(v.devices().is_empty());
    }
}

#[cfg(test)]
mod key_recovery_tests {
    use super::*;
    #[cfg(feature = "live")]
    use matrix_engine::signin::{RecoveryFailure, RecoveryState};

    fn complete(account: u64, complete: bool) -> VerificationEvent {
        VerificationEvent::KeysComplete { account_id: AccountId::new(account), complete }
    }

    #[test]
    fn action_only_on_undecryptable_rows_and_only_while_incomplete() {
        let mut v = DeviceVerification::new(AccountId::new(1));
        assert!(!v.offers_recover_keys(true), "unknown state offers nothing");
        v.apply(complete(2, false));
        assert!(!v.offers_recover_keys(true), "another account's state is ignored");
        v.apply(complete(1, false));
        assert!(v.offers_recover_keys(true));
        assert!(!v.offers_recover_keys(false), "readable rows never offer it");
        v.apply(complete(1, true));
        assert!(!v.offers_recover_keys(true), "gone everywhere once complete");
    }

    #[cfg(feature = "live")]
    #[test]
    fn key_goes_into_one_command_and_failures_use_the_sign_in_wording() {
        let mut v = DeviceVerification::new(AccountId::new(1));
        assert!(!v.recover_keys(zeroize::Zeroizing::new(String::new())));
        assert_eq!(v.key_recovery(), Some(RecoveryState::Failed(RecoveryFailure::InvalidKey)));
        assert_eq!(v.key_recovery_message(), Some("Chiave di recupero Matrix non valida. Controlla e riprova."));
        assert!(v.recover_keys(zeroize::Zeroizing::new("EsTu key".into())));
        assert!(matches!(v.pending, Some(VerificationCommand::RecoverKeys { .. })));
        assert!(!v.recover_keys(zeroize::Zeroizing::new("again".into())), "one attempt at a time");
        v.apply(VerificationEvent::KeysRecovery {
            account_id: AccountId::new(1),
            state: RecoveryState::Failed(RecoveryFailure::Network),
        });
        assert!(v.key_recovery_message().is_some_and(|m| m.starts_with("Server non raggiungibile")));
        v.apply(complete(1, true));
        assert_eq!(v.key_recovery(), None);
    }
}
