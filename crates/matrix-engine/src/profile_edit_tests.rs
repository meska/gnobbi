// Own-profile edit (display name, avatar) on a real Synapse/Tuwunel (driver: the member_detail_probe script with
// PROBE_FN=auth::auth_tests::live_profile_edit_on_a_real_homeserver; NOPROFILE=1 for a Synapse that forbids both changes).
// alice edits through the engine flow (`profile::save_profile`); every outcome is read back as bob, a second user, from the server
// (profile endpoint, m.room.member in a shared room, the avatar bytes). Included by auth_tests; reuses the mw_* helpers.
use crate::profile::{self, AvatarEdit, Current, ProfileApi, ProfileError, Saved};
use matrix_sdk::ruma::api::client::profile::get_profile;

/// (displayname, avatar_url) of `who` as the server answers `viewer`.
async fn pe_profile(viewer: &matrix_sdk::Client, who: &matrix_sdk::Client) -> (Option<String>, Option<String>) {
    let req = get_profile::v3::Request::new(who.user_id().unwrap().to_owned());
    let r = viewer.send(req).await.expect("get_profile");
    let field = |k: &str| r.data.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    (field("displayname"), field("avatar_url"))
}

fn pe_image(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, 90]))).write_to(&mut out, format).unwrap();
    out.into_inner()
}

async fn pe_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = std::path::Path::new(&std::env::var("TMPDIR").unwrap()).join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// The picture bytes behind `mxc`, downloaded by `viewer`, decoded: its size.
async fn pe_avatar_size(viewer: &matrix_sdk::Client, mxc: &str) -> (u32, u32) {
    use matrix_sdk::{media::{MediaFormat, MediaRequestParameters}, ruma::events::room::MediaSource};
    let source = MediaSource::Plain(ruma::OwnedMxcUri::from(mxc));
    let bytes = viewer.media().get_media_content(&MediaRequestParameters { source, format: MediaFormat::File }, false).await.expect("download avatar");
    image::GenericImageView::dimensions(&image::load_from_memory(&bytes).expect("avatar decodes"))
}

#[test]
#[ignore = "requires the member_detail_probe script"]
fn live_profile_edit_on_a_real_homeserver() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let forbidden = std::env::var("NOPROFILE").is_ok_and(|v| !v.is_empty());
    runtime().block_on(async {
        let (alice, bob) = (md_login(&config, "alice").await, md_login(&config, "bob").await);
        let a = alice.user_id().unwrap().to_string();
        let save = async |name: &str, avatar: AvatarEdit| {
            let cur = pe_profile(&alice, &alice).await;
            profile::save_profile(&alice, &Current { name: cur.0, has_avatar: cur.1.is_some() }, name, avatar).await
        };
        let prepared = async |name: &str, bytes: &[u8]| profile::prepare_avatar(pe_file(name, bytes).await).await;

        md_step("capabilities as the server reports them");
        let caps = alice.capabilities().await;
        println!("OBS capabilities name={} avatar={} (expected {})", caps.name, caps.avatar, !forbidden);
        assert_eq!((caps.name, caps.avatar), (!forbidden, !forbidden), "capabilities");

        if forbidden {
            md_step("forbidden by the server: the flow refuses without attempting, profile unchanged");
            let before = pe_profile(&bob, &alice).await;
            let png = prepared("ok.png", &pe_image(64, 64, image::ImageFormat::Png)).await.unwrap();
            assert_eq!(save("Nome vietato", AvatarEdit::Keep).await.unwrap_err().error, ProfileError::NotAllowed);
            assert_eq!(save(&a, AvatarEdit::Set(png)).await.unwrap_err().error, ProfileError::NotAllowed);
            // What the UI would not even offer, asked straight to the server, is refused too (403).
            assert_eq!(alice.set_name("Diretto").await, Err(ProfileError::Rejected), "server refuses a direct rename");
            assert_eq!(pe_profile(&bob, &alice).await, before);
            return;
        }

        md_step("display name: valid, unicode, trimmed; read as bob");
        for (typed, want) in [("Alice Rossi", "Alice Rossi"), ("  Zoë 日本 🦀👨‍👩‍👧  ", "Zoë 日本 🦀👨‍👩‍👧"), (&"é".repeat(256), &"é".repeat(256))] {
            assert_eq!(save(typed, AvatarEdit::Keep).await, Ok(Saved { name: true, avatar: false }), "save {} chars", typed.chars().count());
            assert_eq!(pe_profile(&bob, &alice).await.0.as_deref(), Some(want), "bob reads the name");
        }
        let good = pe_profile(&bob, &alice).await.0;

        md_step("display name: empty / blank / 257 chars are refused before the server; unchanged name is a no-op");
        for (typed, err) in [("", ProfileError::NameEmpty), (" \t ", ProfileError::NameEmpty), (&"x".repeat(257), ProfileError::NameTooLong)] {
            assert_eq!(save(typed, AvatarEdit::Keep).await.unwrap_err().error, err);
        }
        assert_eq!(save(good.as_deref().unwrap(), AvatarEdit::Keep).await, Ok(Saved::default()));
        assert_eq!(pe_profile(&bob, &alice).await.0, good, "server name untouched");
        let long = alice.set_name(&"x".repeat(257)).await;
        println!("OBS server answer to a 257-char name sent straight: {long:?}");
        // Synapse enforces 256; Tuwunel 1.9.4 stores any length, so there only the engine's own limit protects the user.
        if std::env::var("PROBE_KIND").as_deref() == Ok("tuwunel") {
            assert_eq!(long, Ok(()));
            assert_eq!(alice.set_name(good.as_deref().unwrap()).await, Ok(()));
        } else {
            assert_eq!(long, Err(ProfileError::Rejected), "server limit matches MAX_NAME_CHARS");
        }
        assert_eq!(pe_profile(&bob, &alice).await.0, good);

        md_step("avatar: PNG then JPEG (large source is bounded to 512), name+avatar together; bob downloads and decodes it");
        let png = prepared("a.png", &pe_image(900, 600, image::ImageFormat::Png)).await.unwrap();
        assert_eq!(save("Alice Rossi", AvatarEdit::Set(png)).await, Ok(Saved { name: true, avatar: true }));
        let (name, mxc1) = pe_profile(&bob, &alice).await;
        assert_eq!(name.as_deref(), Some("Alice Rossi"));
        assert_eq!(pe_avatar_size(&bob, mxc1.as_deref().expect("avatar_url set")).await, (512, 512));
        let jpg = prepared("a.jpg", &pe_image(300, 400, image::ImageFormat::Jpeg)).await.unwrap();
        assert_eq!(save("Alice Rossi", AvatarEdit::Set(jpg)).await, Ok(Saved { name: false, avatar: true }));
        let mxc2 = pe_profile(&bob, &alice).await.1.expect("avatar_url set");
        assert_ne!(Some(&mxc2), mxc1.as_ref(), "new upload, new mxc");
        assert_eq!(pe_avatar_size(&bob, &mxc2).await, (300, 300));

        md_step("avatar: bad inputs never reach the server and leave the picture alone");
        let big = pe_file("big.png", &vec![0; crate::media::MAX_MEDIA_BYTES + 1]).await;
        let gif = pe_file("a.gif", &pe_image(8, 8, image::ImageFormat::Gif)).await;
        let txt = pe_file("t.png", b"not a picture").await;
        assert_eq!(profile::prepare_avatar(big).await.err(), Some(ProfileError::FileTooLarge));
        assert_eq!(profile::prepare_avatar(gif).await.err(), Some(ProfileError::FileInvalid));
        assert_eq!(profile::prepare_avatar(txt).await.err(), Some(ProfileError::FileInvalid));
        assert_eq!(profile::prepare_avatar("/nonexistent/x.png".into()).await.err(), Some(ProfileError::FileUnreadable));
        assert_eq!(pe_profile(&bob, &alice).await.1.as_deref(), Some(mxc2.as_str()));

        md_step("shared room: the new name reaches alice's m.room.member as bob sees it");
        let room = alice.create_room(ruma::api::client::room::create_room::v3::Request::new()).await.unwrap();
        room.invite_user_by_id(bob.user_id().unwrap()).await.unwrap();
        bob.join_room_by_id(room.room_id()).await.unwrap();
        assert_eq!(save("Alice Verdi", AvatarEdit::Keep).await, Ok(Saved { name: true, avatar: false }));
        let rid = room.room_id().to_owned();
        let mut seen = None;
        for _ in 0..30 {
            let req = matrix_sdk::ruma::api::client::state::get_state_event_for_key::v3::Request::new(rid.clone(), matrix_sdk::ruma::events::StateEventType::RoomMember, a.clone());
            let v: serde_json::Value = serde_json::from_str(bob.send(req).await.unwrap().event_or_content.get()).unwrap();
            seen = v["displayname"].as_str().map(str::to_owned);
            if seen.as_deref() == Some("Alice Verdi") { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        println!("OBS m.room.member displayname after rename: {seen:?}");
        assert_eq!(seen.as_deref(), Some("Alice Verdi"));

        md_step("remove avatar; removing again is a no-op; name kept");
        assert_eq!(save("Alice Verdi", AvatarEdit::Remove).await, Ok(Saved { name: false, avatar: true }));
        assert_eq!(pe_profile(&bob, &alice).await, (Some("Alice Verdi".into()), None), "avatar_url gone for bob");
        assert_eq!(save("Alice Verdi", AvatarEdit::Remove).await, Ok(Saved::default()));

        md_step("network error: a server that does not answer gives the one-line Network error, nothing hangs");
        let dead = matrix_sdk::Client::builder().homeserver_url("http://127.0.0.1:9").build().await.unwrap();
        for e in [dead.set_name("x").await, dead.remove_avatar().await, dead.set_avatar(vec![1]).await] {
            assert_eq!(e, Err(ProfileError::Network));
        }
        assert!(!ProfileError::Network.message().contains('\n'));

        md_step("cleanup");
        for c in [alice, bob] {
            let _ = c.matrix_auth().logout().await;
        }
    });
}
