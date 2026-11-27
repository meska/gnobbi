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
    /// Fetch the original of any attachment into the in-memory auto cache (never to disk), refusing more than
    /// `max_bytes` while it is received; Open, Save and the picture read it from there afterwards.
    Prefetch {
        tag: crate::RequestTag,
        event: EventId,
        max_bytes: usize,
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
    /// The answer of a `Prefetch`: the bytes are in the auto cache when it is `Ok`.
    Prefetched {
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

/// Budget of the auto cache: what the app fetched by itself (see `MediaCommand::Prefetch`), in memory only.
pub const AUTO_BUDGET: usize = if cfg!(any(target_os = "ios", target_os = "android")) { 64 << 20 } else { 128 << 20 };

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

/// The auto cache: one per process, keyed by account, room and event, so a hit can only be served to the scope that asked.
fn auto_cache() -> &'static std::sync::Mutex<MediaCache> {
    static AUTO: std::sync::OnceLock<std::sync::Mutex<MediaCache>> = std::sync::OnceLock::new();
    AUTO.get_or_init(|| std::sync::Mutex::new(MediaCache::new(AUTO_BUDGET)))
}

fn auto_key(scope: &RoomScope, event: &EventId) -> MediaKey {
    MediaKey { variant: MediaVariant::Original, scope: scope.clone(), event_id: event.clone() }
}

/// The original of `event` if the app fetched it by itself and it is still in memory (a hit is the most recent again).
pub(crate) fn auto_get(scope: &RoomScope, event: &EventId) -> Option<Vec<u8>> {
    auto_cache().lock().ok()?.get(&auto_key(scope, event)).map(<[u8]>::to_vec)
}

/// Keeps `bytes` for `event`, evicting the least recently used ones; false when they do not fit the budget at all.
pub(crate) fn auto_put(scope: &RoomScope, event: &EventId, bytes: Vec<u8>) -> bool {
    auto_cache().lock().is_ok_and(|mut cache| cache.insert(auto_key(scope, event), bytes))
}

/// For the integration tests, which cannot reach `auto_clear`.
#[doc(hidden)]
pub fn auto_clear_for_test(account: matrix_domain::AccountId) {
    auto_clear(account);
}

/// Forgets everything of an account (logout, end of its session).
pub(crate) fn auto_clear(account: matrix_domain::AccountId) {
    if let Ok(mut cache) = auto_cache().lock() {
        cache.clear_account(account);
    }
}

#[cfg(test)]
mod auto_tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId};

    fn scope(account: u64) -> RoomScope {
        RoomScope { account_id: AccountId::new(account), room_id: RoomId::new("!auto:fixture.invalid") }
    }

    #[test]
    fn the_auto_cache_serves_only_the_scope_that_stored_and_forgets_a_logged_out_account() {
        let (a, b) = (scope(9101), scope(9102));
        let event = EventId::new("$doc:fixture.invalid");
        assert!(auto_get(&a, &event).is_none());
        assert!(auto_put(&a, &event, vec![1, 2, 3]));
        assert_eq!(auto_get(&a, &event), Some(vec![1, 2, 3]));
        assert!(auto_get(&b, &event).is_none(), "another account never reads it");
        auto_clear(a.account_id);
        assert!(auto_get(&a, &event).is_none());
    }

    #[test]
    fn the_auto_cache_keeps_to_its_budget_dropping_the_least_recently_used_and_refusing_what_never_fits() {
        assert_eq!(AUTO_BUDGET, if cfg!(any(target_os = "ios", target_os = "android")) { 64 << 20 } else { 128 << 20 });
        let s = scope(9103);
        let (one, two, three) = (EventId::new("$1"), EventId::new("$2"), EventId::new("$3"));
        let third = AUTO_BUDGET / 3 + 1;
        assert!(auto_put(&s, &one, vec![1; third]));
        assert!(auto_put(&s, &two, vec![2; third]));
        assert!(auto_get(&s, &one).is_some(), "reading one makes it the most recent");
        assert!(auto_put(&s, &three, vec![3; third]));
        assert!(auto_get(&s, &two).is_none(), "the least recently used went");
        assert!(auto_get(&s, &one).is_some() && auto_get(&s, &three).is_some());
        assert!(!auto_put(&s, &EventId::new("$huge"), vec![0; AUTO_BUDGET + 1]));
        assert!(auto_get(&s, &EventId::new("$huge")).is_none());
        auto_clear(s.account_id);
    }
}
