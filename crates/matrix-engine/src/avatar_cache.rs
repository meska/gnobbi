//! Avatar thumbnails: fetched through the SDK, capped while received, with a small
//! private on-disk cache (dir 0700, files 0600) that is deleted at logout.
//! Avatars are public `mxc` images, so the cache holds exactly what the server sent.
//! A missing, refused or broken avatar is `Ok(None)`: the caller shows the initials.
use crate::auth::AccountSession;
use crate::media::MediaError;
use matrix_sdk::media::{
    DefaultMediaFetcher, MediaFormat, MediaRequestParameters, MediaThumbnailSettings,
};
use matrix_sdk::ruma::{
    MxcUri, api::client::media::get_content_thumbnail::v3::Method, events::room::MediaSource,
};
use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::{io::Write, time::Duration};

/// Requested thumbnail side in pixels (cropped square), not the original.
pub const AVATAR_PX: u32 = 96;
/// Cap on one thumbnail, enforced while the body is received.
pub const AVATAR_MAX_BYTES: usize = 256 * 1024;
/// Files kept on disk; the oldest go first.
pub const AVATAR_MAX_ENTRIES: usize = 512;
/// Simultaneous downloads.
pub const AVATAR_MAX_CONCURRENT: usize = 4;

pub struct AvatarCache {
    dir: PathBuf,
    max_bytes: usize,
    max_entries: usize,
    permits: tokio::sync::Semaphore,
    // One lock per file name: a second request for the same avatar waits, then hits the disk.
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    downloads: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl AvatarCache {
    pub fn new(dir: PathBuf) -> Self {
        Self::with_limits(
            dir,
            AVATAR_MAX_BYTES,
            AVATAR_MAX_ENTRIES,
            AVATAR_MAX_CONCURRENT,
        )
    }

    pub fn with_limits(
        dir: PathBuf,
        max_bytes: usize,
        max_entries: usize,
        concurrent: usize,
    ) -> Self {
        Self {
            dir,
            max_bytes,
            max_entries,
            permits: tokio::sync::Semaphore::new(concurrent),
            inflight: Mutex::default(),
            downloads: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// Network fetches started so far (cache hits are not counted).
    pub fn downloads(&self) -> usize {
        self.downloads.load(Ordering::Relaxed)
    }

    /// Highest number of fetches that ran at the same time.
    pub fn peak_concurrent(&self) -> usize {
        self.peak.load(Ordering::Relaxed)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The avatar thumbnail bytes for `mxc`, from disk when cached.
    pub async fn get(&self, client: &matrix_sdk::Client, mxc: &str) -> Option<Vec<u8>> {
        let uri = <&MxcUri>::from(mxc);
        // The file name comes from the validated URI: media id is [A-Za-z0-9_-], server name is sanitised.
        let (server, id) = uri.parts().ok()?;
        let name = format!("{}-{id}", server.as_str().replace([':', '[', ']'], "_"));
        let path = self.dir.join(&name);
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(bytes);
        }
        let lock = self
            .inflight
            .lock()
            .ok()?
            .entry(name.clone())
            .or_default()
            .clone();
        let _turn = lock.lock().await;
        let result = async {
            if let Ok(bytes) = std::fs::read(&path) {
                return Some(bytes);
            }
            let _permit = self.permits.acquire().await.ok()?;
            let bytes = self.fetch(client, uri).await?;
            self.store(&name, &bytes).ok()?;
            Some(bytes)
        }
        .await;
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.remove(&name);
        }
        result
    }

    async fn fetch(&self, client: &matrix_sdk::Client, uri: &MxcUri) -> Option<Vec<u8>> {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        self.downloads.fetch_add(1, Ordering::SeqCst);
        let request = MediaRequestParameters {
            source: MediaSource::Plain(uri.to_owned()),
            format: MediaFormat::Thumbnail(MediaThumbnailSettings::with_method(
                Method::Crop,
                AVATAR_PX.into(),
                AVATAR_PX.into(),
            )),
        };
        let config = client
            .request_config()
            .timeout(Some(Duration::from_secs(15)))
            .max_response_size(Some(self.max_bytes));
        // Calling the fetcher directly skips the SDK media store: only our private cache holds it.
        let bytes = DefaultMediaFetcher
            .fetch_media_content_with_request_config(client, &request, config)
            .await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        let bytes = bytes.ok()?;
        crate::media_sdk::mime_of(&bytes)?;
        Some(bytes)
    }

    fn store(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        match std::fs::DirBuilder::new().mode(0o700).create(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
        let tmp = self.dir.join(format!(".{name}.tmp"));
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?
            .write_all(bytes)?;
        std::fs::rename(&tmp, self.dir.join(name))?;
        let mut files: Vec<_> = std::fs::read_dir(&self.dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        files.sort();
        let excess = files.len().saturating_sub(self.max_entries);
        for (_, path) in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}

impl AccountSession {
    /// Avatar thumbnail for an `mxc` URI from `RoomInfo.avatar` or `HistorySender.avatar`.
    /// `Ok(None)` means show the initials. Needs a saved profile (the cache lives beside it).
    pub async fn avatar_thumbnail(&self, mxc: &str) -> Result<Option<Vec<u8>>, MediaError> {
        let client = self.client.as_ref().ok_or(MediaError::LoggedOut)?;
        Ok(match self.avatar_cache() {
            Some(cache) => cache.get(client, mxc).await,
            None => None,
        })
    }

    pub fn avatar_cache(&self) -> Option<&AvatarCache> {
        self.store.as_ref().map(|store| &*store.avatars)
    }
}
