//! What the window needs to show avatars, free of SDK types: the worker fills the slot once its
//! session is up and empties it when it ends, so the window never keeps a client alive.
use std::sync::{Arc, Mutex};

/// A decoded avatar, straight RGBA; `None` when there is none, so the initials stay.
pub struct AvatarPicture {
    pub mxc: String,
    pub rgba: Option<(u32, u32, Vec<u8>)>,
    /// With no picture: the failure was transient (offline, timeout, 5xx), ask again later.
    pub retry: bool,
}

pub trait AvatarSource: Send + Sync {
    /// Starts the download and decode away from the caller; the answer arrives on `out`.
    fn request(&self, mxc: String, out: std::sync::mpsc::Sender<AvatarPicture>);
}

pub type AvatarSlot = Arc<Mutex<Option<Box<dyn AvatarSource>>>>;

/// Servers (Tuwunel) send `Some("")` for "no avatar": that is no picture, same as `None`.
pub fn normalize_avatar(mxc: Option<String>) -> Option<String> {
    mxc.filter(|m| !m.trim().is_empty())
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize_avatar;

    #[test]
    fn empty_or_blank_avatar_is_absent() {
        assert_eq!(normalize_avatar(Some(String::new())), None);
        assert_eq!(normalize_avatar(Some("  ".into())), None);
        assert_eq!(normalize_avatar(None), None);
        assert_eq!(normalize_avatar(Some("mxc://s/a".into())).as_deref(), Some("mxc://s/a"));
    }
}
