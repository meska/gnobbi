//! Sticker packs (spec v1.19 `m.room.image_pack` + `m.image_pack.rooms`) and sending `m.sticker`.
//! A sticker is sent with only the pack's plain `url` (no re-upload, no `file`), also in encrypted
//! rooms: Megolm encrypts the event, the pack media is public on the homeserver by spec.
//! The user's own packs live in a private room the app creates on the first sticker (the «library»);
//! every pack shown is one enabled in `m.image_pack.rooms`, so other clients using the stable names see the same.
// shortcut: packs enabled by the account (`m.image_pack.rooms`) plus those of the current room already in the store (sliding sync does
// not sync custom state, so a room's own pack shows only once enabled); packs of parent spaces are a later card.
#[cfg(feature = "sdk")]
use crate::operations::OperationError;

/// Side of the square picture uploaded, the size the encoder aims at and the one never exceeded.
pub const SIDE: u32 = 512;
pub const SOFT_BYTES: usize = 512 << 10;
pub const MAX_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sticker {
    pub shortcode: String,
    pub body: String,
    pub url: String,
    pub mimetype: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerPack {
    /// `<room id>/<state key>`.
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    /// The account may change the pack (it can send the state event).
    pub own: bool,
    /// Listed in `m.image_pack.rooms`.
    pub enabled: bool,
    pub stickers: Vec<Sticker>,
}

/// One explicit change of the packs; `pack` is a `StickerPack::id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackEdit {
    /// Uploads `png` and adds it; with no `pack` the first own pack, created as `default_name` when there is none.
    Add { pack: Option<String>, default_name: String, shortcode: String, png: Vec<u8>, side: u32 },
    /// Adds a picture or sticker message of `room` as it is (its own mxc and mimetype, so a GIF stays animated); a pack as in `Add`.
    AddMessage { pack: Option<String>, default_name: String, room: String, event: String },
    Remove { pack: String, shortcode: String },
    Move { pack: String, shortcode: String, to: String },
    Create { name: String },
    Rename { pack: String, name: String },
    Delete { pack: String },
    Enable { pack: String, on: bool },
    /// Enables every sticker pack of a room the account can read (`!id` or `#alias`).
    Import { room: String },
}

/// Lowercase a-z, 0-9 and `_`, 1 to 32 characters.
pub fn valid_shortcode(code: &str) -> bool {
    (1..=32).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A valid shortcode out of any text (a file name, a description).
pub fn shortcode_from(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { '_' };
        if (c != '_' || !out.ends_with('_')) && out.len() < 32 {
            out.push(c);
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() { "sticker".to_owned() } else { out.to_owned() }
}

#[cfg(feature = "sdk")]
mod sdk {
    use super::*;
    use matrix_sdk::ruma::events::{GlobalAccountDataEventType, StateEventType, sticker::StickerEventContent};
    use matrix_sdk::ruma::{OwnedMxcUri, OwnedRoomId, UInt};
    use serde_json::{Map, Value, json};

    const PACK: &str = "m.room.image_pack";
    const ROOMS: &str = "m.image_pack.rooms";
    /// An event is at most 65 KB on the wire; the rest is the envelope.
    const MAX_EVENT: usize = 60_000;
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

    fn has_sticker_usage(usage: Option<&Value>) -> bool {
        match usage.and_then(Value::as_array) {
            Some(u) if !u.is_empty() => u.iter().any(|v| v == "sticker"),
            _ => true,
        }
    }

    /// The pack's sticker entries. `None` for a deleted pack (no `images`), an emoticon-only one, or one with no usable
    /// sticker that the account cannot fill. Bad entries (no mxc `url`, not an object) are skipped, never fatal.
    pub fn parse_pack(id: &str, fallback_name: &str, content: &Value, own: bool, enabled: bool) -> Option<StickerPack> {
        let images = content.get("images")?.as_object()?;
        let meta = content.get("pack");
        if !has_sticker_usage(meta.and_then(|m| m.get("usage"))) {
            return None;
        }
        let number = |info: Option<&Value>, key: &str| info.and_then(|i| i.get(key)).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
        let stickers: Vec<_> = images
            .iter()
            .filter_map(|(code, image)| {
                let url = image.get("url")?.as_str()?;
                if !OwnedMxcUri::from(url).is_valid() || !has_sticker_usage(image.get("usage")) {
                    return None;
                }
                let info = image.get("info");
                Some(Sticker {
                    shortcode: code.clone(),
                    body: image.get("body").and_then(Value::as_str).filter(|b| !b.is_empty()).unwrap_or(code).to_owned(),
                    url: url.to_owned(),
                    mimetype: info.and_then(|i| i.get("mimetype")).and_then(Value::as_str).map(str::to_owned),
                    width: number(info, "w"),
                    height: number(info, "h"),
                    size: number(info, "size"),
                })
            })
            .collect();
        if stickers.is_empty() && !own {
            return None;
        }
        let text = |key: &str| meta.and_then(|m| m.get(key)).and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_owned);
        Some(StickerPack {
            id: id.to_owned(),
            name: text("display_name").unwrap_or_else(|| fallback_name.to_owned()),
            avatar: text("avatar_url").filter(|u| OwnedMxcUri::from(u.as_str()).is_valid()),
            own,
            enabled,
            stickers,
        })
    }

    fn images(content: &mut Value) -> &mut Map<String, Value> {
        if !content.is_object() {
            *content = json!({});
        }
        let slot = &mut content["images"];
        if !slot.is_object() {
            *slot = json!({});
        }
        slot.as_object_mut().expect("just made an object")
    }

    /// `wanted`, or `wanted_2`, `wanted_3`… when taken.
    pub fn free_shortcode(content: &Value, wanted: &str) -> String {
        let taken = |code: &str| content.get("images").and_then(|i| i.get(code)).is_some();
        if !taken(wanted) {
            return wanted.to_owned();
        }
        (2..).map(|n| { let tail = format!("_{n}"); format!("{}{tail}", &wanted[..wanted.len().min(32 - tail.len())]) }).find(|c| !taken(c)).expect("unbounded")
    }

    /// The pack content with one more image. Refuses a bad or taken shortcode and a pack that would not fit an event;
    /// everything else in the content (other clients' fields) is kept.
    pub fn with_sticker(mut content: Value, sticker: &Sticker) -> Result<Value, OperationError> {
        if !valid_shortcode(&sticker.shortcode) || !OwnedMxcUri::from(sticker.url.as_str()).is_valid() {
            return Err(OperationError::InvalidInput);
        }
        let images = images(&mut content);
        if images.contains_key(&sticker.shortcode) {
            return Err(OperationError::InvalidInput);
        }
        let mut info = Map::new();
        for (key, value) in [("w", sticker.width), ("h", sticker.height), ("size", sticker.size)] {
            if let Some(v) = value {
                info.insert(key.into(), v.into());
            }
        }
        if let Some(mime) = &sticker.mimetype {
            info.insert("mimetype".into(), mime.clone().into());
        }
        images.insert(sticker.shortcode.clone(), json!({"url": sticker.url, "body": sticker.body, "info": info}));
        if content.to_string().len() > MAX_EVENT {
            return Err(OperationError::InvalidInput);
        }
        Ok(content)
    }

    /// The content without `shortcode`, and the removed entry as a sticker (to move it).
    pub fn without_sticker(mut content: Value, id: &str, shortcode: &str) -> (Value, Option<Sticker>) {
        let found = parse_pack(id, "", &content, true, false).and_then(|p| p.stickers.into_iter().find(|s| s.shortcode == shortcode));
        images(&mut content).remove(shortcode);
        (content, found)
    }

    pub fn renamed(mut content: Value, name: &str) -> Value {
        images(&mut content);
        if !content["pack"].is_object() {
            content["pack"] = json!({});
        }
        content["pack"]["display_name"] = name.into();
        content
    }

    /// `m.image_pack.rooms` with one pack switched; an emptied room entry is dropped.
    pub fn rooms_with(mut data: Value, room: &str, key: &str, on: bool) -> Value {
        if !data.is_object() || !data["rooms"].is_object() {
            data = json!({"rooms": {}});
        }
        let rooms = data["rooms"].as_object_mut().expect("just checked");
        if on {
            let entry = rooms.entry(room).or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            entry[key] = json!({});
        } else if let Some(entry) = rooms.get_mut(room).and_then(Value::as_object_mut) {
            entry.remove(key);
            if entry.is_empty() {
                rooms.remove(room);
            }
        }
        data
    }

    /// The enabled `(room, key)` pairs, tolerant of anything that is not an object.
    pub fn enabled_pairs(data: &Value) -> Vec<(String, String)> {
        let rooms = data.get("rooms").and_then(Value::as_object);
        rooms.into_iter().flatten().flat_map(|(room, keys)| keys.as_object().into_iter().flatten().map(move |(k, _)| (room.clone(), k.clone()))).collect()
    }

    /// A picture ready to be a sticker: square, at most `SIDE`, transparent around a picture that is not square.
    pub struct Prepared {
        pub png: Vec<u8>,
        pub side: u32,
        pub rgba: Vec<u8>,
    }

    /// Any PNG/JPEG/WebP/GIF (first frame) as width, height and RGBA, within the media limits.
    pub fn decode(data: &[u8]) -> Result<(u32, u32, Vec<u8>), OperationError> {
        if data.len() > crate::media::MAX_MEDIA_BYTES {
            return Err(OperationError::InvalidInput);
        }
        let reader = || image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().map_err(|_| OperationError::InvalidInput);
        let (w, h) = reader()?.into_dimensions().map_err(|_| OperationError::InvalidInput)?;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > crate::media::MAX_IMAGE_PIXELS {
            return Err(OperationError::InvalidInput);
        }
        let picture = reader()?.decode().map_err(|_| OperationError::InvalidInput)?.into_rgba8();
        Ok((picture.width(), picture.height(), picture.into_raw()))
    }

    /// A picture as a square PNG: the whole picture on a transparent square, or with `fill` its centre cut square.
    /// Shrinks further until it is under `SOFT_BYTES`; over `MAX_BYTES` even at the smallest size is refused.
    pub fn prepare(w: u32, h: u32, rgba: &[u8], fill: bool) -> Result<Prepared, OperationError> {
        use image::{ImageEncoder as _, imageops};
        let mut picture = image::RgbaImage::from_raw(w, h, rgba.to_vec()).filter(|_| w > 0 && h > 0).ok_or(OperationError::InvalidInput)?;
        if fill {
            let side = w.min(h);
            picture = imageops::crop_imm(&picture, (w - side) / 2, (h - side) / 2, side, side).to_image();
        }
        let mut last = Err(OperationError::InvalidInput);
        for side in [SIDE, 384, 256, 192, 128] {
            let side = side.min(picture.width().max(picture.height()));
            let scale = f64::from(side) / f64::from(picture.width().max(picture.height()));
            let (pw, ph) = (((f64::from(picture.width()) * scale).round() as u32).clamp(1, side), ((f64::from(picture.height()) * scale).round() as u32).clamp(1, side));
            let small = if (pw, ph) == picture.dimensions() { picture.clone() } else { imageops::resize(&picture, pw, ph, imageops::FilterType::Lanczos3) };
            let mut square = image::RgbaImage::new(side, side);
            imageops::overlay(&mut square, &small, i64::from((side - pw) / 2), i64::from((side - ph) / 2));
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(square.as_raw(), side, side, image::ExtendedColorType::Rgba8)
                .map_err(|_| OperationError::InvalidInput)?;
            let size = png.len();
            if size <= MAX_BYTES {
                last = Ok(Prepared { png, side, rgba: square.into_raw() });
            }
            if size <= SOFT_BYTES {
                break;
            }
        }
        last
    }

    /// `m.sticker` content with the plain `url` only.
    pub fn content_of(sticker: &Sticker) -> Result<StickerEventContent, OperationError> {
        let url = OwnedMxcUri::from(sticker.url.as_str());
        if !url.is_valid() || sticker.body.is_empty() {
            return Err(OperationError::InvalidInput);
        }
        let mut info = matrix_sdk::ruma::events::room::ImageInfo::new();
        info.mimetype = sticker.mimetype.clone();
        info.width = sticker.width.map(UInt::from);
        info.height = sticker.height.map(UInt::from);
        info.size = sticker.size.map(UInt::from);
        Ok(StickerEventContent::new(sticker.body.clone(), info, url))
    }

    async fn timed<T, E: Into<matrix_sdk::Error>>(call: impl std::future::Future<Output = Result<T, E>>) -> Result<T, OperationError> {
        tokio::time::timeout(LIMIT, call).await.map_err(|_| OperationError::Transport)?.map_err(|e| crate::operations::classify(&e.into()))
    }

    fn split(pack: &str) -> Result<(OwnedRoomId, &str), OperationError> {
        let (room, key) = pack.split_once('/').ok_or(OperationError::InvalidInput)?;
        Ok((OwnedRoomId::try_from(room).map_err(|_| OperationError::InvalidInput)?, key))
    }

    /// The pack as the server has it now (sliding sync does not put custom state in the store); `{}` when there is none.
    async fn read_pack(client: &matrix_sdk::Client, room: &OwnedRoomId, key: &str) -> Result<Value, OperationError> {
        let request = matrix_sdk::ruma::api::client::state::get_state_event_for_key::v3::Request::new(room.clone(), StateEventType::from(PACK), key.to_owned());
        match timed(async { client.send(request).await }).await {
            Ok(answer) => Ok(serde_json::from_str(answer.event_or_content.get()).unwrap_or_else(|_| json!({}))),
            Err(OperationError::Unavailable) => Ok(json!({})),
            Err(e) => Err(e),
        }
    }

    async fn write_pack(client: &matrix_sdk::Client, room: &OwnedRoomId, key: &str, content: Value) -> Result<(), OperationError> {
        let room = client.get_room(room).ok_or(OperationError::Unavailable)?;
        timed(room.send_state_event_raw(PACK, key, content)).await.map(drop)
    }

    /// Read, change, write: always from the server's latest content, so a change made meanwhile by another device is kept.
    // shortcut: Matrix has no compare-and-set, two writes inside the same round trip still overwrite; retry on a hash if it is ever seen.
    async fn change_pack(client: &matrix_sdk::Client, pack: &str, change: impl FnOnce(Value) -> Result<Value, OperationError>) -> Result<(), OperationError> {
        let (room, key) = split(pack)?;
        let content = change(read_pack(client, &room, key).await?)?;
        write_pack(client, &room, key, content).await
    }

    async fn read_rooms(client: &matrix_sdk::Client) -> Result<Value, OperationError> {
        let raw = timed(client.account().fetch_account_data(GlobalAccountDataEventType::from(ROOMS))).await?;
        Ok(raw.and_then(|r| serde_json::from_str(r.json().get()).ok()).unwrap_or_else(|| json!({})))
    }

    async fn enable(client: &matrix_sdk::Client, pack: &str, on: bool) -> Result<(), OperationError> {
        let (room, key) = split(pack)?;
        let data = rooms_with(read_rooms(client).await?, room.as_str(), key, on);
        let raw = serde_json::value::to_raw_value(&data).map_err(|_| OperationError::InvalidInput)?;
        timed(client.account().set_account_data_raw(GlobalAccountDataEventType::from(ROOMS), matrix_sdk::ruma::serde::Raw::from_json(raw))).await.map(drop)
    }

    async fn can_edit(client: &matrix_sdk::Client, room: &OwnedRoomId) -> bool {
        let Some(room) = client.get_room(room).filter(|r| r.state() == matrix_sdk::RoomState::Joined) else { return false };
        match (room.power_levels().await, client.user_id()) {
            (Ok(levels), Some(me)) => levels.user_can_send_state(me, StateEventType::from(PACK)),
            _ => false,
        }
    }

    /// The library: the enabled room this account created; the lowest id when there are several.
    async fn library(client: &matrix_sdk::Client) -> Result<Option<OwnedRoomId>, OperationError> {
        Ok(library_in(client, &read_rooms(client).await?))
    }

    fn library_in(client: &matrix_sdk::Client, data: &Value) -> Option<OwnedRoomId> {
        let me = client.user_id()?;
        let mut rooms: Vec<_> = enabled_pairs(data).into_iter().filter_map(|(room, _)| OwnedRoomId::try_from(room).ok()).collect();
        rooms.sort();
        rooms.dedup();
        rooms.into_iter().find(|id| {
            client.get_room(id).is_some_and(|r| r.state() == matrix_sdk::RoomState::Joined && r.creators().is_some_and(|c| c.iter().any(|u| u.as_str() == me.as_str())))
        })
    }

    /// Libraries just made here: hidden from the room list before the sync brings `m.image_pack.rooms` and the create event.
    static MADE: std::sync::Mutex<Vec<OwnedRoomId>> = std::sync::Mutex::new(Vec::new());

    #[cfg(test)]
    pub fn forget_made() {
        MADE.lock().unwrap().clear();
    }

    /// The library as the room list needs it: same recognition as `library`, from the synced account data (no network).
    pub async fn hidden_library(client: &matrix_sdk::Client) -> Option<OwnedRoomId> {
        let data = match client.account().account_data_raw(GlobalAccountDataEventType::from(ROOMS)).await {
            Ok(Some(raw)) => serde_json::from_str(raw.json().get()).unwrap_or_default(),
            _ => Value::Null,
        };
        library_in(client, &data).or_else(|| MADE.lock().ok()?.iter().find(|id| client.get_room(id).is_some()).cloned())
    }

    /// A new enabled pack named `name`, in the library (made now when missing: private, invite-only, not encrypted
    /// because packs cannot be, federating so it can be shared later).
    async fn create(client: &matrix_sdk::Client, name: &str) -> Result<String, OperationError> {
        use matrix_sdk::ruma::api::client::room::{Visibility, create_room::v3::{Request, RoomPreset}};
        let name = name.trim();
        if name.is_empty() {
            return Err(OperationError::InvalidInput);
        }
        let room = match library(client).await? {
            Some(room) => room,
            None => {
                let mut request = Request::new();
                request.name = Some(name.to_owned());
                (request.preset, request.visibility) = (Some(RoomPreset::PrivateChat), Visibility::Private);
                let made = timed(client.create_room(request)).await?;
                if let Ok(mut made_here) = MADE.lock() {
                    made_here.push(made.room_id().to_owned());
                }
                // Until the sync brings its state the room is not recognised as the library, and a second sticker would make another.
                for _ in 0..25 {
                    if made.creators().is_some() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                made.room_id().to_owned()
            }
        };
        let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let pack = format!("{room}/p{millis}");
        write_pack(client, &room, &pack[room.as_str().len() + 1..], json!({"images": {}, "pack": {"display_name": name, "usage": ["sticker"]}})).await?;
        enable(client, &pack, true).await?;
        Ok(pack)
    }

    async fn import(client: &matrix_sdk::Client, room: &str) -> Result<(), OperationError> {
        let room = room.trim();
        let id = if room.starts_with('#') {
            let alias = matrix_sdk::ruma::OwnedRoomAliasId::try_from(room).map_err(|_| OperationError::InvalidInput)?;
            timed(client.resolve_room_alias(&alias)).await?.room_id
        } else {
            OwnedRoomId::try_from(room).map_err(|_| OperationError::InvalidInput)?
        };
        // The whole state, once, on the user's request: the only way to list a room's packs without knowing their keys.
        let state = timed(async { client.send(matrix_sdk::ruma::api::client::state::get_state_events::v3::Request::new(id.clone())).await }).await?;
        let mut found = false;
        for raw in state.room_state {
            let Ok(event) = serde_json::from_str::<Value>(raw.json().get()) else { continue };
            let (Some(PACK), Some(key)) = (event["type"].as_str(), event["state_key"].as_str()) else { continue };
            let pack = format!("{id}/{key}");
            if parse_pack(&pack, "", &event["content"], false, false).is_some() {
                enable(client, &pack, true).await?;
                found = true;
            }
        }
        if found { Ok(()) } else { Err(OperationError::Unavailable) }
    }

    /// The sticker a picture message or sticker event stands for, with the encrypted file when its media is encrypted (the url is then the encrypted one).
    pub fn original_of(event: &Value) -> Option<(Sticker, Option<matrix_sdk::ruma::events::room::EncryptedFile>)> {
        let content = event.get("content")?;
        let kind = event.get("type")?.as_str()?;
        if kind != "m.sticker" && !(kind == "m.room.message" && content.get("msgtype")?.as_str()? == "m.image") {
            return None;
        }
        let file = content.get("file").and_then(|f| serde_json::from_value::<matrix_sdk::ruma::events::room::EncryptedFile>(f.clone()).ok());
        let url = match &file {
            Some(file) => file.url.to_string(),
            None => content.get("url")?.as_str()?.to_owned(),
        };
        if !url.starts_with("mxc://") {
            return None;
        }
        let info = content.get("info");
        let number = |key| info.and_then(|i| i.get(key)).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        let mimetype = info.and_then(|i| i.get("mimetype")).and_then(Value::as_str).filter(|m| m.starts_with("image/"))?.to_owned();
        let body = content.get("body").and_then(Value::as_str).unwrap_or_default();
        let name = std::path::Path::new(body).file_stem().and_then(|n| n.to_str()).unwrap_or(body);
        let shortcode = shortcode_from(name);
        Some((Sticker { body: shortcode.clone(), shortcode, url, mimetype: Some(mimetype), width: number("w"), height: number("h"), size: number("size") }, file))
    }

    /// The given pack, else the first own enabled one, else a new one called `default_name`.
    async fn own_pack(client: &matrix_sdk::Client, pack: Option<String>, default_name: &str) -> Result<String, OperationError> {
        if let Some(pack) = pack {
            return Ok(pack);
        }
        for (room, key) in enabled_pairs(&read_rooms(client).await?) {
            if let Ok(id) = OwnedRoomId::try_from(room.as_str()) && can_edit(client, &id).await {
                return Ok(format!("{room}/{key}"));
            }
        }
        create(client, default_name).await
    }

    pub async fn edit(client: &matrix_sdk::Client, edit: PackEdit) -> Result<(), OperationError> {
        match edit {
            PackEdit::Add { pack, default_name, shortcode, png, side } => {
                if png.len() > MAX_BYTES || !valid_shortcode(&shortcode) {
                    return Err(OperationError::InvalidInput);
                }
                let pack = own_pack(client, pack, &default_name).await?;
                let size = u32::try_from(png.len()).ok();
                let url = timed(async { client.media().upload(&mime_guess::mime::IMAGE_PNG, png, None).await }).await?.content_uri;
                let sticker = Sticker { body: shortcode.clone(), shortcode, url: url.to_string(), mimetype: Some("image/png".into()), width: Some(side), height: Some(side), size };
                change_pack(client, &pack, |content| with_sticker(content, &sticker)).await
            }
            PackEdit::AddMessage { pack, default_name, room, event } => {
                let room = OwnedRoomId::try_from(room.as_str()).ok().and_then(|id| client.get_room(&id)).ok_or(OperationError::InvalidInput)?;
                let event_id = matrix_sdk::ruma::OwnedEventId::try_from(event.as_str()).map_err(|_| OperationError::InvalidInput)?;
                let raw = timed(room.event(&event_id, None)).await?;
                let (mut sticker, encrypted) = original_of(&serde_json::from_str(raw.raw().json().get()).map_err(|_| OperationError::InvalidInput)?).ok_or(OperationError::InvalidInput)?;
                if let Some(file) = encrypted {
                    // Encrypted media cannot be shared by its mxc: the same bytes (decrypted, never re-encoded) go up again, capped while received.
                    let request = matrix_sdk::media::MediaRequestParameters { source: matrix_sdk::ruma::events::room::MediaSource::Encrypted(Box::new(file)), format: matrix_sdk::media::MediaFormat::File };
                    let config = client.request_config().timeout(Some(std::time::Duration::from_secs(60))).max_response_size(Some(MAX_BYTES));
                    let bytes = timed(matrix_sdk::media::DefaultMediaFetcher.fetch_media_content_with_request_config(client, &request, config)).await?;
                    let mime: mime_guess::mime::Mime = sticker.mimetype.as_deref().and_then(|m| m.parse().ok()).ok_or(OperationError::InvalidInput)?;
                    sticker.size = u32::try_from(bytes.len()).ok();
                    sticker.url = timed(async { client.media().upload(&mime, bytes, None).await }).await?.content_uri.to_string();
                }
                let pack = own_pack(client, pack, &default_name).await?;
                change_pack(client, &pack, |content| {
                    sticker.shortcode = free_shortcode(&content, &sticker.shortcode);
                    with_sticker(content, &sticker)
                })
                .await
            }
            PackEdit::Remove { pack, shortcode } => change_pack(client, &pack, |content| Ok(without_sticker(content, &pack, &shortcode).0)).await,
            PackEdit::Move { pack, shortcode, to } => {
                let (room, key) = split(&pack)?;
                let (rest, moved) = without_sticker(read_pack(client, &room, key).await?, &pack, &shortcode);
                let mut moved = moved.ok_or(OperationError::Unavailable)?;
                // Added first: a failure in between leaves the sticker in both packs, never in none.
                change_pack(client, &to, |content| {
                    moved.shortcode = free_shortcode(&content, &shortcode_from(&moved.shortcode)); // a pack made elsewhere may hold any shortcode
                    with_sticker(content, &moved)
                })
                .await?;
                write_pack(client, &room, key, rest).await
            }
            PackEdit::Create { name } => create(client, &name).await.map(drop),
            PackEdit::Rename { pack, name } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return Err(OperationError::InvalidInput);
                }
                change_pack(client, &pack, |content| Ok(renamed(content, &name))).await
            }
            PackEdit::Delete { pack } => {
                let (room, key) = split(&pack)?;
                // No `images`: the pack is gone for every reader; then it leaves the account's list.
                write_pack(client, &room, key, json!({})).await?;
                enable(client, &pack, false).await
            }
            PackEdit::Enable { pack, on } => enable(client, &pack, on).await,
            PackEdit::Import { room } => import(client, &room).await,
        }
    }

    /// Packs enabled by the account (`m.image_pack.rooms`) plus those of the room itself, in that order, no duplicates.
    pub async fn packs(
        client: &matrix_sdk::Client,
        account: matrix_domain::AccountId,
        scope: &matrix_domain::RoomScope,
    ) -> Result<Vec<StickerPack>, OperationError> {
        use matrix_sdk::deserialized_responses::SyncOrStrippedState::Sync;
        use matrix_sdk::ruma::events::SyncStateEvent::Original;
        use matrix_sdk::ruma::events::room::image_pack::RoomImagePackEventContent;
        if scope.account_id != account {
            return Err(OperationError::WrongScope);
        }
        let current = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
        let mut wanted: Vec<(OwnedRoomId, String, bool)> = Vec::new();
        if let Ok(Some(raw)) = client.account().account_data_raw(GlobalAccountDataEventType::from(ROOMS)).await
            && let Ok(data) = serde_json::from_str::<Value>(raw.json().get())
        {
            wanted.extend(enabled_pairs(&data).into_iter().filter_map(|(room, key)| Some((OwnedRoomId::try_from(room).ok()?, key, true))));
        }
        if let Some(room) = client.get_room(&current) {
            for raw in room.get_state_events_static::<RoomImagePackEventContent>().await.unwrap_or_default() {
                if let Ok(Sync(Original(e))) = raw.deserialize() {
                    wanted.push((current.clone(), e.state_key, false));
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (room_id, key, enabled) in wanted {
            if !seen.insert((room_id.clone(), key.clone())) {
                continue;
            }
            let Ok(content) = read_pack(client, &room_id, &key).await else { continue };
            let room_name = client.get_room(&room_id).and_then(|r| r.cached_display_name()).map(|n| n.to_string()).unwrap_or_default();
            if let Some(pack) = parse_pack(&format!("{room_id}/{key}"), &room_name, &content, can_edit(client, &room_id).await, enabled) {
                out.push(pack);
            }
        }
        Ok(out)
    }

    pub async fn send(
        client: &matrix_sdk::Client,
        account: matrix_domain::AccountId,
        scope: &matrix_domain::RoomScope,
        sticker: &Sticker,
    ) -> Result<matrix_domain::EventId, OperationError> {
        if scope.account_id != account {
            return Err(OperationError::WrongScope);
        }
        let content = content_of(sticker)?;
        let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
        let room = client
            .get_room(&id)
            .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
            .ok_or(OperationError::Unavailable)?;
        let sent = tokio::time::timeout(std::time::Duration::from_secs(30), room.send(content))
            .await
            .map_err(|_| OperationError::Transport)?
            .map_err(|e| crate::operations::classify(&e))?;
        Ok(matrix_domain::EventId::new(sent.response.event_id.as_str()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn pack(json: &str) -> Value {
            serde_json::from_str(json).unwrap()
        }

        fn sticker(code: &str) -> Sticker {
            Sticker { shortcode: code.into(), body: "Ciao".into(), url: "mxc://x/a".into(), mimetype: Some("image/png".into()), width: Some(64), height: None, size: Some(900) }
        }

        #[test]
        fn a_gif_message_becomes_a_sticker_with_its_own_mxc_and_mimetype() {
            let event = pack(r#"{"type":"m.room.message","content":{"msgtype":"m.image","body":"Gatto Felice.gif","url":"mxc://x/gif","info":{"mimetype":"image/gif","w":320,"h":240,"size":12345}}}"#);
            let (sticker, file) = original_of(&event).unwrap();
            assert_eq!((sticker.url.as_str(), sticker.mimetype.as_deref(), sticker.shortcode.as_str()), ("mxc://x/gif", Some("image/gif"), "gatto_felice"));
            assert_eq!((sticker.width, sticker.height, sticker.size, file.is_none()), (Some(320), Some(240), Some(12345), true));
            let added = with_sticker(json!({}), &sticker).unwrap();
            assert_eq!(added["images"]["gatto_felice"]["url"], "mxc://x/gif");
            assert_eq!(added["images"]["gatto_felice"]["info"]["mimetype"], "image/gif");
            let animated_sticker = pack(r#"{"type":"m.sticker","content":{"body":"ok","url":"mxc://x/s","info":{"mimetype":"image/webp"}}}"#);
            assert_eq!(original_of(&animated_sticker).unwrap().0.mimetype.as_deref(), Some("image/webp"));
            assert!(original_of(&pack(r#"{"type":"m.room.message","content":{"msgtype":"m.text","body":"x","url":"mxc://x/a","info":{"mimetype":"image/gif"}}}"#)).is_none());
            assert!(original_of(&pack(r#"{"type":"m.sticker","content":{"body":"x","url":"https://x/a","info":{"mimetype":"image/gif"}}}"#)).is_none());
        }

        #[test]
        fn usage_filters_and_body_defaults_to_shortcode() {
            let sticker = pack(r#"{"images":{"ciao":{"url":"mxc://x/a","info":{"mimetype":"image/webp","w":512,"h":256,"size":7}},"b":{"url":"mxc://x/b","body":"Bi"},"bad":{"url":"https://x/c"}},"pack":{"display_name":"P","avatar_url":"mxc://x/av","usage":["sticker"]}}"#);
            let p = parse_pack("!r:x/k", "Stanza", &sticker, false, true).unwrap();
            assert_eq!((p.name.as_str(), p.stickers.len(), p.avatar.as_deref(), p.enabled), ("P", 2, Some("mxc://x/av"), true));
            assert_eq!(p.stickers[1].body, "ciao"); // map order: "b" < "bad" < "ciao"
            assert_eq!((p.stickers[1].width, p.stickers[1].size, p.stickers[1].mimetype.as_deref()), (Some(512), Some(7), Some("image/webp")));
            let emoji = pack(r#"{"images":{"a":{"url":"mxc://x/a"}},"pack":{"usage":["emoticon"]}}"#);
            assert!(parse_pack("k", "n", &emoji, true, true).is_none());
            let all = pack(r#"{"images":{"a":{"url":"mxc://x/a"}}}"#);
            assert_eq!(parse_pack("k", "Stanza", &all, false, false).unwrap().name, "Stanza");
        }

        #[test]
        fn malformed_content_never_panics_and_bad_entries_are_skipped() {
            for bad in ["null", "[]", "3", r#"{"images":7}"#, r#"{"images":[]}"#, r#"{"pack":{"display_name":"x"}}"#, "{}"] {
                assert!(parse_pack("k", "n", &pack(bad), true, true).is_none(), "{bad}");
            }
            // Entries: not an object, no url, numeric url, bad info types, emoticon-only image, huge width.
            let mixed = pack(r#"{"images":{"a":3,"b":{},"c":{"url":7},"d":{"url":"mxc://x/d","info":"no","body":5},"e":{"url":"mxc://x/e","usage":["emoticon"]},"f":{"url":"mxc://x/f","info":{"w":99999999999,"h":-1}}},"pack":"no"}"#);
            let p = parse_pack("k", "n", &mixed, false, false).unwrap();
            assert_eq!(p.stickers.iter().map(|s| s.shortcode.as_str()).collect::<Vec<_>>(), ["d", "f"]);
            assert_eq!((p.stickers[0].body.as_str(), p.stickers[0].mimetype.as_deref(), p.stickers[1].width, p.stickers[1].height), ("d", None, None, None));
            // An own pack with no sticker is kept (it can be filled); someone else's is not; a deleted one never.
            let empty = pack(r#"{"images":{},"pack":{"display_name":"Vuoto"}}"#);
            assert_eq!(parse_pack("k", "n", &empty, true, true).unwrap().stickers.len(), 0);
            assert!(parse_pack("k", "n", &empty, false, true).is_none());
        }

        #[test]
        fn adding_keeps_foreign_fields_refuses_duplicates_and_oversized_packs() {
            let base = pack(r#"{"images":{"a":{"url":"mxc://x/a","x.custom":1}},"pack":{"attribution":"me"},"x.other":true}"#);
            let added = with_sticker(base.clone(), &sticker("nuovo_1")).unwrap();
            assert_eq!(added["images"]["nuovo_1"], json!({"url": "mxc://x/a", "body": "Ciao", "info": {"w": 64, "size": 900, "mimetype": "image/png"}}));
            assert_eq!((&added["images"]["a"]["x.custom"], &added["pack"]["attribution"], &added["x.other"]), (&json!(1), &json!("me"), &json!(true)));
            assert_eq!(with_sticker(base.clone(), &sticker("a")).err(), Some(OperationError::InvalidInput));
            for bad in ["", "Maiuscola", "con spazio", "è", &"a".repeat(33)] {
                assert_eq!(with_sticker(base.clone(), &sticker(bad)).err(), Some(OperationError::InvalidInput), "{bad}");
            }
            assert_eq!(with_sticker(base.clone(), &Sticker { url: "https://x/a".into(), ..sticker("b") }).err(), Some(OperationError::InvalidInput));
            // Malformed content is replaced by a usable pack instead of failing.
            assert_eq!(with_sticker(pack("[]"), &sticker("b")).unwrap()["images"].as_object().unwrap().len(), 1);
            let mut full = base;
            let mut n = 0;
            while let Ok(next) = with_sticker(full.clone(), &sticker(&format!("s{n}"))) {
                (full, n) = (next, n + 1);
            }
            assert!(n > 300 && full.to_string().len() <= MAX_EVENT, "{n}");
        }

        #[test]
        fn free_shortcode_remove_rename_and_validation() {
            let base = pack(r#"{"images":{"a":{"url":"mxc://x/a"},"a_2":{"url":"mxc://x/b"}}}"#);
            assert_eq!((free_shortcode(&base, "b"), free_shortcode(&base, "a")), ("b".into(), "a_3".into()));
            let long = "a".repeat(32);
            let taken = pack(&format!(r#"{{"images":{{"{long}":{{"url":"mxc://x/a"}}}}}}"#));
            assert!(valid_shortcode(&free_shortcode(&taken, &long)));
            let (rest, moved) = without_sticker(base.clone(), "k", "a");
            assert_eq!((rest["images"].as_object().unwrap().len(), moved.unwrap().url.as_str()), (1, "mxc://x/a"));
            assert!(without_sticker(base.clone(), "k", "zz").1.is_none());
            let named = renamed(base, "Nuovo");
            assert_eq!((named["pack"]["display_name"].as_str(), named["images"].as_object().unwrap().len()), (Some("Nuovo"), 2));
            assert_eq!(renamed(pack("7"), "N")["images"], json!({}));
            assert!(valid_shortcode("ciao_1") && !valid_shortcode("") && !valid_shortcode("Ciao") && !valid_shortcode("a-b"));
            assert_eq!((shortcode_from(" Gatto Felice!.png "), shortcode_from("àè"), shortcode_from(&"x".repeat(50)).len()), ("gatto_felice_png".into(), "sticker".into(), 32));
        }

        #[test]
        fn rooms_merge_keeps_other_packs_and_survives_bad_data() {
            let data = pack(r#"{"rooms":{"!a:x":{"p1":{},"p2":{}},"!b:x":{"q":{}}},"x.keep":1}"#);
            let on = rooms_with(data.clone(), "!c:x", "n", true);
            assert_eq!(enabled_pairs(&on).len(), 4);
            assert_eq!(on["x.keep"], 1);
            assert_eq!(rooms_with(on.clone(), "!c:x", "n", true), on); // twice is the same
            let off = rooms_with(rooms_with(data.clone(), "!a:x", "p1", false), "!b:x", "q", false);
            assert_eq!(enabled_pairs(&off), [("!a:x".to_owned(), "p2".to_owned())]);
            assert!(off["rooms"].get("!b:x").is_none());
            for bad in ["null", "[]", r#"{"rooms":3}"#, r#"{"rooms":{"!a:x":7}}"#] {
                assert_eq!(enabled_pairs(&rooms_with(pack(bad), "!a:x", "p", true)), [("!a:x".to_owned(), "p".to_owned())], "{bad}");
                assert!(enabled_pairs(&rooms_with(pack(bad), "!a:x", "p", false)).is_empty(), "{bad}");
            }
        }

        fn ready(data: &[u8], fill: bool) -> Result<Prepared, OperationError> {
            let (w, h, rgba) = decode(data)?;
            prepare(w, h, &rgba, fill)
        }

        fn png(w: u32, h: u32, noise: bool) -> Vec<u8> {
            use image::ImageEncoder as _;
            let mut seed = 1u32;
            let picture = image::RgbaImage::from_fn(w, h, |x, y| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                if noise { image::Rgba([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8, 255]) } else { image::Rgba([x as u8, y as u8, 9, 255]) }
            });
            let mut out = Vec::new();
            image::codecs::png::PngEncoder::new(&mut out).write_image(&picture, w, h, image::ExtendedColorType::Rgba8).unwrap();
            out
        }

        #[test]
        fn pictures_become_a_bounded_square_with_transparent_sides_or_a_centre_cut() {
            let fit = ready(&png(1000, 500, false), false).unwrap();
            assert_eq!((fit.side, fit.rgba.len()), (512, 512 * 512 * 4));
            let alpha = |p: &Prepared, x: u32, y: u32| p.rgba[((y * p.side + x) * 4 + 3) as usize];
            assert_eq!((alpha(&fit, 256, 10), alpha(&fit, 256, 256), alpha(&fit, 256, 500)), (0, 255, 0));
            let fill = ready(&png(1000, 500, false), true).unwrap();
            assert_eq!((fill.side, alpha(&fill, 250, 10)), (500, 255)); // the 500 px centre, not enlarged
            // Small pictures are never enlarged.
            assert_eq!(ready(&png(40, 20, false), false).unwrap().side, 40);
            // Noise does not compress: the side drops until the file is under the soft limit.
            let noisy = ready(&png(600, 600, true), false).unwrap();
            assert!(noisy.side < 512 && noisy.png.len() <= SOFT_BYTES, "{} {}", noisy.side, noisy.png.len());
            assert_eq!(ready(b"not a picture", false).err(), Some(OperationError::InvalidInput));
            assert_eq!(ready(&[], true).err(), Some(OperationError::InvalidInput));
            assert_eq!(prepare(2, 2, &[0; 3], false).err(), Some(OperationError::InvalidInput));
        }

        #[test]
        fn sticker_event_has_plain_url_and_no_file() {
            let s = sticker("a");
            let json = serde_json::to_value(content_of(&s).unwrap()).unwrap();
            assert_eq!(json["url"], "mxc://x/a");
            assert_eq!(json["body"], "Ciao");
            assert_eq!(json["info"]["size"], 900);
            assert!(json.get("file").is_none());
            let bad = Sticker { url: "https://x/a".into(), ..s };
            assert_eq!(content_of(&bad).err(), Some(OperationError::InvalidInput));
        }
    }
}

#[cfg(feature = "sdk")]
pub use sdk::{Prepared, decode, prepare};
#[cfg(feature = "sdk")]
pub(crate) use sdk::{edit, hidden_library, packs, send};
#[cfg(all(test, feature = "sdk"))]
pub(crate) use sdk::forget_made;
