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
            profile::get_display_name, room::get_room_event, state::get_state_event_for_key,
        },
        events::StateEventType,
    },
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

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Plain or decrypted text message.
    Message {
        sender: String,
        body: String,
        room: Option<String>,
    },
    /// Message of another kind (image, file, call...): show the sender, keep the generic text.
    Other {
        sender: String,
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

/// Reads the app's `session.sealed` (written by matrix-engine `AccountStore::seal`); public only for the contract test.
#[doc(hidden)]
pub fn unseal(dir: &Path, pass: &str) -> Result<(String, MatrixSession), Fail> {
    let bytes = std::fs::read(dir.join("session.sealed")).map_err(|_| Fail::Store)?;
    let record: Value = serde_json::from_slice(&bytes).map_err(|_| Fail::Store)?;
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
    let sender = sender_name
        .or_else(|| event["sender"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let text = event["type"] == "m.room.message"
        && matches!(
            content["msgtype"].as_str(),
            Some("m.text" | "m.notice" | "m.emote")
        );
    match content["body"].as_str() {
        Some(body) if text && !body.is_empty() => Outcome::Message {
            sender,
            body: body.to_owned(),
            room,
        },
        _ => Outcome::Other { sender, room },
    }
}

async fn fetch(dir: &Path, pass: &str, room_id: &str, event_id: &str) -> Result<Outcome, Fail> {
    let rid = <&RoomId>::try_from(room_id).map_err(|_| Fail::BadInput)?;
    let eid = <&EventId>::try_from(event_id).map_err(|_| Fail::BadInput)?;
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
        if synced || sync_encryption(&client).await.is_err() {
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
            #[allow(deprecated)] // the profile-field endpoint is newer than many homeservers
            match sender {
                Some(user) => client
                    .send(get_display_name::v3::Request::new(user))
                    .await
                    .ok()
                    .and_then(|r| r.displayname),
                None => None,
            }
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
    let dir = data_dir.join(store_path(tag).ok_or(Fail::BadInput)?);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Fail::Net)?
        .block_on(async {
            tokio::time::timeout(BUDGET, fetch(&dir, pass, room_id, event_id))
                .await
                .unwrap_or(Err(Fail::Net))
        })
}

fn to_json(o: &Outcome) -> Value {
    match o {
        Outcome::Message { sender, body, room } => {
            json!({"kind": "message", "sender": sender, "body": body, "room": room})
        }
        Outcome::Other { sender, room } => json!({"kind": "other", "sender": sender, "room": room}),
        Outcome::Encrypted => json!({"kind": "encrypted"}),
        Outcome::Suppress => json!({"kind": "suppress"}),
    }
}

/// C ABI. Returns 0 and a malloc'd JSON `{kind, sender?, body?, room?}` in `*out`, or 1 with `*out` NULL (the shell then shows
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
) -> i32 {
    unsafe { *out = std::ptr::null_mut() };
    let s = |p: *const c_char| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    let (dir, tag, pass, room, event) = (s(data_dir), s(tag), s(pass), s(room), s(event));
    match std::panic::catch_unwind(|| run(Path::new(&dir), &tag, &pass, &room, &event)) {
        Ok(Ok(o)) => {
            unsafe {
                *out = CString::new(to_json(&o).to_string())
                    .unwrap_or_default()
                    .into_raw()
            };
            0
        }
        _ => 1,
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
                body: "hi".into(),
                room: Some("Team".into())
            }
        );
        assert_eq!(
            classify(&msg("m.text", "hi"), None, None),
            Outcome::Message {
                sender: "@a:h".into(),
                body: "hi".into(),
                room: None
            }
        );
        assert_eq!(
            classify(&msg("m.image", "x.png"), None, None),
            Outcome::Other {
                sender: "@a:h".into(),
                room: None
            }
        );
        assert_eq!(
            classify(&msg("m.text", ""), None, None),
            Outcome::Other {
                sender: "@a:h".into(),
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
}
