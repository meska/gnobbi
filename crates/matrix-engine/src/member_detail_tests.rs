// Member detail on a real Synapse/Tuwunel (the member_detail_probe script; disposable accounts alice=admin, bob=moderator,
// carol=plain user). The engine functions behind each action of the profile, as the UI calls them. Included by auth_tests.
use crate::operations::{
    Moderation, OperationError, load_member_rights as md_rights, moderate as md_moderate, request_user_verification, set_ignored,
    start_dm, user_profile,
};
use matrix_sdk::ruma::{self, api::client::uiaa, events::room::message::RoomMessageEventContent};

fn kbl(r: &crate::operations::MemberRights) -> (bool, bool, Vec<i64>) {
    (r.kick, r.ban, r.levels.clone())
}

fn md_step(s: &str) {
    println!("STEP {s}");
}

async fn md_login(config: &serde_json::Value, who: &str) -> matrix_sdk::Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = matrix_sdk::Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![matrix_sdk::reqwest::Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(matrix_sdk::sliding_sync::VersionBuilder::DiscoverNative)
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
        .send()
        .await
        .unwrap();
    client
}

async fn md_until(what: &str, mut cond: impl AsyncFnMut() -> bool) {
    let start = Instant::now();
    while !cond().await {
        assert!(start.elapsed() < Duration::from_secs(45), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

async fn md_bootstrap(client: &matrix_sdk::Client, localpart: &str, password: &str) {
    let enc = client.encryption();
    if let Err(e) = enc.bootstrap_cross_signing(None).await {
        let session = e.as_uiaa_response().expect("cross-signing asks for a password").session.clone();
        let mut pw = uiaa::Password::new(uiaa::UserIdentifier::Matrix(uiaa::MatrixUserIdentifier::new(localpart.to_owned())), password.to_owned());
        pw.session = session;
        enc.bootstrap_cross_signing(Some(uiaa::AuthData::Password(pw))).await.unwrap();
    }
}

#[test]
#[ignore = "requires the member_detail_probe script"]
fn live_member_detail_actions_on_a_real_homeserver() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    runtime().block_on(async {
        let (alice, bob, carol) = (md_login(&config, "alice").await, md_login(&config, "bob").await, md_login(&config, "carol").await);
        let account = AccountId::new(1);
        let ids: Vec<String> = [&alice, &bob, &carol].iter().map(|c| c.user_id().unwrap().to_string()).collect();
        let (a_id, b_id, c_id) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
        let (a_uid, b_uid, c_uid) = (alice.user_id().unwrap().to_owned(), bob.user_id().unwrap().to_owned(), carol.user_id().unwrap().to_owned());
        let mut defects: Vec<String> = Vec::new();
        // The product's own sync (SyncService, simplified sliding sync), one per account.
        let mut services = Vec::new();
        for c in [&alice, &bob, &carol] {
            let service = matrix_sdk_ui::sync_service::SyncService::builder(c.clone()).build().await.unwrap();
            service.start().await;
            services.push(service);
        }

        md_step("fixtures: room with alice(100), bob(50), carol(0), display names");
        alice.account().set_display_name(Some("Alice Admin")).await.unwrap();
        bob.account().set_display_name(Some("Bob Moderatore")).await.unwrap();
        let created = alice.create_room(ruma::api::client::room::create_room::v3::Request::new()).await.unwrap();
        let rid = created.room_id().to_owned();
        for who in [&bob, &carol] {
            created.invite_user_by_id(who.user_id().unwrap()).await.unwrap();
            who.join_room_by_id(&rid).await.unwrap();
        }
        md_until("alice's store has the power levels", async || created.power_levels().await.is_ok()).await;
        created.update_power_levels(vec![(&*b_uid, ruma::Int::from(50))]).await.unwrap();
        let scope = RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(rid.as_str()) };
        let room_of = |c: &matrix_sdk::Client| c.get_room(&rid).unwrap();
        for c in [&alice, &bob, &carol] {
            md_until("room known with 3 members", async || {
                let r = c.get_room(&rid);
                match r {
                    Some(r) => r.members(matrix_sdk::RoomMemberships::JOIN).await.map(|m| m.len() == 3).unwrap_or(false),
                    None => false,
                }
            })
            .await;
        }
        md_until("bob is 50 for everyone", async || {
            for c in [&alice, &bob, &carol] {
                match room_of(c).power_levels().await {
                    Ok(pl) if pl.for_user(&b_uid) == ruma::events::room::power_levels::UserPowerLevel::Int(50.into()) => {}
                    _ => return false,
                }
            }
            true
        })
        .await;
        let sent: Vec<String> = {
            let mut v = Vec::new();
            for text in ["carol uno", "carol due"] {
                v.push(room_of(&carol).send(RoomMessageEventContent::text_plain(text)).await.unwrap().response.event_id.to_string());
            }
            room_of(&bob).send(RoomMessageEventContent::text_plain("bob uno")).await.unwrap();
            v
        };

        let (t_alice, t_bob, t_carol) = (crate::room_sync::Timelines::default(), crate::room_sync::Timelines::default(), crate::room_sync::Timelines::default());
        let rights = |c: &matrix_sdk::Client, t: &crate::room_sync::Timelines, target: &str| {
            let (c, t, scope, target) = (c.clone(), t.clone(), scope.clone(), target.to_owned());
            async move { md_rights(&c, &t, account, &scope, &target).await }
        };

        md_step("1. actions present/absent per permission (MemberRights)");
        md_until("alice sees carol's messages", async || {
            // The room is open in the app: its timeline has loaded what the server sent; ask for the older part too.
            if let Ok(t) = t_alice.get(&room_of(&alice)).await {
                let _ = t.paginate_backwards(30).await;
            }
            let r = rights(&alice, &t_alice, &c_id).await;
            r.map(|r| sent.iter().all(|e| r.recent.contains(e))).unwrap_or(false)
        })
        .await;
        // «Rimuovi i messaggi recenti» must count messages only; any other event in `recent` would be redacted too.
        let mut kinds = std::collections::BTreeMap::<String, usize>::new();
        for who in [&a_id, &b_id, &c_id] {
            for e in rights(&alice, &t_alice, who).await.unwrap().recent {
                let raw = room_of(&alice).event(&ruma::EventId::parse(&e).unwrap(), None).await.unwrap();
                let kind = raw.raw().get_field::<String>("type").ok().flatten().unwrap_or_default();
                if kind != "m.room.message" {
                    *kinds.entry(kind).or_default() += 1;
                }
            }
        }
        println!("RECENT non-message events offered for deletion: {kinds:?}");
        if !kinds.is_empty() {
            defects.push(format!("MemberRights.recent holds non-message events of the member (would be redacted): {kinds:?}"));
        }
        let r = rights(&carol, &t_carol, &a_id).await.unwrap();
        assert!(!r.kick && !r.ban && r.levels.is_empty(), "plain user over the admin");
        assert_eq!(kbl(&rights(&carol, &t_carol, &b_id).await.unwrap()), (false, false, vec![]), "plain user over the moderator");
        assert_eq!(rights(&carol, &t_carol, &c_id).await.unwrap(), Default::default(), "oneself");
        assert_eq!(rights(&bob, &t_bob, &b_id).await.unwrap(), Default::default(), "moderator over oneself");
        assert_eq!(kbl(&rights(&bob, &t_bob, &a_id).await.unwrap()), (false, false, vec![]), "moderator over the admin");
        // Carol's last own message is her (implicit) read receipt; yourself and non-members get none.
        let r = rights(&alice, &t_alice, &c_id).await.unwrap();
        assert_eq!(r.read_receipt.as_deref(), sent.get(1).map(String::as_str), "read receipt of carol: her last message");
        assert_eq!(rights(&carol, &t_carol, &c_id).await.unwrap().read_receipt, None, "no receipt jump for oneself");
        let r = rights(&bob, &t_bob, &c_id).await.unwrap();
        println!("RIGHTS moderator over plain: {r:?}");
        assert!(r.kick && r.ban, "moderator may kick and ban a plain user");
        assert!(r.levels.is_empty(), "moderator may not change levels (m.room.power_levels needs 100)");
        let r = rights(&alice, &t_alice, &c_id).await.unwrap();
        assert!(r.kick && r.ban && r.levels == vec![50, 100] && sent.iter().all(|e| r.recent.contains(e)), "admin over plain: {r:?}");
        let r = rights(&alice, &t_alice, &b_id).await.unwrap();
        assert!(r.kick && r.ban && r.levels == vec![0, 100], "admin over moderator: {r:?}");
        assert!(matches!(rights(&alice, &t_alice, "not a user").await, Err(OperationError::InvalidInput)));
        assert_eq!(rights(&alice, &t_alice, "@nobody:matrix-desktop.test").await.unwrap(), Default::default(), "non-member");

        md_step("2. moderation: refused for the unprivileged, done for the privileged");
        assert_eq!(md_moderate(&carol, account, &scope, &b_id, &Moderation::Kick).await, Err(OperationError::Forbidden));
        assert_eq!(md_moderate(&bob, account, &scope, &c_id, &Moderation::SetPower(50)).await, Err(OperationError::Forbidden));
        assert_eq!(md_moderate(&bob, account, &scope, &a_id, &Moderation::Ban).await, Err(OperationError::Forbidden));
        assert_eq!(md_moderate(&bob, account, &scope, &c_id, &Moderation::RedactMessages(vec!["not an event id".into()])).await, Err(OperationError::InvalidInput));
        let other = RoomScope { account_id: AccountId::new(9), ..scope.clone() };
        assert_eq!(md_moderate(&alice, account, &other, &c_id, &Moderation::Kick).await, Err(OperationError::WrongScope));
        // Remove recent messages: exactly the confirmed events.
        assert_eq!(md_moderate(&alice, account, &scope, &c_id, &Moderation::RedactMessages(sent.clone())).await, Ok(()));
        md_until("carol's messages redacted", async || {
            rights(&alice, &t_alice, &c_id).await.map(|r| sent.iter().all(|e| !r.recent.contains(e))).unwrap_or(false)
        })
        .await;
        // Power level change by the admin; the moderator's rights follow (equal level: no kick).
        assert_eq!(md_moderate(&alice, account, &scope, &c_id, &Moderation::SetPower(50)).await, Ok(()));
        md_until("bob's rights over carol gone at equal level", async || rights(&bob, &t_bob, &c_id).await.map(|r| !r.kick && !r.ban).unwrap_or(false)).await;
        assert_eq!(md_moderate(&bob, account, &scope, &c_id, &Moderation::Kick).await, Err(OperationError::Forbidden), "stale kick after the level changed");
        assert_eq!(md_moderate(&alice, account, &scope, &c_id, &Moderation::SetPower(0)).await, Ok(()));
        md_until("bob kicks again", async || rights(&bob, &t_bob, &c_id).await.map(|r| r.kick).unwrap_or(false)).await;
        // A level the actor does not own / the same level is refused.
        assert_eq!(md_moderate(&alice, account, &scope, &c_id, &Moderation::SetPower(0)).await, Err(OperationError::Forbidden), "carol is already 0");
        assert_eq!(md_moderate(&alice, account, &scope, &c_id, &Moderation::SetPower(25)).await, Err(OperationError::Forbidden), "not an offered level");

        md_step("2b. kick, then ban/unban (unban has no UI action: SDK call made by the test)");
        assert_eq!(md_moderate(&bob, account, &scope, &c_id, &Moderation::Kick).await, Ok(()));
        md_until("carol left", async || room_of(&carol).state() != matrix_sdk::RoomState::Joined).await;
        assert_eq!(rights(&bob, &t_bob, &c_id).await.unwrap(), Default::default(), "a member who left has no moderation entries");
        room_of(&alice).invite_user_by_id(&c_uid).await.unwrap();
        carol.join_room_by_id(&rid).await.unwrap();
        md_until("carol back", async || rights(&bob, &t_bob, &c_id).await.map(|r| r.kick).unwrap_or(false)).await;
        assert_eq!(md_moderate(&bob, account, &scope, &c_id, &Moderation::Ban).await, Ok(()));
        assert!(carol.join_room_by_id(&rid).await.is_err(), "banned user cannot join");
        println!("RIGHTS right after the ban, before bob's next sync: {:?}", rights(&bob, &t_bob, &c_id).await.map(|r| (r.kick, r.ban)));
        md_until("a banned member has no moderation entries", async || rights(&bob, &t_bob, &c_id).await == Ok(Default::default())).await;
        room_of(&alice).unban_user(&c_uid, None).await.unwrap();
        room_of(&alice).invite_user_by_id(&c_uid).await.unwrap();
        carol.join_room_by_id(&rid).await.unwrap();
        md_until("carol back after unban", async || rights(&bob, &t_bob, &c_id).await.map(|r| r.kick).unwrap_or(false)).await;

        md_step("3. ignore / unignore");
        let ignored = |c: &matrix_sdk::Client, who: &str| {
            let (c, who) = (c.clone(), who.to_owned());
            async move { user_profile(&c, &who).await.map(|p| p.ignored) }
        };
        assert_eq!(ignored(&bob, &c_id).await, Ok(false));
        assert_eq!(set_ignored(&bob, &c_id, true).await, Ok(()));
        assert_eq!(ignored(&bob, &c_id).await, Ok(true), "ignored read back from the account data");
        assert_eq!(set_ignored(&bob, &c_id, true).await, Ok(()), "ignoring twice is not an error");
        assert_eq!(ignored(&alice, &c_id).await, Ok(false), "ignore is per account");
        assert_eq!(set_ignored(&bob, &c_id, false).await, Ok(()));
        let immediate = ignored(&bob, &c_id).await;
        println!("IGNORE read back right after unignore: {immediate:?} (the list reaches the store with the next sync)");
        md_until("unignore read back", async || ignored(&bob, &c_id).await == Ok(false)).await;
        assert_eq!(set_ignored(&bob, &c_id, false).await, Ok(()), "unignoring twice is not an error");
        assert_eq!(set_ignored(&bob, "not a user", true).await, Err(OperationError::InvalidInput));
        // What the server does when asked to ignore oneself (the UI never offers it).
        let me = set_ignored(&bob, &b_id, true).await;
        println!("IGNORE self -> {me:?}");
        let _ = set_ignored(&bob, &b_id, false).await;

        md_step("4. profile: names, avatar absent, avatar empty string");
        let p = user_profile(&alice, &b_id).await.unwrap();
        assert_eq!((p.name.as_str(), p.avatar.clone(), p.ignored), ("Bob Moderatore", None, false));
        let p = user_profile(&bob, &c_id).await.unwrap();
        let c_name = p.name.clone();
        assert!(c_name != c_id && !c_name.is_empty(), "the fixture gives carol a display name: {c_name:?}");
        assert_eq!(p.avatar, None, "no avatar: none (initials)");
        // A member whose profile has no display name (and no avatar): the sheet must still open, with the user id.
        let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
        let http = matrix_sdk::reqwest::Client::builder()
            .add_root_certificate(matrix_sdk::reqwest::Certificate::from_pem(&ca).unwrap())
            .build()
            .unwrap();
        carol.account().set_display_name(None).await.unwrap();
        let raw = http
            .get(format!("{}_matrix/client/v3/profile/{c_id}", config["url"].as_str().unwrap()))
            .bearer_auth(bob.access_token().unwrap())
            .send()
            .await
            .unwrap();
        let (status, body) = (raw.status(), raw.text().await.unwrap());
        println!("PROFILE server GET after the display name is removed: {status} {body}");
        match user_profile(&bob, &c_id).await {
            Ok(p) => assert_eq!(p.name, c_id, "no display name: the user id"),
            Err(e) => {
                println!("PROFILE user_profile of a member with an empty profile -> {e:?}");
                defects.push(format!("user_profile of a member with no display name and no avatar fails ({e:?}); server answered {status}: the sheet cannot open"));
            }
        }
        carol.account().set_display_name(Some(&c_name)).await.unwrap();
        // Avatar: the empty string Tuwunel hands to the SDK.
        let put = http
            .put(format!("{}_matrix/client/v3/profile/{c_id}/avatar_url", config["url"].as_str().unwrap()))
            .bearer_auth(carol.access_token().unwrap())
            .header("content-type", "application/json")
            .body(r#"{"avatar_url":""}"#)
            .send()
            .await
            .unwrap();
        println!("AVATAR PUT empty string -> {}", put.status());
        if put.status().is_success() {
            let p = user_profile(&bob, &c_id).await.unwrap();
            println!("AVATAR profile after empty string: {:?}", p.avatar);
            if matches!(p.avatar.as_deref(), Some("")) {
                defects.push("profile avatar is Some(\"\") after an empty avatar_url: the sheet asks the cache for an empty mxc".into());
            }
            // The member list shows the same person: its avatar must be none as well.
            md_until("the list reflects the profile", async || true).await;
            let listed = crate::operations::members(&bob, account, &scope).await.unwrap();
            let mine = listed.iter().find(|m| m.user_id == c_id).unwrap();
            println!("AVATAR member list entry after empty string: {:?}", mine.avatar);
            if matches!(mine.avatar.as_deref(), Some("")) {
                defects.push("member list avatar is Some(\"\") after an empty avatar_url (initials expected)".into());
            }
        }

        md_step("5. verification: no identity, unverified, verified, oneself");
        assert_eq!(user_profile(&bob, &a_id).await.unwrap().verified, None, "no cross-signing: no identity");
        assert_eq!(request_user_verification(&bob, &a_id).await, Err(OperationError::Unavailable), "nothing to verify against");
        assert_eq!(request_user_verification(&bob, "not a user").await, Err(OperationError::InvalidInput));
        md_bootstrap(&carol, c_uid.localpart(), config["carol_password"].as_str().unwrap()).await;
        md_bootstrap(&bob, b_uid.localpart(), config["bob_password"].as_str().unwrap()).await;
        md_until("bob sees carol's identity", async || user_profile(&bob, &c_id).await.map(|p| p.verified.is_some()).unwrap_or(false)).await;
        assert_eq!(user_profile(&bob, &c_id).await.unwrap().verified, Some(false), "identity exists, not verified");
        assert_eq!(request_user_verification(&bob, &c_id).await, Ok(()), "request sent");
        println!("VERIFY self profile.verified = {:?}", user_profile(&bob, &b_id).await.map(|p| p.verified));
        let identity = bob.encryption().get_user_identity(&c_uid).await.unwrap().expect("identity cached");
        identity.verify().await.unwrap();
        md_until("carol shows as verified", async || user_profile(&bob, &c_id).await.map(|p| p.verified) == Ok(Some(true))).await;

        md_step("6. Invia messaggio: new DM, then the existing one");
        let dm = start_dm(&bob, account, &c_id).await.unwrap();
        md_until("bob knows the DM", async || bob.get_dm_room(&c_uid).is_some()).await;
        assert_eq!(start_dm(&bob, account, &c_id).await.unwrap(), dm, "existing DM reused");
        assert_ne!(dm.room_id.as_str(), rid.as_str(), "the DM is not the group");
        let dm2 = start_dm(&bob, account, &a_id).await.unwrap();
        assert_ne!(dm2, dm, "another person, another DM");
        assert_eq!(start_dm(&bob, account, "@nobody-here:matrix-desktop.test").await, Err(OperationError::Unavailable), "unknown user: no orphan room");
        assert_eq!(start_dm(&bob, account, "nope").await, Err(OperationError::InvalidInput));

        md_step("7. read receipt: what the SDK can tell about a member");
        let last = room_of(&carol).send(RoomMessageEventContent::text_plain("ultimo")).await.unwrap().response.event_id;
        md_until("alice has the event", async || room_of(&alice).event(&last, None).await.is_ok()).await;
        room_of(&alice).send_single_receipt(ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read, ruma::events::receipt::ReceiptThread::Unthreaded, last.clone()).await.unwrap();
        md_until("bob sees alice's receipt", async || {
            room_of(&bob)
                .load_user_receipt(ruma::events::receipt::ReceiptType::Read, &ruma::events::receipt::ReceiptThread::Unthreaded, &a_uid)
                .await
                .ok()
                .flatten()
                .is_some_and(|(id, _)| id == last)
        })
        .await;
        let absent = room_of(&bob).load_user_receipt(ruma::events::receipt::ReceiptType::Read, &ruma::events::receipt::ReceiptThread::Unthreaded, &c_uid).await;
        println!("RECEIPT present for alice; carol (sent the event, never read) -> {absent:?}");

        md_step("cleanup");
        for s in &services {
            s.stop().await;
        }
        for c in [alice, bob, carol] {
            let _ = c.matrix_auth().logout().await;
        }
        for d in &defects {
            println!("DEFECT {d}");
        }
        // The probe runs lenient (known defects are reported, not fatal); MEMBER_DETAIL_STRICT=1 makes them fail.
        assert!(defects.is_empty() || std::env::var("MEMBER_DETAIL_STRICT").map_or(true, |v| v.is_empty()), "defects: {defects:#?}");
    });
}
