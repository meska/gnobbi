use super::*;
use crate::auth::AccountSession;
use matrix_sdk::{
    Client,
    encryption::verification::{
        SasState, SasVerification, VerificationRequest, VerificationRequestState,
    },
};

/// One exact transaction (and device, once known). No manual trust override or key export.
pub struct VerificationFlow {
    client: Client,
    account_id: AccountId,
    user_id: matrix_sdk::ruma::OwnedUserId,
    /// `None` for a whole-user request until the SDK reports the answering device.
    device_id: Option<matrix_sdk::ruma::OwnedDeviceId>,
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
            device_id: Some(device_id),
            request,
            failed: false,
        })
    }

    /// Sign out another device. First call without auth yields the UIA session; the password
    /// (always typed by the user, never the vault one) is sent only if the server offers it.
    pub async fn delete_device(
        &self,
        device_id: &str,
        password: DevicePassword,
    ) -> Result<(), DeviceDeleteError> {
        self.delete_devices(&[device_id.to_owned()], password).await
    }

    /// Same, for several devices in ONE request with ONE authentication (all or nothing).
    pub async fn delete_devices(
        &self,
        device_ids: &[String],
        password: DevicePassword,
    ) -> Result<(), DeviceDeleteError> {
        use matrix_sdk::ruma::api::client::uiaa::{AuthData, AuthType, MatrixUserIdentifier, Password, UserIdentifier};
        let client = self.client.as_ref().ok_or(DeviceDeleteError::LoggedOut)?;
        let user_id = client.user_id().ok_or(DeviceDeleteError::LoggedOut)?;
        if device_ids.is_empty()
            || device_ids.iter().any(|id| id.is_empty() || id.len() > 255 || id.chars().any(char::is_control))
        {
            return Err(DeviceDeleteError::InvalidTarget);
        }
        let devices: Vec<matrix_sdk::ruma::OwnedDeviceId> = device_ids.iter().map(|id| id.as_str().into()).collect();
        if devices.iter().any(|d| Some(&**d) == client.device_id()) {
            return Err(DeviceDeleteError::CurrentDevice);
        }
        let info = match client.delete_devices(&devices, None).await {
            Ok(_) => return Ok(()),
            Err(error) => match error.as_uiaa_response() {
                Some(info) => info.clone(),
                None => return Err(map_delete_error(&error)),
            },
        };
        if !info
            .flows
            .iter()
            .any(|flow| flow.stages == [AuthType::Password])
        {
            return Err(DeviceDeleteError::Unavailable);
        }
        let mut auth = Password::new(
            UserIdentifier::Matrix(MatrixUserIdentifier::new(user_id.to_string())),
            password.0.to_string(),
        );
        auth.session = info.session;
        client
            .delete_devices(&devices, Some(AuthData::Password(auth)))
            .await
            .map(|_| ())
            .map_err(|error| map_delete_error(&error))
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
            device_id: Some(device_id),
            request,
            failed: false,
        })
    }

    /// «Verifica utente»: SAS-only request to the user's cross-signing identity.
    pub async fn request_user_verification(
        &self,
        user_id: &str,
    ) -> Result<VerificationFlow, VerificationError> {
        let user_id = matrix_sdk::ruma::UserId::parse(user_id.trim())
            .map_err(|_| VerificationError::InvalidTarget)?;
        let client = self.client.as_ref().ok_or(VerificationError::LoggedOut)?;
        let identity = crate::operations::user_identity(client, &user_id)
            .await
            .ok_or(VerificationError::UnknownDevice)?;
        let request = identity
            .request_verification_with_methods(vec![
                matrix_sdk::ruma::events::key::verification::VerificationMethod::SasV1,
            ])
            .await
            .map_err(|_| VerificationError::Transport)?;
        Ok(VerificationFlow {
            client: client.clone(),
            account_id: self.account_id,
            user_id,
            device_id: None,
            request,
            failed: false,
        })
    }
}

fn map_delete_error(error: &matrix_sdk::HttpError) -> DeviceDeleteError {
    use matrix_sdk::ruma::api::error::ErrorKind;
    // A wrong password comes back as a new UIA challenge or M_FORBIDDEN.
    if error.as_uiaa_response().is_some()
        || matches!(error.client_api_error_kind(), Some(ErrorKind::Forbidden))
    {
        DeviceDeleteError::WrongPassword
    } else if matches!(error.client_api_error_kind(), Some(ErrorKind::Unrecognized)) {
        // MAS/OIDC accounts: the homeserver has no delete_devices endpoint (404 M_UNRECOGNIZED).
        DeviceDeleteError::Unavailable
    } else if error.as_client_api_error().is_some() {
        DeviceDeleteError::Rejected
    } else {
        DeviceDeleteError::Transport
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

    /// Automatic protocol steps only (never a trust decision): the side that asked starts SAS when
    /// the request is Ready; the side that received accepts the SAS the peer started. The user's
    /// own `Accept` of the request stays explicit.
    async fn advance(&mut self) -> Result<(), VerificationError> {
        if self.failed {
            return Ok(());
        }
        if self.request.we_started() {
            if self.snapshot().await?.state == VerificationState::Ready {
                return self.perform(VerificationAction::StartSas).await;
            }
        } else if self.sas().await.is_some_and(|sas| matches!(sas.state(), SasState::Started { .. })) {
            return self.perform(VerificationAction::AcceptSas).await;
        }
        Ok(())
    }

    pub async fn snapshot(&self) -> Result<VerificationSnapshot, VerificationError> {
        let sas = self.sas().await;
        // A whole-user request learns its device from the SDK once the peer answers.
        let device_id = self.device_id.clone().or_else(|| match self.request.state() {
            VerificationRequestState::Requested { other_device_data, .. }
            | VerificationRequestState::Ready { other_device_data, .. } => Some(other_device_data.device_id().to_owned()),
            _ => sas.as_ref().map(|sas| sas.other_device().device_id().to_owned()),
        });
        let device_verified = match &device_id {
            Some(id) => self
                .client
                .encryption()
                .get_device(&self.user_id, id)
                .await
                .map_err(|_| VerificationError::Transport)?
                .ok_or(VerificationError::UnknownDevice)?
                .is_verified(),
            None => self
                .client
                .encryption()
                .get_user_identity(&self.user_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|identity| identity.is_verified()),
        };
        let mut sas_emoji = None;
        let state = if self.failed {
            VerificationState::Failed
        } else if let Some(sas) = sas {
            match sas.state() {
                SasState::KeysExchanged { decimals, emojis } => {
                    sas_emoji = emojis.map(|e| {
                        e.emojis.iter().map(|e| (e.symbol.to_owned(), e.description.to_owned())).collect()
                    });
                    VerificationState::Compare { decimals }
                }
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
            device_id: device_id.map(|id| id.to_string()).unwrap_or_default(),
            flow_id: self.request.flow_id().to_owned(),
            state,
            device_verified,
            sas_emoji,
        })
    }
}

/// Dedicated backend task. Sync remains independent even under UI backpressure.
pub(crate) async fn run_service(
    client: Client,
    account_id: AccountId,
    mut commands: crate::Receiver<VerificationCommand>,
    mut events: crate::Sender<VerificationEvent>,
    // The account directory: scratch file of the key export and local-phrase check (legacy profiles).
    account_dir: Option<std::path::PathBuf>,
) {
    let mut session = AccountSession {
        account_id,
        client: Some(client.clone()),
        store: None,
    };
    let mut active: Option<VerificationFlow> = None;
    let mut previous = None;
    // Concurrent requests refused locally whose cancel did not reach the peer yet: re-sent, never reopened.
    let mut unsent_rejects: Vec<(VerificationFlow, u32)> = Vec::new();
    // Requests from other people/devices: the SDK stores them, the handlers only tell us which one.
    let (incoming_tx, mut incoming) = tokio::sync::mpsc::unbounded_channel::<(matrix_sdk::ruma::OwnedUserId, String)>();
    let to_device = incoming_tx.clone();
    let to_device_handler = client.add_event_handler(
        move |event: matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent| {
            let _ = to_device.send((event.sender, event.content.transaction_id.to_string()));
            std::future::ready(())
        },
    );
    let own_id = client.user_id().map(ToOwned::to_owned);
    let room_handler = client.add_event_handler(
        move |event: matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent| {
            use matrix_sdk::ruma::events::room::message::MessageType;
            if matches!(event.content.msgtype, MessageType::VerificationRequest(_)) && Some(&event.sender) != own_id.as_ref() {
                let _ = incoming_tx.send((event.sender, event.event_id.to_string()));
            }
            std::future::ready(())
        },
    );
    let mut ticks = tokio::time::interval(std::time::Duration::from_millis(200));
    // Profile name and picture are re-read every minute: a change made elsewhere shows up without restart.
    let mut shown = None;
    let mut shown_complete = None;
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
                                if let Some(device) = own_device(&client).await
                                    && events.send(VerificationEvent::OwnDevice { account_id, device }).await.is_err() { break; }
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    }
                    VerificationCommand::Begin { account_id: requested, user_id, device_id } if requested == account_id => {
                        if active.is_some() {
                            if events.send(VerificationEvent::Error { account_id, error: VerificationError::Busy }).await.is_err() { break; }
                            continue;
                        }
                        match session.request_device_verification(&user_id, &device_id).await {
                            Ok(flow) => { active = Some(flow); previous = None; Ok(()) }
                            Err(error) => Err(error),
                        }
                    }
                    VerificationCommand::BeginUser { account_id: requested, user_id } if requested == account_id => {
                        if active.is_some() {
                            if events.send(VerificationEvent::Error { account_id, error: VerificationError::Busy }).await.is_err() { break; }
                            continue;
                        }
                        match session.request_user_verification(&user_id).await {
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
                    VerificationCommand::Release { target } if target.account_id == account_id => {
                        if active.as_ref().is_some_and(|flow| target.user_id == flow.user_id.as_str() && target.flow_id == flow.request.flow_id()) {
                            if let Some(mut flow) = active.take()
                                && flow.snapshot().await.is_ok_and(|s| !matches!(s.state, VerificationState::Verified | VerificationState::Cancelled | VerificationState::Mismatched | VerificationState::Failed))
                                && let Err(error) = flow.perform(VerificationAction::Cancel).await {
                                eprintln!("Failed to cancel a closed verification request: {error:?}");
                            }
                            previous = None;
                        }
                        Ok(())
                    }
                    VerificationCommand::RenameOwnDevice { account_id: requested, display_name } if requested == account_id => {
                        match rename_own_device(&client, &display_name).await {
                            // Success is the re-read own device: the UI shows the new name from it.
                            Ok(()) => match own_device(&client).await {
                                Some(device) => {
                                    if events.send(VerificationEvent::OwnDevice { account_id, device }).await.is_err() { break; }
                                    Ok(())
                                }
                                None => Err(VerificationError::Transport),
                            },
                            Err(error) => Err(error),
                        }
                    }
                    VerificationCommand::SaveProfile { account_id: requested, name, avatar } if requested == account_id => {
                        let result = save_own_profile(&client, name, avatar).await;
                        // Re-read at once: the card, the account list and the saved catalog follow this event.
                        if let Some(profile) = own_profile(&client).await {
                            shown = Some(profile.clone());
                            let (user_id, display_name, avatar) = profile;
                            let device_id = client.device_id().map(|d| d.to_string());
                            if events.send(VerificationEvent::Profile { account_id, user_id, display_name, device_id, avatar }).await.is_err() { break; }
                        }
                        if events.send(VerificationEvent::ProfileSaved { account_id, result }).await.is_err() { break; }
                        Ok(())
                    }
                    VerificationCommand::RecoverKeys { account_id: requested, key } if requested == account_id => {
                        if events.send(VerificationEvent::KeysRecovery { account_id, state: crate::signin::RecoveryState::Pending }).await.is_err() { break; }
                        // The key lives only in this call; dropping it zeroizes it.
                        let outcome = crate::recovery::recover_all_with(client.clone(), zeroize::Zeroizing::new(key.0.to_string()), false).await;
                        drop(key);
                        if let Ok(count) = &outcome
                            && events.send(VerificationEvent::KeysRecovered { account_id, count: *count }).await.is_err() { break; }
                        let state = crate::signin::RecoveryState::of_outcome(outcome.map(drop));
                        if events.send(VerificationEvent::KeysRecovery { account_id, state }).await.is_err() { break; }
                        let complete = crate::recovery::keys_recovered(&client).await;
                        shown_complete = Some(complete);
                        if events.send(VerificationEvent::KeysComplete { account_id, complete }).await.is_err() { break; }
                        Ok(())
                    }
                    VerificationCommand::ExportRoomKeys { account_id: requested, dest, passphrase, phrase } if requested == account_id => {
                        let result = crate::key_export::run_export(&client, account_id, account_dir.as_deref(), &dest, &passphrase.0, phrase.as_ref().map(|p| p.0.as_str())).await;
                        drop((passphrase, phrase));
                        match &result {
                            Ok(count) => tracing::info!("key_export ok count={count}"),
                            Err(kind) => tracing::info!("key_export failed kind={kind:?}"),
                        }
                        if events.send(VerificationEvent::KeysExported { account_id, result }).await.is_err() { break; }
                        Ok(())
                    }
                    VerificationCommand::ResetRecoveryKey { account_id: requested, phrase } if requested == account_id => {
                        let result = match crate::key_export::check_phrase_async(account_dir.as_deref(), account_id, phrase.as_ref().map(|p| p.0.as_str())).await {
                            Ok(()) => session.reset_recovery_key().await,
                            Err(_) => Err(crate::recovery::RecoveryError::WrongPhrase),
                        };
                        drop(phrase);
                        match &result {
                            Ok(_) => tracing::info!("recovery_reset ok"),
                            Err(kind) => tracing::info!("recovery_reset failed kind={kind:?}"),
                        }
                        if events.send(VerificationEvent::RecoveryKeyReset { account_id, result }).await.is_err() { break; }
                        Ok(())
                    }
                    VerificationCommand::DeleteDevice { account_id: requested, device_ids, password } if requested == account_id => {
                        match session.delete_devices(&device_ids, password).await {
                            Ok(()) => {
                                // The server accepted the sign-out, but the crypto-store list still holds the device until a later keys query:
                                // always close the dialog first, then send the list without it.
                                if events.send(VerificationEvent::DeviceDeleted { account_id, device_ids: device_ids.clone() }).await.is_err() { break; }
                                // A failed re-read is not a verification error: the device is gone.
                                if let Ok(devices) = device_list(&client).await {
                                    if events.send(VerificationEvent::Devices { account_id, devices: without_devices(devices, &device_ids) }).await.is_err() { break; }
                                }
                                Ok(())
                            }
                            Err(error) => {
                                if events.send(VerificationEvent::DeviceDeleteFailed { account_id, device_ids, error }).await.is_err() { break; }
                                Ok(())
                            }
                        }
                    }
                    _ => Err(VerificationError::WrongScope),
                }
            }
            _ = profile_ticks.tick() => {
                let complete = crate::recovery::keys_recovered(&client).await;
                if shown_complete != Some(complete) {
                    shown_complete = Some(complete);
                    if events.send(VerificationEvent::KeysComplete { account_id, complete }).await.is_err() { break; }
                }
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
            Some((sender, flow_id)) = incoming.recv() => {
                if active.as_ref().is_some_and(|flow| flow.user_id == sender && flow.request.flow_id() == flow_id) {
                    Ok(())
                } else if let Ok(mut flow) = session.receive_device_verification(sender.as_str(), &flow_id).await {
                    if active.is_some() {
                        if unsent_rejects.iter().any(|(f, _)| f.user_id == sender && f.request.flow_id() == flow_id) {
                            // Duplicate delivery of a request we are already rejecting.
                        } else if let Err(error) = flow.perform(VerificationAction::Cancel).await {
                            eprintln!("Failed to reject a concurrent verification request, will retry: {error:?}");
                            unsent_rejects.push((flow, 0));
                        }
                        Ok(())
                    } else {
                        active = Some(flow);
                        previous = None;
                        Ok(())
                    }
                } else {
                    // Already cancelled, expired or unknown: there is nothing to present.
                    Ok(())
                }
            }
            _ = ticks.tick() => {
                resend_rejects(&mut unsent_rejects).await;
                match &mut active {
                    Some(flow) => flow.advance().await,
                    None => Ok(()),
                }
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
    client.remove_event_handler(to_device_handler);
    client.remove_event_handler(room_handler);
}

const REJECT_RETRY_EVERY: u32 = 5; // ticks of 200 ms
const REJECT_MAX_ATTEMPTS: u32 = 30;

/// The SDK already marked the request cancelled before the send failed; `request.cancel()` rebuilds
/// that same cancel event, so re-sending reopens nothing. A request the peer closed meanwhile is
/// simply cancelled twice; after the attempts are used up the peer's own expiry closes it.
async fn resend_rejects(unsent: &mut Vec<(VerificationFlow, u32)>) {
    let mut kept = Vec::new();
    for (flow, ticks) in std::mem::take(unsent) {
        if ticks % REJECT_RETRY_EVERY != REJECT_RETRY_EVERY - 1
            || (flow.request.cancel().await.is_err() && ticks / REJECT_RETRY_EVERY + 1 < REJECT_MAX_ATTEMPTS)
        {
            kept.push((flow, ticks + 1));
        }
    }
    *unsent = kept;
}

const PROFILE_REFRESH: std::time::Duration = std::time::Duration::from_secs(60);

/// Own id, name and picture as the SDK reports them; a failed read keeps what is already shown.
async fn own_profile(client: &Client) -> Option<(String, Option<String>, Option<String>)> {
    let user_id = client.user_id()?.to_string();
    let account = client.account();
    let display_name = account.get_display_name().await.ok()?;
    let avatar = crate::avatar::normalize_avatar(
        account.get_avatar_url().await.ok()?.map(|uri| uri.to_string()),
    );
    Some((user_id, display_name, avatar))
}

/// Runs `profile::save_profile`; errors become the closed `OperationError` (no path, no text).
async fn save_own_profile(
    client: &Client,
    name: Option<String>,
    avatar: crate::verification::ProfileAvatar,
) -> Result<(), crate::operations::OperationError> {
    use crate::{operations::OperationError as E, profile::{self, AvatarEdit}, verification::ProfileAvatar};
    let account = client.account();
    let current_name = account.get_display_name().await.map_err(|_| E::Transport)?;
    let has_avatar = account.get_avatar_url().await.map_err(|_| E::Transport)?.is_some();
    // Unchanged name: hand the current one back (none set yet: the user id) so only the avatar is written.
    let unchanged = current_name.clone().or_else(|| client.user_id().map(|u| u.to_string())).unwrap_or_default();
    let current = profile::Current { name: Some(unchanged.clone()), has_avatar };
    let avatar = match avatar {
        ProfileAvatar::Keep => AvatarEdit::Keep,
        ProfileAvatar::Remove => AvatarEdit::Remove,
        ProfileAvatar::Set(path) => AvatarEdit::Set(profile::prepare_avatar(path).await.map_err(map_profile_error)?),
    };
    profile::save_profile(client, &current, name.as_deref().unwrap_or(&unchanged), avatar)
        .await
        .map(drop)
        .map_err(|e| map_profile_error(e.error))
}

fn map_profile_error(error: crate::profile::ProfileError) -> crate::operations::OperationError {
    use crate::{operations::OperationError as E, profile::ProfileError as P};
    match error {
        P::NameEmpty | P::NameTooLong | P::FileUnreadable | P::FileTooLarge | P::FileInvalid => E::InvalidInput,
        P::NotAllowed | P::Rejected => E::Forbidden,
        P::Network => E::Transport,
    }
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

fn without_devices(mut devices: Vec<VerificationDevice>, device_ids: &[String]) -> Vec<VerificationDevice> {
    devices.retain(|d| !device_ids.contains(&d.device_id));
    devices
}

async fn device_list(client: &Client) -> Result<Vec<VerificationDevice>, VerificationError> {
    let user_id = client.user_id().ok_or(VerificationError::LoggedOut)?;
    let devices = client
        .encryption()
        .get_user_devices(user_id)
        .await
        .map_err(|_| VerificationError::Transport)?;
    // Server-side metadata; failure here must not hide the device list.
    let meta = server_meta(client).await;
    let mut result = Vec::new();
    for device in devices.devices() {
        if Some(device.device_id()) == client.device_id() {
            continue;
        }
        result.push(describe(user_id.as_str(), &meta, device.device_id(), device.is_verified()));
    }
    sort_devices(&mut result);
    Ok(result)
}

async fn server_meta(client: &Client) -> Vec<matrix_sdk::ruma::api::client::device::Device> {
    client.devices().await.map(|r| r.devices).unwrap_or_default()
}

fn describe(
    user_id: &str,
    meta: &[matrix_sdk::ruma::api::client::device::Device],
    device_id: &matrix_sdk::ruma::DeviceId,
    verified: bool,
) -> VerificationDevice {
    let m = meta.iter().find(|m| m.device_id == device_id);
    VerificationDevice {
        user_id: user_id.to_owned(),
        device_id: device_id.to_string(),
        verified,
        display_name: m.and_then(|m| m.display_name.clone()).filter(|n| !n.trim().is_empty()),
        last_seen_ts: m.and_then(|m| m.last_seen_ts).map(|t| t.0.into()),
        last_seen_ip: m.and_then(|m| m.last_seen_ip.clone()).filter(|i| !i.trim().is_empty()),
    }
}

/// This session's own device, with the state the SDK knows locally (no HTTP for the state).
async fn own_device(client: &Client) -> Option<VerificationDevice> {
    let user_id = client.user_id()?;
    let own = client.encryption().get_own_device().await.ok()??;
    let meta = server_meta(client).await;
    Some(describe(user_id.as_str(), &meta, own.device_id(), own.is_verified()))
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

    #[test]
    fn long_device_lists_are_not_truncated_or_rejected() {
        let mut v: Vec<_> = (0..300).map(|i| describe("@a:x", &[], format!("D{i}").as_str().into(), false)).collect();
        sort_devices(&mut v);
        assert_eq!(v.len(), 300);
    }

    #[test]
    fn deleted_device_is_dropped_from_a_stale_list() {
        let d = |id: &str| describe("@a:x", &[], id.into(), true);
        let ids: Vec<_> = without_devices(vec![d("OLD"), d("KEEP")], &["OLD".to_owned()]).into_iter().map(|d| d.device_id).collect();
        assert_eq!(ids, ["KEEP"]);
    }

    #[test]
    fn describe_tolerates_missing_or_blank_server_fields() {
        use matrix_sdk::ruma::{api::client::device::Device, device_id};
        let mut full = Device::new(device_id!("FULL").to_owned());
        full.display_name = Some("Mac".into());
        full.last_seen_ip = Some("203.0.113.1".into());
        full.last_seen_ts = Some(matrix_sdk::ruma::MilliSecondsSinceUnixEpoch(42u32.into()));
        let mut blank = Device::new(device_id!("BLANK").to_owned());
        blank.display_name = Some("  ".into());
        blank.last_seen_ip = Some("".into());
        let meta = [full, blank];
        let d = |id: &str| describe("@a:x", &meta, id.into(), true);
        assert_eq!((d("FULL").last_seen_ts, d("FULL").last_seen_ip.as_deref(), d("FULL").display_name.as_deref()), (Some(42), Some("203.0.113.1"), Some("Mac")));
        assert_eq!((d("BLANK").last_seen_ts, d("BLANK").last_seen_ip, d("BLANK").display_name), (None, None, None));
        // Device unknown to the server list (devices() failed): still described, nothing invented.
        assert_eq!((d("NOMETA").last_seen_ts, d("NOMETA").last_seen_ip, d("NOMETA").display_name), (None, None, None));
    }
}
