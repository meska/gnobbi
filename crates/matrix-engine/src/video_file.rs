//! Video download for the system player: capped while received, with progress and
//! cancellation, then written as plaintext to a private temp file (dir 0700, file
//! 0600) that is deleted when `VideoFile` is dropped or on the next open of the dir.
use crate::auth::AccountSession;
use crate::{RequestTag, media::MediaError};
use matrix_domain::EventId;
use matrix_sdk::media::{DefaultMediaFetcher, MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::events::room::MediaSource;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::{io::Write, time::Duration};

/// Default cap for one video; the caller may pass a lower one.
pub const MAX_VIDEO_BYTES: usize = crate::media::MAX_SAVE_BYTES;

pub use crate::media::VideoFile;

/// Create (0700) or reuse `<profile>/video-tmp`, deleting leftovers of a crashed run.
fn private_dir(profile: &Path) -> std::io::Result<PathBuf> {
    let dir = profile.join("video-tmp");
    match crate::fsx::create_private_dir(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::fsx::tighten_dir(&dir)?;
            for entry in std::fs::read_dir(&dir)? {
                let _ = std::fs::remove_file(entry?.path());
            }
        }
        Err(error) => return Err(error),
    }
    Ok(dir)
}

/// Extensions «Open» may hand to the system's program: documents, pictures, audio, video and archives. Never anything
/// that runs (exe, app, sh, html...), and svg is left out because a browser runs its scripts.
const OPENABLE: &[&str] = &[
    "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp", "png", "jpg", "jpeg",
    "gif", "webp", "heic", "mp3", "ogg", "opus", "m4a", "wav", "flac", "mp4", "mov", "m4v", "mkv", "webm", "avi", "zip", "7z",
];

/// The extension from the file name when it is one of `OPENABLE`: the sender names the file, so it only counts from the list.
pub fn openable_name(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    OPENABLE.iter().copied().find(|e| *e == ext)
}

/// The extension of the temp file for «Open», from the type or else the name; `None` when neither is safe to open.
fn extension(mime: Option<&str>, name: Option<&str>) -> Option<&'static str> {
    Some(match mime.map(|value| value.split(';').next().unwrap_or_default().trim()) {
        Some("video/webm") => "webm",
        Some("video/quicktime") => "mov",
        Some("video/x-matroska") => "mkv",
        Some("audio/ogg" | "audio/opus") => "ogg",
        Some("audio/mpeg") => "mp3",
        Some("audio/mp4" | "audio/aac") => "m4a",
        Some("audio/wav" | "audio/x-wav") => "wav",
        Some("audio/flac") => "flac",
        // Files opened from a chat card: the system opener picks the program by the extension.
        Some("application/pdf") => "pdf",
        Some("application/zip") => "zip",
        Some("text/plain") => "txt",
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/gif") => "gif",
        Some("image/webp") => "webp",
        Some("application/msword") => "doc",
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document") => "docx",
        Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet") => "xlsx",
        Some(m) if m.starts_with("video/") => "mp4",
        // A generic type (octet-stream, a office type not listed above, none at all): the name decides, from the list only.
        _ => return name.and_then(openable_name),
    })
}

/// Fetch `source` through the SDK (auth and E2EE decryption included), refusing more
/// than `max_bytes` while the body is received. `progress` gets the bytes received so
/// far; a cancelled `tag` aborts the request. No media-store (cache) access.
pub(crate) async fn fetch_bytes(
    client: &matrix_sdk::Client,
    source: MediaSource,
    max_bytes: usize,
    tag: &RequestTag,
    mut progress: impl FnMut(usize),
) -> Result<Vec<u8>, MediaError> {
    // shortcut: one leaked 8-byte counter per download, `RequestConfig` is `Copy`; replace if the SDK gains a progress hook.
    let received: &'static AtomicUsize = Box::leak(Box::new(AtomicUsize::new(0)));
    let request = MediaRequestParameters {
        source: crate::media_sdk::checked_source(source)?,
        format: MediaFormat::File,
    };
    let config = client
        .request_config()
        .timeout(Some(Duration::MAX))
        .max_response_size(Some(max_bytes))
        .response_progress(Some(received));
    let fetch =
        DefaultMediaFetcher.fetch_media_content_with_request_config(client, &request, config);
    let mut fetch = std::pin::pin!(fetch);
    let mut reported = 0;
    let bytes = loop {
        // Dropping `fetch` on cancellation closes the connection.
        if tag.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        match tokio::time::timeout(Duration::from_millis(20), &mut fetch).await {
            Ok(result) => {
                break result.map_err(|error| match error {
                    matrix_sdk::Error::Http(error)
                        if matches!(*error, matrix_sdk::HttpError::ResponseTooLarge { .. }) =>
                    {
                        MediaError::TooLarge
                    }
                    _ => MediaError::Failed,
                })?;
            }
            Err(_) => {
                let now = received.load(Ordering::Relaxed);
                if now != reported {
                    reported = now;
                    progress(now);
                }
            }
        }
    };
    if tag.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    if bytes.len() != reported {
        progress(bytes.len());
    }
    Ok(bytes)
}

/// [`fetch_bytes`], then a plaintext private temp file for the system player, named with `ext`.
pub(crate) async fn fetch_to_file(
    client: &matrix_sdk::Client,
    source: MediaSource,
    ext: &str,
    profile: &Path,
    max_bytes: usize,
    tag: &RequestTag,
    progress: impl FnMut(usize),
) -> Result<VideoFile, MediaError> {
    let bytes = fetch_bytes(client, source, max_bytes, tag, progress).await?;
    write_temp(&bytes, ext, profile)
}

/// A plaintext private temp file with `bytes`, named with `ext`: the user asked to open it.
fn write_temp(bytes: &[u8], ext: &str, profile: &Path) -> Result<VideoFile, MediaError> {
    static NAMES: AtomicU64 = AtomicU64::new(0);
    // A zero-byte body is a broken attachment: no player can open an empty file.
    if bytes.is_empty() {
        return Err(MediaError::Failed);
    }
    let dir = private_dir(profile).map_err(|_| MediaError::Failed)?;
    let path = dir.join(format!(
        "{}-{}.{}",
        std::process::id(),
        NAMES.fetch_add(1, Ordering::Relaxed),
        ext
    ));
    let file = VideoFile::new(path);
    crate::fsx::private_file(std::fs::OpenOptions::new().write(true).create_new(true))
        .open(file.path())
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|_| MediaError::Failed)?;
    Ok(file)
}

impl AccountSession {
    /// See [`download_video`].
    pub async fn download_video(
        &self,
        tag: &RequestTag,
        event: &EventId,
        profile: &Path,
        max_bytes: usize,
        progress: impl FnMut(usize),
    ) -> Result<VideoFile, MediaError> {
        let client = self.client.as_ref().ok_or(MediaError::LoggedOut)?;
        download_video(
            client,
            self.account_id,
            tag,
            event,
            profile,
            max_bytes,
            progress,
        )
        .await
    }
}

/// The source, declared size and MIME of an exact scoped message. `any` also accepts
/// images and files (Save); the system player only takes `m.video` and `m.audio`.
async fn attachment(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    tag: &RequestTag,
    event: &EventId,
    any: bool,
) -> Result<(MediaSource, Option<u64>, Option<String>, Option<String>), MediaError> {
    if tag.scope().account_id != account {
        return Err(MediaError::InvalidScope);
    }
    let room_id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
        .map_err(|_| MediaError::InvalidScope)?;
    let event_id =
        matrix_sdk::ruma::EventId::parse(event.as_str()).map_err(|_| MediaError::InvalidScope)?;
    let room = client
        .get_room(&room_id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(MediaError::InvalidScope)?;
    let event = room
        .event(&event_id, None)
        .await
        .map_err(|_| MediaError::Failed)?;
    attachment_content(event.raw(), any)
}

/// `(source, declared size, MIME, file name)` of an attachment event.
fn attachment_content<T>(raw: &matrix_sdk::ruma::serde::Raw<T>, any: bool) -> Result<(MediaSource, Option<u64>, Option<String>, Option<String>), MediaError> {
    use matrix_sdk::ruma::events::room::message::{MessageType, RoomMessageEventContent};
    if raw.get_field::<String>("type").ok().flatten().as_deref() == Some("m.sticker") {
        use matrix_sdk::ruma::events::sticker::{StickerEventContent, StickerMediaSource};
        if !any { return Err(MediaError::UnsupportedType); }
        let sticker = raw.get_field::<StickerEventContent>("content")
            .map_err(|_| MediaError::UnsupportedType)?.ok_or(MediaError::UnsupportedType)?;
        let source = match sticker.source {
            StickerMediaSource::Plain(uri) => MediaSource::Plain(uri),
            StickerMediaSource::Encrypted(file) => MediaSource::Encrypted(file),
            _ => return Err(MediaError::UnsupportedType),
        };
        return Ok((source, sticker.info.size.map(u64::from), sticker.info.mimetype, None));
    }
    let content = raw
        .get_field::<RoomMessageEventContent>("content")
        .map_err(|_| MediaError::UnsupportedType)?
        .ok_or(MediaError::UnsupportedType)?;
    macro_rules! parts {
        ($m:expr) => {{
            let m = $m;
            let i = m.info.as_deref();
            { let name = Some(m.filename().to_owned()); (m.source, i.and_then(|i| i.size).map(u64::from), i.and_then(|i| i.mimetype.clone()), name) }
        }};
    }
    Ok(match content.msgtype {
        MessageType::Video(v) => parts!(v),
        MessageType::Audio(a) => parts!(a),
        MessageType::Image(i) if any => parts!(i),
        MessageType::File(f) if any => parts!(f),
        _ => return Err(MediaError::UnsupportedType),
    })
}

/// Download the original of an exact scoped attachment (video, audio, file or picture) into a private temp file
/// under `profile`. Over `max_bytes` (declared or received) is `TooLarge`.
pub async fn download_video(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    tag: &RequestTag,
    event: &EventId,
    profile: &Path,
    max_bytes: usize,
    progress: impl FnMut(usize),
) -> Result<VideoFile, MediaError> {
    let (source, size, mime, name) = attachment(client, account, tag, event, true).await?;
    // Decided before anything is fetched: a file with no safe extension is for «Save», not «Open».
    let ext = extension(mime.as_deref(), name.as_deref()).ok_or(MediaError::UnsupportedType)?;
    if size.is_some_and(|size| size > max_bytes as u64) {
        return Err(MediaError::TooLarge);
    }
    // What the app already fetched by itself is not downloaded again.
    match crate::media::auto_get(tag.scope(), event) {
        Some(bytes) => write_temp(&bytes, ext, profile),
        None => fetch_to_file(client, source, ext, profile, max_bytes, tag, progress).await,
    }
}

/// Download the original of an exact scoped `m.audio` event through the SDK (auth and E2EE
/// decryption included) and decode it once: the answer carries both the samples the player
/// uses and the waveform of those same samples. Over `max_bytes` (declared or received) is
/// `TooLarge`. Nothing is written to disk.
pub async fn download_audio(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    tag: &RequestTag,
    event: &EventId,
    max_bytes: usize,
) -> Result<crate::media::OpenedAudio, MediaError> {
    let (source, size, ..) = attachment(client, account, tag, event, false).await?;
    if size.is_some_and(|size| size > max_bytes as u64) {
        return Err(MediaError::TooLarge);
    }
    let bytes = fetch_bytes(client, source, max_bytes, tag, |_| ()).await?;
    // Decoding is CPU-bound: keep it off the async worker's thread.
    tokio::task::spawn_blocking(move || opened_audio(&bytes))
        .await
        .map_err(|_| MediaError::Failed)?
}

/// The single decode: player samples and the waveform of those same samples.
fn opened_audio(bytes: &[u8]) -> Result<crate::media::OpenedAudio, MediaError> {
    let decoded = crate::audio_file::decode_audio(bytes)?;
    let waveform =
        crate::audio_file::waveform(&decoded.samples, crate::audio_file::WAVEFORM_POINTS);
    Ok(crate::media::OpenedAudio {
        samples: decoded.samples,
        sample_rate: decoded.sample_rate,
        waveform,
    })
}

/// Save the original of an exact scoped attachment (image, video, audio or file) to `dest`.
/// Bytes are decrypted in memory and only `dest` is written (through `dest.part`, renamed
/// when complete); no thumbnail and no temp file. Over `max_bytes` is `TooLarge`.
pub async fn save_attachment(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    tag: &RequestTag,
    event: &EventId,
    dest: &Path,
    max_bytes: usize,
) -> Result<(), MediaError> {
    save_attachment_with_progress(client, account, tag, event, dest, max_bytes, |_, _| ()).await
}

/// [`save_attachment`] reporting `(bytes received, declared size)` while the body arrives.
pub async fn save_attachment_with_progress(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    tag: &RequestTag,
    event: &EventId,
    dest: &Path,
    max_bytes: usize,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), MediaError> {
    let (source, size, ..) = attachment(client, account, tag, event, true).await?;
    if size.is_some_and(|size| size > max_bytes as u64) {
        return Err(MediaError::TooLarge);
    }
    match crate::media::auto_get(tag.scope(), event) {
        Some(bytes) => write_dest(dest, bytes).await,
        None => save_source(client, source, dest, max_bytes, tag, |n| progress(n as u64, size)).await,
    }
}

/// Fetch the original of an exact scoped attachment into the auto cache, in memory only: nothing is written to disk.
/// Over `max_bytes` (declared or received) is `TooLarge`; `progress` gets `(bytes received, declared size)`.
pub async fn prefetch_attachment(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    tag: &RequestTag,
    event: &EventId,
    max_bytes: usize,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), MediaError> {
    let (source, size, ..) = attachment(client, account, tag, event, true).await?;
    if size.is_some_and(|size| size > max_bytes as u64) {
        return Err(MediaError::TooLarge);
    }
    if crate::media::auto_get(tag.scope(), event).is_some() {
        return Ok(());
    }
    let bytes = fetch_bytes(client, source, max_bytes, tag, |n| progress(n as u64, size)).await?;
    // Bigger than the whole budget: it would be dropped at once, so the user's Open or Save fetches it as before.
    if crate::media::auto_put(tag.scope(), event, bytes) { Ok(()) } else { Err(MediaError::TooLarge) }
}

async fn save_source(
    client: &matrix_sdk::Client,
    source: MediaSource,
    dest: &Path,
    max_bytes: usize,
    tag: &RequestTag,
    progress: impl FnMut(usize),
) -> Result<(), MediaError> {
    let bytes = fetch_bytes(client, source, max_bytes, tag, progress).await?;
    write_dest(dest, bytes).await
}

async fn write_dest(dest: &Path, bytes: Vec<u8>) -> Result<(), MediaError> {
    // Straight into the chosen file: the macOS sandbox grants no sibling files.
    let dest = dest.to_owned();
    tokio::task::spawn_blocking(move || {
        std::fs::write(&dest, bytes).inspect_err(|_| {
            let _ = std::fs::remove_file(&dest);
        })
    })
    .await
    .map_err(|_| MediaError::Failed)?
    .map_err(|_| MediaError::Failed)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use matrix_domain::{AccountId, RoomId, RoomScope};
    use matrix_sdk::ruma::{api::MatrixVersion, owned_mxc_uri};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn save_accepts_sticker_media_source() {
        let event = matrix_sdk::ruma::serde::Raw::<serde_json::Value>::from_json_string(
            r#"{"type":"m.sticker","content":{"body":"sticker","url":"mxc://example.org/sticker","info":{"mimetype":"image/png","size":42}}}"#.to_owned()
        ).unwrap();
        let (source, size, mime, _) = attachment_content(&event, true).expect("stickers can be saved");
        assert!(matches!(source, MediaSource::Plain(_)));
        assert_eq!(size, Some(42));
        assert_eq!(mime.as_deref(), Some("image/png"));
        assert!(attachment_content(&event, false).is_err());
    }

    #[test]
    fn parameterized_audio_mime_keeps_ogg_extension() {
        assert_eq!(extension(Some("audio/ogg; codecs=opus"), None), Some("ogg"));
        // A generic type takes the name's extension only from the list; none of it, or an unsafe one, is not opened.
        assert_eq!(extension(Some("application/octet-stream"), Some("foto.JPG")), Some("jpg"));
        assert_eq!(extension(None, Some("conti.xlsx")), Some("xlsx"));
        assert_eq!(extension(Some("video/x-unknown"), Some("clip")), Some("mp4"));
        for bad in ["setup.exe", "run.sh", "pagina.html", "a.svg", "senza", "x.app"] {
            assert_eq!(extension(Some("application/octet-stream"), Some(bad)), None, "{bad}");
        }
        assert_eq!(extension(None, None), None);
    }

    const CHUNK: usize = 1024;
    const CHUNKS: usize = 8;

    /// Serves `CHUNKS` chunks of `CHUNK` bytes, 40 ms apart, to every request.
    async fn client() -> matrix_sdk::Client {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") {
                        headers.push(stream.read_u8().await.unwrap());
                    }
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                    if stream.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    for _ in 0..CHUNKS {
                        let mut chunk = format!("{CHUNK:x}\r\n").into_bytes();
                        chunk.extend_from_slice(&[7; CHUNK]);
                        chunk.extend_from_slice(b"\r\n");
                        if stream.write_all(&chunk).await.is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(40)).await;
                    }
                    let _ = stream.write_all(b"0\r\n\r\n").await;
                });
            }
        });
        matrix_sdk::Client::builder()
            .homeserver_url(format!("http://{address}"))
            .sliding_sync_version_builder(matrix_sdk::sliding_sync::VersionBuilder::None)
            .server_versions([MatrixVersion::V1_0])
            .build()
            .await
            .unwrap()
    }

    fn tag() -> (crate::ViewRequests, RequestTag) {
        let mut views = crate::ViewRequests::new();
        let tag = views
            .start(RoomScope {
                account_id: AccountId::new(1),
                room_id: RoomId::new("!r:fixture.invalid"),
            })
            .unwrap();
        (views, tag)
    }

    fn source() -> MediaSource {
        MediaSource::Plain(owned_mxc_uri!("mxc://fixture.invalid/video"))
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn temp_profile() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "video-file-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    fn files(profile: &Path) -> usize {
        std::fs::read_dir(profile.join("video-tmp")).map_or(0, Iterator::count)
    }

    #[test]
    fn within_cap_writes_private_file_reports_progress_and_drop_deletes_it() {
        runtime().block_on(async {
            let (client, (_views, tag), profile) = (client().await, tag(), temp_profile());
            let mut seen = Vec::new();
            let file = fetch_to_file(
                &client,
                source(),
                "mp4",
                &profile,
                CHUNK * CHUNKS,
                &tag,
                |n| seen.push(n),
            )
            .await
            .unwrap();
            assert_eq!(std::fs::read(file.path()).unwrap(), vec![7; CHUNK * CHUNKS]);
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(file.path()), 0o600);
            assert_eq!(mode(file.path().parent().unwrap()), 0o700);
            assert!(file.path().extension().is_some_and(|e| e == "mp4"));
            assert!(
                seen.windows(2).all(|w| w[0] < w[1]) && seen.len() > 2,
                "{seen:?}"
            );
            assert_eq!(seen.last(), Some(&(CHUNK * CHUNKS)));
            let path = file.path().to_owned();
            drop(file);
            assert!(!path.exists());
            std::fs::remove_dir_all(profile).unwrap();
        });
    }

    #[test]
    fn over_cap_is_stopped_before_the_whole_body_and_leaves_no_file() {
        runtime().block_on(async {
            let (client, (_views, tag), profile) = (client().await, tag(), temp_profile());
            let started = std::time::Instant::now();
            let result = fetch_to_file(
                &client,
                source(),
                "mp4",
                &profile,
                CHUNK * 2 + 1,
                &tag,
                |_| {},
            )
            .await;
            assert!(matches!(result, Err(MediaError::TooLarge)), "{result:?}");
            assert!(
                started.elapsed() < Duration::from_millis(40 * CHUNKS as u64),
                "stopped early"
            );
            assert_eq!(files(&profile), 0);
            std::fs::remove_dir_all(profile).unwrap();
        });
    }

    #[test]
    fn cancelling_from_progress_aborts_without_a_file() {
        runtime().block_on(async {
            let (client, (mut views, tag), profile) = (client().await, tag(), temp_profile());
            let result = fetch_to_file(
                &client,
                source(),
                "mp4",
                &profile,
                CHUNK * CHUNKS,
                &tag,
                |_| views.cancel(),
            )
            .await;
            assert!(matches!(result, Err(MediaError::Cancelled)), "{result:?}");
            assert_eq!(files(&profile), 0);
            std::fs::remove_dir_all(profile).unwrap();
        });
    }

    #[test]
    fn an_empty_body_is_a_failure_and_leaves_no_temp_file() {
        runtime().block_on(async {
            let (client, (_views, tag), profile) =
                (body_client(Vec::new()).await, tag(), temp_profile());
            let result = fetch_to_file(&client, source(), "mp4", &profile, CHUNK, &tag, |_| ()).await;
            assert!(matches!(result, Err(MediaError::Failed)), "{result:?}");
            assert_eq!(files(&profile), 0);
            std::fs::remove_dir_all(profile).unwrap();
        });
    }

    #[test]
    fn opening_the_dir_removes_leftovers_and_tightens_permissions() {
        let profile = temp_profile();
        let dir = profile.join("video-tmp");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("stale.mp4"), b"x").unwrap();
        private_dir(&profile).unwrap();
        assert_eq!(files(&profile), 0);
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_dir_all(profile).unwrap();
    }

    /// Serves `body` at once, chunked (so only the cap, not Content-Length, can refuse it).
    async fn body_client(body: Vec<u8>) -> matrix_sdk::Client {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let body = body.clone();
                tokio::spawn(async move {
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") {
                        headers.push(stream.read_u8().await.unwrap());
                    }
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                    let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
                });
            }
        });
        matrix_sdk::Client::builder()
            .homeserver_url(format!("http://{address}"))
            .sliding_sync_version_builder(matrix_sdk::sliding_sync::VersionBuilder::None)
            .server_versions([MatrixVersion::V1_0])
            .build()
            .await
            .unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn save_writes_only_the_chosen_file_and_no_temp_copy() {
        runtime().block_on(async {
            let (client, (_views, tag), profile) = (body_client(vec![9; 3000]).await, tag(), temp_profile());
            let dest = profile.join("scelto.pdf");
            save_source(&client, source(), &dest, 3000, &tag, |_| ()).await.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), vec![9; 3000]);
            assert_eq!(names(&profile), ["scelto.pdf"], "no .part, no video-tmp");
            std::fs::remove_dir_all(profile).unwrap();
        });
    }

    #[test]
    fn save_over_the_cap_is_refused_and_writes_nothing() {
        runtime().block_on(async {
            let (client, (_views, tag), profile) = (body_client(vec![9; 3000]).await, tag(), temp_profile());
            let dest = profile.join("scelto.pdf");
            let result = save_source(&client, source(), &dest, 2999, &tag, |_| ()).await;
            assert!(matches!(result, Err(MediaError::TooLarge)), "{result:?}");
            assert!(names(&profile).is_empty());
            std::fs::remove_dir_all(profile).unwrap();
        });
    }

    #[test]
    fn a_cancelled_save_writes_nothing() {
        runtime().block_on(async {
            let (client, (mut views, tag), profile) = (body_client(vec![9; 3000]).await, tag(), temp_profile());
            views.cancel();
            let result = save_source(&client, source(), &profile.join("x"), 3000, &tag, |_| ()).await;
            assert!(matches!(result, Err(MediaError::Cancelled)), "{result:?}");
            assert!(names(&profile).is_empty());
            std::fs::remove_dir_all(profile).unwrap();
        });
    }

    #[test]
    fn an_encrypted_attachment_is_decrypted_in_memory_and_only_the_plaintext_is_written() {
        use std::io::Read;
        runtime().block_on(async {
            let plaintext = b"contenuto riservato".repeat(100);
            let mut input = std::io::Cursor::new(&plaintext);
            let mut encryptor = matrix_sdk_crypto::AttachmentEncryptor::new(&mut input);
            let mut ciphertext = Vec::new();
            encryptor.read_to_end(&mut ciphertext).unwrap();
            let info = encryptor.finish();
            let file = matrix_sdk::ruma::events::room::EncryptedFile::new(
                owned_mxc_uri!("mxc://fixture.invalid/file"),
                info.encryption_info,
                info.hashes,
            );
            let (client, (_views, tag), profile) = (body_client(ciphertext.clone()).await, tag(), temp_profile());
            let dest = profile.join("riservato.txt");
            let source = MediaSource::Encrypted(Box::new(file));
            save_source(&client, source, &dest, ciphertext.len(), &tag, |_| ()).await.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), plaintext);
            assert_eq!(names(&profile), ["riservato.txt"]);
            std::fs::remove_dir_all(profile).unwrap();
        });
    }
}
