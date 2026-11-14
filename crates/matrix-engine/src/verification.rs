//! SDK SAS verification. Backend runtime only; never log snapshots or SAS values.
use matrix_domain::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationError {
    InvalidTarget,
    LoggedOut,
    UnknownDevice,
    Transport,
    InvalidState,
    UnknownFlow,
    WrongScope,
    Busy,
    TooManyDevices,
    InvalidName,
}

/// Why a remote device could not be signed out. One-line user messages live in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceDeleteError {
    /// This session: use logout instead.
    CurrentDevice,
    InvalidTarget,
    LoggedOut,
    /// Server offers no m.login.password flow (SSO/OIDC): "Non disponibile con questo account".
    Unavailable,
    WrongPassword,
    Rejected,
    Transport,
}

/// Password typed by the user for one call. Wiped on drop; Debug never prints it.
// The inner value is read only by the SDK adapter.
#[cfg_attr(not(feature = "sdk"), allow(dead_code))]
pub struct DevicePassword(zeroize::Zeroizing<String>);
impl DevicePassword {
    pub fn new(password: String) -> Self {
        Self(zeroize::Zeroizing::new(password))
    }
}
impl std::fmt::Debug for DevicePassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DevicePassword(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationAction {
    Accept,
    StartSas,
    AcceptSas,
    Confirm,
    Cancel,
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationState {
    Waiting,
    Requested,
    Ready,
    Negotiating,
    Compare { decimals: (u16, u16, u16) },
    Confirmed,
    Verified,
    Cancelled,
    Mismatched,
    Failed,
}

/// Identifiers and comparison values are private UI data, not diagnostics.
#[derive(Clone, PartialEq, Eq)]
pub struct VerificationSnapshot {
    pub account_id: AccountId,
    pub user_id: String,
    pub device_id: String,
    pub flow_id: String,
    pub state: VerificationState,
    /// SDK device trust, distinct from completion of this particular flow.
    pub device_verified: bool,
    /// Seven (symbol, SDK description) pairs while comparing; `None` if the peer negotiated only
    /// decimals. Kept out of `VerificationState` so that stays `Copy`.
    pub sas_emoji: Option<Vec<(String, String)>>,
}

#[cfg(feature = "sdk")]
#[path = "verification_sdk.rs"]
mod sdk;
#[cfg(feature = "sdk")]
pub use sdk::VerificationFlow;

#[derive(Clone, PartialEq, Eq)]
pub struct VerificationTarget {
    pub account_id: AccountId,
    pub user_id: String,
    pub flow_id: String,
}
impl VerificationSnapshot {
    pub fn target(&self) -> VerificationTarget {
        VerificationTarget {
            account_id: self.account_id,
            user_id: self.user_id.clone(),
            flow_id: self.flow_id.clone(),
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct VerificationDevice {
    pub user_id: String,
    pub device_id: String,
    pub verified: bool,
    pub display_name: Option<String>,
    /// Milliseconds since the Unix epoch; `None` if the server does not report it.
    pub last_seen_ts: Option<u64>,
    pub last_seen_ip: Option<String>,
}
/// Most recently active first; devices without a timestamp last; ties by id.
pub fn sort_devices(devices: &mut [VerificationDevice]) {
    devices.sort_by(|a, b| {
        b.last_seen_ts
            .cmp(&a.last_seen_ts)
            .then_with(|| a.device_id.cmp(&b.device_id))
    });
}
pub enum VerificationCommand {
    List {
        account_id: AccountId,
    },
    Begin {
        account_id: AccountId,
        user_id: String,
        device_id: String,
    },
    /// «Verifica utente»: request verification of the whole user (their identity), not a device.
    BeginUser {
        account_id: AccountId,
        user_id: String,
    },
    Act {
        target: VerificationTarget,
        action: VerificationAction,
    },
    /// Release the flow once its popup is closed.
    Release { target: VerificationTarget },
    /// Rename the session's own device (`Client::rename_device`, PUT /devices/{id}).
    RenameOwnDevice {
        account_id: AccountId,
        display_name: String,
    },
    /// «Recupera chiavi» after sign-in: the recovery key is consumed by this one call. The answer is
    /// `KeysRecovery` events (same states and failures as the sign-in step).
    #[cfg(feature = "sdk")]
    RecoverKeys {
        account_id: AccountId,
        key: crate::signin::Secret,
    },
    /// Sign out other devices in one request (UIA password). On success a fresh `Devices` event follows.
    DeleteDevice {
        account_id: AccountId,
        device_ids: Vec<String>,
        password: DevicePassword,
    },
}
pub enum VerificationEvent {
    /// Own identity as the SDK reports it (display name is the account's profile name, if set).
    Profile {
        account_id: AccountId,
        user_id: String,
        display_name: Option<String>,
        /// This session's own device (excluded from `Devices`), from `client.device_id()`.
        device_id: Option<String>,
        /// `mxc` of the profile picture, if the server has one.
        avatar: Option<String>,
    },
    Devices {
        account_id: AccountId,
        devices: Vec<VerificationDevice>,
    },
    /// This session's device (never in `Devices`): name, state and last activity.
    OwnDevice {
        account_id: AccountId,
        device: VerificationDevice,
    },
    Snapshot(VerificationSnapshot),
    /// Progress of a `RecoverKeys` command.
    #[cfg(feature = "sdk")]
    KeysRecovery {
        account_id: AccountId,
        state: crate::signin::RecoveryState,
    },
    /// Whether key recovery is complete (see `recovery::keys_recovered`); sent at start, on change and after a recovery.
    KeysComplete {
        account_id: AccountId,
        complete: bool,
    },
    Error {
        account_id: AccountId,
        error: VerificationError,
    },
    DeviceDeleteFailed {
        account_id: AccountId,
        device_ids: Vec<String>,
        error: DeviceDeleteError,
    },
    /// Sign-out succeeded but the list could not be re-read: the UI drops the row itself.
    DeviceDeleted {
        account_id: AccountId,
        device_ids: Vec<String>,
    },
}
#[cfg(feature = "sdk")]
pub(crate) use sdk::run_service;

#[cfg(test)]
mod sort_tests {
    use super::*;
    fn d(id: &str, ts: Option<u64>) -> VerificationDevice {
        VerificationDevice {
            user_id: "@a:x".into(),
            device_id: id.into(),
            verified: false,
            display_name: None,
            last_seen_ts: ts,
            last_seen_ip: None,
        }
    }
    #[test]
    fn newest_first_missing_last_ties_by_id() {
        let mut v = vec![d("Z", None), d("B", Some(5)), d("A", None), d("C", Some(9)), d("A2", Some(5))];
        sort_devices(&mut v);
        let ids: Vec<_> = v.iter().map(|x| x.device_id.as_str()).collect();
        assert_eq!(ids, ["C", "A2", "B", "A", "Z"]);
    }
}
