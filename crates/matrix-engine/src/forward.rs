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

#[derive(Debug, Clone, PartialEq)]
pub enum Forward {
    /// Queue this event as is.
    Send { event_type: &'static str, content: Value },
    /// Plain media into an encrypted room: fetch `mxc`, upload it encrypted as a new attachment.
    /// shortcut: the new attachment carries the caption only, no forwarded attribution (its content is only known after the upload); add it with the extra content of the attachment config.
    Reupload { mxc: String, filename: String, mime: String, caption: Option<String> },
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

/// The content to send to a room, or the upload to do first. `dest_encrypted`: the destination is an E2EE room.
pub fn build(source: &Source, dest_encrypted: bool) -> Result<Forward, ForwardError> {
    let event_type = match source.event_type {
        MESSAGE => MESSAGE,
        STICKER => STICKER,
        _ => return Err(ForwardError::NotForwardable),
    };
    let latest = source.edit.and_then(|e| e.get("m.new_content")).unwrap_or(source.content);
    if !can_forward(source.event_type, latest) {
        return Err(ForwardError::NotForwardable);
    }
    // A forward of a forward keeps the first attribution and the content it carries.
    let old = FORWARDED_KEYS.iter().find_map(|k| source.content.get(*k)).and_then(Value::as_object);
    let inner = match old.and_then(|o| o.get("content")).filter(|c| can_forward(source.event_type, c)) {
        Some(content) => content.clone(),
        None => original_content(source, latest),
    };
    let msgtype = inner.get("msgtype").and_then(Value::as_str).unwrap_or_default().to_owned();
    let plain_media = MEDIA.contains(&msgtype.as_str()) && inner.get("url").is_some();
    if dest_encrypted && plain_media {
        // A plain `url` must not reach an encrypted room; the key of an `EncryptedFile` may.
        let filename = text(&inner, "filename").or_else(|| text(&inner, "body")).unwrap_or("file");
        return Ok(Forward::Reupload {
            mxc: text(&inner, "url").unwrap_or_default().to_owned(),
            filename: filename.to_owned(),
            mime: inner["info"]["mimetype"].as_str().unwrap_or("application/octet-stream").to_owned(),
            caption: caption(&inner).map(str::to_owned),
        });
    }
    let mut inner = inner;
    if dest_encrypted && msgtype == "m.location" {
        // A map thumbnail is plain too; the location itself is the geo URI.
        if let Some(info) = inner.get_mut("info").and_then(Value::as_object_mut) {
            info.remove("thumbnail_url");
            info.remove("thumbnail_info");
        }
    }
    let mut forwarded = old.cloned().unwrap_or_else(|| {
        Map::from_iter([
            ("event_id".into(), json!(source.event_id)),
            ("room_id".into(), json!(source.room_id)),
            ("sender".into(), json!(source.sender)),
            ("origin_server_ts".into(), json!(source.origin_server_ts)),
        ])
    });
    forwarded.insert("content".into(), inner.clone());
    let mut top = inner;
    apply_fallback(&mut top, &forwarded);
    let top = top.as_object_mut().expect("content is an object");
    top.insert("m.mentions".into(), json!({}));
    top.insert(FORWARDED.into(), Value::Object(forwarded));
    Ok(Forward::Send { event_type, content: Value::Object(std::mem::take(top)) })
}

/// The latest content without relation, mentions, forward marks and reply fallback.
fn original_content(source: &Source, latest: &Value) -> Value {
    let mut content = latest.clone();
    let Some(map) = content.as_object_mut() else { return content };
    // The first line of a reply is quoted only when the event was a reply.
    let was_reply = source.content["m.relates_to"]["m.in_reply_to"].is_object();
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
    id[1..].bytes().fold(String::new(), |mut out, b| {
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

/// `forward` for the message `target` of `scope`, looked up in the room's timeline; every room fails alike when it is not found.
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
        (Some(room), Some(id)) => match timelines.get(&room).await {
            Ok(t) => t.item_by_event_id(&id).await,
            Err(_) => None,
        },
        _ => None,
    };
    match item {
        Some(item) => forward(client, scope.room_id.as_str(), &item, rooms).await,
        None => rooms.iter().map(|r| (r.clone(), Err(ForwardError::NotForwardable))).collect(),
    }
}

#[cfg(feature = "sdk")]
async fn send_to(client: &matrix_sdk::Client, room_id: &str, source: &Source<'_>) -> Result<(), ForwardError> {
    use matrix_sdk::{
        media::{DefaultMediaFetcher, MediaFormat, MediaRequestParameters},
        ruma::{OwnedRoomId, events::room::MediaSource, serde::Raw},
    };
    use matrix_sdk_ui::timeline::{AttachmentConfig, RoomExt};
    let room = OwnedRoomId::try_from(room_id).ok().and_then(|id| client.get_room(&id)).ok_or(ForwardError::UnknownRoom)?;
    // When the state is unknown, assume encryption: a plain `url` is the thing that must never leak.
    let encrypted = room.latest_encryption_state().await.is_ok_and(|s| s.is_encrypted()) || room.encryption_state().is_unknown();
    match build(source, encrypted)? {
        Forward::Send { event_type, content } => {
            let raw = Raw::new(&content).map_err(|_| ForwardError::Transport)?.cast_unchecked();
            room.send_queue().send_raw(raw, event_type.to_owned()).await.map_err(|_| ForwardError::Transport)?;
            Ok(())
        }
        Forward::Reupload { mxc, filename, mime, caption } => {
            let source = crate::media_sdk::checked_source(MediaSource::Plain(mxc.into())).map_err(|_| ForwardError::Transport)?;
            let request = MediaRequestParameters { source, format: MediaFormat::File };
            let config = client.request_config().timeout(Some(std::time::Duration::MAX)).max_response_size(Some(crate::media::MAX_SAVE_BYTES));
            let bytes = DefaultMediaFetcher.fetch_media_content_with_request_config(client, &request, config).await.map_err(|_| ForwardError::Transport)?;
            let mime = mime.parse().unwrap_or(mime_guess::mime::APPLICATION_OCTET_STREAM);
            let mut config = AttachmentConfig::default();
            config.caption = caption.map(matrix_sdk::ruma::events::room::message::TextMessageEventContent::plain);
            let timeline = room.timeline().await.map_err(|_| ForwardError::Transport)?;
            crate::sending::queue_attachment_waiting(client, &room, &timeline, (filename, bytes), mime, config, None)
                .await
                .map(|_| ())
                .map_err(|_| ForwardError::Transport)
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

    #[test]
    fn encrypted_media_is_reused_in_any_room() {
        let file = json!({"url": "mxc://hs.org/a", "key": {"k": "K"}, "iv": "I", "hashes": {"sha256": "H"}, "v": "v2"});
        let content = json!({"msgtype": "m.image", "body": "cat.png", "file": file, "info": {"mimetype": "image/png"}});
        for dest in [true, false] {
            let out = sent(build(&src(&content, None), dest).unwrap());
            assert_eq!(out["file"], file);
            assert!(out.get("url").is_none());
            assert_eq!(out[FORWARDED]["content"]["file"], file);
            // A file name is not a caption: nothing is quoted.
            assert_eq!(out["filename"], "cat.png");
            assert!(out["body"].as_str().unwrap().ends_with("?via=hs.org"));
        }
    }

    #[test]
    fn plain_media_keeps_its_url_in_a_plain_room_and_is_reuploaded_for_an_encrypted_one() {
        let content = json!({"msgtype": "m.video", "body": "look", "filename": "v.mp4", "url": "mxc://hs.org/v", "info": {"mimetype": "video/mp4", "thumbnail_url": "mxc://hs.org/t"}});
        let out = sent(build(&src(&content, None), false).unwrap());
        assert_eq!(out["url"], "mxc://hs.org/v");
        assert!(out["body"].as_str().unwrap().ends_with("\nlook"));
        assert_eq!(
            build(&src(&content, None), true).unwrap(),
            Forward::Reupload { mxc: "mxc://hs.org/v".into(), filename: "v.mp4".into(), mime: "video/mp4".into(), caption: Some("look".into()) }
        );
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
}
