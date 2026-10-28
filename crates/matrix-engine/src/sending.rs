//! Durable, account/room-scoped sends. No view cancellation or payload logging.
use matrix_domain::{EventId, RoomScope, TransactionId};

#[derive(Clone)]
pub struct SendRequest {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    /// Text message body, or the caption when `attachment` is set.
    pub body: String,
    pub attachment: Option<Attachment>,
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
            if spans.iter().all(|&(s, e, _)| end <= s || start >= e) {
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
    (hard_breaks(&source), spans.into_iter().map(|(_, _, user)| user).collect())
}

/// A typed line break stays a line break in the HTML (markdown folds it into a space), except inside code fences.
fn hard_breaks(source: &str) -> String {
    let (mut fenced, mut out) = (false, String::new());
    let mut lines = source.split('\n').peekable();
    while let Some(line) = lines.next() {
        out.push_str(line);
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
        }
        if lines.peek().is_some() {
            if !fenced && !line.trim().is_empty() {
                out.push_str("  ");
            }
            out.push('\n');
        }
    }
    out
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
#[derive(Clone)]
pub struct Attachment {
    pub filename: String,
    pub data: Vec<u8>,
    /// Set for a recorded voice note: sent with the voice markers, duration and waveform.
    pub voice: Option<Voice>,
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
    Transport,
}

pub struct SendResult {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    pub result: Result<EventId, SendError>,
}

#[cfg(all(test, feature = "sdk"))]
mod tests {
    #[test]
    fn image_size_reads_png_header() {
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(5, 3)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(super::image_size(&png), Some((5, 3)));
        assert_eq!(super::image_size(b"not an image"), None);
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
        let v = sent("**forte** _corsivo_ `codice`\n- uno\n- due\n\n> cita [sito](https://e.org)", &[]);
        assert_eq!(v["format"], "org.matrix.custom.html");
        assert_eq!(v["body"], "**forte** _corsivo_ `codice`\n- uno\n- due\n\n> cita [sito](https://e.org)");
        let html = v["formatted_body"].as_str().unwrap();
        for tag in ["<strong>forte</strong>", "<em>corsivo</em>", "<code>codice</code>", "<li>uno</li>", "<blockquote>", "<a href=\"https://e.org\">sito</a>"] {
            assert!(html.contains(tag), "{tag} missing in {html}");
        }
        // Single newlines stay line breaks.
        assert!(sent("a\nb", &[])["formatted_body"].as_str().is_some_and(|h| h.contains("<br")));
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
            &[Mention { user_id: "@a:x".into(), label: "@Anna_B".into() }],
        );
        assert_eq!(v["body"], "**ciao** @Anna_B *ok*");
        let html = v["formatted_body"].as_str().unwrap();
        assert!(html.contains("<strong>ciao</strong>") && html.contains("<em>ok</em>"), "{html}");
        assert!(html.contains("<a href=\"https://matrix.to/#/@a:x\">@Anna_B</a>"), "{html}");
        assert_eq!(v["m.mentions"]["user_ids"], serde_json::json!(["@a:x"]));
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

/// Pixel size read from the header only, without decoding the image.
#[cfg(feature = "sdk")]
fn image_size(data: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

#[cfg(feature = "sdk")]
pub async fn send_text(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    request: &SendRequest,
) -> Result<EventId, SendError> {
    use matrix_sdk::{
        attachment::{AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseImageInfo},
        ruma::{
            RoomId,
            events::room::message::TextMessageEventContent,
        },
    };
    if request.scope.account_id != account {
        return Err(SendError::WrongScope);
    }
    if (request.body.trim().is_empty() && request.attachment.is_none())
        || request.transaction_id.as_str().is_empty()
    {
        return Err(SendError::InvalidInput);
    }
    let id = RoomId::parse(request.scope.room_id.as_str()).map_err(|_| SendError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(SendError::Unavailable)?;
    let transaction: matrix_sdk::ruma::OwnedTransactionId = request.transaction_id.as_str().into();
    let response = match &request.attachment {
        None => tokio::time::timeout(
            std::time::Duration::from_secs(5),
            room.send(markdown_content(&request.body, &request.mentions))
            .with_transaction_id(transaction),
        )
        .await
        .map_err(|_| SendError::Transport)?
        .map(|sent| sent.response.event_id),
        // shortcut: no client thumbnail and no total-upload progress, add when the timeline shows uploads
        Some(file) => {
            let mime = mime_guess::from_path(&file.filename).first_or_octet_stream();
            let mut config = AttachmentConfig::new().txn_id(transaction);
            if let Some(voice) = &file.voice {
                config = config.info(AttachmentInfo::Voice(BaseAudioInfo {
                    duration: Some(std::time::Duration::from_millis(voice.duration_ms)),
                    waveform: Some(voice.waveform.clone()),
                    size: u32::try_from(file.data.len()).ok().map(Into::into),
                }));
            } else if mime.type_().as_str() == "image"
                && let Some((w, h)) = image_size(&file.data)
            {
                config = config.info(AttachmentInfo::Image(BaseImageInfo {
                    width: Some(w.into()),
                    height: Some(h.into()),
                    size: u32::try_from(file.data.len()).ok().map(Into::into),
                    ..Default::default()
                }));
            }
            if !request.body.trim().is_empty() {
                config = config.caption(Some(TextMessageEventContent::plain(&request.body)));
            }
            tokio::time::timeout(
                std::time::Duration::from_secs(300),
                room.send_attachment(&file.filename, &mime, file.data.clone(), config),
            )
            .await
            .map_err(|_| SendError::Transport)?
            .map(|sent| sent.event_id)
        }
    }
    .map_err(|error| match error {
        matrix_sdk::Error::NoOlmMachine
        | matrix_sdk::Error::CryptoStoreError(_)
        | matrix_sdk::Error::OlmError(_)
        | matrix_sdk::Error::MegolmError(_)
        | matrix_sdk::Error::DecryptorError(_)
        | matrix_sdk::Error::SignatureError(_) => SendError::Encryption,
        _ => SendError::Transport,
    })?;
    Ok(EventId::new(response.as_str()))
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
        assert_eq!(mention_markdown("ciao", &[m("@a:x", "@Anna")]), ("ciao".into(), vec![]));
    }
}
