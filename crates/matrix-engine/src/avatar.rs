//! What the window needs to show avatars, free of SDK types: the worker fills the slot once its
//! session is up and empties it when it ends, so the window never keeps a client alive.
use std::sync::{Arc, Mutex};

/// A decoded avatar, straight RGBA; `None` when there is none, so the initials stay.
pub struct AvatarPicture {
    pub mxc: String,
    pub rgba: Option<(u32, u32, Vec<u8>)>,
}

pub trait AvatarSource: Send + Sync {
    /// Starts the download and decode away from the caller; the answer arrives on `out`.
    fn request(&self, mxc: String, out: std::sync::mpsc::Sender<AvatarPicture>);
}

pub type AvatarSlot = Arc<Mutex<Option<Box<dyn AvatarSource>>>>;
