use matrix_desktop::DeviceVerification;
use matrix_domain::AccountId;
use matrix_engine::verification::{
    VerificationAction as Action, VerificationEvent as Event, VerificationSnapshot,
    VerificationState as State,
};

fn snapshot(state: State) -> VerificationSnapshot {
    VerificationSnapshot {
        account_id: AccountId::new(7),
        user_id: "@alice:fixture.invalid".into(),
        device_id: "PEER".into(),
        flow_id: "flow-one".into(),
        state,
        device_verified: false,
        sas_emoji: None,
    }
}

#[test]
fn initiation_availability_tracks_pending_active_and_terminal_flows() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    assert!(panel.can_begin());
    assert!(panel.refresh());
    assert!(!panel.can_begin());
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    panel.flush(&mut sender);
    assert!(receiver.try_recv().is_ok());
    assert!(panel.can_begin());
    for state in [
        State::Waiting,
        State::Requested,
        State::Ready,
        State::Negotiating,
        State::Compare {
            decimals: (1234, 5678, 9012),
        },
        State::Confirmed,
    ] {
        panel.apply(Event::Snapshot(snapshot(state)));
        assert!(!panel.can_begin());
    }
    for state in [
        State::Cancelled,
        State::Mismatched,
        State::Failed,
        State::Verified,
    ] {
        panel.apply(Event::Snapshot(snapshot(state)));
        assert!(panel.can_begin());
    }
}

#[test]
fn device_selection_retries_full_queue_and_ignores_foreign_account() {
    use matrix_engine::verification::{
        VerificationCommand as Command, VerificationDevice, VerificationError,
    };
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Devices {
        account_id: AccountId::new(8),
        devices: vec![VerificationDevice {
            user_id: "@other:fixture.invalid".into(),
            device_id: "WRONG".into(),
            verified: true,
            display_name: None,
            last_seen_ts: None,
            last_seen_ip: None,
        }],
    });
    assert!(panel.devices().is_empty());
    panel.apply(Event::Devices {
        account_id: AccountId::new(7),
        devices: vec![VerificationDevice {
            user_id: "@alice:fixture.invalid".into(),
            device_id: "PEER".into(),
            verified: false,
            display_name: None,
            last_seen_ts: None,
            last_seen_ip: None,
        }],
    });
    assert!(!panel.begin(5));
    assert!(panel.begin(0));
    assert!(!panel.begin(0));
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    assert!(
        sender
            .try_send(Command::List {
                account_id: AccountId::new(7)
            })
            .is_ok()
    );
    panel.flush(&mut sender);
    assert!(receiver.try_recv().is_ok());
    panel.flush(&mut sender);
    assert!(
        matches!(receiver.try_recv(), Ok(Command::Begin { account_id, user_id, device_id }) if account_id == AccountId::new(7) && user_id == "@alice:fixture.invalid" && device_id == "PEER")
    );
    let mut foreign = snapshot(State::Verified);
    foreign.account_id = AccountId::new(8);
    foreign.device_verified = true;
    panel.apply(Event::Snapshot(foreign));
    assert!(!panel.verified());
    panel.apply(Event::Error {
        account_id: AccountId::new(7),
        error: VerificationError::Transport,
    });
    assert_eq!(panel.status_label(), "Verifica non riuscita");
    assert!(!panel.verified());
    assert!(panel.begin(0), "explicit retry after failure");
}

#[test]
fn refresh_and_closed_backend_are_visible_without_fabricated_trust() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    assert!(panel.refresh());
    assert!(!panel.refresh());
    let (mut sender, receiver) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    drop(receiver);
    panel.flush(&mut sender);
    assert_eq!(panel.status_label(), "Verifica non riuscita");
    assert!(panel.snapshot().is_none());
    assert!(!panel.can_act(Action::Confirm));
}

#[test]
fn active_flow_cannot_be_replaced_or_confirmed_after_transport_error() {
    use matrix_engine::verification::{VerificationDevice, VerificationError};
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Devices {
        account_id: AccountId::new(7),
        devices: vec![VerificationDevice {
            user_id: "@alice:fixture.invalid".into(),
            device_id: "PEER".into(),
            verified: false,
            display_name: None,
            last_seen_ts: None,
            last_seen_ip: None,
        }],
    });
    panel.apply(Event::Snapshot(snapshot(State::Compare {
        decimals: (1111, 2222, 3333),
    })));
    assert!(
        !panel.begin(0),
        "an active SAS must be cancelled, not hidden by another request"
    );
    panel.apply(Event::Error {
        account_id: AccountId::new(7),
        error: VerificationError::Transport,
    });
    assert_eq!(panel.comparison(), None);
    assert!(!panel.can_act(Action::Confirm));
    assert!(!panel.verified());
}

#[test]
fn comparison_requires_explicit_confirmation_and_preserves_exact_target() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    assert!(!panel.act(Action::Confirm));
    panel.apply(Event::Snapshot(snapshot(State::Compare {
        decimals: (1234, 5678, 9012),
    })));
    assert_eq!(panel.comparison(), Some((1234, 5678, 9012)));
    assert!(!panel.verified());
    assert!(panel.act(Action::Confirm));
    assert!(!panel.act(Action::Confirm), "one pending decision only");
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    panel.flush(&mut sender);
    match receiver
        .try_recv()
        .unwrap_or_else(|_| panic!("explicit decision missing"))
    {
        matrix_engine::verification::VerificationCommand::Act { target, action } => {
            assert_eq!(target.account_id, AccountId::new(7));
            assert_eq!(target.user_id, "@alice:fixture.invalid");
            assert_eq!(target.flow_id, "flow-one");
            assert_eq!(action, Action::Confirm);
        }
        _ => panic!("wrong command"),
    }
    assert!(!panel.verified(), "enqueue is not SDK trust");
}

#[test]
fn terminal_states_are_distinct_untrusted_and_comparison_is_removed() {
    for (state, label) in [
        (State::Cancelled, "Verifica annullata"),
        (State::Mismatched, "Numeri diversi · verifica rifiutata"),
        (State::Failed, "Verifica non riuscita"),
    ] {
        let mut panel = DeviceVerification::new(AccountId::new(7));
        panel.apply(Event::Snapshot(snapshot(State::Compare {
            decimals: (1234, 5678, 9012),
        })));
        assert!(panel.act(Action::Cancel));
        panel.apply(Event::Snapshot(snapshot(state)));
        assert_eq!(panel.status_label(), label);
        assert_eq!(panel.trust_label(), "Dispositivo non verificato");
        assert_eq!(panel.comparison(), None);
        assert!(!panel.act(Action::Confirm));
        assert!(!panel.verified());
    }
}
