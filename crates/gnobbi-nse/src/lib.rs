//! Core of the iOS notification service extension: opens ONE account's crypto store, fetches the pushed event, decrypts it,
//! returns sender / text / room name. "Lite" client: real crypto store, NO persistent state store, no room
//! list, no event cache, so memory does not grow with the number of rooms. Never logs or returns keys; never logs text.
use matrix_sdk::{
    Client, SqliteCryptoStore,
    authentication::matrix::MatrixSession,
    config::{RequestConfig, StoreConfig},
    cross_process_lock::CrossProcessLockConfig,
    ruma::{
        EventId, RoomId,
        api::client::{
            profile::{get_avatar_url, get_display_name}, room::get_room_event, state::get_state_event_for_key,
        },
        events::StateEventType,
    },
    media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings},
};
use matrix_sdk_store_encryption::StoreCipher;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    path::{Path, PathBuf},
    time::Duration,
};

/// Must differ from the app's holder name ("main", the SDK default): the cross-process lock is keyed by holder.
const HOLDER: &str = "gnobbi-nse";
/// The system gives the extension about 30 s; the shell falls back to the generic text earlier on its own.
const BUDGET: Duration = Duration::from_secs(20);
/// Extra time for the sender's picture, after the text is ready: 20 + 5 stays under the system's 30 s.
const AVATAR_BUDGET: Duration = Duration::from_secs(5);
/// Sender picture: a square thumbnail this big, and at most this many bytes accepted while receiving (media rule: capped before buffering).
const AVATAR_PX: u32 = 96;
pub const AVATAR_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Plain or decrypted text message.
    Message {
        sender: String,
        /// The sender's MXID (`sender` is the display name when there is one).
        sender_id: String,
        body: String,
        room: Option<String>,
    },
    /// Message of another kind (image, file, call...): show the sender, keep the generic text.
    Other {
        sender: String,
        sender_id: String,
        room: Option<String>,
    },
    /// Encrypted and the key is not (yet) available.
    Encrypted,
    /// Edit or redaction: the user must not be alerted a second time.
    Suppress,
}

/// Why nothing could be read; the shell shows the generic text for all of them (account gone, offline, token refused...).
#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
    BadInput,
    Store,
    Net,
}

/// `gnobbi_account` of the pusher payload: `<catalog>:<worker>:<dir>` (see matrix_engine::push). Returns the account folder
/// relative to the data dir. Only the two shapes the app writes are accepted: no `..`, no absolute paths.
pub fn store_path(tag: &str) -> Option<PathBuf> {
    let mut parts = tag.splitn(3, ':');
    let (catalog, worker, dir) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(catalog)
        || !digits(worker)
        || (dir != "profile" && dir != format!("accounts/{catalog}"))
    {
        return None;
    }
    Some(Path::new(dir).join(format!("account-{worker}")))
}

/// `version` of `session.sealed` this reader understands; must equal matrix-engine's `SEALED_VERSION` (contract test).
#[doc(hidden)]
pub const SEALED_VERSION: u64 = 1;

/// Reads the app's `session.sealed` (written by matrix-engine `AccountStore::seal`); public only for the contract test.
#[doc(hidden)]
pub fn unseal(dir: &Path, pass: &str) -> Result<(String, MatrixSession), Fail> {
    let bytes = std::fs::read(dir.join("session.sealed")).map_err(|_| Fail::Store)?;
    let record: Value = serde_json::from_slice(&bytes).map_err(|_| Fail::Store)?;
    if record["version"] != SEALED_VERSION {
        return Err(Fail::Store);
    }
    let exported: Vec<u8> =
        serde_json::from_value(record["cipher"].clone()).map_err(|_| Fail::Store)?;
    let payload: Vec<u8> =
        serde_json::from_value(record["payload"].clone()).map_err(|_| Fail::Store)?;
    let plain: Value = StoreCipher::import(pass, &exported)
        .and_then(|c| c.decrypt_value(&payload))
        .map_err(|_| Fail::Store)?;
    let value = &plain["value"];
    Ok((
        value["homeserver"].as_str().ok_or(Fail::Store)?.to_owned(),
        serde_json::from_value(value["session"].clone()).map_err(|_| Fail::Store)?,
    ))
}

/// One sliding sync "encryption" pass (no lists, e2ee + to-device only, like the SDK's EncryptionSyncService): brings in room keys
/// the app has not stored yet. The NSE keeps no cursor, so the app receives the same to-device events again and discards them.
async fn sync_encryption(client: &Client) -> Result<(), Fail> {
    use matrix_sdk::ruma::{api::client::sync::sync_events::v5 as http, assign};
    client.set_sliding_sync_version(matrix_sdk::sliding_sync::Version::Native);
    client
        .encryption()
        .enable_cross_process_store_lock(HOLDER.into())
        .await
        .map_err(|_| Fail::Store)?;
    let ss = client
        .sliding_sync("encryption")
        .map_err(|_| Fail::Net)?
        .with_to_device_extension(
            assign!(http::request::ToDevice::default(), { enabled: Some(true) }),
        )
        .with_e2ee_extension(assign!(http::request::E2EE::default(), { enabled: Some(true) }))
        .poll_timeout(Duration::ZERO)
        .network_timeout(Duration::from_secs(10))
        .build()
        .await
        .map_err(|_| Fail::Net)?;
    let _lock = client
        .encryption()
        .spin_lock_store(Some(10_000))
        .await
        .map_err(|_| Fail::Store)?;
    ss.sync_once().await.map_err(|_| Fail::Net)?;
    Ok(())
}

/// Pure: what to show for an event (already decrypted, or never encrypted).
fn classify(event: &Value, sender_name: Option<String>, room: Option<String>) -> Outcome {
    let content = &event["content"];
    if event["type"] == "m.room.redaction" || content["m.relates_to"]["rel_type"] == "m.replace" {
        return Outcome::Suppress;
    }
    let sender_id = event["sender"].as_str().unwrap_or_default().to_owned();
    let sender = sender_name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| sender_id.clone());
    let text = event["type"] == "m.room.message"
        && matches!(
            content["msgtype"].as_str(),
            Some("m.text" | "m.notice" | "m.emote")
        );
    match content["body"].as_str() {
        Some(body) if text && !body.is_empty() => Outcome::Message {
            sender,
            sender_id,
            body: body.to_owned(),
            room,
        },
        _ => Outcome::Other { sender, sender_id, room },
    }
}

/// Body for display: trimmed, and every run of line breaks (blank lines, spaces between them) collapsed into one `\n`.
pub fn trim_body(body: &str) -> String {
    // Every Unicode line break (not only \n / \r\n: lone \r, NEL, LS, PS) counts; inner indentation is kept.
    let brk = |c: char| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
    body.split(brk).map(str::trim_end).filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n").trim().to_owned()
}

/// What a notification shows. Title: the sender's name, else the MXID. Subtitle: the MXID (none when it is the title). A room
/// name that is not the sender's own name (a group; in a bridged DM the room is named after the contact) prefixes the text.
#[derive(Debug, PartialEq, Eq)]
pub struct Shown {
    pub title: String,
    pub subtitle: Option<String>,
    pub body: String,
}

pub fn shown(sender: &str, sender_id: &str, room: Option<&str>, body: &str) -> Shown {
    let body = trim_body(body);
    // A blank sender name falls back to the MXID (the title is never empty).
    let sender = if sender.trim().is_empty() { sender_id } else { sender };
    let body = match room.filter(|r| !r.trim().is_empty() && *r != sender) {
        Some(r) => format!("{r}: {body}"),
        None => body,
    };
    Shown {
        title: sender.to_owned(),
        subtitle: Some(sender_id.to_owned()).filter(|id| !id.is_empty() && id != sender),
        body,
    }
}

async fn fetch(dir: &Path, pass: &str, room_id: &str, event_id: &str, avatar: bool) -> Result<(Outcome, Option<Vec<u8>>), Fail> {
    // Refused before any IO.
    <&RoomId>::try_from(room_id).map_err(|_| Fail::BadInput)?;
    <&EventId>::try_from(event_id).map_err(|_| Fail::BadInput)?;
    let (homeserver, session) = unseal(dir, pass)?;
    let crypto = SqliteCryptoStore::open(dir, Some(pass))
        .await
        .map_err(|_| Fail::Store)?;
    let lock = CrossProcessLockConfig::MultiProcess {
        holder_name: HOLDER.into(),
    };
    let client = Client::builder()
        .homeserver_url(homeserver)
        .disable_well_known_lookup(true)
        .request_config(
            RequestConfig::new()
                .retry_limit(0)
                .timeout(Duration::from_secs(10)),
        )
        .cross_process_store_config(lock.clone())
        .store_config(StoreConfig::new(lock).crypto_store(crypto))
        .build()
        .await
        .map_err(|_| Fail::Store)?;
    client
        .restore_session(session)
        .await
        .map_err(|_| Fail::Store)?;
    let outcome = read_event(&client, room_id, event_id, true).await?;
    let picture = match &outcome {
        Outcome::Message { sender_id, .. } | Outcome::Other { sender_id, .. } if avatar => {
            // Any failure or timeout: no picture, the notification stays as it is.
            tokio::time::timeout(AVATAR_BUDGET, sender_avatar(&client, room_id, sender_id)).await.ok().flatten()
        }
        _ => None,
    };
    Ok((outcome, picture))
}

/// The `mxc://` URI in a member event's content or a profile answer; blank or missing = none.
fn avatar_uri(v: &Value) -> Option<String> {
    v["avatar_url"].as_str().or(v["content"]["avatar_url"].as_str()).filter(|u| safe_mxc(u)).map(str::to_owned)
}

/// `mxc://server/id` whose server part is under 250 bytes: ruma's `validate()` panics on longer ones (same guard as the engine's `checked_source`).
fn safe_mxc(u: &str) -> bool {
    u.strip_prefix("mxc://").and_then(|r| r.find('/')).is_some_and(|i| i < 250)
}

/// The sender's picture as a small thumbnail (the per-room one first, then the profile's), or None on any failure.
#[allow(deprecated)] // the profile-field endpoint is newer than many homeservers
async fn sender_avatar(client: &Client, room_id: &str, sender_id: &str) -> Option<Vec<u8>> {
    let rid = <&RoomId>::try_from(room_id).ok()?;
    let user = <&matrix_sdk::ruma::UserId>::try_from(sender_id).ok()?.to_owned();
    let member = client
        .send(get_state_event_for_key::v3::Request::new(rid.to_owned(), StateEventType::RoomMember, user.to_string()))
        .await
        .ok()
        .and_then(|r| serde_json::to_value(&r.event_or_content).ok())
        .and_then(|v| avatar_uri(&v));
    let uri = match member {
        Some(u) => u,
        None => client.send(get_avatar_url::v3::Request::new(user)).await.ok()?.avatar_url?.to_string(),
    };
    download_avatar(client, &uri, AVATAR_MAX_BYTES).await
}

/// Thumbnail of `mxc`, refused as soon as more than `max_bytes` arrive (SDK `max_response_size`); only plausible pictures pass.
async fn download_avatar(client: &Client, mxc: &str, max_bytes: usize) -> Option<Vec<u8>> {
    if !safe_mxc(mxc) {
        return None;
    }
    let uri = <&matrix_sdk::ruma::MxcUri>::from(mxc);
    uri.validate().ok()?;
    let request = MediaRequestParameters {
        source: matrix_sdk::ruma::events::room::MediaSource::Plain(uri.to_owned()),
        format: MediaFormat::Thumbnail(MediaThumbnailSettings::with_method(
            matrix_sdk::ruma::api::client::media::get_content_thumbnail::v3::Method::Crop,
            AVATAR_PX.into(),
            AVATAR_PX.into(),
        )),
    };
    let config = client.request_config().timeout(Some(AVATAR_BUDGET)).max_response_size(Some(max_bytes));
    let bytes = matrix_sdk::media::DefaultMediaFetcher.fetch_media_content_with_request_config(client, &request, config).await.ok()?;
    plausible_picture(bytes, max_bytes)
}

/// PNG or JPEG magic and within the cap (INImage reads those); the server may return anything.
fn plausible_picture(bytes: Vec<u8>, max_bytes: usize) -> Option<Vec<u8>> {
    let known = bytes.starts_with(&[0x89, b'P', b'N', b'G']) || bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
    (known && bytes.len() <= max_bytes).then_some(bytes)
}

/// Fetches, decrypts and classifies one pushed event with an already logged-in `client`. `sync_keys`: when the key is missing, run one
/// encryption sync (the lite client of the extension); false for the app's own client, whose sync brings the keys: then `Encrypted`.
pub async fn read_event(client: &Client, room_id: &str, event_id: &str, sync_keys: bool) -> Result<Outcome, Fail> {
    let rid = <&RoomId>::try_from(room_id).map_err(|_| Fail::BadInput)?;
    let eid = <&EventId>::try_from(event_id).map_err(|_| Fail::BadInput)?;
    let mut synced = false;
    let event = loop {
        let raw = client
            .send(get_room_event::v3::Request::new(
                rid.to_owned(),
                eid.to_owned(),
            ))
            .await
            .map_err(|_| Fail::Net)?
            .event;
        let plain: Value = serde_json::from_str(raw.json().get()).map_err(|_| Fail::Net)?;
        if plain["type"] != "m.room.encrypted" {
            break plain;
        }
        if let Some(d) = client.decrypt_room_event_raw(&raw, rid).await {
            break serde_json::from_str(d.json().get()).map_err(|_| Fail::Net)?;
        }
        // Key missing: one sync of keys, then one more try; a failed or useless sync leaves the message undecryptable.
        if !sync_keys || synced || sync_encryption(client).await.is_err() {
            return Ok(Outcome::Encrypted);
        }
        synced = true;
    };
    // Cheap early exit: no names for something we do not show.
    if classify(&event, None, None) == Outcome::Suppress {
        return Ok(Outcome::Suppress);
    }
    // No state store: sender name and room name by two small requests.
    let sender =
        <&matrix_sdk::ruma::UserId>::try_from(event["sender"].as_str().unwrap_or_default())
            .ok()
            .map(ToOwned::to_owned);
    let (who, room) = tokio::join!(
        async {
            let Some(user) = sender else { return None };
            // The per-room name first (bridge ghosts often have no global profile name), then the profile.
            let member = client
                .send(get_state_event_for_key::v3::Request::new(
                    rid.to_owned(),
                    StateEventType::RoomMember,
                    user.to_string(),
                ))
                .await
                .ok()
                .and_then(|r| serde_json::to_value(&r.event_or_content).ok())
                .and_then(|v| {
                    v["displayname"]
                        .as_str()
                        .or(v["content"]["displayname"].as_str())
                        .filter(|n| !n.trim().is_empty())
                        .map(str::to_owned)
                });
            if member.is_some() {
                return member;
            }
            #[allow(deprecated)] // the profile-field endpoint is newer than many homeservers
            client
                .send(get_display_name::v3::Request::new(user))
                .await
                .ok()
                .and_then(|r| r.displayname)
        },
        async {
            let r = client
                .send(get_state_event_for_key::v3::Request::new(
                    rid.to_owned(),
                    StateEventType::RoomName,
                    String::new(),
                ))
                .await
                .ok()?;
            let v = serde_json::to_value(&r.event_or_content).ok()?;
            v["name"]
                .as_str()
                .or(v["content"]["name"].as_str())
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
        }
    );
    Ok(classify(&event, who, room))
}

/// Blocking entry point: a current-thread runtime (no worker pool inside the extension) bounded by [`BUDGET`].
pub fn run(
    data_dir: &Path,
    tag: &str,
    pass: &str,
    room_id: &str,
    event_id: &str,
) -> Result<Outcome, Fail> {
    run_for(BUDGET, data_dir, tag, pass, room_id, event_id)
}

/// [`run`] plus the sender's picture (PNG/JPEG, at most [`AVATAR_MAX_BYTES`]) when there is one: iOS Communication Notifications.
pub fn run_with_avatar(
    data_dir: &Path,
    tag: &str,
    pass: &str,
    room_id: &str,
    event_id: &str,
) -> Result<(Outcome, Option<Vec<u8>>), Fail> {
    run_inner(BUDGET + AVATAR_BUDGET, data_dir, tag, pass, room_id, event_id, true)
}

/// [`run`] with another time budget (Android: onMessageReceived has about 10 s).
pub fn run_for(
    budget: Duration,
    data_dir: &Path,
    tag: &str,
    pass: &str,
    room_id: &str,
    event_id: &str,
) -> Result<Outcome, Fail> {
    run_inner(budget, data_dir, tag, pass, room_id, event_id, false).map(|(o, _)| o)
}

fn run_inner(
    budget: Duration,
    data_dir: &Path,
    tag: &str,
    pass: &str,
    room_id: &str,
    event_id: &str,
    avatar: bool,
) -> Result<(Outcome, Option<Vec<u8>>), Fail> {
    let dir = data_dir.join(store_path(tag).ok_or(Fail::BadInput)?);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Fail::Net)?
        .block_on(async {
            tokio::time::timeout(budget, fetch(&dir, pass, room_id, event_id, avatar))
                .await
                .unwrap_or(Err(Fail::Net))
        })
}

/// The room name when the room is a group: any name that is not the sender's own (same rule as the body prefix in [`shown`]).
fn group_name<'a>(sender: &str, room: Option<&'a str>) -> Option<&'a str> {
    room.filter(|r| !r.trim().is_empty() && *r != sender)
}

fn to_json(o: &Outcome) -> Value {
    match o {
        Outcome::Message { sender, sender_id, body, room } => {
            let s = shown(sender, sender_id, room.as_deref(), body);
            // `text`: the body without the room prefix, for the group intent (iOS shows the group name itself).
            json!({"kind": "message", "title": s.title, "subtitle": s.subtitle, "body": s.body, "text": trim_body(body),
                   "sender_id": sender_id, "group": group_name(sender, room.as_deref())})
        }
        Outcome::Other { sender, sender_id, room } => {
            json!({"kind": "other", "title": sender, "sender_id": sender_id, "group": group_name(sender, room.as_deref())})
        }
        Outcome::Encrypted => json!({"kind": "encrypted"}),
        Outcome::Suppress => json!({"kind": "suppress"}),
    }
}

/// C ABI. Returns 0 and a malloc'd JSON `{kind, title?, subtitle?, body?, text?, sender_id?, group?}` in `*out` (and, when the sender
/// has a picture, its bytes in `*avatar`/`*avatar_len`, freed with [`gnobbi_nse_free_bytes`]), or 1 with `*out` NULL (the shell then shows
/// the generic text). Free with [`gnobbi_nse_free`]. `data_dir` is the App Group data dir (`.../Library/Application Support/Gnobbi`).
///
/// # Safety
/// All string pointers must be valid NUL-terminated strings; `out` a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnobbi_nse_fetch(
    data_dir: *const c_char,
    tag: *const c_char,
    pass: *const c_char,
    room: *const c_char,
    event: *const c_char,
    out: *mut *mut c_char,
    avatar: *mut *mut u8,
    avatar_len: *mut usize,
) -> i32 {
    unsafe {
        *out = std::ptr::null_mut();
        *avatar = std::ptr::null_mut();
        *avatar_len = 0;
    }
    let s = |p: *const c_char| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    let (dir, tag, pass, room, event) = (s(data_dir), s(tag), s(pass), s(room), s(event));
    match std::panic::catch_unwind(|| run_with_avatar(Path::new(&dir), &tag, &pass, &room, &event)) {
        Ok(Ok((o, picture))) => {
            unsafe {
                *out = CString::new(to_json(&o).to_string())
                    .unwrap_or_default()
                    .into_raw()
            };
            if let Some(bytes) = picture {
                let boxed = bytes.into_boxed_slice();
                unsafe {
                    *avatar_len = boxed.len();
                    *avatar = Box::into_raw(boxed) as *mut u8;
                }
            }
            0
        }
        _ => 1,
    }
}

/// # Safety
/// `p`/`len` come from the `avatar` out parameters of [`gnobbi_nse_fetch`] (or `p` is NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnobbi_nse_free_bytes(p: *mut u8, len: usize) {
    if !p.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, len)) })
    }
}

/// # Safety
/// `p` comes from [`gnobbi_nse_fetch`] (or is NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnobbi_nse_free(p: *mut c_char) {
    if !p.is_null() {
        drop(unsafe { CString::from_raw(p) })
    }
}

/// Peak `phys_footprint` of this process in bytes (what jetsam enforces); 0 off Apple. The shell logs the number, nothing else.
#[unsafe(no_mangle)]
pub extern "C" fn gnobbi_nse_peak_footprint() -> u64 {
    #[cfg(target_vendor = "apple")]
    {
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buf: *mut u64) -> i32;
            fn getpid() -> i32;
        }
        let mut b = [0u64; 64];
        // RUSAGE_INFO_V4: ri_lifetime_max_phys_footprint is u64 #28 after the 16-byte uuid.
        if unsafe { proc_pid_rusage(getpid(), 4, b.as_mut_ptr()) } == 0 {
            return b[2 + 28];
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(v: Value) -> Value {
        v
    }

    #[test]
    fn tag_maps_to_the_account_folder_and_nothing_else() {
        assert_eq!(
            store_path("3:7:accounts/3"),
            Some(PathBuf::from("accounts/3/account-7"))
        );
        assert_eq!(
            store_path("1:1:profile"),
            Some(PathBuf::from("profile/account-1"))
        );
        for bad in [
            "",
            "3:7",
            "3:7:accounts/4",
            "3:7:../x",
            "3:7:/etc",
            "a:7:profile",
            "3::profile",
            "3:7:accounts/3/..",
            "3:7:accounts/3:x",
        ] {
            assert_eq!(store_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn classifies_what_the_server_pushed() {
        let msg = |t: &str, body: &str| {
            ev(
                json!({"type": "m.room.message", "sender": "@a:h", "content": {"msgtype": t, "body": body}}),
            )
        };
        assert_eq!(
            classify(
                &msg("m.text", "hi"),
                Some("Ann".into()),
                Some("Team".into())
            ),
            Outcome::Message {
                sender: "Ann".into(),
                sender_id: "@a:h".into(),
                body: "hi".into(),
                room: Some("Team".into())
            }
        );
        assert_eq!(
            classify(&msg("m.text", "hi"), None, None),
            Outcome::Message {
                sender: "@a:h".into(),
                sender_id: "@a:h".into(),
                body: "hi".into(),
                room: None
            }
        );
        assert_eq!(
            classify(&msg("m.image", "x.png"), None, None),
            Outcome::Other {
                sender: "@a:h".into(),
                sender_id: "@a:h".into(),
                room: None
            }
        );
        assert_eq!(
            classify(&msg("m.text", ""), None, None),
            Outcome::Other {
                sender: "@a:h".into(),
                sender_id: "@a:h".into(),
                room: None
            }
        );
        let edit = json!({"type": "m.room.message", "sender": "@a:h", "content": {"msgtype": "m.text", "body": "* hi", "m.relates_to": {"rel_type": "m.replace", "event_id": "$e"}}});
        assert_eq!(classify(&edit, None, None), Outcome::Suppress);
        assert_eq!(
            classify(
                &json!({"type": "m.room.redaction", "sender": "@a:h", "content": {}}),
                None,
                None
            ),
            Outcome::Suppress
        );
    }

    #[test]
    fn body_is_trimmed_and_line_breaks_collapse() {
        assert_eq!(trim_body("\n\n  ciao\n\n\nmondo  "), "ciao\nmondo");
        assert_eq!(trim_body("a\n \t\n\nb"), "a\nb");
        assert_eq!(trim_body(" \n "), "");
    }

    #[test]
    fn dm_and_group_titles() {
        // name present: name + MXID; bridged DM room named after the contact adds nothing
        assert_eq!(
            shown("Idealmedica", "@wa_1:h", Some("Idealmedica"), "\n\nciao"),
            Shown { title: "Idealmedica".into(), subtitle: Some("@wa_1:h".into()), body: "ciao".into() }
        );
        // name absent: MXID alone, no subtitle
        assert_eq!(
            shown("@a:h", "@a:h", None, "hi"),
            Shown { title: "@a:h".into(), subtitle: None, body: "hi".into() }
        );
        // group: sender stays the title, the room name leads the text
        assert_eq!(
            shown("Ann", "@a:h", Some("Team"), " hi "),
            Shown { title: "Ann".into(), subtitle: Some("@a:h".into()), body: "Team: hi".into() }
        );
    }

    #[test]
    fn body_edge_cases() {
        assert_eq!(trim_body(""), "");
        assert_eq!(trim_body("\n\n\r\n \u{2028}\u{85}"), "");
        assert_eq!(trim_body("a\r\n\r\nb\rc"), "a\nb\nc");
        assert_eq!(trim_body("a\u{2028}\u{2029}\u{85}b"), "a\nb");
        assert_eq!(trim_body("a\n\n\n\n\nb"), "a\nb");
        assert_eq!(trim_body("a\nb\nc"), "a\nb\nc"); // single inner breaks stay
        assert_eq!(trim_body("  \n  riga\n    indentata  \n"), "riga\n    indentata");
        assert_eq!(trim_body("\n 🎉 ciao à 日本語 \n\n👩\u{200d}👩\u{200d}👧 \n"), "🎉 ciao à 日本語\n👩\u{200d}👩\u{200d}👧");
        let long = "x".repeat(10_000);
        assert_eq!(trim_body(&format!("\n{long}\n")), long);
    }

    #[test]
    fn title_and_subtitle_edge_cases() {
        // blank name: MXID is the title, no subtitle
        for blank in ["", "   ", "\t\n"] {
            assert_eq!(
                shown(blank, "@a:h", None, "hi"),
                Shown { title: "@a:h".into(), subtitle: None, body: "hi".into() },
                "{blank:?}"
            );
        }
        // name == MXID: no duplicate subtitle
        assert_eq!(shown("@a:h", "@a:h", None, "hi").subtitle, None);
        // blank room name adds no prefix; empty body stays empty
        assert_eq!(shown("Ann", "@a:h", Some("  "), "hi").body, "hi");
        assert_eq!(shown("Ann", "@a:h", Some("Team"), " \n ").body, "Team: ");
        // MXID of a WhatsApp bridge ghost, very long: passed through untouched
        let id = format!("@whatsapp_{}:bridge.example.org", "3".repeat(200));
        let s = shown("Mario Rossi 🇮🇹", &id, None, "ciao");
        assert_eq!((s.title.as_str(), s.subtitle.as_deref()), ("Mario Rossi 🇮🇹", Some(id.as_str())));
        // JSON handed to the Swift shell
        let j = to_json(&Outcome::Message { sender: "Ann".into(), sender_id: "@a:h".into(), body: "\n\nhi\n\n\nyo".into(), room: Some("Team".into()) });
        assert_eq!(
            j,
            json!({"kind": "message", "title": "Ann", "subtitle": "@a:h", "body": "Team: hi\nyo", "text": "hi\nyo", "sender_id": "@a:h", "group": "Team"})
        );
        // DM (room named like the sender) or no room: not a group
        let dm = to_json(&Outcome::Other { sender: "Ann".into(), sender_id: "@a:h".into(), room: Some("Ann".into()) });
        assert_eq!(dm, json!({"kind": "other", "title": "Ann", "sender_id": "@a:h", "group": null}));
    }

    #[test]
    fn bad_input_is_refused_before_any_io() {
        assert_eq!(
            run(Path::new("/nonexistent"), "9:9:profile", "p", "!r:h", "$e"),
            Err(Fail::Store)
        ); // valid shape, no store: generic text
        assert_eq!(
            run(Path::new("/nonexistent"), "../..", "p", "!r:h", "$e"),
            Err(Fail::BadInput)
        );
        assert_eq!(
            run(
                Path::new("/nonexistent"),
                "1:1:profile",
                "p",
                "not a room",
                "$e"
            ),
            Err(Fail::BadInput)
        );
    }

    #[test]
    fn avatar_uri_from_member_or_profile() {
        assert_eq!(avatar_uri(&json!({"content": {"avatar_url": "mxc://h/abc"}})).as_deref(), Some("mxc://h/abc"));
        assert_eq!(avatar_uri(&json!({"avatar_url": "mxc://h/abc"})).as_deref(), Some("mxc://h/abc"));
        for none in [json!({}), json!({"avatar_url": ""}), json!({"avatar_url": "https://h/x.png"}), json!({"avatar_url": null}), json!({"avatar_url": format!("mxc://{}/x", "a".repeat(250))})] {
            assert_eq!(avatar_uri(&none), None, "{none}");
        }
    }

    #[test]
    fn hostile_server_length_is_refused_not_a_panic() {
        assert!(safe_mxc(&format!("mxc://{}/x", "a".repeat(249))));
        assert!(!safe_mxc(&format!("mxc://{}/x", "a".repeat(250))));
        assert!(!safe_mxc("mxc://noslash"));
    }

    #[test]
    fn picture_must_be_png_or_jpeg_within_the_cap() {
        let png = [&[0x89u8, b'P', b'N', b'G'][..], &[0; 10]].concat();
        let jpg = [&[0xFFu8, 0xD8, 0xFF][..], &[0; 10]].concat();
        assert_eq!(plausible_picture(png.clone(), 100), Some(png.clone()));
        assert_eq!(plausible_picture(jpg.clone(), 100), Some(jpg));
        assert_eq!(plausible_picture(png.clone(), png.len() - 1), None); // over the cap
        assert_eq!(plausible_picture(b"<html>not found".to_vec(), 100), None); // junk
        assert_eq!(plausible_picture(vec![], 100), None);
    }

    #[test]
    fn failure_leaves_both_abi_outputs_empty() {
        // no store: failure (generic text), the avatar out parameters stay empty
        let c = |s: &str| CString::new(s).unwrap();
        let (dir, tag, pass, room, ev) = (c("/nonexistent"), c("9:9:profile"), c("p"), c("!r:h"), c("$e"));
        let (mut out, mut av, mut len) = (std::ptr::null_mut(), std::ptr::null_mut(), 7usize);
        let rc = unsafe { gnobbi_nse_fetch(dir.as_ptr(), tag.as_ptr(), pass.as_ptr(), room.as_ptr(), ev.as_ptr(), &mut out, &mut av, &mut len) };
        assert_eq!((rc, out.is_null(), av.is_null(), len), (1, true, true, 0));
    }

    #[test]
    fn avatar_bytes_are_freed_with_their_length() {
        let b = vec![1u8, 2, 3].into_boxed_slice();
        let len = b.len();
        let p = Box::into_raw(b) as *mut u8;
        unsafe { gnobbi_nse_free_bytes(p, len) };
        unsafe { gnobbi_nse_free_bytes(std::ptr::null_mut(), 0) };
    }
}
