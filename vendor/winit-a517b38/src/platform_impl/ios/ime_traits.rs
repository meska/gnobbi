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
    /// The typed text is never kept or shown to UIKit (document stays empty).
    pub secret: bool,
    /// No autocapitalisation, autocorrection or spell checking.
    pub raw: bool,
    /// `UITextContentTypeURL` on the main view (server address); every other field leaves the content type unspecified, which
    /// keeps the Passwords bar away from the composer, search and the like.
    pub url: bool,
    pub credential: Option<Credential>,
}

pub(super) fn traits(purpose: ImePurpose) -> Traits {
    let t = |secret, raw, url, credential| Traits { secret, raw, url, credential };
    match purpose {
        ImePurpose::Terminal => t(false, true, false, None),
        ImePurpose::Url => t(false, true, true, None),
        // Other secrets (passphrase, recovery key): kept internal, no AutoFill.
        ImePurpose::Password => t(true, true, false, None),
        ImePurpose::Username => t(false, true, false, Some(Credential::Username)),
        ImePurpose::LoginPassword => t(true, true, false, Some(Credential::Password)),
        _ => t(false, false, false, None),
    }
}
