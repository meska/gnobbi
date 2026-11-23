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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::{io::Write, time::Duration};

/// Requested thumbnail side in pixels (cropped square), not the original.
pub const AVATAR_PX: u32 = 96;
/// Cap on one thumbnail, enforced while the body is received.
pub const AVATAR_MAX_BYTES: usize = 256 * 1024;
/// The big picture of a profile card (about 110 pt at 3x), with its own cap.
pub const AVATAR_LARGE_PX: u32 = 384;
pub const AVATAR_LARGE_MAX_BYTES: usize = 1024 * 1024;
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
        self.try_get(client, mxc).await.ok()
    }

    /// Like `get`; `Err(true)` is a transient failure (no connection, timeout, 5xx, 429)
    /// worth asking again later, `Err(false)` is final (missing, refused, junk, too big).
    pub async fn try_get(&self, client: &matrix_sdk::Client, mxc: &str) -> Result<Vec<u8>, bool> {
        self.try_get_sized(client, mxc, AVATAR_PX, self.max_bytes, "").await
    }

    /// The big thumbnail of the profile card; cached beside the small one.
    pub async fn try_get_large(&self, client: &matrix_sdk::Client, mxc: &str) -> Result<Vec<u8>, bool> {
        self.try_get_sized(client, mxc, AVATAR_LARGE_PX, AVATAR_LARGE_MAX_BYTES, "-large").await
    }

    async fn try_get_sized(&self, client: &matrix_sdk::Client, mxc: &str, px: u32, max_bytes: usize, suffix: &str) -> Result<Vec<u8>, bool> {
        let uri = <&MxcUri>::from(mxc);
        crate::media_sdk::checked_source(MediaSource::Plain(uri.to_owned())).map_err(|_| false)?;
        // The file name comes from the validated URI: media id is [A-Za-z0-9_-], server name is sanitised.
        let (server, id) = uri.parts().map_err(|_| false)?;
        let name = format!("{}-{id}{suffix}", server.as_str().replace([':', '[', ']'], "_"));
        let path = self.dir.join(&name);
        if let Ok(bytes) = std::fs::read(&path) {
            return Ok(bytes);
        }
        let lock = self
            .inflight
            .lock()
            .map_err(|_| false)?
            .entry(name.clone())
            .or_default()
            .clone();
        let _turn = lock.lock().await;
        let result = async {
            if let Ok(bytes) = std::fs::read(&path) {
                return Ok(bytes);
            }
            let _permit = self.permits.acquire().await.map_err(|_| false)?;
            let thumbnail = MediaFormat::Thumbnail(MediaThumbnailSettings::with_method(Method::Crop, px.into(), px.into()));
            let bytes = self.fetch(client, uri, thumbnail, max_bytes).await?;
            self.store(&name, &bytes).map_err(|_| false)?;
            Ok(bytes)
        }
        .await;
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.remove(&name);
        }
        result
    }

    /// The picture as uploaded, for retouching: same fetch as the thumbnail, capped at the media limit while received, never stored.
    pub async fn original(&self, client: &matrix_sdk::Client, mxc: &str) -> Result<Vec<u8>, bool> {
        self.original_capped(client, mxc, crate::media::MAX_MEDIA_BYTES).await
    }

    pub async fn original_capped(&self, client: &matrix_sdk::Client, mxc: &str, max_bytes: usize) -> Result<Vec<u8>, bool> {
        let uri = <&MxcUri>::from(mxc);
        crate::media_sdk::checked_source(MediaSource::Plain(uri.to_owned())).map_err(|_| false)?;
        let _permit = self.permits.acquire().await.map_err(|_| false)?;
        self.fetch(client, uri, MediaFormat::File, max_bytes).await
    }

    async fn fetch(&self, client: &matrix_sdk::Client, uri: &MxcUri, format: MediaFormat, max_bytes: usize) -> Result<Vec<u8>, bool> {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        self.downloads.fetch_add(1, Ordering::SeqCst);
        let request = MediaRequestParameters { source: MediaSource::Plain(uri.to_owned()), format };
        let config = client
            .request_config()
            .timeout(Some(Duration::from_secs(15)))
            .max_response_size(Some(max_bytes));
        // Calling the fetcher directly skips the SDK media store: only our private cache holds it.
        let bytes = DefaultMediaFetcher
            .fetch_media_content_with_request_config(client, &request, config)
            .await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        let bytes = bytes.map_err(|e| is_transient(&e))?;
        crate::media_sdk::mime_of(&bytes).ok_or(false)?;
        Ok(bytes)
    }

    fn store(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        match crate::fsx::create_private_dir(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
        let tmp = self.dir.join(format!(".{name}.tmp"));
        crate::fsx::private_file(std::fs::OpenOptions::new().write(true).create(true).truncate(true))
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

// shortcut: a 502 from a proxy with a non-Matrix body counts as final, upgrade if it shows up.
fn is_transient(e: &matrix_sdk::Error) -> bool {
    match e {
        matrix_sdk::Error::Http(h) => http_transient(h),
        _ => false,
    }
}

fn http_transient(e: &matrix_sdk::HttpError) -> bool {
    use matrix_sdk::HttpError;
    match e {
        HttpError::Reqwest(_) => true,
        // The SDK caches a failed `/versions` call and hands it back wrapped.
        HttpError::Cached(inner) => http_transient(inner),
        _ => e.as_client_api_error().is_some_and(|api| {
            api.status_code.is_server_error() || api.status_code.as_u16() == 429
        }),
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

/// The worker's avatar source: the account client, the shared cache and the worker runtime.
#[derive(Clone)]
pub struct AvatarFetcher {
    client: matrix_sdk::Client,
    cache: Arc<AvatarCache>,
    runtime: tokio::runtime::Handle,
}

impl AvatarFetcher {
    pub fn new(session: &AccountSession, runtime: tokio::runtime::Handle) -> Option<Self> {
        Some(Self {
            client: session.client.clone()?,
            cache: session.store.as_ref()?.avatars.clone(),
            runtime,
        })
    }
}

impl crate::avatar::AvatarSource for AvatarFetcher {
    fn request(&self, mxc: String, out: std::sync::mpsc::Sender<crate::avatar::AvatarPicture>) {
        let me = self.clone();
        self.runtime.spawn(async move {
            let (rgba, retry) = match me.cache.try_get(&me.client, &mxc).await {
                Ok(bytes) => (
                    tokio::task::spawn_blocking(move || decode(&bytes))
                        .await
                        .ok()
                        .flatten(),
                    false,
                ),
                Err(retry) => (None, retry),
            };
            let _ = out.send(crate::avatar::AvatarPicture { mxc, rgba, retry, anim: None });
        });
    }

    fn request_large(&self, mxc: String, out: std::sync::mpsc::Sender<crate::avatar::AvatarPicture>) {
        let me = self.clone();
        self.runtime.spawn(async move {
            let (rgba, retry) = match me.cache.try_get_large(&me.client, &mxc).await {
                Ok(bytes) => (tokio::task::spawn_blocking(move || decode(&bytes)).await.ok().flatten(), false),
                Err(retry) => (None, retry),
            };
            let _ = out.send(crate::avatar::AvatarPicture { mxc: crate::avatar::large_key(&mxc), rgba, retry, anim: None });
        });
    }

    fn request_original(&self, mxc: String, out: std::sync::mpsc::Sender<crate::avatar::AvatarPicture>) {
        let me = self.clone();
        self.runtime.spawn(async move {
            let (rgba, retry) = match me.cache.original(&me.client, &mxc).await {
                Ok(bytes) => {
                    let decoded = tokio::task::spawn_blocking(move || crate::profile::decode_bytes(&bytes).ok().map(|p| p.into_rgba8()));
                    (decoded.await.ok().flatten().map(|p| (p.width(), p.height(), p.into_raw())), false)
                }
                Err(retry) => (None, retry),
            };
            let _ = out.send(crate::avatar::AvatarPicture { mxc, rgba, retry, anim: None });
        });
    }

    fn request_animated(&self, mxc: String, out: std::sync::mpsc::Sender<crate::avatar::AvatarPicture>) {
        let me = self.clone();
        self.runtime.spawn(async move {
            let Ok(bytes) = me.cache.original_capped(&me.client, &mxc, ANIMATED_MAX_BYTES).await else { return };
            let Ok(Some(anim)) = tokio::task::spawn_blocking(move || crate::anim::decode(&bytes)).await else { return };
            let first = (anim.width, anim.height, anim.rgba[0].clone());
            let _ = out.send(crate::avatar::AvatarPicture { mxc, rgba: Some(first), retry: false, anim: Some(anim) });
        });
    }
}

/// A sticker animated on hover is a small file; the cap is enforced while it is received.
const ANIMATED_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Square thumbnails are 96 px; anything much larger is not an avatar we asked for.
fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

#[cfg(test)]
mod decode_tests {
    #[test]
    fn decodes_a_small_png_and_refuses_junk_and_oversize() {
        let mut png = Vec::new();
        let write = |w, h, out: &mut Vec<u8>| {
            image::RgbaImage::new(w, h)
                .write_to(&mut std::io::Cursor::new(out), image::ImageFormat::Png)
                .unwrap()
        };
        write(96, 96, &mut png);
        let (w, h, rgba) = super::decode(&png).unwrap();
        assert_eq!((w, h, rgba.len()), (96, 96, 96 * 96 * 4));
        assert!(super::decode(b"not an image").is_none());
        let mut big = Vec::new();
        write(600, 10, &mut big);
        assert!(super::decode(&big).is_none());
    }
}
