use matrix_desktop::{DeviceVerification, PopupBody, PopupButton};
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
        assert!(!panel.can_begin(), "close the outcome before beginning another flow");
    }
    panel.popup_close();
    panel.flush(&mut sender);
    assert!(matches!(receiver.try_recv(), Ok(matrix_engine::verification::VerificationCommand::Release { .. })));
    panel.flush(&mut sender);
    assert!(matches!(receiver.try_recv(), Ok(matrix_engine::verification::VerificationCommand::List { .. })), "Verified also refreshes the list");
    assert!(panel.can_begin());
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
    assert!(panel.snapshot().is_none(), "another account's snapshot is ignored");
    panel.apply(Event::Error {
        account_id: AccountId::new(7),
        error: VerificationError::Transport,
    });
    assert_eq!(panel.popup_view().unwrap().body, PopupBody::Outcome("Verification failed"));
    panel.popup_close();
    assert!(panel.begin(0), "explicit retry after closing the error");
}

#[test]
fn refresh_and_closed_backend_are_visible_without_fabricated_trust() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    assert!(panel.refresh());
    assert!(!panel.refresh());
    let (mut sender, receiver) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    drop(receiver);
    panel.flush(&mut sender);
    assert!(panel.popup_view().is_none(), "a failed list refresh opens no popup");
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
    assert!(!panel.can_act(Action::Confirm));
}

#[test]
fn comparison_requires_explicit_confirmation_and_preserves_exact_target() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    assert!(!panel.act(Action::Confirm));
    panel.apply(Event::Snapshot(snapshot(State::Compare {
        decimals: (1234, 5678, 9012),
    })));
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
    assert!(panel.popup_view().is_none(), "enqueue is not SDK trust: nothing shows «Verificato»");
}

#[test]
fn terminal_states_are_distinct_untrusted_and_the_comparison_is_removed() {
    for (state, label) in [
        (State::Cancelled, "Verification cancelled"),
        (State::Mismatched, "The emoji don't match"),
        (State::Failed, "Verification failed"),
    ] {
        let mut panel = compare_popup(true);
        assert!(panel.act(Action::Cancel));
        panel.apply(Event::Snapshot(snapshot(state)));
        let view = panel.popup_view().unwrap();
        assert_eq!(view.body, PopupBody::Outcome(label));
        assert_eq!(view.buttons, vec![PopupButton::Ok]);
        assert!(!panel.act(Action::Confirm));
    }
}

const EMOJI: [(&str, &str); 7] = [
    ("💡", "Light Bulb"),
    ("🐱", "Cat"),
    ("🦋", "Butterfly"),
    ("🎅", "Santa"),
    ("🎧", "Headphone"),
    ("✏️", "Pencil"),
    ("✂️", "Scissors"),
];

fn compare_snapshot(emoji: bool) -> VerificationSnapshot {
    let mut s = snapshot(State::Compare { decimals: (1234, 5678, 9012) });
    s.sas_emoji = emoji.then(|| EMOJI.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect());
    s
}

/// A popup opened by a request from the other side, already comparing.
fn compare_popup(emoji: bool) -> DeviceVerification {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Snapshot(snapshot(State::Requested)));
    panel.apply(Event::Snapshot(compare_snapshot(emoji)));
    panel
}

fn drain(panel: &mut DeviceVerification) -> Option<matrix_engine::verification::VerificationCommand> {
    let (mut sender, mut receiver) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    panel.flush(&mut sender);
    receiver.try_recv().ok()
}

#[test]
fn every_start_opens_the_popup_waiting_with_only_a_cancel() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    assert!(panel.popup_view().is_none());
    assert!(panel.begin_user("@bob:fixture.invalid".into()));
    let view = panel.popup_view().expect("«Verifica utente» opens the popup at once");
    assert_eq!((view.body, view.buttons), (PopupBody::Waiting, vec![PopupButton::Cancel]));
    assert_eq!(panel.popup_focused(), Some(PopupButton::Cancel), "first button focused");
    // Device «Verifica» does the same.
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Devices {
        account_id: AccountId::new(7),
        devices: vec![matrix_engine::verification::VerificationDevice {
            user_id: "@alice:fixture.invalid".into(),
            device_id: "PEER".into(),
            verified: false,
            display_name: None,
            last_seen_ts: None,
            last_seen_ip: None,
        }],
    });
    assert!(panel.begin(0));
    assert_eq!(panel.popup_view().unwrap().body, PopupBody::Waiting);
}

#[test]
fn received_request_opens_the_popup_and_accept_or_refuse_go_through_the_sdk_actions() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Snapshot(snapshot(State::Requested)));
    let view = panel.popup_view().expect("a request from outside opens the popup");
    assert_eq!(view.title, "Verification request");
    assert_eq!(view.body, PopupBody::Request("@alice:fixture.invalid · PEER".into()));
    assert_eq!(view.buttons, vec![PopupButton::Accept, PopupButton::Decline]);
    assert!(panel.popup_press(PopupButton::Accept));
    assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::Act { action: Action::Accept, .. })));
    // Refuse = Cancel through the SDK, and the popup is gone.
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Snapshot(snapshot(State::Requested)));
    assert!(panel.popup_press(PopupButton::Decline));
    assert!(!panel.popup_open());
    assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::Act { action: Action::Cancel, .. })));
}

#[test]
fn comparison_shows_seven_emoji_with_sdk_names_or_the_numbers() {
    let panel = compare_popup(true);
    let view = panel.popup_view().unwrap();
    assert_eq!(view.buttons, vec![PopupButton::Match, PopupButton::NoMatch]);
    let PopupBody::Emoji(emoji) = view.body else { panic!("emoji expected") };
    assert_eq!(emoji.len(), 7);
    assert_eq!(emoji.iter().map(|(s, n)| (s.as_str(), n.as_str())).collect::<Vec<_>>(), EMOJI);
    assert_eq!(panel.popup_focused(), Some(PopupButton::Match));
    // The peer negotiated decimals only, or sent nothing usable: the three numbers.
    assert_eq!(compare_popup(false).popup_view().unwrap().body, PopupBody::Decimals((1234, 5678, 9012)));
    let mut empty = compare_snapshot(true);
    empty.sas_emoji = Some(vec![]);
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Snapshot(snapshot(State::Requested)));
    panel.apply(Event::Snapshot(empty));
    assert_eq!(panel.popup_view().unwrap().body, PopupBody::Decimals((1234, 5678, 9012)));
}

#[test]
fn match_and_mismatch_are_explicit_and_send_exact_sdk_actions() {
    for (button, action) in [(PopupButton::Match, Action::Confirm), (PopupButton::NoMatch, Action::Mismatch)] {
        let mut panel = compare_popup(true);
        assert!(panel.popup_press(button));
        assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::Act { action: a, target }) if a == action && target.flow_id == "flow-one"));
        // Until the SDK answers the popup waits; the buttons are gone, a second press does nothing.
        assert_eq!(panel.popup_view().unwrap().body, PopupBody::Waiting);
        assert!(!panel.popup_press(button));
    }
    // A button that is not on screen cannot be pressed.
    let mut panel = compare_popup(true);
    assert!(!panel.popup_press(PopupButton::Ok));
    assert!(!panel.popup_press(PopupButton::Accept));
}

#[test]
fn leaving_a_comparison_asks_first_and_other_states_leave_at_once() {
    // Esc / X / click outside: confirmation inside the same box, No goes back, Yes cancels through the SDK.
    for _ in 0..2 {
        let mut panel = compare_popup(true);
        assert!(panel.popup_press(PopupButton::Close));
        let ask = panel.popup_view().unwrap();
        assert_eq!((ask.title, ask.body, ask.buttons.clone()), ("Cancel the verification?", PopupBody::Empty, vec![PopupButton::Yes, PopupButton::No]));
        assert!(drain(&mut panel).is_none(), "asking cancels nothing");
        panel.popup_close(); // Esc again = back to the comparison
        assert_eq!(panel.popup_view().unwrap().buttons, vec![PopupButton::Match, PopupButton::NoMatch]);
        panel.popup_close();
        assert!(panel.popup_press(PopupButton::No));
        assert_eq!(panel.popup_view().unwrap().buttons, vec![PopupButton::Match, PopupButton::NoMatch]);
        panel.popup_close();
        assert!(panel.popup_press(PopupButton::Yes));
        assert!(!panel.popup_open());
        assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::Act { action: Action::Cancel, .. })), "the SDK flow is cancelled, not left running");
    }
    // Waiting: no question, cancel and close.
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Snapshot(snapshot(State::Requested)));
    panel.apply(Event::Snapshot(snapshot(State::Ready)));
    panel.popup_close();
    assert!(!panel.popup_open());
    assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::Act { action: Action::Cancel, .. })));
    // An outcome: leaving cancels nothing.
    let mut panel = compare_popup(true);
    panel.apply(Event::Snapshot(snapshot(State::Verified)));
    assert_eq!(panel.popup_view().unwrap().body, PopupBody::Outcome("Verified"));
    panel.popup_close();
    assert!(!panel.popup_open());
    assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::Release { target }) if target.flow_id == "flow-one"));
    assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::List { .. })), "after the release, the device list is refreshed");
}

#[test]
fn closing_before_the_sdk_answers_cancels_the_flow_as_soon_as_it_exists() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    assert!(panel.begin_user("@bob:fixture.invalid".into()));
    assert!(drain(&mut panel).is_some()); // BeginUser sent, no snapshot yet
    panel.popup_close();
    assert!(!panel.popup_open());
    assert!(drain(&mut panel).is_none(), "no flow to cancel yet");
    panel.apply(Event::Snapshot(snapshot(State::Waiting)));
    assert!(matches!(drain(&mut panel), Some(matrix_engine::verification::VerificationCommand::Act { action: Action::Cancel, .. })));
    assert!(!panel.popup_open(), "the late snapshot does not reopen it");
}

#[test]
fn a_closed_request_stays_closed_but_a_new_one_opens() {
    let mut panel = DeviceVerification::new(AccountId::new(7));
    panel.apply(Event::Snapshot(snapshot(State::Requested)));
    panel.popup_close();
    panel.apply(Event::Snapshot(snapshot(State::Requested)));
    assert!(!panel.popup_open(), "same flow, closed by the user");
    let mut other = snapshot(State::Requested);
    other.flow_id = "flow-two".into();
    panel.apply(Event::Snapshot(other));
    assert!(panel.popup_open());
}

#[test]
fn tab_cycles_inside_the_popup_and_enter_style_press_follows_the_focus() {
    let mut panel = compare_popup(true);
    let seen: Vec<_> = (0..4).map(|_| { let f = panel.popup_focused(); panel.popup_tab(false); f }).collect();
    assert_eq!(seen, [Some(PopupButton::Match), Some(PopupButton::NoMatch), Some(PopupButton::Close), Some(PopupButton::Match)]);
    panel.popup_tab(true);
    panel.popup_tab(true);
    assert_eq!(panel.popup_focused(), Some(PopupButton::Close), "Shift+Tab wraps backwards");
    // A state change puts the focus back on the first button.
    panel.apply(Event::Snapshot(snapshot(State::Verified)));
    assert_eq!(panel.popup_focused(), Some(PopupButton::Ok));
    // Closed: nothing to focus.
    panel.popup_close();
    assert_eq!(panel.popup_focused(), None);
}
