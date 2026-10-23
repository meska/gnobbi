use matrix_desktop::{LinkConfirmation, MessageLinkRequests};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};

fn event() -> ScopedTimelineId {
    ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(67),
            room_id: RoomId::new("!links:test"),
        },
        id: TimelineId::Server(EventId::new("$links")),
    }
}

#[test]
fn replacement_even_with_same_url_retires_captured_confirmation() {
    let id = event();
    let text = "caffè https://example.invalid/a https://example.invalid/a";
    let mut sources = MessageLinkRequests::default();
    let targets = sources.targets(&id, text);
    let mut confirmation = LinkConfirmation::default();
    assert!(sources.request(&targets[0], &mut confirmation));
    let old = confirmation.capture().unwrap();
    assert!(sources.request(&targets[0], &mut confirmation));
    let current = confirmation.capture().unwrap();
    assert_ne!(old.generation(), current.generation());
    assert_eq!(current.source().event(), &id);
    assert_eq!(current.source().text(), text);
    assert_eq!(current.source().range(), targets[0].range());
    assert_eq!(current.url().as_str(), targets[0].url().as_str());
    assert!(confirmation.confirm_current(&old, &sources).is_none());
    assert!(!confirmation.cancel_current(&old, &sources));
    assert_eq!(
        confirmation.capture().unwrap().generation(),
        current.generation()
    );
    assert_eq!(
        confirmation
            .confirm_current(&current, &sources)
            .unwrap()
            .as_str(),
        targets[0].url().as_str()
    );
    assert!(confirmation.confirm_current(&current, &sources).is_none());
    assert!(!confirmation.cancel_current(&current, &sources));
    assert!(sources.request(&targets[1], &mut confirmation));
    let other_occurrence = confirmation.capture().unwrap();
    assert_eq!(other_occurrence.source().instance(), targets[1].instance());
    assert_eq!(other_occurrence.source().range(), targets[1].range());
    assert!(!confirmation.cancel_current(&current, &sources));
    assert!(confirmation.cancel_current(&other_occurrence, &sources));
    assert!(
        confirmation
            .confirm_current(&other_occurrence, &sources)
            .is_none()
    );
}

#[test]
fn queued_confirmation_callbacks_revalidate_source_before_effects() {
    use std::{cell::RefCell, rc::Rc};
    for change in ["body", "message", "room", "account", "removal"] {
        let id = event();
        let text = "caffè https://example.invalid/exact";
        let sources = Rc::new(RefCell::new(MessageLinkRequests::default()));
        let confirmation = Rc::new(RefCell::new(LinkConfirmation::default()));
        let target = sources.borrow_mut().targets(&id, text)[0].clone();
        assert!(
            sources
                .borrow_mut()
                .request(&target, &mut confirmation.borrow_mut())
        );
        let captured = confirmation.borrow().capture().unwrap();
        let cancel = {
            let (sources, confirmation, captured) =
                (sources.clone(), confirmation.clone(), captured.clone());
            move || {
                confirmation
                    .borrow_mut()
                    .cancel_current(&captured, &sources.borrow())
            }
        };
        let open = {
            let (sources, confirmation) = (sources.clone(), confirmation.clone());
            move || {
                confirmation
                    .borrow_mut()
                    .confirm_current(&captured, &sources.borrow())
            }
        };
        let mut other = id.clone();
        match change {
            "body" => {
                sources.borrow_mut().targets(&id, &format!("new {text}"));
            }
            "message" => {
                other.id = TimelineId::Server(EventId::new("$other"));
            }
            "room" => {
                other.scope.room_id = RoomId::new("!other:test");
            }
            "account" => {
                other.scope.account_id = AccountId::new(72);
            }
            "removal" => sources.borrow_mut().retain_current(std::iter::empty()),
            _ => unreachable!(),
        }
        if other != id {
            sources.borrow_mut().targets(&other, text);
            sources.borrow_mut().retain_event(&other);
        }
        assert!(!cancel(), "retired source cancelled: {change}");
        assert!(open().is_none(), "retired source opened: {change}");
        // Same IDs, exact body and URL returning cannot revive a queued callback.
        let fresh = sources.borrow_mut().targets(&id, text)[0].clone();
        assert!(sources.borrow_mut().focus(&fresh));
        assert!(!cancel(), "cancel revived on return: {change}");
        assert!(open().is_none(), "open revived on return: {change}");
        assert!(
            sources
                .borrow_mut()
                .request(&fresh, &mut confirmation.borrow_mut())
        );
        let current = confirmation.borrow().capture().unwrap();
        assert!(!cancel(), "stale cancel consumed replacement: {change}");
        assert!(
            open().is_none(),
            "stale open consumed replacement: {change}"
        );
        assert_eq!(
            confirmation
                .borrow_mut()
                .confirm_current(&current, &sources.borrow())
                .unwrap()
                .as_str(),
            fresh.url().as_str()
        );
        assert!(open().is_none());
    }
}

#[test]
fn requesting_another_message_retires_captured_inline_targets() {
    let id = event();
    let mut other = id.clone();
    other.id = TimelineId::Server(EventId::new("$other"));
    let mut sources = MessageLinkRequests::default();
    let old = sources.targets(&id, "https://example.invalid/old")[0].clone();
    let fresh = sources.targets(&other, "https://example.invalid/new")[0].clone();
    let mut confirmation = LinkConfirmation::default();
    assert!(sources.request(&fresh, &mut confirmation));
    assert!(!sources.request(&old, &mut confirmation));
    assert_eq!(
        confirmation.pending().unwrap().as_str(),
        fresh.url().as_str()
    );
}

#[test]
fn stale_focus_cannot_retire_a_newer_source_after_message_away_back() {
    let id = event();
    let mut other = id.clone();
    other.id = TimelineId::Server(EventId::new("$other"));
    let text = "https://example.invalid/exact";
    let mut sources = MessageLinkRequests::default();
    let old = sources.targets(&id, text)[0].clone();
    let next = sources.targets(&other, text)[0].clone();
    assert!(sources.focus(&next));
    let fresh = sources.targets(&id, text)[0].clone();
    assert!(sources.focus(&fresh));
    assert!(!sources.focus(&old));
    assert!(sources.is_current(&fresh));
}

#[test]
fn changed_body_rejects_queued_request_without_replacing_a_newer_target() {
    let id = event();
    let mut sources = MessageLinkRequests::default();
    let old = sources.targets(&id, "https://example.invalid/old")[0].clone();
    let fresh = sources.targets(&id, "https://example.invalid/new")[0].clone();
    let mut confirmation = LinkConfirmation::default();
    assert!(sources.request(&fresh, &mut confirmation));
    assert!(!sources.request(&old, &mut confirmation));
    assert_eq!(
        confirmation.pending().unwrap().as_str(),
        fresh.url().as_str()
    );
    sources.targets(&id, old.text());
    assert!(!sources.is_current(&old));
}

#[test]
fn removed_sources_cannot_revive_on_same_id_body_away_back() {
    let id = event();
    let text = "https://example.invalid/exact";
    let mut sources = MessageLinkRequests::default();
    let old = sources.targets(&id, text)[0].clone();
    sources.retain_current(std::iter::empty());
    let new = sources.targets(&id, text)[0].clone();
    assert!(!sources.is_current(&old));
    assert!(sources.is_current(&new));
    sources.clear();
    sources.targets(&id, text);
    assert!(!sources.is_current(&new));
}

#[test]
fn message_scope_switch_retires_old_queued_targets_even_after_return() {
    for account in [false, true] {
        let first = event();
        let mut other = first.clone();
        if account {
            other.scope.account_id = AccountId::new(68);
        } else {
            other.scope.room_id = RoomId::new("!other:test");
        }
        let text = "https://example.invalid/exact";
        let mut sources = MessageLinkRequests::default();
        let old = sources.targets(&first, text)[0].clone();
        sources.targets(&other, text);
        sources.retain_event(&other);
        let fresh = sources.targets(&first, text)[0].clone();
        sources.retain_event(&first);
        let mut confirmation = LinkConfirmation::default();
        assert!(sources.request(&fresh, &mut confirmation));
        assert!(!sources.request(&old, &mut confirmation));
        assert_eq!(
            confirmation.pending().unwrap().as_str(),
            fresh.url().as_str()
        );
    }
}

#[test]
fn scoped_link_occurrences_only_request_validated_confirmation() {
    let id = event();
    let text = "caffè https://example.invalid/a https://example.invalid/a javascript:alert(1)";
    let mut sources = MessageLinkRequests::default();
    let links = sources.targets(&id, text);
    assert_eq!(links.len(), 2);
    assert_ne!(links[0].instance(), links[1].instance());
    assert_ne!(links[0].range(), links[1].range());
    assert_eq!(links[1].event(), &id);
    assert_eq!(links[1].text(), text);
    let mut confirmation = LinkConfirmation::default();
    assert!(sources.request(&links[1], &mut confirmation));
    assert_eq!(
        confirmation.pending().unwrap().as_str(),
        "https://example.invalid/a"
    );
    assert_eq!(
        sources.targets(&id, text)[1].instance(),
        links[1].instance()
    );
}
