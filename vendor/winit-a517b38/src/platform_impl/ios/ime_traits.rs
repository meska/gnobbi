//! Which UIKit text-input traits a field purpose asks for (pure, host-tested in `tests/ios_ime_traits.rs`).
use crate::window::ImePurpose;

/// Sign-in fields that are backed by a real, hidden `UITextField` while they have the focus: only a native field pair lets
/// Passwords AutoFill fill username and password together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Credential {
    Username,
    Password,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Traits {
    /// Only an explicit message composer accepts images or adaptive glyphs.
    pub images: bool,
    /// The typed text is never kept or shown to UIKit (document stays empty).
    pub secret: bool,
    /// No autocapitalisation, autocorrection or spell checking.
    pub raw: bool,
    /// A search field: on top of `raw`, no smart quotes/dashes/insert-delete and a Search return key. Nothing in the field may
    /// change by itself, least of all when it loses the focus (the results are looked up from its text).
    pub search: bool,
    /// `UITextContentTypeURL` on the main view (server address); every other field leaves the content type unspecified, which
    /// keeps the Passwords bar away from the composer, search and the like.
    pub url: bool,
    pub credential: Option<Credential>,
}

pub(super) fn traits(purpose: ImePurpose) -> Traits {
    let t = |secret, raw, url, credential| Traits { images: purpose == ImePurpose::Message, search: purpose == ImePurpose::Search, secret, raw, url, credential };
    match purpose {
        ImePurpose::Terminal => t(false, true, false, None),
        ImePurpose::Search => t(false, true, false, None),
        ImePurpose::Url => t(false, true, true, None),
        // Other secrets (passphrase, recovery key): kept internal, no AutoFill.
        ImePurpose::Password => t(true, true, false, None),
        ImePurpose::Username => t(false, true, false, Some(Credential::Username)),
        ImePurpose::LoginPassword => t(true, true, false, Some(Credential::Password)),
        _ => t(false, false, false, None),
    }
}

/// Routes what AutoFill left in the two hidden fields to the two sign-in fields: `(username, password)`. Each value goes only to
/// its own field and is never joined. Seen on a real iPhone: the password field received the username followed by the password;
/// that username (the one in the username field, else `last_user`) is cut off, and a password field holding only the username gets nothing.
pub(super) fn split_autofill(user: Option<String>, pass: Option<String>, last_user: Option<&str>) -> (Option<String>, Option<String>) {
    let known = user.as_deref().or(last_user).filter(|u| !u.is_empty());
    let pass = pass.and_then(|p| match known {
        Some(u) if p == u => None,
        Some(u) => Some(p.strip_prefix(u).map_or(p.clone(), str::to_owned)),
        None => Some(p),
    });
    (user, pass)
}
