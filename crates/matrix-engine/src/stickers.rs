//! Sticker packs (spec v1.19 `m.room.image_pack` + `m.image_pack.rooms`) and sending `m.sticker`.
//! A sticker is sent with only the pack's plain `url` (no re-upload, no `file`), also in encrypted
//! rooms: Megolm encrypts the event, the pack media is public on the homeserver by spec.
// shortcut: packs enabled by the account (`m.image_pack.rooms`) plus those of the current room already in the store (sliding sync does
// not sync custom state, so a room's own pack shows only once enabled); packs of parent spaces are a later card.
use crate::operations::OperationError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sticker {
    pub shortcode: String,
    pub body: String,
    pub url: String,
    pub mimetype: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerPack {
    pub id: String,
    pub name: String,
    pub stickers: Vec<Sticker>,
}

#[cfg(feature = "sdk")]
mod sdk {
    use super::*;
    use matrix_sdk::ruma::events::{
        image_pack::rooms::ImagePackRoomsEventContent,
        room::image_pack::{PackUsage, RoomImagePackEventContent},
        sticker::StickerEventContent,
    };
    use matrix_sdk::ruma::{OwnedMxcUri, UInt};

    /// The pack's sticker entries, or `None` when the pack is for emoticons only or has no sticker.
    pub fn pack_of(id: &str, fallback_name: &str, content: &RoomImagePackEventContent) -> Option<StickerPack> {
        let usage = &content.pack.usage;
        if !usage.is_empty() && !usage.contains(&PackUsage::Sticker) {
            return None;
        }
        let stickers: Vec<_> = content
            .images
            .iter()
            .filter(|(_, i)| i.url.is_valid())
            .map(|(code, i)| Sticker {
                shortcode: code.clone(),
                body: i.body.clone().filter(|b| !b.is_empty()).unwrap_or_else(|| code.clone()),
                url: i.url.to_string(),
                mimetype: i.info.as_ref().and_then(|n| n.mimetype.clone()),
                width: i.info.as_ref().and_then(|n| n.width).and_then(|v| u32::try_from(u64::from(v)).ok()),
                height: i.info.as_ref().and_then(|n| n.height).and_then(|v| u32::try_from(u64::from(v)).ok()),
            })
            .collect();
        if stickers.is_empty() {
            return None;
        }
        let name = content.pack.display_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| fallback_name.to_owned());
        Some(StickerPack { id: id.to_owned(), name, stickers })
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
        Ok(StickerEventContent::new(sticker.body.clone(), info, url))
    }

    /// Packs enabled by the account (`m.image_pack.rooms`) plus those of the room itself, in that order, no duplicates.
    pub async fn packs(
        client: &matrix_sdk::Client,
        account: matrix_domain::AccountId,
        scope: &matrix_domain::RoomScope,
    ) -> Result<Vec<StickerPack>, OperationError> {
        use matrix_sdk::deserialized_responses::SyncOrStrippedState::Sync;
        use matrix_sdk::ruma::events::SyncStateEvent::Original;
        if scope.account_id != account {
            return Err(OperationError::WrongScope);
        }
        let current = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
        let mut wanted: Vec<(matrix_sdk::ruma::OwnedRoomId, String)> = Vec::new();
        if let Ok(Some(raw)) = client.account().account_data::<ImagePackRoomsEventContent>().await
            && let Ok(rooms) = raw.deserialize()
        {
            for (room, keys) in rooms.rooms {
                wanted.extend(keys.into_keys().map(|k| (room.clone(), k)));
            }
        }
        if let Some(room) = client.get_room(&current) {
            for raw in room.get_state_events_static::<RoomImagePackEventContent>().await.unwrap_or_default() {
                if let Ok(Sync(Original(e))) = raw.deserialize() {
                    wanted.push((current.clone(), e.state_key));
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (room_id, key) in wanted {
            if !seen.insert((room_id.clone(), key.clone())) {
                continue;
            }
            // Over the network: sliding sync does not put custom state types in the store.
            let request = matrix_sdk::ruma::api::client::state::get_state_event_for_key::v3::Request::new(
                room_id.clone(),
                matrix_sdk::ruma::events::StateEventType::from("m.room.image_pack"),
                key.clone(),
            );
            let Ok(Ok(answer)) = tokio::time::timeout(std::time::Duration::from_secs(10), client.send(request)).await else { continue };
            let Ok(content) = serde_json::from_str::<RoomImagePackEventContent>(answer.event_or_content.get()) else { continue };
            let room_name = client.get_room(&room_id).and_then(|r| r.cached_display_name()).map(|n| n.to_string()).unwrap_or_default();
            if let Some(pack) = pack_of(&format!("{room_id}/{key}"), &room_name, &content) {
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

        fn pack(json: &str) -> RoomImagePackEventContent {
            serde_json::from_str(json).unwrap()
        }

        #[test]
        fn usage_filters_and_body_defaults_to_shortcode() {
            let sticker = pack(r#"{"images":{"ciao":{"url":"mxc://x/a","info":{"mimetype":"image/webp","w":512,"h":256}},"b":{"url":"mxc://x/b","body":"Bi"},"bad":{"url":"https://x/c"}},"pack":{"display_name":"P","usage":["sticker"]}}"#);
            let p = pack_of("!r:x/k", "Stanza", &sticker).unwrap();
            assert_eq!((p.name.as_str(), p.stickers.len()), ("P", 2));
            assert_eq!(p.stickers[1].body, "ciao"); // BTreeMap order: "b" < "bad" < "ciao"
            assert_eq!((p.stickers[1].width, p.stickers[1].mimetype.as_deref()), (Some(512), Some("image/webp")));
            let emoji = pack(r#"{"images":{"a":{"url":"mxc://x/a"}},"pack":{"usage":["emoticon"]}}"#);
            assert!(pack_of("k", "n", &emoji).is_none());
            let all = pack(r#"{"images":{"a":{"url":"mxc://x/a"}}}"#);
            assert_eq!(pack_of("k", "Stanza", &all).unwrap().name, "Stanza");
        }

        #[test]
        fn sticker_event_has_plain_url_and_no_file() {
            let s = Sticker { shortcode: "a".into(), body: "Ciao".into(), url: "mxc://x/a".into(), mimetype: Some("image/png".into()), width: Some(64), height: None };
            let json = serde_json::to_value(content_of(&s).unwrap()).unwrap();
            assert_eq!(json["url"], "mxc://x/a");
            assert_eq!(json["body"], "Ciao");
            assert!(json.get("file").is_none());
            let bad = Sticker { url: "https://x/a".into(), ..s };
            assert_eq!(content_of(&bad).err(), Some(OperationError::InvalidInput));
        }
    }
}

#[cfg(feature = "sdk")]
pub(crate) use sdk::{packs, send};
