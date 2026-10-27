use matrix_desktop::{LinkConfirmation, SafeWebUrl, message_links};

#[test]
fn detection_does_not_rewrite_valid_url_punctuation_or_ipv6_authorities() {
    let text = "https://[::1] https://example.invalid/path(a) https://example.invalid/?q=wow! (https://example.invalid/path(a)).";
    let links = message_links(text);
    let originals: Vec<_> = links.iter().map(|l| &text[l.range.clone()]).collect();
    assert_eq!(
        originals,
        [
            "https://[::1]",
            "https://example.invalid/path(a)",
            "https://example.invalid/?q=wow!",
            "https://example.invalid/path(a)"
        ]
    );
}

#[test]
fn urls_require_nonempty_authority_and_have_a_bounded_size() {
    for raw in [
        "https:///example.invalid",
        "https:////example.invalid",
        "http://?x=1",
        "https://#fragment",
    ] {
        assert!(SafeWebUrl::parse(raw).is_none(), "accepted {raw:?}");
    }
    assert!(SafeWebUrl::parse(&format!("https://example.invalid/{}", "a".repeat(4096))).is_none());
}

#[test]
fn demo_has_safe_and_unsafe_link_fixtures_for_native_interaction() {
    use matrix_desktop::DemoHistory;
    use matrix_domain::{AccountId, RoomId, RoomScope};
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(0),
            room_id: RoomId::new("!s:fixture.invalid"),
        },
        5,
    );
    assert!(
        history.events()[4]
            .text
            .contains("https://example.invalid/docs?x=1&y=2")
    );
    assert!(history.events()[4].text.contains("javascript:alert(1)"));
    assert_eq!(message_links(&history.events()[4].text).len(), 1);
}

#[test]
fn opening_requires_confirmation_and_consumes_request_once() {
    use matrix_desktop::MessageLinkRequests;
    use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(71),
            room_id: RoomId::new("!links:test"),
        },
        id: TimelineId::Server(EventId::new("$links")),
    };
    let mut sources = MessageLinkRequests::default();
    let target = sources.targets(&id, "https://example.invalid/?q=$(touch%20NEVER);x=1")[0].clone();
    let mut request = LinkConfirmation::default();
    assert!(request.capture().is_none());
    assert!(sources.request(&target, &mut request));
    let captured = request.capture().unwrap();
    assert_eq!(
        request.pending().unwrap().as_str(),
        "https://example.invalid/?q=$(touch%20NEVER);x=1"
    );
    assert!(request.confirm_current(&captured, &sources).is_some());
    assert!(request.confirm_current(&captured, &sources).is_none());
    assert!(sources.request(&target, &mut request));
    let captured = request.capture().unwrap();
    request.cancel();
    assert!(request.confirm_current(&captured, &sources).is_none());
    assert!(sources.targets(&id, "javascript:alert(1)").is_empty());
    assert!(!sources.request(&target, &mut request));
    assert!(request.pending().is_none());
}

#[test]
fn message_links_keep_utf8_offsets_and_ignore_unsafe_schemes() {
    let text = "caffè https://example.invalid/a?x=1&y=2\n(javascript:alert(1)) (https://example.invalid/docs). file:///etc/passwd";
    let links = message_links(text);
    assert_eq!(links.len(), 2);
    assert_eq!(
        &text[links[0].range.clone()],
        "https://example.invalid/a?x=1&y=2"
    );
    assert_eq!(
        &text[links[1].range.clone()],
        "https://example.invalid/docs"
    );
    assert_eq!(links[0].url.as_str(), "https://example.invalid/a?x=1&y=2");
}

#[test]
fn only_explicit_http_https_urls_without_credentials_are_openable() {
    for raw in [
        "https://example.invalid/a?x=1&y=2#part",
        "http://localhost:8080/",
        "HTTPS://EXAMPLE.INVALID/a",
        "https://[::1]:443/",
        "https://città.invalid/caffè",
    ] {
        let url = SafeWebUrl::parse(raw).expect(raw);
        assert!(url.as_str().starts_with("http"));
    }
    for raw in [
        "",
        "example.invalid",
        "//example.invalid",
        "https:example.invalid",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,test",
        "matrix:u/test",
        "ftp://example.invalid",
        "https://",
        "https://u:p@example.invalid/",
        "https://@example.invalid/",
        " https://example.invalid",
        "https://example.invalid/ ",
        "https://example.invalid/\n",
        "https://example.invalid/\t",
        "https://example.invalid/\\evil",
        "https://example.invalid/\u{202e}txt",
        "https://example.invalid/\u{200b}",
    ] {
        assert!(SafeWebUrl::parse(raw).is_none(), "accepted {raw:?}");
    }
}

#[test]
fn sample_message_links_cover_each_web_url_and_skip_other_schemes() {
    let text = "Guarda https://example.invalid/docs/guida e poi https://example.invalid/archivio/2026/10/25/un-percorso-molto-lungo?x=1&y=2 oppure javascript:alert(1) e ftp://example.invalid/file";
    let found: Vec<_> = message_links(text)
        .iter()
        .map(|l| text[l.range.clone()].to_owned())
        .collect();
    assert_eq!(
        found,
        [
            "https://example.invalid/docs/guida",
            "https://example.invalid/archivio/2026/10/25/un-percorso-molto-lungo?x=1&y=2"
        ]
    );
}
