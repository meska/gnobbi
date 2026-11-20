use matrix_domain::{
    AccountId, RoomId, RoomScope,
    notifications::{MessagePreview, NotificationPolicy, PREVIEW_MAX, PreviewContent, notification_body},
};

fn msg(content: PreviewContent, sender: Option<&str>, direct: bool) -> MessagePreview {
    MessagePreview { content, sender: sender.map(Into::into), direct, sender_avatar: None }
}
fn text(t: &str) -> MessagePreview {
    msg(PreviewContent::Text(t.into()), Some("Anna"), true)
}
fn body(p: &MessagePreview) -> String {
    notification_body(Some(p), true, false)
}

fn scope(account: u64) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new("!private:fixture.invalid"),
    }
}

#[test]
fn notifications_are_off_by_default_and_content_free_when_on() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    assert!(!policy.enabled());
    assert!(policy.alert(&scope(1), None).is_none());
    policy.set_enabled(true);
    let alert = policy.alert(&scope(1), None).expect("enabled");
    assert_eq!(alert.title(), "Gnobbi");
    assert_eq!(alert.body(), "New messages");
}

#[test]
fn preview_opt_in_exposes_only_a_fixed_count_never_remote_content() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    assert!(!policy.count_preview());
    policy.set_enabled(true);
    policy.set_count_preview(true);
    assert_eq!(policy.alert(&scope(1), None).unwrap().body(), "1 new message");
    policy.set_count_preview(false);
    assert_eq!(policy.alert(&scope(1), None).unwrap().body(), "New messages");
}

#[test]
fn foreign_account_cannot_notify() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    assert!(policy.alert(&scope(2), None).is_none());
}

#[test]
fn an_alert_names_its_account_and_room_for_the_click() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    let alert = policy.alert(&scope(1), None).unwrap();
    assert_eq!(alert.target(), (None, &scope(1)));
    // The window gives the catalog ID: worker IDs repeat between profile folders.
    let alert = alert.from_account(AccountId::new(7), "gnobbi-test-bob".into());
    assert_eq!(alert.title(), "gnobbi-test-bob");
    assert_eq!(alert.target(), (Some(AccountId::new(7)), &scope(1)));
    assert_eq!(alert.body(), "New messages");
}

#[test]
fn truncates_on_grapheme_boundaries_with_ellipsis() {
    for unit in ["👨‍👩‍👧‍👦", "e\u{301}", "你", "🇮🇹", "a"] {
        let out = body(&text(&unit.repeat(300)));
        assert!(out.ends_with('…'));
        let head = out.trim_end_matches('…');
        assert_eq!(head.matches(unit).count() * unit.len(), head.len(), "{unit}: cut inside cluster");
        assert!(out.chars().count() >= 1 && head.len() <= unit.len() * PREVIEW_MAX);
    }
    let exact = "a".repeat(PREVIEW_MAX);
    assert_eq!(body(&text(&exact)), exact);
}

#[test]
fn whitespace_is_collapsed_and_controls_dropped() {
    assert_eq!(body(&text("  ciao \n\n  mondo\t! \u{202E}\u{7}x ")), "ciao mondo ! x");
}

#[test]
fn media_labels() {
    for (c, l) in [
        (PreviewContent::Image, "📷 Photo"),
        (PreviewContent::Video, "🎥 Video"),
        (PreviewContent::Voice, "🎤 Voice message"),
        (PreviewContent::Audio, "🎵 Audio"),
        (PreviewContent::File, "📎 File"),
        (PreviewContent::Sticker, "Sticker"),
        (PreviewContent::Location, "📍 Location"),
        (PreviewContent::Poll, "Poll"),
    ] {
        assert_eq!(body(&msg(c, None, true)), l);
    }
}

#[test]
fn sender_prefix_only_in_rooms() {
    let room = msg(PreviewContent::Text("ciao 😀".into()), Some("Anna"), false);
    assert_eq!(body(&room), "Anna: ciao 😀");
    assert_eq!(body(&text("ciao")), "ciao");
    assert_eq!(body(&msg(PreviewContent::Poll, Some("Anna"), false)), "Anna: Poll");
}

#[test]
fn switch_off_undecrypted_empty_and_other_give_the_fixed_body() {
    assert_eq!(notification_body(Some(&text("segreto")), false, false), "New messages");
    for p in [
        msg(PreviewContent::Undecrypted, Some("Anna"), false),
        msg(PreviewContent::Other, None, true),
        text(" \n "),
    ] {
        assert_eq!(body(&p), "New messages");
    }
    assert_eq!(notification_body(None, true, false), "New messages");
}

#[test]
fn policy_preview_defaults_on_and_shows_only_the_latest_message() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    assert!(policy.message_preview());
    assert_eq!(policy.alert(&scope(1), Some(text("uno"))).unwrap().body(), "uno");
    assert_eq!(policy.alert(&scope(1), Some(text("due"))).unwrap().body(), "due");
    policy.set_message_preview(false);
    assert_eq!(policy.alert(&scope(1), Some(text("due"))).unwrap().body(), "New messages");
}

#[test]
fn avatar_prefers_sender_then_room_then_none() {
    let scope = matrix_domain::RoomScope { account_id: matrix_domain::AccountId::new(1), room_id: matrix_domain::RoomId::new("!r:x") };
    let mut policy = NotificationPolicy::new(scope.account_id);
    policy.set_enabled(true);
    let with = |a: Option<&str>| policy.alert(&scope, Some(MessagePreview { sender_avatar: a.map(Into::into), ..msg(PreviewContent::Sticker, None, false) })).unwrap();
    assert_eq!(with(Some("mxc://s/a")).avatar_mxc(Some("mxc://s/room")), Some("mxc://s/a"));
    assert_eq!(with(None).avatar_mxc(Some("mxc://s/room")), Some("mxc://s/room"));
    assert_eq!(with(None).avatar_mxc(None), None);
    assert_eq!(policy.alert(&scope, None).unwrap().avatar_mxc(None), None);
}
