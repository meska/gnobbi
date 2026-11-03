use super::*;
use crate::auth::AccountSession;
use matrix_sdk::{
    Client,
    encryption::verification::{
        SasState, SasVerification, VerificationRequest, VerificationRequestState,
    },
};

/// One exact device and transaction. No manual trust override or key export.
pub struct VerificationFlow {
    client: Client,
    account_id: AccountId,
    user_id: matrix_sdk::ruma::OwnedUserId,
    device_id: matrix_sdk::ruma::OwnedDeviceId,
    request: VerificationRequest,
    failed: bool,
}

impl AccountSession {
    /// Look up a received request by exact sender and SDK transaction; never auto-accept.
    pub async fn receive_device_verification(
        &self,
        user_id: &str,
        flow_id: &str,
    ) -> Result<VerificationFlow, VerificationError> {
        let user_id = matrix_sdk::ruma::UserId::parse(user_id)
            .map_err(|_| VerificationError::InvalidTarget)?;
        let client = self.client.as_ref().ok_or(VerificationError::LoggedOut)?;
        let request = client
            .encryption()
            .get_verification_request(&user_id, flow_id)
            .await
            .ok_or(VerificationError::UnknownFlow)?;
        let device_id = match request.state() {
            VerificationRequestState::Requested {
                other_device_data, ..
            } => other_device_data.device_id().to_owned(),
            _ => return Err(VerificationError::InvalidState),
        };
        Ok(VerificationFlow {
            client: client.clone(),
            account_id: self.account_id,
            user_id,
            device_id,
            request,
            failed: false,
        })
    }

    /// Request only SAS v1 from an explicitly selected, already tracked device.
    pub async fn request_device_verification(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<VerificationFlow, VerificationError> {
        let user_id = matrix_sdk::ruma::UserId::parse(user_id)
            .map_err(|_| VerificationError::InvalidTarget)?;
        if device_id.is_empty() || device_id.len() > 255 || device_id.chars().any(char::is_control)
        {
            return Err(VerificationError::InvalidTarget);
        }
        let device_id: matrix_sdk::ruma::OwnedDeviceId = device_id.into();
        let client = self.client.as_ref().ok_or(VerificationError::LoggedOut)?;
        let device = client
            .encryption()
            .get_device(&user_id, &device_id)
            .await
            .map_err(|_| VerificationError::Transport)?
            .ok_or(VerificationError::UnknownDevice)?;
        let request = device
            .request_verification_with_methods(vec![
                matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
            ])
            .await
            .map_err(|_| VerificationError::Transport)?;
        Ok(VerificationFlow {
            client: client.clone(),
            account_id: self.account_id,
            user_id,
            device_id,
            request,
            failed: false,
        })
    }
}

fn cancellation_state(
    code: &matrix_sdk::ruma::events::key::verification::cancel::CancelCode,
) -> VerificationState {
    if *code == matrix_sdk::ruma::events::key::verification::cancel::CancelCode::MismatchedSas {
        VerificationState::Mismatched
    } else {
        VerificationState::Cancelled
    }
}

impl VerificationFlow {
    async fn sas(&self) -> Option<SasVerification> {
        self.client
            .encryption()
            .get_verification(&self.user_id, self.request.flow_id())
            .await
            .and_then(|verification| verification.sas())
    }

    /// Explicit user decisions only. Confirm is refused until SAS is displayable.
    pub async fn perform(&mut self, action: VerificationAction) -> Result<(), VerificationError> {
        use VerificationAction::*;
        if self.failed {
            return Err(VerificationError::InvalidState);
        }
        let state = self.snapshot().await?.state;
        let result = match action {
            Mismatch if matches!(state, VerificationState::Compare { .. }) => {
                self.sas()
                    .await
                    .ok_or(VerificationError::InvalidState)?
                    .mismatch()
                    .await
            }
            Cancel
                if !matches!(
                    state,
                    VerificationState::Verified | VerificationState::Cancelled
                ) =>
            {
                self.request.cancel().await
            }
            Accept if state == VerificationState::Requested => {
                self.request
                    .accept_with_methods(vec![
                        matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
                    ])
                    .await
            }
            StartSas if state == VerificationState::Ready => match self.request.start_sas().await {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => return Err(VerificationError::InvalidState),
                Err(_) => {
                    self.failed = true;
                    return Err(VerificationError::Transport);
                }
            },
            AcceptSas => {
                let sas = self.sas().await.ok_or(VerificationError::InvalidState)?;
                if !matches!(sas.state(), SasState::Started { .. }) {
                    return Err(VerificationError::InvalidState);
                }
                sas.accept().await
            }
            Confirm if matches!(state, VerificationState::Compare { .. }) => {
                self.sas()
                    .await
                    .ok_or(VerificationError::InvalidState)?
                    .confirm()
                    .await
            }
            _ => return Err(VerificationError::InvalidState),
        };
        if result.is_err() {
            self.failed = true;
        }
        result.map_err(|_| VerificationError::Transport)
    }

    pub async fn snapshot(&self) -> Result<VerificationSnapshot, VerificationError> {
        let device = self
            .client
            .encryption()
            .get_device(&self.user_id, &self.device_id)
            .await
            .map_err(|_| VerificationError::Transport)?
            .ok_or(VerificationError::UnknownDevice)?;
        let device_verified = device.is_verified();
        let state = if self.failed {
            VerificationState::Failed
        } else if let Some(sas) = self.sas().await {
            match sas.state() {
                SasState::KeysExchanged { decimals, .. } => VerificationState::Compare { decimals },
                SasState::Confirmed => VerificationState::Confirmed,
                SasState::Done { .. } if device_verified => VerificationState::Verified,
                SasState::Cancelled(info) => cancellation_state(info.cancel_code()),
                _ => VerificationState::Negotiating,
            }
        } else {
            match self.request.state() {
                VerificationRequestState::Requested { .. } => VerificationState::Requested,
                VerificationRequestState::Ready { .. } => VerificationState::Ready,
                VerificationRequestState::Done if device_verified => VerificationState::Verified,
                VerificationRequestState::Cancelled(info) => cancellation_state(info.cancel_code()),
                _ => VerificationState::Waiting,
            }
        };
        Ok(VerificationSnapshot {
            account_id: self.account_id,
            user_id: self.user_id.to_string(),
            device_id: self.device_id.to_string(),
            flow_id: self.request.flow_id().to_owned(),
            state,
            device_verified,
        })
    }
}

/// Dedicated backend task. Sync remains independent even under UI backpressure.
pub(crate) async fn run_service(
    client: Client,
    account_id: AccountId,
    mut commands: crate::Receiver<VerificationCommand>,
    mut events: crate::Sender<VerificationEvent>,
) {
    let session = AccountSession {
        account_id,
        client: Some(client.clone()),
        store: None,
    };
    let mut active: Option<VerificationFlow> = None;
    let mut previous = None;
    let mut ticks = tokio::time::interval(std::time::Duration::from_millis(200));
    // Profile name and picture are re-read every minute: a change made elsewhere shows up without restart.
    let mut shown = None;
    let mut profile_ticks = tokio::time::interval(PROFILE_REFRESH);
    loop {
        let result = tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match command {
                    VerificationCommand::List { account_id: requested } if requested == account_id => {
                        match device_list(&client).await {
                            Ok(devices) => {
                                if events.send(VerificationEvent::Devices { account_id, devices }).await.is_err() { break; }
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    }
                    VerificationCommand::Begin { account_id: requested, user_id, device_id } if requested == account_id => {
                        if let Some(flow) = &active {
                            let state = flow.snapshot().await.map(|s| s.state);
                            if !matches!(state, Ok(VerificationState::Verified | VerificationState::Cancelled | VerificationState::Mismatched | VerificationState::Failed)) {
                                if events.send(VerificationEvent::Error { account_id, error: VerificationError::Busy }).await.is_err() { break; }
                                continue;
                            }
                        }
                        match session.request_device_verification(&user_id, &device_id).await {
                            Ok(flow) => { active = Some(flow); previous = None; Ok(()) }
                            Err(error) => Err(error),
                        }
                    }
                    VerificationCommand::Act { target, action } if target.account_id == account_id => {
                        match &mut active {
                            Some(flow) if target.user_id == flow.user_id.as_str() && target.flow_id == flow.request.flow_id() => flow.perform(action).await,
                            _ => Err(VerificationError::UnknownFlow),
                        }
                    }
                    VerificationCommand::RenameOwnDevice { account_id: requested, display_name } if requested == account_id => {
                        rename_own_device(&client, &display_name).await
                    }
                    _ => Err(VerificationError::WrongScope),
                }
            }
            _ = profile_ticks.tick() => {
                match own_profile(&client).await {
                    Some(profile) if shown.as_ref() != Some(&profile) => {
                        shown = Some(profile.clone());
                        let (user_id, display_name, avatar) = profile;
                        let device_id = client.device_id().map(|d| d.to_string());
                        if events.send(VerificationEvent::Profile { account_id, user_id, display_name, device_id, avatar }).await.is_err() { break; }
                    }
                    _ => (),
                }
                Ok(())
            }
            _ = ticks.tick() => {
                if let Some(flow) = &mut active {
                    match flow.snapshot().await {
                        Ok(snapshot) if snapshot.state == VerificationState::Ready => flow.perform(VerificationAction::StartSas).await,
                        Ok(_) => Ok(()),
                        Err(error) => Err(error),
                    }
                } else { Ok(()) }
            }
        };
        if let Err(error) = result
            && events
                .send(VerificationEvent::Error { account_id, error })
                .await
                .is_err()
        {
            break;
        }
        if let Some(flow) = &active {
            match flow.snapshot().await {
                Ok(snapshot) if previous.as_ref() != Some(&snapshot) => {
                    previous = Some(snapshot.clone());
                    if events
                        .send(VerificationEvent::Snapshot(snapshot))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                _ => (),
            }
        }
    }
}

const PROFILE_REFRESH: std::time::Duration = std::time::Duration::from_secs(60);

/// Own id, name and picture as the SDK reports them; a failed read keeps what is already shown.
async fn own_profile(client: &Client) -> Option<(String, Option<String>, Option<String>)> {
    let user_id = client.user_id()?.to_string();
    let account = client.account();
    let display_name = account.get_display_name().await.ok()?;
    let avatar = account
        .get_avatar_url()
        .await
        .ok()?
        .map(|uri| uri.to_string());
    Some((user_id, display_name, avatar))
}

async fn rename_own_device(client: &Client, name: &str) -> Result<(), VerificationError> {
    let name = valid_device_name(name)?;
    let id = client.device_id().ok_or(VerificationError::LoggedOut)?;
    client.rename_device(id, name).await.map_err(|_| VerificationError::Transport)?;
    Ok(())
}

fn valid_device_name(name: &str) -> Result<&str, VerificationError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
        return Err(VerificationError::InvalidName);
    }
    Ok(name)
}

#[cfg(test)]
mod rename_tests {
    use super::*;
    #[test]
    fn device_name_validation() {
        assert_eq!(valid_device_name("  Gnobbi (macOS) ").unwrap(), "Gnobbi (macOS)");
        assert!(valid_device_name("  ").is_err());
        assert!(valid_device_name("a\nb").is_err());
        assert!(valid_device_name(&"x".repeat(129)).is_err());
    }
}

async fn device_list(client: &Client) -> Result<Vec<VerificationDevice>, VerificationError> {
    let user_id = client.user_id().ok_or(VerificationError::LoggedOut)?;
    let devices = client
        .encryption()
        .get_user_devices(user_id)
        .await
        .map_err(|_| VerificationError::Transport)?;
    // Server-side metadata; failure here must not hide the device list.
    let meta = client.devices().await.map(|r| r.devices).unwrap_or_default();
    let mut result = Vec::new();
    for device in devices.devices() {
        if Some(device.device_id()) == client.device_id() {
            continue;
        }
        if result.len() == 128 {
            return Err(VerificationError::TooManyDevices);
        }
        result.push(VerificationDevice {
            user_id: user_id.to_string(),
            device_id: device.device_id().to_string(),
            verified: device.is_verified(),
            display_name: meta
                .iter()
                .find(|m| m.device_id == device.device_id())
                .and_then(|m| m.display_name.clone())
                .filter(|n| !n.trim().is_empty()),
            last_seen_ts: meta
                .iter()
                .find(|m| m.device_id == device.device_id())
                .and_then(|m| m.last_seen_ts)
                .map(|t| t.0.into()),
            last_seen_ip: meta
                .iter()
                .find(|m| m.device_id == device.device_id())
                .and_then(|m| m.last_seen_ip.clone())
                .filter(|i| !i.trim().is_empty()),
        });
    }
    sort_devices(&mut result);
    Ok(result)
}
