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
    Act {
        target: VerificationTarget,
        action: VerificationAction,
    },
}
pub enum VerificationEvent {
    /// Own identity as the SDK reports it (display name is the account's profile name, if set).
    Profile {
        account_id: AccountId,
        user_id: String,
        display_name: Option<String>,
    },
    Devices {
        account_id: AccountId,
        devices: Vec<VerificationDevice>,
    },
    Snapshot(VerificationSnapshot),
    Error {
        account_id: AccountId,
        error: VerificationError,
    },
}
#[cfg(feature = "sdk")]
pub(crate) use sdk::run_service;
