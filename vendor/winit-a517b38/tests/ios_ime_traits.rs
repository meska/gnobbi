//! Host-side test of the purpose -> UIKit traits mapping (the iOS view itself needs a device).
#![allow(dead_code)]

// Stand-in for `crate::window::ImePurpose` (same variants), so the iOS module compiles on the host.
mod window {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ImePurpose {
        Normal,
        Message,
        Password,
        Terminal,
        Username,
        Url,
        LoginPassword,
        Search,
    }
}

#[path = "../src/platform_impl/ios/ime_traits.rs"]
mod ime_traits;

use ime_traits::{split_autofill, traits, Credential};
use window::ImePurpose::*;

#[test]
fn only_the_sign_in_fields_are_backed_by_a_native_field() {
    assert_eq!(traits(Username).credential, Some(Credential::Username));
    assert_eq!(traits(LoginPassword).credential, Some(Credential::Password));
    // Composer, search, passphrase, server, terminal: the main view alone, so no Passwords bar there.
    for p in [Normal, Password, Terminal, Url, Search] {
        assert_eq!(traits(p).credential, None, "{p:?}");
    }
}

#[test]
fn only_the_server_asks_for_the_url_content_type() {
    for p in [Normal, Password, Terminal, Username, Url, LoginPassword, Search] {
        assert_eq!(traits(p).url, p == Url, "{p:?}");
    }
}

#[test]
fn secrets_stay_internal_and_raw() {
    assert!(traits(Password).secret && traits(LoginPassword).secret);
    assert!(!traits(Username).secret && !traits(Normal).secret);
    assert!(traits(Username).raw && traits(Url).raw && traits(Password).raw && !traits(Normal).raw);
}

#[test]
fn images_require_explicit_message_purpose() {
    for purpose in [Normal, Message, Password, Terminal, Username, Url, LoginPassword, Search] {
        assert_eq!(traits(purpose).images, purpose == Message);
    }
    assert!(!traits(Message).secret);
    assert_eq!(traits(Message).credential, None);
}

fn some(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[test]
fn autofill_values_go_to_their_own_field() {
    assert_eq!(split_autofill(some("afuser"), some("hunter2"), None), (some("afuser"), some("hunter2")));
    assert_eq!(split_autofill(some("afuser"), None, None), (some("afuser"), None), "username only: password untouched");
    assert_eq!(split_autofill(None, some("hunter2"), None), (None, some("hunter2")));
}

#[test]
fn username_glued_before_the_password_is_cut_off() {
    // Second AutoFill on the password field: username + password both landed in the password proxy.
    assert_eq!(split_autofill(None, some("afuserhunter2"), Some("afuser")), (None, some("hunter2")));
    assert_eq!(split_autofill(some("afuser"), some("afuserhunter2"), None), (some("afuser"), some("hunter2")));
    assert_eq!(split_autofill(None, some("afuser"), Some("afuser")), (None, None), "only the username arrived there");
    assert_eq!(split_autofill(None, some("hunter2"), Some("afuser")), (None, some("hunter2")));
    assert_eq!(split_autofill(None, some("hunter2"), Some("")), (None, some("hunter2")), "no known username: nothing to cut");
}

#[test]
fn search_and_the_sign_in_fields_never_autocorrect() {
    for p in [Search, Username, Url, LoginPassword, Password, Terminal] {
        assert!(traits(p).raw, "{p:?}");
    }
    // Message and Normal keep autocorrection: the composer needs it.
    for p in [Normal, Message] {
        assert!(!traits(p).raw, "{p:?}");
    }
    for p in [Normal, Message, Password, Terminal, Username, Url, LoginPassword] {
        assert!(!traits(p).search, "{p:?}");
    }
    assert!(traits(Search).search && !traits(Search).secret && traits(Search).credential.is_none() && !traits(Search).images);
}
