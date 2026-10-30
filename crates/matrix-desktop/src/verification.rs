//! UI-only projection; SAS and device identities never enter diagnostics.
use matrix_domain::AccountId;
use matrix_engine::verification::{
    VerificationAction, VerificationCommand, VerificationEvent, VerificationSnapshot,
    VerificationState,
};

pub struct DeviceVerification {
    account: AccountId,
    snapshot: Option<VerificationSnapshot>,
    pending: Option<VerificationCommand>,
    deciding: bool,
    devices: Vec<matrix_engine::verification::VerificationDevice>,
    failed: bool,
    profile: Option<(String, Option<String>)>,
    own_device: Option<String>,
}
impl DeviceVerification {
    pub fn new(account: AccountId) -> Self {
        Self {
            account,
            snapshot: None,
            pending: None,
            deciding: false,
            devices: Vec::new(),
            failed: false,
            profile: None,
            own_device: None,
        }
    }
    /// (user id, display name) from the SDK; None until the first sync starts.
    pub fn profile(&self) -> Option<&(String, Option<String>)> {
        self.profile.as_ref()
    }
    /// This session's device ID (not in `devices()`); None until the profile arrives.
    pub fn own_device(&self) -> Option<&str> {
        self.own_device.as_deref()
    }
    pub fn devices(&self) -> &[matrix_engine::verification::VerificationDevice] {
        &self.devices
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
            } if account_id == self.account => {
                self.profile = Some((user_id, display_name));
                self.own_device = device_id;
            }
            VerificationEvent::Snapshot(snapshot) if snapshot.account_id == self.account => {
                self.snapshot = Some(snapshot);
                self.deciding = false;
                self.failed = false;
            }
            VerificationEvent::Devices {
                account_id,
                devices,
            } if account_id == self.account => self.devices = devices,
            VerificationEvent::Error { account_id, .. } if account_id == self.account => {
                self.failed = true;
                self.pending = None;
                self.deciding = false;
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
        };
        v.apply(ev(2, Some("Altro")));
        assert!(v.profile().is_none());
        v.apply(ev(1, Some("Ada")));
        assert_eq!(
            v.profile(),
            Some(&("@a:x".to_string(), Some("Ada".to_string())))
        );
        assert_eq!(v.own_device(), Some("OWNDEV"));
    }
}
