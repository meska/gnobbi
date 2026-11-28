//! Forwarding a message to other rooms (MSC4553, unstable prefix `org.matrix.msc4553.forwarded`).
//! `build` is the pure part (JSON in, JSON out); `forward` sends through the room's SDK send queue.
use serde_json::{Map, Value, json};

/// The unstable name of `m.forwarded`; the older MSC2723 and the stable name are only read.
pub const FORWARDED: &str = "org.matrix.msc4553.forwarded";
const FORWARDED_KEYS: [&str; 3] = ["m.forwarded", FORWARDED, "com.famedly.app.forwarded"];
const MESSAGE: &str = "m.room.message";
const STICKER: &str = "m.sticker";
const MEDIA: [&str; 4] = ["m.image", "m.video", "m.audio", "m.file"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardError {
    /// A state event, a message not decrypted yet, a redaction, an unsent echo, a type MSC4553 leaves out.
    NotForwardable,
    /// The destination room is not known to this account.
    UnknownRoom,
    /// The plain media could not be fetched or the queue refused the message.
    Transport,
}

/// The event to forward, as the timeline holds it.
pub struct Source<'a> {
    pub room_id: &'a str,
    pub event_id: &'a str,
    pub sender: &'a str,
    pub origin_server_ts: u64,
    pub event_type: &'a str,
    /// `content` of the original event (decrypted).
    pub content: &'a Value,
    /// `content` of the latest edit event, if the message was edited.
    pub edit: Option<&'a Value>,
}

/// Where the bytes of a media message come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    /// `mxc://` URI of a plain upload.
    Plain(String),
    /// The `file` object (`EncryptedFile`) of an encrypted upload.
    Encrypted(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Forward {
    /// Queue this event as is.
    Send { event_type: &'static str, content: Value },
    /// Media whose upload cannot be reused (plain into an encrypted room, encrypted into a plain one): download `source`
    /// (decrypted by the SDK), upload it again as a new attachment. `content` is the event to send without `url`, `file`
    /// and `info`: its text becomes the caption, its marker an extra field; `info` and the `thumbnail` carry the metadata over.
    Reupload { source: Origin, thumbnail: Option<(Origin, Value)>, mime: String, info: Value, content: Value },
}

/// What the UI asks before offering «Forward»: an `m.room.message` of a forwardable kind with its payload
/// still readable, or an `m.sticker`.
pub fn can_forward(event_type: &str, content: &Value) -> bool {
    let has = |key: &str| content.get(key).is_some_and(Value::is_string);
    match event_type {
        STICKER => has("url"),
        MESSAGE => match content.get("msgtype").and_then(Value::as_str) {
            Some("m.text" | "m.notice" | "m.emote") => has("body"),
            Some("m.location") => has("geo_uri"),
            Some(t) if MEDIA.contains(&t) => has("url") || content.get("file").is_some_and(Value::is_object),
            _ => false,
        },
        _ => false,
    }
}

/// The forward mark of a received message, when its three IDs are valid Matrix IDs: they come from the network and
/// end up in a `matrix:` link, so anything else counts as no mark.
fn forwarded_ref(content: &Value) -> Option<&Map<String, Value>> {
    use matrix_sdk::ruma::{EventId, RoomId, UserId};
    let old = FORWARDED_KEYS.iter().find_map(|k| content.get(*k))?.as_object()?;
    let id = |k: &str| old.get(k).and_then(Value::as_str);
    (UserId::parse(id("sender")?).is_ok() && RoomId::parse(id("room_id")?).is_ok() && EventId::parse(id("event_id")?).is_ok()).then_some(old)
}

const TEXT: [&str; 3] = ["m.text", "m.notice", "m.emote"];

/// A forwarded message as the user sees it: `body` holds the fallback for old clients, the mark holds the true text.
/// The mark's text replaces it when its content is of the same kind; the attachment, the geo URI and everything else
/// stay the message's own, so nothing is shown or forwarded that the message did not carry. `None` when not forwarded.
pub fn shown(content: &Value) -> Option<Value> {
    let nested = forwarded_ref(content)?.get("content")?.as_object()?;
    let (top, inner) = (text(content, "msgtype")?, nested.get("msgtype")?.as_str()?);
    if !nested.get("body").is_some_and(Value::is_string) || !(top == inner || (TEXT.contains(&top) && TEXT.contains(&inner))) {
        return None;
    }
    let mut out = content.clone();
    let map = out.as_object_mut()?;
    for key in ["body", "format", "formatted_body"] {
        map.remove(key);
        if let Some(v) = nested.get(key) {
            map.insert(key.into(), v.clone());
        }
    }
    if TEXT.contains(&inner) {
        map.insert("msgtype".into(), json!(inner));
    }
    Some(out)
}

/// The content to send to a room, or the upload to do first. `dest_encrypted`: the destination is an E2EE room.
pub fn build(source: &Source, dest_encrypted: bool) -> Result<Forward, ForwardError> {
    let event_type = match source.event_type {
        MESSAGE => MESSAGE,
        STICKER => STICKER,
        _ => return Err(ForwardError::NotForwardable),
    };
    // A forward of a forward keeps the first attribution and the text it shows; an edit replaces both.
    let edited = source.edit.and_then(|e| e.get("m.new_content"));
    let shown_content = if edited.is_none() { shown(source.content) } else { None };
    let latest = edited.or(shown_content.as_ref()).unwrap_or(source.content);
    if !can_forward(source.event_type, latest) {
        return Err(ForwardError::NotForwardable);
    }
    let mut inner = original_content(source, latest, shown_content.is_none());
    let msgtype = inner.get("msgtype").and_then(Value::as_str).unwrap_or_default().to_owned();
    let mut forwarded = Map::new();
    match forwarded_ref(source.content) {
        Some(old) => {
            for key in ["event_id", "room_id", "sender", "origin_server_ts"] {
                forwarded.extend(old.get(key).map(|v| (key.to_owned(), v.clone())));
            }
        }
        None => forwarded.extend([
            ("event_id".into(), json!(source.event_id)),
            ("room_id".into(), json!(source.room_id)),
            ("sender".into(), json!(source.sender)),
            ("origin_server_ts".into(), json!(source.origin_server_ts)),
        ]),
    }
    let media = MEDIA.contains(&msgtype.as_str());
    let origin = |file: &str, url: &str| match (inner.get(file), text(&inner, url)) {
        (Some(f), _) if f.is_object() => Some(Origin::Encrypted(f.clone())),
        (_, Some(u)) => Some(Origin::Plain(u.to_owned())),
        _ => None,
    };
    let reupload = origin("file", "url").filter(|o| media && matches!((o, dest_encrypted), (Origin::Plain(_), true) | (Origin::Encrypted(_), false)));
    let thumbnail = reupload.is_some().then(|| origin_in(&inner["info"])).flatten();
    let info = inner.get("info").cloned().unwrap_or(json!({}));
    if reupload.is_some() {
        // The new upload has its own `url` or `file`: the old source, with its key, goes nowhere, nor do its thumbnails.
        let map = inner.as_object_mut().expect("content is an object");
        map.remove("url");
        map.remove("file");
        if let Some(info) = map.get_mut("info").and_then(Value::as_object_mut) {
            info.retain(|k, _| !k.starts_with("thumbnail_"));
        }
    }
    if msgtype == "m.location"
        && let Some(info) = inner.get_mut("info").and_then(Value::as_object_mut)
    {
        // A thumbnail follows the room: a plain one never into an encrypted room, a file with its key never into a plain one.
        info.remove(if dest_encrypted { "thumbnail_url" } else { "thumbnail_file" });
        if !info.contains_key("thumbnail_url") && !info.contains_key("thumbnail_file") {
            info.remove("thumbnail_info");
        }
    }
    forwarded.insert("content".into(), inner.clone());
    let mut top = inner;
    apply_fallback(&mut top, &forwarded);
    let top = top.as_object_mut().expect("content is an object");
    top.insert("m.mentions".into(), json!({}));
    top.insert(FORWARDED.into(), Value::Object(forwarded));
    if reupload.is_some() {
        top.remove("info");
    }
    let content = Value::Object(std::mem::take(top));
    Ok(match reupload {
        Some(source) => {
            let mime = info["mimetype"].as_str().unwrap_or("application/octet-stream").to_owned();
            Forward::Reupload { source, thumbnail, mime, info, content }
        }
        None => Forward::Send { event_type, content },
    })
}

/// The thumbnail of a media `info` with its description, where to download it.
fn origin_in(info: &Value) -> Option<(Origin, Value)> {
    let origin = match (info.get("thumbnail_file"), text(info, "thumbnail_url")) {
        (Some(f), _) if f.is_object() => Origin::Encrypted(f.clone()),
        (_, Some(u)) => Origin::Plain(u.to_owned()),
        _ => return None,
    };
    Some((origin, info.get("thumbnail_info").cloned().unwrap_or(json!({}))))
}

/// The content without relation, mentions, forward marks and, for a reply, its quoted first line.
fn original_content(source: &Source, latest: &Value, strip_quote: bool) -> Value {
    let mut content = latest.clone();
    let Some(map) = content.as_object_mut() else { return content };
    // The first line of a reply is quoted only when the event was a reply.
    let was_reply = strip_quote && source.content["m.relates_to"]["m.in_reply_to"].is_object();
    for key in ["m.relates_to", "m.mentions", "m.new_content"].into_iter().chain(FORWARDED_KEYS) {
        map.remove(key);
    }
    if was_reply {
        if let Some(Value::String(body)) = map.get_mut("body") {
            let rest = body.lines().skip_while(|l| l.starts_with('>')).skip_while(|l| l.is_empty());
            *body = rest.collect::<Vec<_>>().join("\n");
        }
        if let Some(Value::String(html)) = map.get_mut("formatted_body")
            && let Some(end) = html.find("</mx-reply>")
            && html.starts_with("<mx-reply>")
        {
            html.drain(..end + "</mx-reply>".len());
        }
    }
    content
}

fn text<'a>(content: &'a Value, key: &str) -> Option<&'a str> {
    content.get(key).and_then(Value::as_str)
}

/// Media caption: the body when a `filename` says the body is not the file name.
fn caption(content: &Value) -> Option<&str> {
    let (filename, body) = (text(content, "filename")?, text(content, "body")?);
    (filename != body && !body.is_empty()).then_some(body)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "<br>")
}

/// `matrix:` URI part: an identifier without its sigil, percent-encoded.
fn uri_id(id: &str) -> String {
    id.get(1..).unwrap_or_default().bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-._~:".contains(&b) { out.push(b as char) } else { out.push_str(&format!("%{b:02X}")) }
        out
    })
}

/// MSC4553 text/HTML fallback for clients that do not know the forward. Stickers, locations and unknown
/// types keep their own body; an emote becomes a text that names the actor.
fn apply_fallback(top: &mut Value, forwarded: &Map<String, Value>) {
    let (Some(sender), Some(room), Some(event)) = (
        forwarded.get("sender").and_then(Value::as_str),
        forwarded.get("room_id").and_then(Value::as_str),
        forwarded.get("event_id").and_then(Value::as_str),
    ) else {
        return;
    };
    let msgtype = text(top, "msgtype").unwrap_or_default().to_owned();
    let is_media = MEDIA.contains(&msgtype.as_str());
    if !(is_media || matches!(msgtype.as_str(), "m.text" | "m.notice" | "m.emote")) {
        return;
    }
    let via = sender.split_once(':').map_or(String::new(), |(_, server)| format!("?via={server}"));
    let permalink = format!("matrix:roomid/{}/e/{}{via}", uri_id(room), uri_id(event));
    let sender_link = format!("matrix:u/{}", uri_id(sender));
    // The quoted part: the text, or the caption of a media message (a file name is not quoted).
    let (quoted, quoted_html) = if is_media {
        let caption = caption(top).map(str::to_owned);
        let html = caption.as_deref().map(|c| match (text(top, "format"), text(top, "formatted_body")) {
            (Some("org.matrix.custom.html"), Some(html)) => html.to_owned(),
            _ => escape(c),
        });
        (caption, html)
    } else {
        let body = text(top, "body").unwrap_or_default().to_owned();
        let html = match (text(top, "format"), text(top, "formatted_body")) {
            (Some("org.matrix.custom.html"), Some(html)) => html.to_owned(),
            _ => escape(&body),
        };
        (Some(body), Some(html))
    };
    let emote = msgtype == "m.emote";
    let quoted = quoted.filter(|q| !q.is_empty()).map(|q| if emote { format!("{sender} {q}") } else { q });
    let quoted_html = quoted.as_ref().and(quoted_html).map(|h| {
        if emote {
            format!("<em><strong><a href=\"{}\">{}</a></strong> {h}</em>", escape(&sender_link), escape(sender))
        } else {
            h
        }
    });
    let head = format!("Forwarded from {sender} - view original message: {permalink}");
    let head_html = format!(
        "<strong>Forwarded from <a href=\"{}\">{}</a> - <a href=\"{}\">view original message</a></strong>",
        escape(&sender_link),
        escape(sender),
        escape(&permalink)
    );
    let map = top.as_object_mut().expect("content is an object");
    if is_media {
        let name = text_of(map, "filename").or_else(|| text_of(map, "body")).unwrap_or_default();
        map.insert("filename".into(), json!(name));
    } else if emote {
        map.insert("msgtype".into(), json!("m.text"));
    }
    map.insert("body".into(), json!(match &quoted { Some(q) => format!("{head}\n{q}"), None => head }));
    map.insert("format".into(), json!("org.matrix.custom.html"));
    map.insert(
        "formatted_body".into(),
        json!(match quoted_html { Some(h) => format!("{head_html}<br><blockquote>{h}</blockquote>"), None => head_html }),
    );
}

fn text_of(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Whether a timeline item can be forwarded: sent (it has a remote event), readable, of a forwardable kind.
#[cfg(feature = "sdk")]
pub fn can_forward_item(item: &matrix_sdk_ui::timeline::EventTimelineItem) -> bool {
    event_of(item).is_some_and(|(event_type, content, _)| can_forward(&event_type, &content))
}

/// Type, content and latest-edit content of a timeline item's event.
#[cfg(feature = "sdk")]
fn event_of(item: &matrix_sdk_ui::timeline::EventTimelineItem) -> Option<(String, Value, Option<Value>)> {
    let parse = |raw: &matrix_sdk::ruma::serde::Raw<matrix_sdk::ruma::events::AnySyncTimelineEvent>| {
        serde_json::from_str::<Value>(raw.json().get()).ok()
    };
    item.event_id()?;
    let event = parse(item.original_json()?)?;
    let edit = item.latest_edit_json().and_then(parse).map(|e| e["content"].clone());
    Some((event["type"].as_str()?.to_owned(), event["content"].clone(), edit))
}

/// Forward `item` (an event of `source_room`) to every room in `rooms`; one result per room, in order. A message
/// goes into each room's SDK send queue like a typed one, so the outbox and cancelling work the same.
#[cfg(feature = "sdk")]
pub async fn forward(
    client: &matrix_sdk::Client,
    source_room: &str,
    item: &matrix_sdk_ui::timeline::EventTimelineItem,
    rooms: &[String],
) -> Vec<(String, Result<(), ForwardError>)> {
    let mut results = Vec::with_capacity(rooms.len());
    let event = event_of(item);
    for id in rooms {
        let outcome = match &event {
            Some((event_type, content, edit)) => {
                let source = Source {
                    room_id: source_room,
                    event_id: item.event_id().map_or("", |e| e.as_str()),
                    sender: item.sender().as_str(),
                    origin_server_ts: item.timestamp().0.into(),
                    event_type,
                    content,
                    edit: edit.as_ref(),
                };
                send_to(client, id, &source).await
            }
            None => Err(ForwardError::NotForwardable),
        };
        results.push((id.clone(), outcome));
    }
    results
}

/// `forward` for the message `target` of `scope`, looked up in the room's timeline or in one of its open threads; every room fails alike when it is not found.
#[cfg(feature = "sdk")]
pub(crate) async fn forward_event(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    target: &matrix_domain::EventId,
    rooms: &[String],
) -> Vec<(String, Result<(), ForwardError>)> {
    let room = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).ok().and_then(|id| client.get_room(&id)).filter(|_| scope.account_id == account);
    let id = matrix_sdk::ruma::EventId::parse(target.as_str()).ok();
    let item = match (room, id) {
        (Some(room), Some(id)) => timelines.item(&room, &id).await,
        _ => None,
    };
    match item {
        Some(item) => forward(client, scope.room_id.as_str(), &item, rooms).await,
        None => rooms.iter().map(|r| (r.clone(), Err(ForwardError::NotForwardable))).collect(),
    }
}

/// The bytes of `origin`, decrypted by the SDK, at most `cap` of them.
#[cfg(feature = "sdk")]
async fn fetch(client: &matrix_sdk::Client, origin: &Origin, cap: usize) -> Result<Vec<u8>, ForwardError> {
    use matrix_sdk::{
        media::{DefaultMediaFetcher, MediaFormat, MediaRequestParameters},
        ruma::events::room::MediaSource,
    };
    let source = match origin {
        Origin::Plain(mxc) => MediaSource::Plain(mxc.as_str().into()),
        Origin::Encrypted(file) => MediaSource::Encrypted(Box::new(serde_json::from_value(file.clone()).map_err(|_| ForwardError::Transport)?)),
    };
    let source = crate::media_sdk::checked_source(source).map_err(|_| ForwardError::Transport)?;
    let config = client.request_config().timeout(Some(std::time::Duration::MAX)).max_response_size(Some(cap));
    DefaultMediaFetcher.fetch_media_content_with_request_config(client, &MediaRequestParameters { source, format: MediaFormat::File }, config).await.map_err(|_| ForwardError::Transport)
}

/// The SDK's attachment metadata from the `info` of the original message.
#[cfg(feature = "sdk")]
fn attachment_info(msgtype: &str, voice: bool, info: &Value) -> Option<matrix_sdk::attachment::AttachmentInfo> {
    use matrix_sdk::attachment::{AttachmentInfo as A, BaseAudioInfo, BaseFileInfo, BaseImageInfo, BaseVideoInfo};
    let uint = |k: &str| info.get(k).and_then(Value::as_u64).and_then(matrix_sdk::ruma::UInt::new);
    let duration = info.get("duration").and_then(Value::as_u64).map(std::time::Duration::from_millis);
    let blurhash = || text(info, "xyz.amorgan.blurhash").map(str::to_owned);
    Some(match msgtype {
        "m.image" => A::Image(BaseImageInfo { height: uint("h"), width: uint("w"), size: uint("size"), blurhash: blurhash(), is_animated: info.get("is_animated").and_then(Value::as_bool) }),
        "m.video" => A::Video(BaseVideoInfo { duration, height: uint("h"), width: uint("w"), size: uint("size"), blurhash: blurhash() }),
        "m.audio" => {
            let audio = BaseAudioInfo { duration, size: uint("size"), waveform: None };
            if voice { A::Voice(audio) } else { A::Audio(audio) }
        }
        "m.file" => A::File(BaseFileInfo { size: uint("size") }),
        _ => return None,
    })
}

/// The attachment of a `Forward::Reupload`: its metadata, thumbnail, caption (the fallback text of the content), no mention,
/// and the forward mark as an extra field of the event.
#[cfg(feature = "sdk")]
fn attachment_config(content: &Value, info: &Value, thumbnail: Option<matrix_sdk::attachment::Thumbnail>) -> matrix_sdk_ui::timeline::AttachmentConfig {
    use matrix_sdk::ruma::events::{Mentions, room::message::TextMessageEventContent};
    let body = text(content, "body").unwrap_or_default();
    let caption = match text(content, "formatted_body") {
        Some(html) => TextMessageEventContent::html(body, html),
        None => TextMessageEventContent::plain(body),
    };
    matrix_sdk_ui::timeline::AttachmentConfig {
        info: attachment_info(text(content, "msgtype").unwrap_or_default(), content.get("org.matrix.msc3245.voice").is_some(), info),
        thumbnail,
        caption: Some(caption),
        mentions: Some(Mentions::new()),
        extra_content: Some(Map::from_iter([(FORWARDED.to_owned(), content[FORWARDED].clone())])),
        ..Default::default()
    }
}

#[cfg(feature = "sdk")]
async fn send_to(client: &matrix_sdk::Client, room_id: &str, source: &Source<'_>) -> Result<(), ForwardError> {
    use matrix_sdk::ruma::{OwnedRoomId, serde::Raw};
    use matrix_sdk_ui::timeline::RoomExt;
    let room = OwnedRoomId::try_from(room_id).ok().and_then(|id| client.get_room(&id)).ok_or(ForwardError::UnknownRoom)?;
    // When the state is unknown, assume encryption: a plain `url` is the thing that must never leak.
    let encrypted = room.latest_encryption_state().await.is_ok_and(|s| s.is_encrypted()) || room.encryption_state().is_unknown();
    match build(source, encrypted)? {
        Forward::Send { event_type, content } => {
            let raw = Raw::new(&content).map_err(|_| ForwardError::Transport)?.cast_unchecked();
            room.send_queue().send_raw(raw, event_type.to_owned()).await.map_err(|_| ForwardError::Transport)?;
            Ok(())
        }
        Forward::Reupload { source, thumbnail, mime, info, content } => {
            let bytes = fetch(client, &source, crate::media::MAX_SAVE_BYTES).await?;
            // A thumbnail is only metadata: without one the message still goes.
            let thumbnail = match thumbnail {
                Some((origin, described)) => fetch(client, &origin, crate::media::MAX_MEDIA_BYTES).await.ok().and_then(|data| {
                    let uint = |k: &str| described.get(k).and_then(Value::as_u64).and_then(matrix_sdk::ruma::UInt::new);
                    Some(matrix_sdk::attachment::Thumbnail { content_type: text(&described, "mimetype")?.parse().ok()?, height: uint("h")?, width: uint("w")?, size: (data.len() as u64).try_into().ok()?, data })
                }),
                None => None,
            };
            let filename = text(&content, "filename").or_else(|| text(&content, "body")).unwrap_or("file").to_owned();
            let mime = mime.parse().unwrap_or(mime_guess::mime::APPLICATION_OCTET_STREAM);
            let config = attachment_config(&content, &info, thumbnail);
            let timeline = room.timeline().await.map_err(|_| ForwardError::Transport)?;
            crate::sending::queue_attachment_waiting(client, &room, &timeline, (filename, bytes), mime, config, None).await.map(|_| ()).map_err(|_| ForwardError::Transport)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src<'a>(content: &'a Value, edit: Option<&'a Value>) -> Source<'a> {
        Source { room_id: "!src:hs.org", event_id: "$ev", sender: "@alice:hs.org", origin_server_ts: 7, event_type: MESSAGE, content, edit }
    }
    fn sent(f: Forward) -> Value {
        match f {
            Forward::Send { content, .. } => content,
            other => panic!("expected Send, got {other:?}"),
        }
    }

    #[test]
    fn relations_mentions_and_reply_fallback_are_dropped() {
        let reply = json!({
            "msgtype": "m.text",
            "body": "> <@bob:hs.org> hi\n\nanswer",
            "format": "org.matrix.custom.html",
            "formatted_body": "<mx-reply><blockquote>hi</blockquote></mx-reply>answer",
            "m.relates_to": {"rel_type": "m.thread", "event_id": "$t", "m.in_reply_to": {"event_id": "$r"}},
            "m.mentions": {"user_ids": ["@bob:hs.org"]}
        });
        let out = sent(build(&src(&reply, None), false).unwrap());
        assert!(out.get("m.relates_to").is_none());
        assert_eq!(out["m.mentions"], json!({}));
        let inner = &out[FORWARDED]["content"];
        assert_eq!(inner["body"], "answer");
        assert_eq!(inner["formatted_body"], "answer");
        assert!(inner.get("m.relates_to").is_none() && inner.get("m.mentions").is_none());
        assert_eq!(out[FORWARDED]["sender"], "@alice:hs.org");
        assert!(out["body"].as_str().unwrap().starts_with("Forwarded from @alice:hs.org - view original message: matrix:roomid/src:hs.org/e/ev?via=hs.org\nanswer"));
    }

    #[test]
    fn latest_edit_is_forwarded_without_the_edit_relation() {
        let original = json!({"msgtype": "m.text", "body": "old"});
        let edit = json!({
            "msgtype": "m.text", "body": "* new",
            "m.new_content": {"msgtype": "m.text", "body": "new", "format": "org.matrix.custom.html", "formatted_body": "<b>new</b>"},
            "m.relates_to": {"rel_type": "m.replace", "event_id": "$ev"}
        });
        let out = sent(build(&src(&original, Some(&edit)), false).unwrap());
        assert_eq!(out[FORWARDED]["content"]["body"], "new");
        assert!(out[FORWARDED]["content"].get("m.new_content").is_none() && out.get("m.relates_to").is_none() && out.get("m.new_content").is_none());
        assert!(out["formatted_body"].as_str().unwrap().contains("<blockquote><b>new</b></blockquote>"));
    }

    fn reuploaded(f: Forward) -> (Origin, Option<(Origin, Value)>, String, Value, Value) {
        match f {
            Forward::Reupload { source, thumbnail, mime, info, content } => (source, thumbnail, mime, info, content),
            other => panic!("expected Reupload, got {other:?}"),
        }
    }

    #[test]
    fn encrypted_media_is_reused_in_an_encrypted_room_and_downloaded_again_for_a_plain_one() {
        let file = json!({"url": "mxc://hs.org/a", "key": {"k": "K"}, "iv": "I", "hashes": {"sha256": "H"}, "v": "v2"});
        let thumb = json!({"url": "mxc://hs.org/t", "key": {"k": "T"}, "iv": "I", "hashes": {"sha256": "H"}, "v": "v2"});
        let content = json!({"msgtype": "m.image", "body": "cat.png", "file": file, "info": {"mimetype": "image/png", "w": 3, "h": 2, "thumbnail_file": thumb, "thumbnail_info": {"w": 1, "h": 1, "mimetype": "image/jpeg"}}});
        let out = sent(build(&src(&content, None), true).unwrap());
        assert_eq!(out["file"], file);
        assert!(out.get("url").is_none());
        assert_eq!(out[FORWARDED]["content"]["file"], file);
        // A file name is not a caption: nothing is quoted.
        assert_eq!(out["filename"], "cat.png");
        assert!(out["body"].as_str().unwrap().ends_with("?via=hs.org"));

        // Into a plain room no key may travel: not in the event, not in the mark, not in the thumbnail.
        let (source, thumbnail, mime, info, out) = reuploaded(build(&src(&content, None), false).unwrap());
        assert_eq!((source, mime.as_str(), info["w"].as_u64(), info["h"].as_u64()), (Origin::Encrypted(file), "image/png", Some(3), Some(2)));
        assert_eq!(thumbnail, Some((Origin::Encrypted(thumb), json!({"w": 1, "h": 1, "mimetype": "image/jpeg"}))));
        let wire = out.to_string();
        assert!(!wire.contains("\"key\"") && !wire.contains("\"iv\"") && !wire.contains("mxc://"), "{wire}");
        assert_eq!(out[FORWARDED]["sender"], "@alice:hs.org");
        assert_eq!((out["filename"].as_str(), out["m.mentions"].clone()), (Some("cat.png"), json!({})));
        assert!(out["body"].as_str().unwrap().starts_with("Forwarded from @alice:hs.org"));
    }

    #[test]
    fn plain_media_keeps_its_url_in_a_plain_room_and_is_reuploaded_with_its_attribution_for_an_encrypted_one() {
        let content = json!({"msgtype": "m.video", "body": "look", "filename": "v.mp4", "url": "mxc://hs.org/v", "info": {"mimetype": "video/mp4", "duration": 4000, "w": 640, "h": 360, "size": 99, "thumbnail_url": "mxc://hs.org/t"}});
        let out = sent(build(&src(&content, None), false).unwrap());
        assert_eq!(out["url"], "mxc://hs.org/v");
        assert!(out["body"].as_str().unwrap().ends_with("\nlook"));
        let (source, thumbnail, mime, info, out) = reuploaded(build(&src(&content, None), true).unwrap());
        assert_eq!((source, mime.as_str()), (Origin::Plain("mxc://hs.org/v".into()), "video/mp4"));
        assert_eq!(thumbnail.map(|t| t.0), Some(Origin::Plain("mxc://hs.org/t".into())));
        // The attribution and the caption the plain room's message has; the source and the thumbnail are not in the event.
        assert_eq!(out[FORWARDED]["event_id"], "$ev");
        assert_eq!(out[FORWARDED]["content"]["body"], "look");
        assert!(out["body"].as_str().unwrap().ends_with("\nlook") && out["filename"] == "v.mp4");
        assert!(out.get("url").is_none() && out.get("info").is_none() && out[FORWARDED]["content"]["info"].get("thumbnail_url").is_none());
        // The metadata reaches the SDK's attachment config.
        let config = attachment_config(&out, &info, None);
        let Some(matrix_sdk::attachment::AttachmentInfo::Video(v)) = config.info else { panic!("video info") };
        assert_eq!((v.duration, v.width.map(u64::from), v.height.map(u64::from), v.size.map(u64::from)), (Some(std::time::Duration::from_secs(4)), Some(640), Some(360), Some(99)));
        assert_eq!(config.extra_content.unwrap()[FORWARDED], out[FORWARDED]);
        assert!(config.caption.unwrap().body.ends_with("\nlook"));
    }

    #[test]
    fn location_loses_its_plain_thumbnail_in_an_encrypted_room_and_sticker_keeps_its_body() {
        let place = json!({"msgtype": "m.location", "body": "Home", "geo_uri": "geo:1,2", "info": {"thumbnail_url": "mxc://hs.org/t", "thumbnail_info": {}}});
        let out = sent(build(&src(&place, None), true).unwrap());
        assert_eq!(out["body"], "Home");
        assert!(out["info"].get("thumbnail_url").is_none() && out[FORWARDED]["content"]["info"].get("thumbnail_url").is_none());
        assert!(sent(build(&src(&place, None), false).unwrap())["info"].get("thumbnail_url").is_some());

        let sticker = json!({"body": "cat", "url": "mxc://hs.org/s", "info": {}});
        let s = Source { event_type: STICKER, ..src(&sticker, None) };
        let Forward::Send { event_type, content } = build(&s, true).unwrap() else { panic!() };
        assert_eq!((event_type, content["body"].as_str(), content["url"].as_str()), (STICKER, Some("cat"), Some("mxc://hs.org/s")));
    }

    #[test]
    fn emote_becomes_text_naming_the_actor() {
        let out = sent(build(&src(&json!({"msgtype": "m.emote", "body": "waves"}), None), false).unwrap());
        assert_eq!(out["msgtype"], "m.text");
        assert_eq!(out[FORWARDED]["content"]["msgtype"], "m.emote");
        assert!(out["body"].as_str().unwrap().ends_with("\n@alice:hs.org waves"));
    }

    #[test]
    fn a_forward_of_a_forward_keeps_the_first_attribution() {
        let first = sent(build(&src(&json!({"msgtype": "m.text", "body": "hi"}), None), false).unwrap());
        let second_source = Source { sender: "@carol:hs.org", event_id: "$fw", room_id: "!mid:hs.org", ..src(&first, None) };
        let second = sent(build(&second_source, false).unwrap());
        assert_eq!(second[FORWARDED], first[FORWARDED]);
        assert!(second["body"].as_str().unwrap().contains("@alice:hs.org"));
        // The MSC2723 name is read too, and the result uses the unstable MSC4553 name.
        let old = json!({"msgtype": "m.text", "body": "x", "com.famedly.app.forwarded": {"sender": "@dan:hs.org", "room_id": "!o:hs.org", "event_id": "$o"}});
        assert_eq!(sent(build(&src(&old, None), false).unwrap())[FORWARDED]["sender"], "@dan:hs.org");
    }

    #[test]
    fn what_cannot_be_forwarded_is_refused() {
        let text = json!({"msgtype": "m.text", "body": "x"});
        for event_type in ["m.room.encrypted", "m.room.topic", "m.reaction", "org.matrix.msc3381.poll.start"] {
            assert_eq!(build(&Source { event_type, ..src(&text, None) }, false), Err(ForwardError::NotForwardable));
            assert!(!can_forward(event_type, &text));
        }
        // Redacted (empty content), unknown msgtype, a media message with no source.
        for content in [json!({}), json!({"msgtype": "m.key.verification.request", "body": "x"}), json!({"msgtype": "m.image", "body": "a.png"})] {
            assert_eq!(build(&src(&content, None), false), Err(ForwardError::NotForwardable));
        }
        assert!(can_forward(MESSAGE, &text));
    }

    #[test]
    fn a_hostile_forward_mark_is_no_mark_and_never_panics() {
        for bad in [json!({"sender": "", "room_id": "", "event_id": ""}), json!({"sender": "\u{e9}x", "room_id": "\u{e9}x", "event_id": "\u{e9}x"}), json!({"sender": "@a:b", "room_id": "!r:b"}), json!("x")] {
            let content = json!({"msgtype": "m.text", "body": "b", "m.forwarded": bad});
            assert_eq!(shown(&content), None);
            // The message is forwarded as a new one: the attribution is the sender of the message.
            let out = sent(build(&src(&content, None), false).unwrap());
            assert_eq!(out[FORWARDED]["sender"], "@alice:hs.org");
            assert_eq!(out[FORWARDED]["event_id"], "$ev");
        }
        assert_eq!(uri_id("\u{e9}x"), "");
    }

    #[test]
    fn what_is_forwarded_is_what_the_message_shows_and_nothing_hidden_travels() {
        let hidden = json!({"msgtype": "m.text", "body": "HIDDEN", "m.relates_to": {"m.in_reply_to": {"event_id": "$x"}}, "m.mentions": {"room": true}, "m.new_content": {"body": "z"}, FORWARDED: {"sender": "@x:y"}});
        let mark = json!({"sender": "@bob:hs.org", "room_id": "!o:hs.org", "event_id": "$o", "content": hidden, "extra": 1});
        // A text of another kind than the message's own does not count: the message's own body is forwarded.
        let media = json!({"msgtype": "m.image", "body": "innocent", "url": "mxc://hs.org/a", "m.forwarded": mark});
        assert_eq!(shown(&media), None);
        assert_eq!(sent(build(&src(&media, None), false).unwrap())[FORWARDED]["content"]["body"], "innocent");
        // Same kind: the text the message shows is the mark's, cleaned like any other content.
        let text = json!({"msgtype": "m.text", "body": "Forwarded from @bob:hs.org - view original message: matrix:x\ninnocent", "m.forwarded": mark});
        assert_eq!(shown(&text).unwrap()["body"], "HIDDEN");
        let out = sent(build(&src(&text, None), false).unwrap());
        let inner = &out[FORWARDED]["content"];
        assert_eq!(inner["body"], "HIDDEN");
        for content in [&out, inner] {
            assert!(content.get("m.relates_to").is_none() && content.get("m.new_content").is_none(), "{content}");
        }
        assert_eq!(out["m.mentions"], json!({}));
        assert!(inner.get("m.mentions").is_none() && inner.get(FORWARDED).is_none() && inner.get("m.forwarded").is_none());
        // The first attribution stays; the unknown keys of the mark do not.
        assert_eq!(out[FORWARDED]["sender"], "@bob:hs.org");
        assert!(out[FORWARDED].get("extra").is_none());
        // The attachment is the message's own, whatever the mark says.
        let swapped = json!({"msgtype": "m.file", "body": "a.txt", "filename": "a.txt", "url": "mxc://hs.org/own", "m.forwarded": {"sender": "@bob:hs.org", "room_id": "!o:hs.org", "event_id": "$o", "content": {"msgtype": "m.file", "body": "caption", "url": "mxc://evil/x"}}});
        let out = sent(build(&src(&swapped, None), false).unwrap());
        assert_eq!((out["url"].as_str(), out[FORWARDED]["content"]["url"].as_str()), (Some("mxc://hs.org/own"), Some("mxc://hs.org/own")));
        assert!(out["body"].as_str().unwrap().ends_with("\ncaption"));
    }
}
