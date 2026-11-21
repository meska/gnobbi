//! Host-side test of the purpose -> UIKit traits mapping (the iOS view itself needs a device).
#![allow(dead_code)]

// Stand-in for `crate::window::ImePurpose` (same variants), so the iOS module compiles on the host.
mod window {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ImePurpose {
        Normal,
        Password,
        Terminal,
        Username,
        Url,
        LoginPassword,
    }
}

#[path = "../src/platform_impl/ios/ime_traits.rs"]
mod ime_traits;

use ime_traits::{traits, Credential};
use window::ImePurpose::*;

#[test]
fn only_the_sign_in_fields_are_backed_by_a_native_field() {
    assert_eq!(traits(Username).credential, Some(Credential::Username));
    assert_eq!(traits(LoginPassword).credential, Some(Credential::Password));
    // Composer, search, passphrase, server, terminal: the main view alone, so no Passwords bar there.
    for p in [Normal, Password, Terminal, Url] {
        assert_eq!(traits(p).credential, None, "{p:?}");
    }
}

#[test]
fn only_the_server_asks_for_the_url_content_type() {
    for p in [Normal, Password, Terminal, Username, Url, LoginPassword] {
        assert_eq!(traits(p).url, p == Url, "{p:?}");
    }
}

#[test]
fn secrets_stay_internal_and_raw() {
    assert!(traits(Password).secret && traits(LoginPassword).secret);
    assert!(!traits(Username).secret && !traits(Normal).secret);
    assert!(traits(Username).raw && traits(Url).raw && traits(Password).raw && !traits(Normal).raw);
}
