//! Bounded, in-memory attachment data. No paths, SDK types or payload diagnostics.
use matrix_domain::{EventId, RoomScope};
use std::collections::VecDeque;

pub enum MediaCommand {
    Send {
        scope: RoomScope,
        transaction: matrix_domain::TransactionId,
        bytes: Vec<u8>,
    },
    Download {
        tag: crate::RequestTag,
        event: EventId,
        /// The full-resolution original for the large view, not the bubble thumbnail.
        original: bool,
    },
    /// Download the original video to a private temp file for the system player.
    OpenVideo {
        tag: crate::RequestTag,
        event: EventId,
    },
    /// Download an `m.audio` event through the SDK and decode it once for the in-app player:
    /// the answer carries both the samples and the waveform of those same samples.
    OpenAudio {
        tag: crate::RequestTag,
        event: EventId,
    },
    /// Save the original of any attachment to `path` (chosen by the user); runs beside the other reads.
    Save {
        tag: crate::RequestTag,
        event: EventId,
        path: std::path::PathBuf,
    },
    /// The thumbnail of a video row; the answer is a `Downloaded` event like an image's.
    VideoThumbnail {
        tag: crate::RequestTag,
        event: EventId,
    },
}

pub enum MediaEvent {
    Sent {
        scope: RoomScope,
        transaction: matrix_domain::TransactionId,
        result: Result<EventId, MediaError>,
    },
    Downloaded {
        tag: crate::RequestTag,
        event: EventId,
        result: Result<DownloadedPng, MediaError>,
    },
    VideoDownloaded {
        tag: crate::RequestTag,
        event: EventId,
        result: Result<VideoFile, MediaError>,
    },
    /// The decoded audio of an `m.audio` event, ready to play and to draw as a waveform.
    AudioOpened {
        tag: crate::RequestTag,
        event: EventId,
        result: Result<OpenedAudio, MediaError>,
    },
    Saved {
        tag: crate::RequestTag,
        event: EventId,
        result: Result<(), MediaError>,
    },
    /// Bytes received so far by a save or a video open; `total` is the size the sender declared.
    Progress {
        tag: crate::RequestTag,
        event: EventId,
        received: u64,
        total: Option<u64>,
    },
}

/// Decoded mono audio for the in-app player, plus the waveform of those same samples:
/// one decode feeds both, never a second pass.
pub struct OpenedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub waveform: Vec<f32>,
}

impl std::fmt::Debug for OpenedAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the samples themselves: user content.
        f.debug_struct("OpenedAudio")
            .field("samples", &self.samples.len())
            .field("sample_rate", &self.sample_rate)
            .field("waveform", &self.waveform.len())
            .finish()
    }
}

/// A plaintext video in the account's private temp dir; the file is removed on drop.
#[derive(Debug)]
pub struct VideoFile {
    path: std::path::PathBuf,
}

impl VideoFile {
    // Only the SDK download path builds one.
    #[cfg(feature = "sdk")]
    pub(crate) fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for VideoFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaError {
    InvalidImage,
    UnsupportedType,
    TooLarge,
    Cancelled,
    InvalidScope,
    LoggedOut,
    Deadline,
    Failed,
}
pub const MAX_MEDIA_BYTES: usize = 25 * 1024 * 1024;
/// Cap for saving one attachment or opening one video.
pub const MAX_SAVE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// Thumbnail and original are separate resources with separate cache entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaVariant {
    Thumbnail,
    Original,
}

pub struct DownloadedPng {
    /// Encoded bytes of the resource named by `variant`.
    pub variant: MediaVariant,
    pub bytes: Vec<u8>,
    pub preview: Preview,
}

/// Owned straight-alpha RGBA8; no SDK or UI objects cross the boundary.
pub struct Preview {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// All the frames when the picture is animated and within the limits; `rgba` is then the first one.
    pub anim: Option<crate::anim::Frames>,
    #[cfg(all(test, feature = "sdk"))]
    pub(crate) decoded_on: std::thread::ThreadId,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MediaKey {
    pub variant: MediaVariant,
    pub scope: RoomScope,
    pub event_id: EventId,
}

/// LRU byte cache. Caller owns one cache per active account and drops it on logout.
/// Keys include account/room/event even if a caller accidentally shares a cache.
pub struct MediaCache {
    budget: usize,
    used: usize,
    entries: VecDeque<(MediaKey, Vec<u8>)>,
}
impl MediaCache {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            entries: VecDeque::new(),
        }
    }
    pub fn clear_account(&mut self, account_id: matrix_domain::AccountId) {
        self.entries
            .retain(|(key, _)| key.scope.account_id != account_id);
        self.used = self.entries.iter().map(|(_, value)| value.len()).sum();
    }
    pub fn used_bytes(&self) -> usize {
        self.used
    }
    pub fn get(&mut self, key: &MediaKey) -> Option<&[u8]> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        self.entries.push_back(entry);
        self.entries.back().map(|(_, value)| value.as_slice())
    }
    pub fn insert(&mut self, key: MediaKey, data: Vec<u8>) -> bool {
        if data.is_empty() || data.len() > self.budget {
            return false;
        }
        if let Some(index) = self.entries.iter().position(|(k, _)| k == &key) {
            self.used -= self.entries.remove(index).expect("known index").1.len();
        }
        while self.used > self.budget - data.len() {
            self.used -= self
                .entries
                .pop_front()
                .expect("nonempty occupied cache")
                .1
                .len();
        }
        self.used += data.len();
        self.entries.push_back((key, data));
        true
    }
}
