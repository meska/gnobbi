use matrix_desktop::{DeviceVerification, PopupButton};
use matrix_domain::AccountId;
use matrix_engine::verification::{
    VerificationCommand, VerificationDevice, VerificationEvent, VerificationSnapshot,
    VerificationState,
};

#[test]
fn verified_snapshot_refreshes_the_device_list_after_ok() {
    let account = AccountId::new(7);
    let mut panel = DeviceVerification::new(account);
    let device = VerificationDevice {
        user_id: "@alice:fixture.invalid".into(),
        device_id: "PEER".into(),
        verified: false,
        display_name: None,
        last_seen_ts: None,
        last_seen_ip: None,
    };
    panel.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![device.clone()],
    });
    let snapshot = |state| VerificationSnapshot {
        account_id: account,
        user_id: device.user_id.clone(),
        device_id: device.device_id.clone(),
        flow_id: "flow".into(),
        state,
        device_verified: state == VerificationState::Verified,
        sas_emoji: None,
    };
    panel.apply(VerificationEvent::Snapshot(snapshot(
        VerificationState::Requested,
    )));
    panel.apply(VerificationEvent::Snapshot(snapshot(
        VerificationState::Verified,
    )));
    assert!(panel.popup_press(PopupButton::Ok));
    let (mut tx, mut rx) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    panel.flush(&mut tx);
    assert!(matches!(rx.try_recv(), Ok(VerificationCommand::Release { .. })));
    panel.flush(&mut tx);
    assert!(
        matches!(rx.try_recv(), Ok(VerificationCommand::List { account_id }) if account_id == account),
        "verified outcome must request a fresh list"
    );
    let mut updated = device;
    updated.verified = true;
    panel.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![updated],
    });
    assert!(panel.devices()[0].verified);
    assert!(!panel.begin(0));
}

#[test]
fn verification_refresh_waits_for_an_existing_command() {
    let account = AccountId::new(7);
    let mut panel = DeviceVerification::new(account);
    let snapshot = |state| VerificationSnapshot {
        account_id: account,
        user_id: "@alice:fixture.invalid".into(),
        device_id: "PEER".into(),
        flow_id: "flow".into(),
        state,
        device_verified: state == VerificationState::Verified,
        sas_emoji: None,
    };
    panel.apply(VerificationEvent::Snapshot(snapshot(
        VerificationState::Requested,
    )));
    assert!(panel.refresh());
    panel.apply(VerificationEvent::Snapshot(snapshot(
        VerificationState::Verified,
    )));
    assert!(panel.popup_press(PopupButton::Ok));
    let (mut tx, mut rx) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    panel.flush(&mut tx);
    assert!(matches!(
        rx.try_recv(),
        Ok(VerificationCommand::Release { .. })
    ));
    panel.flush(&mut tx);
    assert!(matches!(
        rx.try_recv(),
        Ok(VerificationCommand::List { .. })
    ));
    panel.flush(&mut tx);
    assert!(matches!(
        rx.try_recv(),
        Ok(VerificationCommand::List { .. })
    ));
    panel.flush(&mut tx);
    assert!(rx.try_recv().is_err());
}
