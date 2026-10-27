//! Backend-only image processing. No UI, filesystem paths or remote diagnostics.
use crate::RequestTag;
use crate::auth::AccountSession;
use matrix_domain::{EventId, RoomScope, TransactionId};
use std::time::Duration;

#[cfg(test)]
tokio::task_local! {
    pub(crate) static VIDEO_DECODE_ENTRIES: std::cell::Cell<(usize, usize)>;
}

async fn cancellable<T>(
    tag: &RequestTag,
    future: impl std::future::Future<Output = Result<T, MediaError>>,
) -> Result<T, MediaError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut future = std::pin::pin!(future);
    loop {
        if tag.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(MediaError::Deadline);
        }
        match tokio::time::timeout((deadline - now).min(Duration::from_millis(20)), &mut future)
            .await
        {
            Ok(result) => {
                return if tag.is_cancelled() {
                    Err(MediaError::Cancelled)
                } else {
                    result
                };
            }
            Err(_) => continue,
        }
    }
}

const VIDEO_THUMBNAIL_MAX_BYTES: usize = 1_048_576;

impl AccountSession {
    /// Fetch only the thumbnail of an exact scoped video event. SDK decryption
    /// and image decoding stay in memory; persistent media-store caching is disabled.
    /// A video without a thumbnail returns `None`, never its original MP4.
    pub async fn download_video_thumbnail(
        &self,
        tag: &RequestTag,
        event: &EventId,
    ) -> Result<Option<DownloadedPng>, MediaError> {
        use matrix_sdk::media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings};
        use matrix_sdk::ruma::events::room::message::{MessageType, RoomMessageEventContent};

        if tag.scope().account_id != self.account_id {
            return Err(MediaError::InvalidScope);
        }
        let client = self.client.as_ref().ok_or(MediaError::LoggedOut)?;
        let room_id = matrix_sdk::ruma::RoomId::parse(tag.scope().room_id.as_str())
            .map_err(|_| MediaError::InvalidScope)?;
        let event_id = matrix_sdk::ruma::EventId::parse(event.as_str())
            .map_err(|_| MediaError::InvalidScope)?;
        let room = client
            .get_room(&room_id)
            .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
            .ok_or(MediaError::InvalidScope)?;
        cancellable(tag, async {
            let event = room
                .event(&event_id, None)
                .await
                .map_err(|_| MediaError::Failed)?;
            let content = event
                .raw()
                .get_field::<RoomMessageEventContent>("content")
                .map_err(|_| MediaError::UnsupportedType)?
                .ok_or(MediaError::UnsupportedType)?;
            let MessageType::Video(video) = content.msgtype else {
                return Err(MediaError::UnsupportedType);
            };
            let Some(source) = video.info.and_then(|info| info.thumbnail_source) else {
                return Ok(None);
            };
            // shortcut: server events use remote MXC URIs; revisit for local send-queue previews.
            // Calling the fetcher directly bypasses all persistent media-store reads/writes.
            let request = MediaRequestParameters {
                source,
                format: MediaFormat::Thumbnail(MediaThumbnailSettings::new(
                    320u32.into(),
                    240u32.into(),
                )),
            };
            let config = client
                .request_config()
                .timeout(Some(Duration::MAX))
                .max_response_size(Some(VIDEO_THUMBNAIL_MAX_BYTES));
            let bytes = matrix_sdk::media::DefaultMediaFetcher
                .fetch_media_content_with_request_config(client, &request, config)
                .await
                .map_err(|error| match error {
                    matrix_sdk::Error::Http(error)
                        if matches!(*error, matrix_sdk::HttpError::ResponseTooLarge { .. }) =>
                    {
                        MediaError::TooLarge
                    }
                    _ => MediaError::Failed,
                })?;
            let mime = mime_of(&bytes).ok_or(MediaError::UnsupportedType)?;
            let preview = decode_png(bytes.clone(), mime, tag).await?;
            Ok(Some(DownloadedPng { bytes, preview }))
        })
        .await
    }

    pub async fn download_png(
        &self,
        tag: &RequestTag,
        event: &EventId,
    ) -> Result<DownloadedPng, MediaError> {
        download_png(
            self.client.as_ref().ok_or(MediaError::LoggedOut)?,
            self.account_id,
            tag,
            event,
        )
        .await
    }
    pub async fn send_png(
        &self,
        scope: &RoomScope,
        transaction: &TransactionId,
        data: Vec<u8>,
    ) -> Result<EventId, MediaError> {
        send_png(
            self.client.as_ref().ok_or(MediaError::LoggedOut)?,
            self.account_id,
            scope,
            transaction,
            data,
        )
        .await
    }
}

// The free functions also serve the session-owned backend worker.

/// Read an exact scoped server event, never an arbitrary caller URL. Media
/// uses authenticated HTTPS on this session's homeserver with no redirects.
pub(crate) async fn download_png(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    tag: &RequestTag,
    event: &EventId,
) -> Result<DownloadedPng, MediaError> {
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
    cancellable(tag, async {
        let event = room
            .event(&event_id, None)
            .await
            .map_err(|_| MediaError::Failed)?;
        let content = event
            .raw()
            .get_field::<matrix_sdk::ruma::events::room::message::RoomMessageEventContent>(
                "content",
            )
            .map_err(|_| MediaError::UnsupportedType)?
            .ok_or(MediaError::UnsupportedType)?;
        let matrix_sdk::ruma::events::room::message::MessageType::Image(image) = content.msgtype
        else {
            return Err(MediaError::UnsupportedType);
        };
        let mime = image
            .info
            .as_ref()
            .and_then(|i| i.mimetype.clone())
            .ok_or(MediaError::UnsupportedType)?;
        format_for_mime(&mime).ok_or(MediaError::UnsupportedType)?;
        let (uri, encrypted_file) = match image.source {
            matrix_sdk::ruma::events::room::MediaSource::Plain(uri) => (uri, None),
            matrix_sdk::ruma::events::room::MediaSource::Encrypted(file) => {
                (file.url.clone(), Some(file))
            }
        };
        let (server, media_id) = uri.parts().map_err(|_| MediaError::InvalidScope)?;
        if media_id.is_empty()
            || !media_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(MediaError::InvalidScope);
        }
        let mut url = client.homeserver();
        url.path_segments_mut()
            .map_err(|_| MediaError::InvalidScope)?
            .pop_if_empty()
            .extend([
                "_matrix",
                "client",
                "v1",
                "media",
                "download",
                server.as_str(),
                media_id,
            ]);
        url.query_pairs_mut().append_pair("allow_redirect", "false");
        let token = zeroize::Zeroizing::new(client.access_token().ok_or(MediaError::LoggedOut)?);
        let mut response = client
            .http_client()
            .get(url)
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|_| MediaError::Failed)?;
        if !response.status().is_success() {
            return Err(MediaError::Failed);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_MEDIA_BYTES as u64)
        {
            return Err(MediaError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| MediaError::Failed)? {
            if chunk.len() > MAX_MEDIA_BYTES - bytes.len() {
                return Err(MediaError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        if let Some(file) = encrypted_file {
            bytes = tokio::task::spawn_blocking(move || {
                use std::io::Read;
                let mut cursor = std::io::Cursor::new(bytes);
                let mut decryptor =
                    matrix_sdk_crypto::AttachmentDecryptor::new(&mut cursor, (*file).into())
                        .map_err(|_| MediaError::InvalidImage)?;
                let mut plaintext = Vec::new();
                decryptor
                    .read_to_end(&mut plaintext)
                    .map_err(|_| MediaError::InvalidImage)?;
                Ok::<_, MediaError>(plaintext)
            })
            .await
            .map_err(|_| MediaError::Failed)??;
        }
        let preview = decode_png(bytes.clone(), &mime, tag).await?;
        Ok(DownloadedPng { bytes, preview })
    })
    .await
}

/// Explicit attachment write. A deadline may leave an orphan upload or an ACK
/// not yet observed; caller must retain transaction identity for safe retry.
/// Filename is fixed: no personal path or untrusted executable is transmitted.
pub(crate) async fn send_png(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    transaction: &TransactionId,
    data: Vec<u8>,
) -> Result<EventId, MediaError> {
    if scope.account_id != account {
        return Err(MediaError::InvalidScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| MediaError::InvalidScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(MediaError::InvalidScope)?;
    if transaction.as_str().is_empty() || transaction.as_str().len() > 256 {
        return Err(MediaError::InvalidScope);
    }
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut requests = crate::ViewRequests::new();
        let tag = requests
            .start(scope.clone())
            .map_err(|_| MediaError::Failed)?;
        decode_png(data.clone(), "image/png", &tag).await?;
        let mime = "image/png"
            .parse()
            .map_err(|_| MediaError::UnsupportedType)?;
        let config =
            matrix_sdk::attachment::AttachmentConfig::new().txn_id(transaction.as_str().into());
        let response = room
            .send_attachment("image.png", &mime, data, config)
            .await
            .map_err(|_| MediaError::Failed)?;
        Ok(EventId::new(response.event_id.as_str()))
    })
    .await
    .map_err(|_| MediaError::Deadline)?
}

pub use crate::media::{DownloadedPng, MAX_MEDIA_BYTES, MediaError, Preview};

/// MIME type of already-downloaded bytes (cache hits keep no metadata).
pub(crate) fn mime_of(bytes: &[u8]) -> Option<&'static str> {
    #[cfg(test)]
    let _ = VIDEO_DECODE_ENTRIES
        .try_with(|entries| entries.set((entries.get().0 + 1, entries.get().1)));
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::WebP => Some("image/webp"),
        image::ImageFormat::Gif => Some("image/gif"),
        _ => None,
    }
}

fn format_for_mime(mime: &str) -> Option<image::ImageFormat> {
    match mime {
        "image/png" => Some(image::ImageFormat::Png),
        "image/jpeg" => Some(image::ImageFormat::Jpeg),
        "image/webp" => Some(image::ImageFormat::WebP),
        "image/gif" => Some(image::ImageFormat::Gif),
        _ => None,
    }
}

/// Decodes a PNG, JPEG, WebP or static GIF into a bounded preview.
pub async fn decode_png(
    data: Vec<u8>,
    mime: &str,
    tag: &RequestTag,
) -> Result<Preview, MediaError> {
    #[cfg(test)]
    let _ = VIDEO_DECODE_ENTRIES
        .try_with(|entries| entries.set((entries.get().0, entries.get().1 + 1)));
    if tag.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let tag = tag.clone();
    let format = format_for_mime(mime).ok_or(MediaError::UnsupportedType)?;
    if data.len() > MAX_MEDIA_BYTES {
        return Err(MediaError::TooLarge);
    }
    tokio::task::spawn_blocking(move || {
        if tag.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        let reader = || {
            let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&data), format);
            let mut limits = image::Limits::default();
            limits.max_image_width = None;
            limits.max_image_height = None;
            limits.max_alloc = Some(512 * 1024 * 1024);
            reader.limits(limits);
            reader
        };
        let (w, h) = reader()
            .into_dimensions()
            .map_err(|_| MediaError::InvalidImage)?;
        if u64::from(w) * u64::from(h) > crate::media::MAX_IMAGE_PIXELS {
            return Err(MediaError::TooLarge);
        }
        let image = reader().decode().map_err(|_| MediaError::InvalidImage)?;
        let image = if image.width() > 320 || image.height() > 240 {
            image.thumbnail(320, 240)
        } else {
            image
        };
        let image = image.into_rgba8();
        if tag.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        Ok(Preview {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
            #[cfg(test)]
            decoded_on: std::thread::current().id(),
        })
    })
    .await
    .map_err(|_| MediaError::Failed)?
}
