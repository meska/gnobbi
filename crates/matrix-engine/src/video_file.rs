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
pub const MAX_VIDEO_BYTES: usize = 256 * 1024 * 1024;

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

fn extension(mime: Option<&str>) -> &'static str {
    match mime {
        Some("video/webm") => "webm",
        Some("video/quicktime") => "mov",
        Some("video/x-matroska") => "mkv",
        Some("audio/ogg" | "audio/opus") => "ogg",
        Some("audio/mpeg") => "mp3",
        Some("audio/mp4" | "audio/aac") => "m4a",
        Some("audio/wav" | "audio/x-wav") => "wav",
        Some("audio/flac") => "flac",
        _ => "mp4",
    }
}

/// Fetch `source` through the SDK (auth and E2EE decryption included), refusing more
/// than `max_bytes` while the body is received. `progress` gets the bytes received so
/// far; a cancelled `tag` aborts the request. No media-store (cache) access.
pub(crate) async fn fetch_to_file(
    client: &matrix_sdk::Client,
    source: MediaSource,
    mime: Option<&str>,
    profile: &Path,
    max_bytes: usize,
    tag: &RequestTag,
    mut progress: impl FnMut(usize),
) -> Result<VideoFile, MediaError> {
    static NAMES: AtomicU64 = AtomicU64::new(0);
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
    let dir = private_dir(profile).map_err(|_| MediaError::Failed)?;
    let path = dir.join(format!(
        "{}-{}.{}",
        std::process::id(),
        NAMES.fetch_add(1, Ordering::Relaxed),
        extension(mime)
    ));
    let file = VideoFile::new(path);
    crate::fsx::private_file(std::fs::OpenOptions::new().write(true).create_new(true))
        .open(file.path())
        .and_then(|mut f| f.write_all(&bytes))
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

/// Download the original of an exact scoped `m.video` or `m.audio` event into a private temp file
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
    use matrix_sdk::ruma::events::room::message::{MessageType, RoomMessageEventContent};
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
    let content = event
        .raw()
        .get_field::<RoomMessageEventContent>("content")
        .map_err(|_| MediaError::UnsupportedType)?
        .ok_or(MediaError::UnsupportedType)?;
    let (source, size, mime) = match content.msgtype {
        MessageType::Video(v) => {
            let i = v.info.as_deref();
            (v.source, i.and_then(|i| i.size), i.and_then(|i| i.mimetype.clone()))
        }
        MessageType::Audio(a) => {
            let i = a.info.as_deref();
            (a.source, i.and_then(|i| i.size), i.and_then(|i| i.mimetype.clone()))
        }
        _ => return Err(MediaError::UnsupportedType),
    };
    if size.is_some_and(|size| u64::from(size) > max_bytes as u64) {
        return Err(MediaError::TooLarge);
    }
    fetch_to_file(
        client,
        source,
        mime.as_deref(),
        profile,
        max_bytes,
        tag,
        progress,
    )
    .await
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use matrix_domain::{AccountId, RoomId, RoomScope};
    use matrix_sdk::ruma::{api::MatrixVersion, owned_mxc_uri};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
                Some("video/mp4"),
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
                None,
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
                None,
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
}
