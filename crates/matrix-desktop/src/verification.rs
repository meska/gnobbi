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

pub struct DeviceVerification {
    delete: Option<DeleteDialog>,
    account: AccountId,
    snapshot: Option<VerificationSnapshot>,
    pending: Option<VerificationCommand>,
    deciding: bool,
    devices: Vec<matrix_engine::verification::VerificationDevice>,
    failed: bool,
    profile: Option<(String, Option<String>)>,
    own_device: Option<String>,
    own: Option<matrix_engine::verification::VerificationDevice>,
    profile_avatar: Option<String>,
}
impl DeviceVerification {
    pub fn new(account: AccountId) -> Self {
        Self {
            delete: None,
            account,
            snapshot: None,
            pending: None,
            deciding: false,
            devices: Vec::new(),
            failed: false,
            profile: None,
            own_device: None,
            own: None,
            profile_avatar: None,
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
        if self.own_device.as_deref() == Some(&d.device_id) || self.delete.is_some() {
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
    pub fn can_begin(&self) -> bool {
        self.pending.is_none()
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
        self.snapshot = None;
        self.failed = false;
        self.deciding = true;
        true
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
                self.snapshot = Some(snapshot);
                self.deciding = false;
                self.failed = false;
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
                self.own = Some(device)
            }
            VerificationEvent::Error { account_id, .. } if account_id == self.account => {
                self.failed = true;
                self.pending = None;
                self.deciding = false;
                self.delete = None;
            }
            _ => (),
        }
    }
    pub fn comparison(&self) -> Option<(u16, u16, u16)> {
        if self.failed {
            return None;
        }
        match self.snapshot.as_ref()?.state {
            VerificationState::Compare { decimals } => Some(decimals),
            _ => None,
        }
    }
    pub fn verified(&self) -> bool {
        !self.failed
            && self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.state == VerificationState::Verified && s.device_verified)
    }
    pub fn status_label(&self) -> &'static str {
        if self.failed {
            return "Verifica non riuscita";
        }
        if self.snapshot.is_none() && self.deciding {
            return "Richiesta in corso";
        }
        use VerificationState::*;
        match self.snapshot.as_ref().map(|s| s.state) {
            None => "Seleziona un dispositivo",
            Some(Waiting) => "Richiesta inviata · in attesa",
            Some(Requested) => "Richiesta ricevuta · accetta solo se attesa",
            Some(Ready | Negotiating) => "Negoziazione SAS in corso",
            Some(Compare { .. }) => "Confronta i tre numeri sull’altro dispositivo",
            Some(Confirmed) => "Confermato qui · in attesa del dispositivo",
            Some(Verified) => "Verifica completata",
            Some(Cancelled) => "Verifica annullata",
            Some(Mismatched) => "Numeri diversi · verifica rifiutata",
            Some(Failed) => "Verifica non riuscita",
        }
    }
    pub fn trust_label(&self) -> &'static str {
        if self.snapshot.as_ref().is_some_and(|s| s.device_verified) {
            "Dispositivo verificato"
        } else {
            "Dispositivo non verificato"
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
            VerificationAction::Cancel => {
                !matches!(snapshot.state, Verified | Cancelled | Mismatched | Failed)
            }
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
    pub fn flush(&mut self, sender: &mut matrix_engine::Sender<VerificationCommand>) {
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

#[cfg(test)]
mod profile_tests {
    use super::*;

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
