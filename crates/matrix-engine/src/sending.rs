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

/// `body` as HTML with each mention's first free occurrence turned into a matrix.to link; None when none of them is still in the text.
pub fn mention_html(body: &str, mentions: &[Mention]) -> Option<String> {
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
    if spans.is_empty() {
        return None;
    }
    spans.sort();
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\n', "<br>")
    };
    let (mut html, mut at) = (String::new(), 0);
    for (start, end, user) in spans {
        html.push_str(&escape(&body[at..start]));
        html.push_str(&format!(
            "<a href=\"https://matrix.to/#/{user}\">{}</a>",
            escape(&body[start..end])
        ));
        at = end;
    }
    html.push_str(&escape(&body[at..]));
    Some(html)
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
            events::{
                Mentions,
                room::message::{RoomMessageEventContent, TextMessageEventContent},
            },
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
            room.send(match mention_html(&request.body, &request.mentions) {
                Some(html) => {
                    let ids = request
                        .mentions
                        .iter()
                        .filter_map(|m| m.user_id.parse().ok());
                    RoomMessageEventContent::text_html(&request.body, html)
                        .add_mentions(Mentions::with_user_ids(ids))
                }
                None => RoomMessageEventContent::text_plain(&request.body),
            })
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
    use super::{Mention, mention_html};
    fn m(user: &str, label: &str) -> Mention {
        Mention {
            user_id: user.into(),
            label: label.into(),
        }
    }
    #[test]
    fn links_each_mention_once_and_escapes_the_rest() {
        let html = mention_html(
            "<hi> @Anna & @Anna",
            &[m("@a:x", "@Anna"), m("@b:x", "@Anna")],
        )
        .unwrap();
        assert_eq!(
            html,
            "&lt;hi&gt; <a href=\"https://matrix.to/#/@a:x\">@Anna</a> &amp; <a href=\"https://matrix.to/#/@b:x\">@Anna</a>"
        );
        assert_eq!(mention_html("ciao", &[m("@a:x", "@Anna")]), None);
        assert!(
            mention_html("a\n@Anna", &[m("@a:x", "@Anna")])
                .unwrap()
                .starts_with("a<br>")
        );
    }
}
