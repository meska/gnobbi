//! Durable, account/room-scoped sends. No view cancellation or payload logging.
use matrix_domain::{EventId, RoomScope, TransactionId};

#[derive(Clone)]
pub struct SendRequest {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    /// Text message body, or the caption when `attachment` is set.
    pub body: String,
    pub attachment: Option<Attachment>,
    /// A poll to start; `body` is then ignored (the fallback text is built from the draft).
    pub poll: Option<matrix_domain::PollDraft>,
    /// Users named in `body` by their `label`; ignored for attachments.
    pub mentions: Vec<Mention>,
}

/// A user picked from the completion list; `label` is the text shown in the body («@Anna»).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    pub user_id: String,
    pub label: String,
}

/// Markdown source of `body`: each mention's first free occurrence becomes a matrix.to link, a literal `<` is
/// escaped (the sender never types HTML). Also returns the user ids that were linked.
pub fn mention_markdown<'a>(body: &str, mentions: &'a [Mention]) -> (String, Vec<&'a str>) {
    let mut spans: Vec<(usize, usize, &str)> = Vec::new();
    for mention in mentions {
        let mut from = 0;
        while let Some(found) = body[from..]
            .find(&mention.label)
            .filter(|_| !mention.label.is_empty())
        {
            let (start, end) = (from + found, from + found + mention.label.len());
            // «@Anna» inside a hand-typed «@Annabella» is not her.
            let whole_word = !body[end..].starts_with(char::is_alphanumeric);
            if whole_word && spans.iter().all(|&(s, e, _)| end <= s || start >= e) {
                spans.push((start, end, &mention.user_id));
                break;
            }
            from = end;
        }
    }
    spans.sort();
    let escape = |text: &str| text.replace('<', "\\<");
    let (mut source, mut at) = (String::new(), 0);
    for &(start, end, user) in &spans {
        source.push_str(&escape(&body[at..start]));
        let label = body[start..end]
            .replace('\\', "\\\\")
            .replace('[', "\\[")
            .replace(']', "\\]");
        let user = user.replace('(', "%28").replace(')', "%29");
        source.push_str(&format!("[{}](https://matrix.to/#/{user})", escape(&label)));
        at = end;
    }
    source.push_str(&escape(&body[at..]));
    // Typed newlines stay line breaks: the SDK markdown pass turns soft breaks into <br>.
    (source, spans.into_iter().map(|(_, _, user)| user).collect())
}

/// The message to send: the typed text as `body`, its markdown as `formatted_body` (the SDK `markdown`
/// feature; none when the text has no markup) and `m.mentions` for the linked users.
#[cfg(feature = "sdk")]
pub fn markdown_content(
    body: &str,
    mentions: &[Mention],
) -> matrix_sdk::ruma::events::room::message::RoomMessageEventContent {
    use matrix_sdk::ruma::events::{
        Mentions,
        room::message::{MessageType, RoomMessageEventContent},
    };
    let (source, linked) = mention_markdown(body, mentions);
    let mut content = RoomMessageEventContent::text_markdown(source);
    if let MessageType::Text(text) = &mut content.msgtype {
        text.body = body.to_owned();
    }
    if linked.is_empty() {
        content
    } else {
        content.add_mentions(Mentions::with_user_ids(
            linked.into_iter().filter_map(|id| id.parse().ok()),
        ))
    }
}

/// File or image sent with the request body as caption.
#[derive(Clone, Default)]
pub struct Attachment {
    pub filename: String,
    pub data: Vec<u8>,
    /// Set for a recorded voice note: sent with the voice markers, duration and waveform.
    pub voice: Option<Voice>,
    pub delivery: Delivery,
    /// Set on each item of an album (see docs: N consecutive media events sharing one id).
    pub album: Option<Album>,
}

/// How a picture or video leaves: as picked, downscaled to `COMPRESSED_SIDE`, or as a plain `m.file`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Delivery {
    #[default]
    Original,
    Compressed,
    File,
}

/// Longest side of a compressed picture, in pixels.
pub const COMPRESSED_SIDE: u32 = 2560;

/// Key of the namespaced field every album item carries; other clients ignore unknown content keys.
pub const ALBUM_KEY: &str = "chat.gnobbi.album";

/// Position of an item in an album: all items share `id`; `index` counts from 0 and `count` is the album size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Album {
    pub id: String,
    pub index: u8,
    pub count: u8,
}

#[derive(Clone)]
pub struct Voice {
    pub duration_ms: u64,
    /// Amplitudes in 0..=1.
    pub waveform: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    WrongScope,
    InvalidInput,
    Unavailable,
    /// SDK crypto/session policy failure; never fall back to plaintext.
    Encryption,
    /// The server refused the upload as too large (M_TOO_LARGE); trying again cannot work.
    TooLarge,
    Transport,
}

pub struct SendResult {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    pub result: Result<EventId, SendError>,
}

/// What the send queue reports besides a final result. Bodies are private: no Debug.
pub enum SendNotice {
    /// A message an earlier run left in the queue (it restarts by itself unless `failed`).
    Restored { body: String, failed: bool },
    /// Sending was tried again.
    Retrying,
    /// The queued message was withdrawn before it went out.
    Cancelled,
    /// The queued message now carries this text.
    Edited { body: String },
}

pub enum SendEvent {
    Result(SendResult),
    Notice {
        scope: RoomScope,
        transaction_id: TransactionId,
        notice: SendNotice,
    },
}

impl SendEvent {
    pub fn result(self) -> Option<SendResult> {
        match self {
            Self::Result(result) => Some(result),
            Self::Notice { .. } => None,
        }
    }
}

/// Retry, cancel and edit of a queued text, by the transaction ID the UI knows it by. Each
/// returns whether the message is in the queue; the outcome arrives as a `SendEvent`.
pub trait SendControl: Send + Sync {
    fn retry(&self, transaction_id: &TransactionId) -> bool;
    fn cancel(&self, transaction_id: &TransactionId) -> bool;
    fn edit(&self, transaction_id: &TransactionId, body: &str, mentions: &[Mention]) -> bool;
}

/// Filled by the worker once its session is up, emptied when it ends (the window never keeps a client).
pub type ControlSlot = std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<dyn SendControl>>>>;

#[cfg(all(test, feature = "sdk"))]
pub(crate) mod tests {
    #[test]
    fn image_size_reads_png_header() {
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(5, 3)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(super::image_size(&png), Some((5, 3)));
        assert_eq!(super::image_size(b"not an image"), None);
    }

    #[test]
    fn downscale_shrinks_only_big_jpeg_and_png() {
        let encode = |w, h, format| {
            let mut out = Vec::new();
            image::DynamicImage::new_rgb8(w, h).write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
            out
        };
        let big = encode(3000, 40, image::ImageFormat::Png);
        let small = super::downscale(&big).unwrap();
        assert_eq!(super::image_size(&small), Some((super::COMPRESSED_SIDE, 34)));
        assert!(small.len() < big.len());
        let jpeg = encode(2000, 3000, image::ImageFormat::Jpeg);
        assert_eq!(super::image_size(&super::downscale(&jpeg).unwrap()), Some((1707, super::COMPRESSED_SIDE)));
        assert!(super::downscale(&encode(100, 100, image::ImageFormat::Png)).is_none(), "small: untouched");
        assert!(super::downscale(&two_frame_gif(3000, 10)).is_none(), "GIF keeps its frames");
        assert!(super::downscale(b"not an image").is_none());
    }

    /// A two-frame GIF, red then blue.
    pub(crate) fn two_frame_gif(w: u32, h: u32) -> Vec<u8> {
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                let frame = image::RgbaImage::from_pixel(w, h, image::Rgba(color));
                encoder.encode_frame(image::Frame::new(frame)).unwrap();
            }
        }
        gif
    }

    #[test]
    fn gif_thumbnail_is_a_static_png_of_the_first_frame() {
        let thumb = super::gif_thumbnail(&two_frame_gif(400, 300)).unwrap();
        assert_eq!((thumb.width, thumb.height, thumb.content_type.as_ref()), (320u32.into(), 240u32.into(), "image/png"));
        assert_eq!(thumb.size, u32::try_from(thumb.data.len()).unwrap().into());
        let png = image::load_from_memory_with_format(&thumb.data, image::ImageFormat::Png).unwrap().to_rgba8();
        let [r, _, b, _] = png.get_pixel(0, 0).0;
        assert!(r > 200 && b < 60, "first frame, not the last: {r} {b}");
        assert!(super::gif_thumbnail(b"GIF89a nothing").is_none());
    }

    use super::{Mention, markdown_content};
    use matrix_sdk::ruma::events::room::message::MessageType;

    fn sent(body: &str, mentions: &[Mention]) -> serde_json::Value {
        let content = markdown_content(body, mentions);
        assert!(matches!(content.msgtype, MessageType::Text(_)));
        serde_json::to_value(&content).unwrap()
    }

    #[test]
    fn markdown_goes_out_as_html_with_the_typed_body() {
        let v = sent(
            "**forte** _corsivo_ `codice`\n- uno\n- due\n\n> cita [sito](https://e.org)",
            &[],
        );
        assert_eq!(v["format"], "org.matrix.custom.html");
        assert_eq!(
            v["body"],
            "**forte** _corsivo_ `codice`\n- uno\n- due\n\n> cita [sito](https://e.org)"
        );
        let html = v["formatted_body"].as_str().unwrap();
        for tag in [
            "<strong>forte</strong>",
            "<em>corsivo</em>",
            "<code>codice</code>",
            "<li>uno</li>",
            "<blockquote>",
            "<a href=\"https://e.org\">sito</a>",
        ] {
            assert!(html.contains(tag), "{tag} missing in {html}");
        }
        // A plain multi-line text needs no HTML: every client shows the newlines of `body`.
        assert!(sent("a\nb", &[]).get("formatted_body").is_none());
    }

    #[test]
    fn plain_text_stays_plain_and_typed_html_is_escaped() {
        assert!(sent("ciao a tutti", &[]).get("formatted_body").is_none());
        let v = sent("<script>x</script> & ok", &[]);
        let html = v["formatted_body"].as_str().unwrap_or_default();
        assert!(!html.contains("<script"), "{html}");
        assert_eq!(v["body"], "<script>x</script> & ok");
    }

    #[test]
    fn mentions_coexist_with_markdown() {
        let v = sent(
            "**ciao** @Anna_B *ok*",
            &[Mention {
                user_id: "@a:x".into(),
                label: "@Anna_B".into(),
            }],
        );
        assert_eq!(v["body"], "**ciao** @Anna_B *ok*");
        let html = v["formatted_body"].as_str().unwrap();
        assert!(
            html.contains("<strong>ciao</strong>") && html.contains("<em>ok</em>"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"https://matrix.to/#/@a:x\">@Anna_B</a>"),
            "{html}"
        );
        assert_eq!(v["m.mentions"]["user_ids"], serde_json::json!(["@a:x"]));
    }

    #[test]
    fn mention_labels_with_special_characters_link_the_right_user() {
        // A display name that is itself markdown or HTML must stay a literal label.
        for name in [
            "@A]n[na *B*",
            "@](https://evil.example)",
            "@<b>x</b>",
            "@Zoë 👩‍💻",
            "@a\\b",
        ] {
            let body = format!("ciao {name} ok");
            let v = sent(
                &body,
                &[Mention {
                    user_id: "@a:x".into(),
                    label: name.into(),
                }],
            );
            assert_eq!(v["body"], body);
            let html = v["formatted_body"].as_str().unwrap();
            assert!(
                html.contains("<a href=\"https://matrix.to/#/@a:x\">"),
                "{html}"
            );
            assert!(
                !html.contains("evil.example\">") && !html.contains("<b>"),
                "{html}"
            );
            assert_eq!(v["m.mentions"]["user_ids"], serde_json::json!(["@a:x"]));
        }
    }

    #[test]
    fn a_mention_label_is_not_linked_inside_a_longer_word() {
        // The picked «@Anna» was deleted and «@Annabella» typed by hand: no link, no ping.
        let v = sent(
            "ciao @Annabella",
            &[Mention {
                user_id: "@a:x".into(),
                label: "@Anna".into(),
            }],
        );
        assert!(v.get("formatted_body").is_none(), "{v}");
        assert!(v.get("m.mentions").is_none(), "{v}");
        let v = sent(
            "ciao @Anna, @Annabella",
            &[Mention {
                user_id: "@a:x".into(),
                label: "@Anna".into(),
            }],
        );
        assert_eq!(v["m.mentions"]["user_ids"], serde_json::json!(["@a:x"]));
        assert!(
            v["formatted_body"]
                .as_str()
                .unwrap()
                .contains(">@Anna</a>, @Annabella"),
            "{v}"
        );
    }

    #[test]
    fn newlines_markup_and_code_blocks_survive_the_markdown_pass() {
        // Markup plus a typed newline: the SDK turns the soft break into <br>.
        let html = sent("**a**\nb", &[])["formatted_body"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(html.contains("<strong>a</strong><br"), "{html}");
        // An indented code block keeps its lines verbatim (no trailing blanks added).
        let v = sent("x\n\n    riga 1\n    riga 2\n", &[]);
        let html = v["formatted_body"].as_str().unwrap();
        assert!(
            html.contains("<pre><code>riga 1\nriga 2\n</code></pre>"),
            "{html}"
        );
        // Long and emoji-heavy text goes through untouched (no trailing blank: the SDK treats
        // trailing whitespace as markup and emits an identical formatted_body).
        let long = "👩‍👩‍👧‍👦 ".repeat(3000).trim_end().to_owned();
        let v = sent(&long, &[]);
        assert_eq!(v["body"], long);
        assert!(
            v.get("formatted_body").is_none(),
            "{}",
            v["formatted_body"].as_str().map_or(0, str::len)
        );
    }

    #[test]
    fn fresh_transaction_ids_do_not_repeat_across_sends() {
        let first = super::fresh_transaction_id();
        let second = super::fresh_transaction_id();
        assert!(!first.as_str().is_empty());
        assert_ne!(first, second);
    }
}

/// SDK generates random transaction identity; callers retain it for all retries.
#[cfg(feature = "sdk")]
pub fn fresh_transaction_id() -> TransactionId {
    TransactionId::new(matrix_sdk::ruma::TransactionId::new().as_str())
}

/// A static PNG of a GIF's first frame (at most 320x240), the thumbnail that goes with the animated original.
#[cfg(feature = "sdk")]
fn gif_thumbnail(data: &[u8]) -> Option<matrix_sdk::attachment::Thumbnail> {
    use image::ImageEncoder as _;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), image::ImageFormat::Gif);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 << 20);
    reader.limits(limits);
    let picture = reader.decode().ok()?; // the GIF decoder yields the first frame only
    let picture = if picture.width() > 320 || picture.height() > 240 { picture.thumbnail(320, 240) } else { picture }.to_rgba8();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(picture.as_raw(), picture.width(), picture.height(), image::ExtendedColorType::Rgba8).ok()?;
    Some(matrix_sdk::attachment::Thumbnail {
        size: u32::try_from(png.len()).ok()?.into(),
        data: png,
        content_type: mime_guess::mime::IMAGE_PNG,
        width: picture.width().into(),
        height: picture.height().into(),
    })
}

/// Pixel size read from the header only, without decoding the image.
#[cfg(feature = "sdk")]
fn image_size(data: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// The joined room a request addresses; nothing is queued for another account's scope.
#[cfg(feature = "sdk")]
pub(crate) fn room_for(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    request: &SendRequest,
) -> Result<matrix_sdk::Room, SendError> {
    if request.scope.account_id != account {
        return Err(SendError::WrongScope);
    }
    if (request.body.trim().is_empty() && request.attachment.is_none() && request.poll.is_none())
        || request.transaction_id.as_str().is_empty()
        || (request.poll.is_some() && request.attachment.is_some())
        || request.poll.as_ref().is_some_and(|p| p.validate().is_err())
    {
        return Err(SendError::InvalidInput);
    }
    let id = matrix_sdk::ruma::RoomId::parse(request.scope.room_id.as_str())
        .map_err(|_| SendError::WrongScope)?;
    client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(SendError::Unavailable)
}

#[cfg(feature = "sdk")]
pub(crate) fn map_error(error: &matrix_sdk::Error) -> SendError {
    match error {
        matrix_sdk::Error::NoOlmMachine
        | matrix_sdk::Error::CryptoStoreError(_)
        | matrix_sdk::Error::OlmError(_)
        | matrix_sdk::Error::MegolmError(_)
        | matrix_sdk::Error::DecryptorError(_)
        | matrix_sdk::Error::SignatureError(_) => SendError::Encryption,
        // The SDK refuses by itself, before uploading, once the server's m.upload.size is known.
        matrix_sdk::Error::Media(matrix_sdk::media::MediaError::MediaTooLargeToUpload { .. }) => SendError::TooLarge,
        _ if matches!(
            error.client_api_error_kind(),
            Some(matrix_sdk::ruma::api::error::ErrorKind::TooLarge)
        ) =>
        {
            SendError::TooLarge
        }
        _ => SendError::Transport,
    }
}

/// Body and "is a plain text, not an edit" of a queued event's content.
#[cfg(feature = "sdk")]
pub(crate) fn queued_text(content: matrix_sdk::store::SerializableEventContent) -> Option<(String, bool)> {
    let (raw, kind) = content.into_raw();
    if kind != "m.room.message" {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw.json().get()).ok()?;
    let plain = value["msgtype"] == "m.text" && value["m.relates_to"]["rel_type"] != "m.replace";
    Some((value["body"].as_str()?.to_owned(), plain))
}

#[cfg(feature = "sdk")]
const POLL_START: &str = "org.matrix.msc3381.poll.start";

/// MSC3381 start event of a validated draft: option ids "1".."N", one selection or all of them.
#[cfg(feature = "sdk")]
pub(crate) fn poll_start(draft: &matrix_domain::PollDraft) -> matrix_sdk::ruma::events::poll::unstable_start::UnstablePollStartEventContent {
    use matrix_sdk::ruma::{
        UInt,
        events::poll::{
            start::PollKind,
            unstable_start::{NewUnstablePollStartEventContent, UnstablePollAnswer, UnstablePollStartContentBlock},
        },
    };
    let answers: Vec<_> = draft
        .answers
        .iter()
        .enumerate()
        .map(|(ix, text)| UnstablePollAnswer::new((ix + 1).to_string(), text.trim()))
        .collect();
    let count = answers.len();
    // `validate` bounds the count to 2..=20, which is what `try_from` accepts.
    let mut block = UnstablePollStartContentBlock::new(draft.question.trim(), answers.try_into().expect("2..=20 answers"));
    block.kind = if draft.disclosed { PollKind::Disclosed } else { PollKind::Undisclosed };
    block.max_selections = UInt::from(if draft.multiple { count as u32 } else { 1 });
    NewUnstablePollStartEventContent::plain_text(draft.fallback_text(), block).into()
}

/// Put a text on the room's SDK send queue. The queue picks the transaction ID, keeps the message
/// across restarts and resends it with the same ID. The handle has no accessor for that ID, so it
/// is read from the local echo the queue announces before `send` returns.
#[cfg(feature = "sdk")]
pub(crate) async fn queue_text(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::Room,
    timeline: &matrix_sdk_ui::timeline::Timeline,
    request: &SendRequest,
) -> Result<(matrix_sdk::ruma::OwnedTransactionId, matrix_sdk::send_queue::SendHandle), SendError> {
    use matrix_sdk::send_queue::{LocalEchoContent, RoomSendQueueUpdate::NewLocalEvent};
    let mut updates = client.send_queue().subscribe();
    let content = match &request.poll {
        Some(draft) => poll_start(draft).into(),
        None => markdown_content(&request.body, &request.mentions).into(),
    };
    let handle = timeline.send(content).await.map_err(|_| SendError::Transport)?;
    while let Ok(update) = updates.try_recv() {
        if update.room_id == room.room_id()
            && let NewLocalEvent(echo) = update.update
            && let LocalEchoContent::Event { serialized_event, .. } = echo.content
            && if request.poll.is_some() {
                serialized_event.into_raw().1 == POLL_START
            } else {
                queued_text(serialized_event).is_some_and(|(body, plain)| plain && body == request.body)
            }
        {
            return Ok((echo.transaction_id, handle));
        }
    }
    let _ = handle.abort().await;
    Err(SendError::Transport)
}

/// Queue a text and wait for the server to accept it (tools and tests; the worker follows the
/// queue instead). A failure or a timeout takes the message out of the queue again, so an error
/// never leaves a message behind that would go out later.
#[cfg(feature = "sdk")]
pub(crate) async fn send_text_waiting(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::Room,
    timeline: &matrix_sdk_ui::timeline::Timeline,
    request: &SendRequest,
) -> Result<EventId, SendError> {
    use matrix_sdk::send_queue::RoomSendQueueUpdate::{SendError as Failed, SentEvent};
    use tokio::sync::broadcast::error::RecvError;
    let mut updates = client.send_queue().subscribe();
    let (txn, handle) = queue_text(client, room, timeline, request).await?;
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let update = match updates.recv().await {
                Ok(update) => update,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return Err(SendError::Transport),
            };
            if update.room_id != room.room_id() {
                continue;
            }
            match update.update {
                SentEvent { transaction_id, event_id } if transaction_id == txn => {
                    return Ok(EventId::new(event_id.as_str()));
                }
                Failed { transaction_id, error, .. } if transaction_id == txn => {
                    return Err(map_error(&error));
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap_or(Err(SendError::Transport));
    if outcome.is_err() {
        let _ = handle.abort().await;
    }
    outcome
}

/// A text or an attachment, both through the SDK send queue; returns once the server confirmed it.
#[cfg(feature = "sdk")]
pub async fn send_text(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    request: &SendRequest,
) -> Result<EventId, SendError> {
    use matrix_sdk_ui::timeline::RoomExt;
    let room = room_for(client, account, request)?;
    let Some(file) = &request.attachment else {
        let timeline = room.timeline().await.map_err(|_| SendError::Unavailable)?;
        return send_text_waiting(client, &room, &timeline, request).await;
    };
    let timeline = room.timeline().await.map_err(|_| SendError::Unavailable)?;
    send_attachment(client, &room, &timeline, request, file).await
}

#[cfg(feature = "sdk")]
pub(crate) async fn send_attachment(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::Room,
    timeline: &matrix_sdk_ui::timeline::Timeline,
    request: &SendRequest,
    file: &Attachment,
) -> Result<EventId, SendError> {
    use matrix_sdk::{
        attachment::{AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo},
    };
    use matrix_sdk_ui::timeline::AttachmentConfig;
    let transaction: matrix_sdk::ruma::OwnedTransactionId = request.transaction_id.as_str().into();
    // shortcut: no total-upload progress, add when the timeline shows uploads; only a GIF gets a client thumbnail
    let mut mime = mime_guess::from_path(&file.filename).first_or_octet_stream();
    let mut data = std::borrow::Cow::Borrowed(file.data.as_slice());
    if file.delivery == Delivery::Compressed
        && let Some(smaller) = compress_picture(&file.data).await
    {
        data = smaller.into();
    }
    // The SDK picks the message type from the content type alone; a generic one makes a picture or video a plain m.file.
    if file.delivery == Delivery::File && matches!(mime.type_().as_str(), "image" | "video" | "audio") {
        mime = mime_guess::mime::APPLICATION_OCTET_STREAM;
    }
    let mut config = AttachmentConfig { txn_id: Some(transaction), ..Default::default() };
    let size = u32::try_from(data.len()).ok().map(Into::into);
    if let Some(voice) = &file.voice {
        config.info = Some(AttachmentInfo::Voice(BaseAudioInfo {
            duration: Some(std::time::Duration::from_millis(voice.duration_ms)),
            waveform: Some(voice.waveform.clone()),
            size,
        }));
    } else if mime.type_().as_str() == "image"
        && let Some((w, h)) = image_size(&data)
    {
        config.info = Some(AttachmentInfo::Image(BaseImageInfo {
            width: Some(w.into()),
            height: Some(h.into()),
            size,
            ..Default::default()
        }));
    } else {
        // An image whose header does not parse is still an m.image, whose info is image info.
        config.info = Some(if mime.type_().as_str() == "image" {
            AttachmentInfo::Image(BaseImageInfo { size, ..Default::default() })
        } else {
            AttachmentInfo::File(BaseFileInfo { size })
        });
    }
    if mime == mime_guess::mime::IMAGE_GIF {
        config.thumbnail = gif_thumbnail(&data);
    }
    if !request.body.trim().is_empty() {
        // The caption is the composer text: markdown and mentions like a text message.
        let caption = markdown_content(&request.body, &request.mentions);
        config.mentions = caption.mentions;
        if let matrix_sdk::ruma::events::room::message::MessageType::Text(text) = caption.msgtype {
            config.caption = Some(text);
        }
    }
    if let Some(album) = &file.album {
        config.extra_content = Some(serde_json::Map::from_iter([(
            ALBUM_KEY.to_owned(),
            serde_json::json!({ "id": album.id, "index": album.index, "count": album.count }),
        )]));
    }
    queue_attachment_waiting(client, room, timeline, (file.filename.clone(), data.into_owned()), mime, config).await
}

/// The picture downscaled to `COMPRESSED_SIDE` (same format, metadata gone); `None` when it is small enough, not a
/// JPEG or PNG, or unreadable: those go out as they are. Decoding runs off the async threads.
#[cfg(feature = "sdk")]
async fn compress_picture(data: &[u8]) -> Option<Vec<u8>> {
    let data = data.to_vec();
    tokio::task::spawn_blocking(move || downscale(&data)).await.ok()?
}

#[cfg(feature = "sdk")]
fn downscale(data: &[u8]) -> Option<Vec<u8>> {
    use image::ImageFormat::{Jpeg, Png};
    let format = image::guess_format(data).ok().filter(|f| matches!(f, Jpeg | Png))?;
    let (w, h) = image_size(data)?;
    if w.max(h) <= COMPRESSED_SIDE {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 << 20);
    reader.limits(limits);
    let small = reader.decode().ok()?.resize(COMPRESSED_SIDE, COMPRESSED_SIDE, image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    small.write_to(&mut std::io::Cursor::new(&mut out), format).ok()?;
    (out.len() < data.len()).then_some(out)
}

/// An attachment through the room's SDK send queue (local echo, upload and event in the queue; the
/// bytes live in the encrypted-capable SDK media cache, never in a temp file) and wait for the
/// server to accept it. A failure or the 300 s deadline takes it out of the queue again, so an
/// error never leaves a file behind that would go out later.
#[cfg(feature = "sdk")]
pub(crate) async fn queue_attachment_waiting(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::Room,
    timeline: &matrix_sdk_ui::timeline::Timeline,
    (filename, bytes): (String, Vec<u8>),
    mime: mime_guess::mime::Mime,
    mut config: matrix_sdk_ui::timeline::AttachmentConfig,
) -> Result<EventId, SendError> {
    use matrix_sdk::{
        ruma::OwnedTransactionId,
        send_queue::{LocalEchoContent, RoomSendQueueUpdate::{SendError as Failed, SentEvent}},
    };
    use tokio::sync::broadcast::error::RecvError;
    let txn: OwnedTransactionId = config.txn_id.get_or_insert_with(matrix_sdk::ruma::TransactionId::new).clone();
    let mut updates = client.send_queue().subscribe();
    timeline
        .send_attachment(matrix_sdk_ui::timeline::AttachmentSource::Data { bytes, filename }, mime, config)
        .use_send_queue()
        .await
        .map_err(|_| SendError::Transport)?;
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(300), async {
        loop {
            let update = match updates.recv().await {
                Ok(update) => update,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return Err(SendError::Transport),
            };
            if update.room_id != room.room_id() {
                continue;
            }
            match update.update {
                SentEvent { transaction_id, event_id } if transaction_id == txn => {
                    return Ok(EventId::new(event_id.as_str()));
                }
                Failed { transaction_id, error, .. } if transaction_id == txn => {
                    return Err(map_error(&error));
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap_or(Err(SendError::Transport));
    if outcome.is_err()
        && let Ok(echoes) = client.send_queue().local_echoes().await
        && let Some(echo) = echoes.into_values().flatten().find(|e| e.transaction_id == txn)
        && let LocalEchoContent::Event { send_handle, .. } = echo.content
    {
        let _ = send_handle.abort().await;
    }
    if outcome.is_err() {
        // A failed send switches every queue off; the offender is gone, so later sends must not stall.
        client.send_queue().set_enabled(true).await;
    }
    outcome
}

#[cfg(test)]
mod mention_tests {
    use super::{Mention, mention_markdown};
    fn m(user: &str, label: &str) -> Mention {
        Mention {
            user_id: user.into(),
            label: label.into(),
        }
    }
    #[test]
    fn links_each_mention_once_and_escapes_html() {
        let mentions = [m("@a:x", "@Anna"), m("@b:x", "@Anna")];
        let (source, linked) = mention_markdown("<hi> @Anna & @Anna", &mentions);
        assert_eq!(
            source,
            "\\<hi> [@Anna](https://matrix.to/#/@a:x) & [@Anna](https://matrix.to/#/@b:x)"
        );
        assert_eq!(linked, ["@a:x", "@b:x"]);
        assert_eq!(
            mention_markdown("ciao", &[m("@a:x", "@Anna")]),
            ("ciao".into(), vec![])
        );
    }
}
