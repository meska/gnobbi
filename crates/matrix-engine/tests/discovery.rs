#![cfg(feature = "sdk")]
use matrix_engine::discovery::HomeserverUrl;

#[test]
fn homeserver_url_rejects_unsafe_or_ambiguous_input() {
    for value in [
        "http://example.invalid",
        "ftp://example.invalid",
        "https://user:secret@example.invalid",
        "https://example.invalid?secret",
        "https://example.invalid/#secret",
        " https://example.invalid",
        "https://example.invalid\n",
        "https:\\example.invalid",
        "https://",
        "not a url",
        "https:example.invalid",
        "https:///example.invalid",
        "https://@example.invalid",
    ] {
        assert!(
            HomeserverUrl::parse(value).is_err(),
            "accepted unsafe fixture input"
        );
    }
    assert_eq!(
        HomeserverUrl::parse("https://example.invalid:8448/prefix")
            .unwrap()
            .as_str(),
        "https://example.invalid:8448/prefix/"
    );
}
