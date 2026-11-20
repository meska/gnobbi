//! UI-only projection; SAS and device identities never enter diagnostics.
use crate::i18n::{plural, t};
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
    let mut parts = vec![if d.verified { t("Verified") } else { t("Not verified") }.to_owned()];
    parts.extend(d.last_seen_ts.map(|ms| t("Last activity {time}").replace("{time}", &time(ms))));
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
    /// One id for a row's «Disconnetti»; the idle devices for «Disconnetti inattivi…» (never this session's).
    pub device_ids: Vec<String>,
    pub name: String,
    /// Some only in the «inattivi» form: the selected idle threshold.
    pub threshold: Option<crate::inactive::InactiveThreshold>,
    /// Devices left out of the list because their last activity is unknown.
    pub unknown_activity: usize,
    pub busy: bool,
    pub error: Option<&'static str>,
    /// Set by a failed try; the view consumes it once to put the focus back in the (emptied) field.
    refocus: std::cell::Cell<bool>,
}

impl DeleteDialog {
    /// One rule for colour, a11y and submit: something to sign out and no request in flight (the typed text is checked by the view).
    pub fn confirm_enabled(&self) -> bool {
        !self.busy && !self.device_ids.is_empty()
    }
    pub fn counter(&self) -> String {
        plural(self.device_ids.len() as u64, "{n} device to sign out", "{n} devices to sign out")
    }
    pub fn unknown_note(&self) -> Option<String> {
        let n = self.unknown_activity;
        (n > 0).then(|| plural(n as u64, "{n} device without last activity not included", "{n} devices without last activity not included"))
    }
    pub fn take_refocus(&self) -> bool {
        self.refocus.replace(false)
    }
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
            Self::Accept => t("Accept"),
            Self::Decline => t("Decline"),
            Self::Cancel => t("Cancel"),
            Self::Match => t("They match"),
            Self::NoMatch => t("They don't match"),
            Self::Yes => t("Yes"),
            Self::No => "No",
            Self::Ok => "OK",
            Self::Close => t("Close"),
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
    refresh_after_verification: bool,
    deciding: bool,
    devices: Vec<matrix_engine::verification::VerificationDevice>,
    failed: bool,
    profile: Option<(String, Option<String>)>,
    own_device: Option<String>,
    own: Option<matrix_engine::verification::VerificationDevice>,
    profile_avatar: Option<String>,
    /// Key recovery complete (backup on, cross-signing keys held); None until the engine says.
    keys_complete: Option<bool>,
    key_recovery: Option<matrix_engine::signin::RecoveryState>,
    /// «Esporta chiavi…» / «Rigenera chiave di recupero…» sub-page of «Privacy e sicurezza».
    pub keys: Option<crate::key_export::KeysPage>,
    /// The engine's view of the account's recovery key: Some(true) = none exists; None until known.
    recovery_disabled: Option<bool>,
    /// The offer was answered or accepted in this session.
    offer_asked: bool,
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
            refresh_after_verification: false,
            deciding: false,
            devices: Vec::new(),
            failed: false,
            profile: None,
            own_device: None,
            own: None,
            profile_avatar: None,
            keys_complete: None,
            key_recovery: None,
            keys: None,
            recovery_disabled: None,
            offer_asked: false,
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
            device_ids: vec![d.device_id.clone()],
            name: device_title(d).to_owned(),
            threshold: None,
            unknown_activity: 0,
            busy: false,
            error: None,
            refocus: Default::default(),
        });
        true
    }
    /// «Disconnetti inattivi…» is offered only when some device is idle past the smallest threshold.
    pub fn has_inactive(&self, now_ms: u64) -> bool {
        crate::inactive::has_inactive_devices(&self.devices, self.own_device.as_deref(), now_ms)
    }
    /// Opens the shared dialog in its «inattivi» form with the default threshold.
    pub fn open_delete_inactive(&mut self, now_ms: u64) -> bool {
        if self.delete.is_some() || self.rename.is_some() || !self.has_inactive(now_ms) {
            return false;
        }
        self.delete = Some(DeleteDialog {
            device_ids: vec![],
            name: t("Sign out inactive").to_owned(),
            threshold: Some(Default::default()),
            unknown_activity: 0,
            busy: false,
            error: None,
            refocus: Default::default(),
        });
        self.set_delete_threshold(Default::default(), now_ms)
    }
    /// Recomputes the list for `threshold`; ignored while the request is in flight.
    pub fn set_delete_threshold(&mut self, threshold: crate::inactive::InactiveThreshold, now_ms: u64) -> bool {
        let Some(d) = self.delete.as_mut().filter(|d| !d.busy && d.threshold.is_some()) else { return false };
        let found = crate::inactive::inactive_devices(&self.devices, self.own_device.as_deref(), threshold, now_ms);
        d.device_ids = found.devices.iter().map(|x| x.device_id.clone()).collect();
        d.unknown_activity = found.unknown_activity;
        d.threshold = Some(threshold);
        d.error = None;
        true
    }
    /// Next threshold of the selector (1 mese → 3 mesi → 6 mesi → 1 anno → 1 mese).
    pub fn cycle_delete_threshold(&mut self, now_ms: u64) -> bool {
        let next = self.delete.as_ref().and_then(|d| d.threshold).map(|t| t.next());
        next.is_some_and(|t| self.set_delete_threshold(t, now_ms))
    }
    /// Devices of the open dialog, for the list (name, last activity).
    pub fn delete_targets(&self) -> Vec<&matrix_engine::verification::VerificationDevice> {
        let Some(d) = &self.delete else { return vec![] };
        self.devices.iter().filter(|x| d.device_ids.contains(&x.device_id)).collect()
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
    /// Queues «Modifica profilo»; false while another command waits (the dialog stays open to retry).
    pub fn save_profile(&mut self, name: Option<String>, avatar: matrix_engine::verification::ProfileAvatar) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(VerificationCommand::SaveProfile { account_id: self.account, name, avatar });
        true
    }
    /// «Disconnetti» is live only with a typed password and no request in flight; the view checks it before emptying the field.
    pub fn can_submit_delete(&self, has_text: bool) -> bool {
        has_text && self.pending.is_none() && self.delete.as_ref().is_some_and(DeleteDialog::confirm_enabled)
    }
    /// Queues the sign-out with the password typed now; the password lives only in the command.
    pub fn submit_delete(&mut self, mut password: zeroize::Zeroizing<String>) -> bool {
        if !self.can_submit_delete(!password.is_empty()) {
            return false;
        }
        let Some(dialog) = self.delete.as_mut() else { return false };
        dialog.busy = true;
        dialog.error = None;
        self.pending = Some(VerificationCommand::DeleteDevice {
            account_id: self.account,
            device_ids: dialog.device_ids.clone(),
            password: matrix_engine::verification::DevicePassword::new(std::mem::take(&mut *password)),
        });
        true
    }
    /// «Recupera chiavi» is offered on an undecryptable row only while the recovery is known to be incomplete.
    pub fn offers_recover_keys(&self, utd: bool) -> bool {
        utd && self.keys_complete == Some(false)
    }
    /// State of the last «Recupera chiavi» attempt, for the key field.
    pub fn key_recovery(&self) -> Option<matrix_engine::signin::RecoveryState> {
        self.key_recovery
    }
    /// One-line failure shown by the key field: the same wording as the sign-in step.
    pub fn key_recovery_message(&self) -> Option<&'static str> {
        self.key_recovery.and_then(crate::signin_form::recovery_message)
    }
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
    /// Queues the export once the file is chosen. The passphrase and the phrase live in the command;
    /// copies are kept in the page only to refill the fields after a failure.
    pub fn export_keys(
        &mut self,
        dest: std::path::PathBuf,
        password: &zeroize::Zeroizing<String>,
        phrase: Option<&zeroize::Zeroizing<String>>,
    ) -> bool {
        use matrix_engine::signin::Secret;
        let Some(page) = self.keys.as_mut().filter(|p| p.kind == crate::key_export::KeysKind::Export) else {
            return false;
        };
        if self.pending.is_some() {
            page.busy = false;
            return false;
        }
        let Ok(passphrase) = Secret::new(password.to_string()) else {
            page.busy = false;
            page.line = Some((crate::key_export::PasswordError::TooShort.text().into(), true));
            return false;
        };
        let phrase_secret = match phrase.as_ref().map(|p| Secret::new(p.to_string())) {
            Some(Ok(secret)) => Some(secret),
            Some(Err(_)) => {
                page.busy = false;
                page.line = Some((crate::i18n::t("Wrong unlock passphrase").into(), true));
                return false;
            }
            None => None,
        };
        page.held = Some(crate::key_export::Held { password: password.clone(), phrase: phrase.cloned() });
        page.busy = true;
        page.line = None;
        self.pending = Some(VerificationCommand::ExportRoomKeys {
            account_id: self.account,
            dest,
            passphrase,
            phrase: phrase_secret,
        });
        true
    }
    pub fn reset_recovery(&mut self, phrase: Option<&zeroize::Zeroizing<String>>) -> bool {
        use matrix_engine::signin::Secret;
        let Some(page) = self.keys.as_mut().filter(|p| p.kind == crate::key_export::KeysKind::Reset && !p.busy && !p.shown()) else {
            return false;
        };
        let phrase = match phrase.map(|p| Secret::new(p.to_string())) {
            Some(Err(_)) => {
                page.line = Some((crate::i18n::t("Wrong unlock passphrase").into(), true));
                return false;
            }
            Some(Ok(p)) => Some(p),
            None => None,
        };
        if self.pending.is_some() {
            return false;
        }
        page.busy = true;
        page.line = None;
        self.pending = Some(VerificationCommand::ResetRecoveryKey { account_id: self.account, phrase });
        true
    }
    /// Which recovery dialog (create / enter the existing key) is due now, if any.
    pub fn recovery_offer(&self, suppressed: bool) -> Option<crate::key_export::OfferKind> {
        let kind = crate::key_export::offer_kind(self.recovery_disabled, self.keys_complete);
        crate::key_export::should_offer(kind, suppressed, self.offer_asked, self.keys.is_some())
    }
    pub fn offers_recovery_key(&self, suppressed: bool) -> bool {
        self.recovery_offer(suppressed) == Some(crate::key_export::OfferKind::Create)
    }
    /// «Non ora» / «Non chiedere più»: nothing more this session.
    pub fn decline_offer(&mut self) {
        self.offer_asked = true;
    }
    /// «Crea chiave di recupero»: the same command and key page as «Rigenera chiave di recupero…».
    pub fn accept_offer(&mut self) {
        self.offer_asked = true;
        if self.recovery_disabled == Some(true) {
            self.keys = Some(crate::key_export::KeysPage::new(crate::key_export::KeysKind::Reset));
            self.reset_recovery(None);
        }
    }
    /// «L'ho salvata»: the key is dropped (wiped) and the page closes.
    pub fn confirm_key_saved(&mut self) {
        self.keys = None;
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
                // Fallback only: the engine sends DeviceDeleted first, so a list without the device is not needed to close.
                if self.delete.as_ref().is_some_and(|d| d.busy && d.device_ids.iter().all(|id| !devices.iter().any(|x| x.device_id == *id))) {
                    self.delete = None;
                }
                self.devices = devices
            }
            VerificationEvent::DeviceDeleted { account_id, device_ids } if account_id == self.account => {
                self.devices.retain(|x| !device_ids.contains(&x.device_id));
                if self.delete.as_ref().is_some_and(|d| d.device_ids == device_ids) {
                    self.delete = None;
                }
            }
            VerificationEvent::DeviceDeleteFailed { account_id, device_ids, error }
                if account_id == self.account =>
            {
                use matrix_engine::verification::DeviceDeleteError::*;
                if let Some(d) = self.delete.as_mut().filter(|d| d.device_ids == device_ids) {
                    d.busy = false;
                    d.refocus.set(true);
                    d.error = Some(match error {
                        Unavailable => t("Not available with this account"),
                        WrongPassword => t("Incorrect password"),
                        _ => t("Failed. Try again."),
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
                    matrix_engine::verification::VerificationError::InvalidName => t("Invalid name"),
                    _ => t("Failed. Try again."),
                });
            }
            VerificationEvent::KeysRecovery { account_id, state } if account_id == self.account => {
                self.key_recovery = Some(state);
            }
            VerificationEvent::KeysExported { account_id, result } if account_id == self.account => {
                if let Some(page) = self.keys.as_mut().filter(|p| p.kind == crate::key_export::KeysKind::Export) {
                    page.busy = false;
                    page.refill = true;
                    page.line = Some(match result {
                        Ok(count) => (crate::key_export::exported_text(count), false),
                        Err(error) => (crate::key_export::export_error_text(error).into(), true),
                    });
                    // Success wipes the typed password; a failure leaves it for the retry.
                    if result.is_ok() {
                        page.held = None;
                    }
                }
            }
            VerificationEvent::RecoveryKeyReset { account_id, result } if account_id == self.account => {
                if let Some(page) = self.keys.as_mut().filter(|p| p.kind == crate::key_export::KeysKind::Reset) {
                    page.busy = false;
                    match result {
                        Ok(secret) => (page.secret, page.line) = (Some(secret), None),
                        Err(error) => page.line = Some((crate::key_export::reset_error_text(error).into(), true)),
                    }
                }
            }
            VerificationEvent::RecoveryDisabled { account_id, disabled } if account_id == self.account => {
                self.recovery_disabled = Some(disabled);
            }
            VerificationEvent::KeysComplete { account_id, complete } if account_id == self.account => {
                self.keys_complete = Some(complete);
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
            return view(t("Verification"), PopupBody::Outcome(t("Verification failed")), vec![Ok]);
        }
        let Some(s) = self.snapshot.as_ref() else {
            return view(t("Verification"), PopupBody::Waiting, vec![Cancel]);
        };
        match s.state {
            Requested => view(
                t("Verification request"),
                PopupBody::Request(format!("{} · {}", s.user_id, s.device_id)),
                vec![Accept, Decline],
            ),
            Compare { .. } if self.confirming => view(t("Cancel the verification?"), PopupBody::Empty, vec![Yes, No]),
            Compare { decimals } if !self.deciding => view(
                t("Verification"),
                match s.sas_emoji.as_ref().filter(|e| !e.is_empty()) {
                    Some(emoji) => PopupBody::Emoji(emoji.clone()),
                    None => PopupBody::Decimals(decimals),
                },
                vec![Match, NoMatch],
            ),
            Verified => view(t("Verification"), PopupBody::Outcome(t("Verified")), vec![Ok]),
            Cancelled => view(t("Verification"), PopupBody::Outcome(t("Verification cancelled")), vec![Ok]),
            Mismatched => view(t("Verification"), PopupBody::Outcome(t("The emoji don't match")), vec![Ok]),
            Failed => view(t("Verification"), PopupBody::Outcome(t("Verification failed")), vec![Ok]),
            _ => view(t("Verification"), PopupBody::Waiting, vec![Cancel]),
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
        if !self.failed && self.snapshot.as_ref().is_some_and(|s| s.state == VerificationState::Verified) {
            self.refresh_after_verification = true;
        }
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
        if self.refresh_after_verification && self.refresh() {
            self.refresh_after_verification = false;
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

    #[test]
    fn save_profile_queues_one_command_for_own_account() {
        use matrix_engine::verification::ProfileAvatar;
        let mut v = DeviceVerification::new(AccountId::new(3));
        assert!(v.save_profile(Some("Ada".into()), ProfileAvatar::Remove));
        assert!(matches!(v.pending(), Some(VerificationCommand::SaveProfile { account_id, name, avatar: ProfileAvatar::Remove }) if *account_id == AccountId::new(3) && name.as_deref() == Some("Ada")));
        assert!(!v.save_profile(None, ProfileAvatar::Keep), "one command at a time");
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
        assert_eq!(device_detail(&full, t), "Verified · Last activity T7 · 192.0.2.17 · ABCDEF");
        let bare = dev(None, false, None, None);
        assert_eq!(device_title(&bare), "ABCDEF");
        assert_eq!(device_detail(&bare, t), "Not verified");
        assert_eq!(device_detail(&dev(Some("N"), false, None, Some("::1")), t), "Not verified · ::1 · ABCDEF");
    }

    #[test]
    fn screen_reader_label_has_the_row_data() {
        assert_eq!(device_label(&dev(None, true, None, None)), "ABCDEF · Verified");
        let l = device_label(&dev(Some("Mac"), false, Some(1), Some("203.0.113.1")));
        assert!(l.starts_with("Mac · Not verified · Last activity ") && l.ends_with(" · 203.0.113.1 · ABCDEF"), "{l}");
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
        let fail = |error| VerificationEvent::DeviceDeleteFailed { account_id: a, device_ids: vec!["OLD".into()], error };
        v.apply(fail(DeviceDeleteError::WrongPassword));
        assert_eq!(v.delete_dialog().unwrap().error, Some("Incorrect password"));
        v.apply(fail(DeviceDeleteError::Unavailable));
        assert_eq!(v.delete_dialog().unwrap().error, Some("Not available with this account"));
        // success but the re-read failed: engine says DeviceDeleted; dialog closes, row goes, no failure
        assert!(v.submit_delete(pw()));
        v.pending = None;
        v.apply(VerificationEvent::DeviceDeleted { account_id: a, device_ids: vec!["OLD".into()] });
        assert!(v.delete_dialog().is_none() && !v.failed);
        assert!(v.devices.iter().all(|d| d.device_id != "OLD"));
        v.apply(list(&["OWN", "OLD"]));
        assert!(v.open_delete(1));
        // success: DeviceDeleted closes the dialog even if the next list is stale and still holds the device
        assert!(v.submit_delete(pw()));
        v.pending = None;
        v.apply(VerificationEvent::DeviceDeleted { account_id: a, device_ids: vec!["OLD".into()] });
        assert!(v.delete_dialog().is_none());
        v.apply(list(&["OWN", "OLD"]));
        assert!(v.open_delete(1));
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
    fn delete_inactive_state_machine() {
        use crate::inactive::InactiveThreshold::*;
        use matrix_engine::verification::DeviceDeleteError;
        const DAY: u64 = 86_400_000;
        const NOW: u64 = 1000 * DAY;
        let a = AccountId::new(1);
        let mut v = DeviceVerification::new(a);
        let pw = || zeroize::Zeroizing::new("pw".to_owned());
        let list = |items: &[(&str, Option<u64>)]| VerificationEvent::Devices {
            account_id: a,
            devices: items.iter().map(|(i, ts)| { let mut d = dev(Some(i), true, *ts, None); d.device_id = (*i).into(); d }).collect(),
        };
        v.apply(VerificationEvent::Profile { account_id: a, user_id: "@a:x".into(), display_name: None, device_id: Some("OWN".into()), avatar: None });
        // empty list: no link, no dialog
        v.apply(list(&[("NEW", Some(NOW - DAY)), ("OWN", Some(1))]));
        assert!(!v.has_inactive(NOW) && !v.open_delete_inactive(NOW));
        // idle (never «Questo dispositivo»), unknown activity counted apart
        v.apply(list(&[("OWN", Some(1)), ("M2", Some(NOW - 60 * DAY)), ("M8", Some(NOW - 240 * DAY)), ("Y2", Some(NOW - 700 * DAY)), ("UNK", None)]));
        assert!(v.has_inactive(NOW) && v.open_delete_inactive(NOW));
        let d = v.delete_dialog().unwrap();
        assert_eq!((d.threshold, d.device_ids.clone(), d.unknown_activity), (Some(SixMonths), vec!["M8".to_owned(), "Y2".to_owned()], 1));
        assert_eq!((d.counter().as_str(), d.unknown_note().as_deref()), ("2 devices to sign out", Some("1 device without last activity not included")));
        assert!(!v.open_delete_inactive(NOW) && !v.open_delete(1), "one dialog at a time");
        // threshold change refreshes the list: 1 anno → only Y2; 1 mese → three
        assert!(v.set_delete_threshold(OneYear, NOW));
        assert_eq!(v.delete_dialog().unwrap().device_ids, ["Y2"]);
        assert_eq!(v.delete_targets().len(), 1);
        assert!(v.cycle_delete_threshold(NOW), "1 anno wraps to 1 mese");
        assert_eq!((v.delete_dialog().unwrap().threshold, v.delete_dialog().unwrap().device_ids.len()), (Some(OneMonth), 3));
        // empty list: «Disconnetti» disabled
        v.apply(list(&[("OWN", Some(1)), ("M2", Some(NOW - 60 * DAY))]));
        assert!(v.set_delete_threshold(OneYear, NOW));
        assert!(v.delete_dialog().unwrap().device_ids.is_empty() && !v.can_submit_delete(true) && !v.submit_delete(pw()));
        // in progress: one command with every id; the threshold is frozen; no second submit
        assert!(v.set_delete_threshold(OneMonth, NOW));
        assert!(v.submit_delete(pw()));
        assert!(matches!(&v.pending, Some(VerificationCommand::DeleteDevice { device_ids, .. }) if device_ids == &["M2".to_owned()]));
        v.pending = None;
        assert!(!v.set_delete_threshold(OneYear, NOW) && !v.submit_delete(pw()) && v.delete_dialog().unwrap().busy);
        // error: dialog stays, button active again, list kept
        v.apply(VerificationEvent::DeviceDeleteFailed { account_id: a, device_ids: vec!["M2".into()], error: DeviceDeleteError::WrongPassword });
        let d = v.delete_dialog().unwrap();
        assert!(!d.busy && d.error == Some("Incorrect password") && d.take_refocus());
        assert!(v.can_submit_delete(true));
        // success: dialog closes, every row goes
        assert!(v.submit_delete(pw()));
        v.pending = None;
        v.apply(VerificationEvent::DeviceDeleted { account_id: a, device_ids: vec!["M2".into()] });
        assert!(v.delete_dialog().is_none() && v.devices.iter().all(|d| d.device_id != "M2"));
    }

    #[test]
    fn inactive_dialog_survives_a_refresh_that_already_dropped_some_targets() {
        // «Aggiorna dispositivi» (or another client) removes a target while the dialog is open:
        // the dialog keeps its snapshot, stays open, and the submit still carries every id.
        use crate::inactive::InactiveThreshold::*;
        const DAY: u64 = 86_400_000;
        const NOW: u64 = 1000 * DAY;
        let a = AccountId::new(1);
        let mut v = DeviceVerification::new(a);
        let list = |ids: &[&str]| VerificationEvent::Devices {
            account_id: a,
            devices: ids.iter().map(|i| { let mut d = dev(Some(i), true, Some(NOW - 400 * DAY), None); d.device_id = (*i).into(); d }).collect(),
        };
        v.apply(list(&["A", "B"]));
        assert!(v.open_delete_inactive(NOW));
        v.apply(list(&["A"]));
        let d = v.delete_dialog().expect("an idle refresh does not close the dialog");
        assert_eq!((d.threshold, d.device_ids.clone()), (Some(SixMonths), vec!["A".to_owned(), "B".to_owned()]));
        assert!(v.submit_delete(zeroize::Zeroizing::new("pw".to_owned())));
        assert!(matches!(&v.pending, Some(VerificationCommand::DeleteDevice { device_ids, .. }) if device_ids.len() == 2));
        v.pending = None;
        // busy and every target already gone from the list: the fallback closes the dialog
        v.apply(list(&["Z"]));
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
        assert_eq!(v.rename_dialog().unwrap().error, Some("Invalid name"));
        assert!(!v.rename_dialog().unwrap().busy && !v.failed);
        // retry succeeds: the fresh own device closes the dialog and carries the new name
        assert!(v.submit_rename("Portatile"));
        v.pending = None;
        v.apply(VerificationEvent::Error { account_id: a, error: VerificationError::Transport });
        assert_eq!(v.rename_dialog().unwrap().error, Some("Failed. Try again."));
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
        assert_eq!(at(THU_2330 + 60_000), "Verified · Last activity 00:30 · ABCDEF");
        assert_eq!(at(THU_2330 + 3 * DAY), "Verified · Last activity Ven 00:30 · ABCDEF");
        assert_eq!(at(THU_2330 + 30 * DAY), "Verified · Last activity 16/01/26 00:30 · ABCDEF");
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
            Some(VerificationCommand::DeleteDevice { account_id, device_ids, .. }) => assert_eq!((account_id, device_ids.as_slice()), (a, &["MINE".to_owned()][..])),
            _ => panic!("expected DeleteDevice"),
        }
        // A failure reported for another account or another device leaves this dialog untouched.
        v.apply(VerificationEvent::DeviceDeleteFailed { account_id: b, device_ids: vec!["MINE".into()], error: DeviceDeleteError::WrongPassword });
        v.apply(VerificationEvent::DeviceDeleteFailed { account_id: a, device_ids: vec!["ELSE".into()], error: DeviceDeleteError::WrongPassword });
        assert_eq!(v.delete_dialog().unwrap().error, None);
        // Every error text is a single line; empty password and a second submit are refused.
        for e in [DeviceDeleteError::WrongPassword, DeviceDeleteError::Unavailable, DeviceDeleteError::Transport, DeviceDeleteError::Rejected] {
            v.apply(VerificationEvent::DeviceDeleteFailed { account_id: a, device_ids: vec!["MINE".into()], error: e });
            let text = v.delete_dialog().unwrap().error.unwrap();
            assert!(!text.contains('\n') && !text.is_empty(), "{text:?}");
        }
        assert!(!v.can_submit_delete(false), "empty field: button disabled, text kept");
        assert!(v.can_submit_delete(true));
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
    use matrix_engine::signin::{RecoveryFailure, RecoveryState};

    fn complete(account: u64, complete: bool) -> VerificationEvent {
        VerificationEvent::KeysComplete { account_id: AccountId::new(account), complete }
    }

    #[test]
    fn accepting_the_offer_queues_one_create_and_never_when_enabled() {
        let disabled = |account: u64, disabled| VerificationEvent::RecoveryDisabled { account_id: AccountId::new(account), disabled };
        let mut v = DeviceVerification::new(AccountId::new(1));
        assert!(!v.offers_recovery_key(false), "unknown: no offer");
        v.apply(disabled(2, true));
        assert!(!v.offers_recovery_key(false), "another account is ignored");
        v.apply(disabled(1, true));
        assert!(v.offers_recovery_key(false) && !v.offers_recovery_key(true));
        v.apply(disabled(1, false));
        assert!(!v.offers_recovery_key(false), "a key appeared meanwhile");
        v.accept_offer();
        assert!(v.pending.is_none() && v.keys.is_none(), "nothing is written when the account has a key");
        v.apply(disabled(1, true));
        v.offer_asked = false;
        v.accept_offer();
        assert!(matches!(v.pending, Some(VerificationCommand::ResetRecoveryKey { .. })));
        assert!(!v.offers_recovery_key(false), "asked once per session");
    }

    #[test]
    fn existing_key_on_this_device_missing_offers_enter_never_create() {
        use crate::key_export::OfferKind::{Create, Enter};
        let a = AccountId::new(1);
        let disabled = |d| VerificationEvent::RecoveryDisabled { account_id: a, disabled: d };
        let mut v = DeviceVerification::new(a);
        v.apply(complete(1, false));
        assert_eq!(v.recovery_offer(false), None, "state unknown yet");
        v.apply(disabled(false));
        assert_eq!(v.recovery_offer(false), Some(Enter));
        assert_eq!(v.recovery_offer(true), None, "never ask again");
        assert!(!v.offers_recovery_key(false), "no Create over an existing key");
        v.apply(complete(1, true));
        assert_eq!(v.recovery_offer(false), None, "enabled and complete");
        v.apply(disabled(true));
        assert_eq!(v.recovery_offer(false), Some(Create));
        v.decline_offer();
        assert_eq!(v.recovery_offer(false), None);
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

    #[test]
    fn key_goes_into_one_command_and_failures_use_the_sign_in_wording() {
        let mut v = DeviceVerification::new(AccountId::new(1));
        assert!(!v.recover_keys(zeroize::Zeroizing::new(String::new())));
        assert_eq!(v.key_recovery(), Some(RecoveryState::Failed(RecoveryFailure::InvalidKey)));
        assert_eq!(v.key_recovery_message(), Some("Wrong key. Check it and try again."));
        assert!(v.recover_keys(zeroize::Zeroizing::new("EsTu key".into())));
        assert!(matches!(v.pending, Some(VerificationCommand::RecoverKeys { .. })));
        assert!(!v.recover_keys(zeroize::Zeroizing::new("again".into())), "one attempt at a time");
        v.apply(VerificationEvent::KeysRecovery {
            account_id: AccountId::new(1),
            state: RecoveryState::Failed(RecoveryFailure::Network),
        });
        assert!(v.key_recovery_message().is_some_and(|m| m.starts_with("Server unreachable")));
        v.apply(complete(1, true));
        assert_eq!(v.key_recovery(), None);
    }

    #[test]
    fn export_goes_into_one_command_and_failure_keeps_the_secrets_for_a_retry() {
        use crate::key_export::{KeysKind, KeysPage};
        use matrix_engine::key_export::KeyExportError;
        let a = AccountId::new(1);
        let z = |s: &str| zeroize::Zeroizing::new(s.to_owned());
        let mut v = DeviceVerification::new(a);
        assert!(!v.export_keys("/tmp/k".into(), &z("long enough pass"), None), "no page open");
        v.keys = Some(KeysPage::new(KeysKind::Export));
        assert!(v.export_keys("/tmp/k".into(), &z("long enough pass"), None));
        assert!(matches!(v.pending, Some(VerificationCommand::ExportRoomKeys { .. })));
        assert!(v.keys.as_ref().unwrap().busy);
        v.pending = None;
        v.apply(VerificationEvent::KeysExported { account_id: a, result: Err(KeyExportError::NoKeys) });
        let page = v.keys.as_ref().unwrap();
        assert!(!page.busy && page.refill && page.held.is_some(), "the page stays open and retryable");
        assert_eq!(page.line.as_ref().map(|l| (l.0.as_str(), l.1)), Some(("No keys to export on this device", true)));
        assert!(v.export_keys("/tmp/k".into(), &z("long enough pass"), None));
        v.pending = None;
        v.apply(VerificationEvent::KeysExported { account_id: a, result: Ok(3) });
        let page = v.keys.as_ref().unwrap();
        assert!(page.held.is_none(), "success wipes the kept copy");
        assert_eq!(page.line.as_ref().map(|l| l.0.as_str()), Some("3 keys exported"));
    }

    #[test]
    fn reset_shows_the_new_key_once_and_only_confirmation_closes_the_page() {
        use crate::key_export::{KeysKind, KeysPage};
        use matrix_engine::recovery::{RecoveryError, RecoverySecret};
        let a = AccountId::new(1);
        let mut v = DeviceVerification::new(a);
        v.keys = Some(KeysPage::new(KeysKind::Reset));
        assert!(v.reset_recovery(None));
        assert!(matches!(v.pending, Some(VerificationCommand::ResetRecoveryKey { .. })));
        assert!(!v.keys.as_ref().unwrap().closable(), "no way out while the new key is being made");
        v.pending = None;
        v.apply(VerificationEvent::RecoveryKeyReset { account_id: a, result: Err(RecoveryError::IdentityRequired) });
        let page = v.keys.as_ref().unwrap();
        assert!(page.closable() && page.line.as_ref().is_some_and(|l| l.1));
        assert!(v.reset_recovery(None));
        v.pending = None;
        v.apply(VerificationEvent::RecoveryKeyReset { account_id: a, result: Ok(RecoverySecret::fixture("EsTx abcd")) });
        let page = v.keys.as_ref().unwrap();
        assert!(page.shown() && !page.closable());
        assert!(!v.reset_recovery(None), "no second reset while the key is shown");
        v.confirm_key_saved();
        assert!(v.keys.is_none());
    }
}
