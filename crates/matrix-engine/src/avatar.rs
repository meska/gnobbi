//! What the window needs to show avatars, free of SDK types: the worker fills the slot once its
//! session is up and empties it when it ends, so the window never keeps a client alive.
use std::sync::{Arc, Mutex};

/// A decoded avatar, straight RGBA; `None` when there is none, so the initials stay.
pub struct AvatarPicture {
    pub mxc: String,
    pub rgba: Option<(u32, u32, Vec<u8>)>,
    /// With no picture: the failure was transient (offline, timeout, 5xx), ask again later.
    pub retry: bool,
    /// Every frame of an animated picture (`rgba` is then its first); only `request_animated` fills it.
    pub anim: Option<crate::anim::Frames>,
}

pub trait AvatarSource: Send + Sync {
    /// Starts the download and decode away from the caller; the answer arrives on `out`.
    fn request(&self, mxc: String, out: std::sync::mpsc::Sender<AvatarPicture>);
    /// The big thumbnail for the profile card; the answer's `mxc` is `large_key(mxc)`.
    fn request_large(&self, mxc: String, out: std::sync::mpsc::Sender<AvatarPicture>);
    /// Like `request`, but the picture as uploaded instead of the small thumbnail (to retouch the own avatar). Not cached.
    fn request_original(&self, mxc: String, out: std::sync::mpsc::Sender<AvatarPicture>);
    /// The picture as uploaded with all its frames, when it is a small animated GIF/WebP/APNG; nothing is sent otherwise. Not cached.
    fn request_animated(&self, mxc: String, out: std::sync::mpsc::Sender<AvatarPicture>);
}

/// Key of the big thumbnail of `mxc` in the window's picture map.
pub fn large_key(mxc: &str) -> String {
    format!("{mxc}#large")
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
