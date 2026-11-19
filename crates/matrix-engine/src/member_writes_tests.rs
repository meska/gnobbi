// Member writes on a real Synapse/Tuwunel (driver: the member_detail_probe script with PROBE_FN=auth::auth_tests::live_member_writes_on_a_real_homeserver;
// disposable accounts alice=admin(100), bob=moderator(50), carol=plain(0)). Invite, kick/ban with and without reason, unban, banned list and the
// power-level checks, on a room AND on a space. Every outcome is read back from the server (the m.room.member state), not from the SDK store.
// Included by auth_tests.
use matrix_sdk::ruma::{
    api::client::state::get_state_event_for_key,
    events::StateEventType,
};

/// (membership, reason) of `who` in `rid`, as the server holds it.
async fn mw_member(c: &matrix_sdk::Client, rid: &ruma::RoomId, who: &str) -> (String, Option<String>) {
    let req = get_state_event_for_key::v3::Request::new(rid.to_owned(), StateEventType::RoomMember, who.to_owned());
    let v: serde_json::Value = serde_json::from_str(c.send(req).await.expect("member state").event_or_content.get()).unwrap();
    (v["membership"].as_str().unwrap().to_owned(), v["reason"].as_str().map(str::to_owned))
}

async fn mw_pl(c: &matrix_sdk::Client, rid: &ruma::RoomId) -> serde_json::Value {
    let req = get_state_event_for_key::v3::Request::new(rid.to_owned(), StateEventType::RoomPowerLevels, String::new());
    serde_json::from_str(c.send(req).await.expect("power levels").event_or_content.get()).unwrap()
}

async fn mw_perms(c: &matrix_sdk::Client, scope: &RoomScope) -> (bool, bool, bool) {
    let p = crate::operations::member_permissions(c, scope.account_id, scope).await.expect("member_permissions");
    (p.can_invite, p.can_kick, p.can_ban)
}

async fn mw_banned(c: &matrix_sdk::Client, scope: &RoomScope) -> Vec<(String, Option<String>)> {
    let mut v: Vec<_> = crate::operations::banned_members(c, scope.account_id, scope)
        .await
        .expect("banned_members")
        .into_iter()
        .map(|b| {
            assert!(!b.name.is_empty(), "banned member without a name");
            (b.user_id, b.reason)
        })
        .collect();
    v.sort();
    v
}

/// One scenario on one room or space. Starts with alice(100), bob(50), carol(0) joined.
async fn mw_scenario(label: &str, alice: &matrix_sdk::Client, bob: &matrix_sdk::Client, carol: &matrix_sdk::Client, scope: &RoomScope) {
    let rid = ruma::RoomId::parse(scope.room_id.as_str()).unwrap();
    let acc = scope.account_id;
    let (a, b, c) = (alice.user_id().unwrap().to_string(), bob.user_id().unwrap().to_string(), carol.user_id().unwrap().to_string());
    let c_uid = carol.user_id().unwrap().to_owned();
    let ra = alice.get_room(&rid).unwrap();
    let at = |who: &str| {
        let (alice, rid, who) = (alice.clone(), rid.clone(), who.to_owned());
        async move { mw_member(&alice, &rid, &who).await }
    };
    let moderate = |actor: &matrix_sdk::Client, target: &str, act: Moderation| {
        let (actor, scope, target) = (actor.clone(), scope.clone(), target.to_owned());
        async move { md_moderate(&actor, acc, &scope, &target, &act).await }
    };
    let put_pl = async |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut content = mw_pl(alice, &rid).await;
        edit(&mut content);
        ra.send_state_event_raw("m.room.power_levels", "", content).await.expect("set power levels");
    };
    let carol_in = async || {
        let r = carol.get_room(&rid);
        match r {
            Some(r) => r.state() == matrix_sdk::RoomState::Joined,
            None => false,
        }
    };
    // Carol back in: invited by the moderator through the engine (also the positive invite case), then she joins.
    let rejoin = async |who: &str| {
        let r = moderate(bob, who, Moderation::Invite).await;
        if r.is_err() {
            // Names the raw server answer behind a collapsed OperationError.
            let raw = bob.get_room(&rid).unwrap().invite_user_by_id(&c_uid).await;
            println!("OBS {label}: invite refused {r:?}; raw SDK answer: {raw:?}");
        }
        assert_eq!(r, Ok(()), "{label}: invite {who}");
        assert_eq!(at(who).await, ("invite".into(), None), "{label}: invite event");
        carol.join_room_by_id(&rid).await.unwrap_or_else(|e| panic!("{label}: carol joins: {e}"));
        assert_eq!(at(who).await.0, "join", "{label}: joined");
        md_until("carol is joined in her store", async || carol_in().await).await;
    };

    md_step(&format!("{label}: thresholds invite/kick/ban = 50 for everyone, bob moderator"));
    put_pl(&|pl| {
        pl["invite"] = 50.into();
        pl["kick"] = 50.into();
        pl["ban"] = 50.into();
        pl["users"][b.as_str()] = 50.into();
    })
    .await;
    for (who, cl, want) in [("alice", alice, (true, true, true)), ("bob", bob, (true, true, true)), ("carol", carol, (false, false, false))] {
        md_until(&format!("{label}: {who} permissions"), async || {
            if cl.get_room(&rid).is_none() { return false }
            mw_perms(cl, scope).await == want
        })
        .await;
    }
    println!("OBS {label}: permissions (invite,kick,ban) alice={:?} bob={:?} carol={:?}", mw_perms(alice, scope).await, mw_perms(bob, scope).await, mw_perms(carol, scope).await);

    md_step(&format!("{label}: a user without the power level is refused, nothing changes on the server"));
    for act in [Moderation::Kick, Moderation::KickWith("x".into()), Moderation::Ban, Moderation::BanWith("x".into()), Moderation::Unban(None)] {
        assert_eq!(moderate(carol, &b, act.clone()).await, Err(OperationError::Forbidden), "{label}: carol {act:?} over bob");
        assert_eq!(moderate(carol, &a, act.clone()).await, Err(OperationError::Forbidden), "{label}: carol {act:?} over alice");
    }
    assert_eq!(moderate(carol, "@nobody:matrix-desktop.test", Moderation::Invite).await, Err(OperationError::Forbidden), "{label}: carol invite");
    // The moderator is below the admin: no kick/ban/unban over them either.
    for act in [Moderation::Kick, Moderation::BanWith("x".into()), Moderation::Unban(None)] {
        assert_eq!(moderate(bob, &a, act.clone()).await, Err(OperationError::Forbidden), "{label}: bob {act:?} over alice");
    }
    for who in [&a, &b, &c] {
        assert_eq!(at(who).await.0, "join", "{label}: {who} untouched");
    }

    md_step(&format!("{label}: kick with reason, then invite and rejoin"));
    assert_eq!(moderate(bob, &c, Moderation::KickWith("troppo rumore".into())).await, Ok(()));
    assert_eq!(at(&c).await, ("leave".into(), Some("troppo rumore".into())), "{label}: kick event");
    rejoin(&c).await;

    md_step(&format!("{label}: kick without reason / blank reason leaves no reason"));
    assert_eq!(moderate(bob, &c, Moderation::Kick).await, Ok(()));
    assert_eq!(at(&c).await, ("leave".into(), None), "{label}: plain kick");
    rejoin(&c).await;
    assert_eq!(moderate(bob, &c, Moderation::KickWith("   ".into())).await, Ok(()));
    assert_eq!(at(&c).await, ("leave".into(), None), "{label}: blank-reason kick");
    rejoin(&c).await;

    md_step(&format!("{label}: ban with reason, banned list, banned user cannot join"));
    assert_eq!(moderate(bob, &c, Moderation::BanWith("spam".into())).await, Ok(()));
    assert_eq!(at(&c).await, ("ban".into(), Some("spam".into())), "{label}: ban event");
    for who in [alice, bob] {
        md_until(&format!("{label}: banned list shows carol"), async || mw_banned(who, scope).await == vec![(c.clone(), Some("spam".to_owned()))]).await;
    }
    assert!(carol.join_room_by_id(&rid).await.is_err(), "{label}: banned user cannot join");
    assert_eq!(moderate(bob, &c, Moderation::Invite).await.is_ok(), false, "{label}: a banned user cannot be invited");
    assert_eq!(at(&c).await.0, "ban", "{label}: still banned after the refused invite");

    md_step(&format!("{label}: unban refused when the ban level is raised above the moderator"));
    put_pl(&|pl| pl["ban"] = 100.into()).await;
    md_until(&format!("{label}: bob lost the ban right"), async || mw_perms(bob, scope).await == (true, true, false)).await;
    assert_eq!(moderate(bob, &c, Moderation::Unban(None)).await, Err(OperationError::Forbidden), "{label}: unban without ban right");
    assert_eq!(moderate(bob, &c, Moderation::BanWith("again".into())).await, Err(OperationError::Forbidden), "{label}: ban without ban right");
    assert_eq!(at(&c).await, ("ban".into(), Some("spam".into())), "{label}: ban untouched");
    put_pl(&|pl| pl["ban"] = 50.into()).await;
    md_until(&format!("{label}: bob has the ban right again"), async || mw_perms(bob, scope).await == (true, true, true)).await;

    md_step(&format!("{label}: unban with reason, list empties"));
    assert_eq!(moderate(bob, &c, Moderation::Unban(Some("perdonato".into()))).await, Ok(()));
    assert_eq!(at(&c).await, ("leave".into(), Some("perdonato".into())), "{label}: unban event");
    md_until(&format!("{label}: banned list empty"), async || mw_banned(alice, scope).await.is_empty()).await;

    md_step(&format!("{label}: ban/unban without reason and with blank reason"));
    assert_eq!(moderate(bob, &c, Moderation::Ban).await, Ok(()));
    assert_eq!(at(&c).await, ("ban".into(), None), "{label}: plain ban");
    md_until(&format!("{label}: banned list, no reason"), async || mw_banned(bob, scope).await == vec![(c.clone(), None)]).await;
    assert_eq!(moderate(bob, &c, Moderation::Unban(None)).await, Ok(()));
    assert_eq!(at(&c).await, ("leave".into(), None), "{label}: plain unban");
    assert_eq!(moderate(bob, &c, Moderation::BanWith("  ".into())).await, Ok(()));
    assert_eq!(at(&c).await, ("ban".into(), None), "{label}: blank-reason ban");
    assert_eq!(moderate(bob, &c, Moderation::Unban(Some(" ".into()))).await, Ok(()));
    assert_eq!(at(&c).await, ("leave".into(), None), "{label}: blank-reason unban");
    md_until(&format!("{label}: banned list empty again"), async || mw_banned(alice, scope).await.is_empty()).await;
    rejoin(&c).await;

    md_step(&format!("{label}: admin over the moderator: ban with reason, unban, invite, rejoin"));
    assert_eq!(moderate(alice, &b, Moderation::BanWith("prova admin".into())).await, Ok(()));
    assert_eq!(at(&b).await, ("ban".into(), Some("prova admin".into())), "{label}: admin ban event");
    assert_eq!(moderate(alice, &b, Moderation::Unban(None)).await, Ok(()));
    assert_eq!(moderate(alice, &b, Moderation::Invite).await, Ok(()));
    bob.join_room_by_id(&rid).await.unwrap_or_else(|e| panic!("{label}: bob rejoins: {e}"));
    md_until(&format!("{label}: bob moderates again"), async || bob.get_room(&rid).is_some_and(|r| r.state() == matrix_sdk::RoomState::Joined) && mw_perms(bob, scope).await == (true, true, true)).await;

    md_step(&format!("{label}: observations that may differ between servers"));
    let ghost = "@ghost-nobody:matrix-desktop.test";
    let r = moderate(bob, ghost, Moderation::BanWith("fantasma".into())).await;
    println!("OBS {label}: ban of a user that never existed -> {r:?}; state {:?}", mw_member(alice, &rid, ghost).await);
    if r.is_ok() {
        let _ = moderate(bob, ghost, Moderation::Unban(None)).await;
    }
    let r = moderate(bob, &c, Moderation::Invite).await;
    println!("OBS {label}: invite of a user already joined -> {r:?}");
    let r = moderate(bob, &c, Moderation::Unban(None)).await;
    println!("OBS {label}: unban of a user who is not banned -> {r:?}");
}

#[test]
#[ignore = "requires the member_detail_probe script"]
fn live_member_writes_on_a_real_homeserver() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    runtime().block_on(async {
        let (alice, bob, carol) = (md_login(&config, "alice").await, md_login(&config, "bob").await, md_login(&config, "carol").await);
        let account = AccountId::new(1);
        let mut services = Vec::new();
        for c in [&alice, &bob, &carol] {
            let service = matrix_sdk_ui::sync_service::SyncService::builder(c.clone()).build().await.unwrap();
            service.start().await;
            services.push(service);
        }

        md_step("fixtures: a room and a space, alice creates, bob and carol join");
        let room = alice.create_room(ruma::api::client::room::create_room::v3::Request::new()).await.unwrap();
        let space = crate::operations::create_space(&alice, account, "Spazio prova membri", None, false, None).await.unwrap();
        let space_room = alice.get_room(&ruma::RoomId::parse(space.scope.room_id.as_str()).unwrap()).unwrap();
        let mut scopes = Vec::new();
        for (label, r) in [("stanza", room.clone()), ("spazio", space_room)] {
            for who in [&bob, &carol] {
                r.invite_user_by_id(who.user_id().unwrap()).await.unwrap();
                who.join_room_by_id(r.room_id()).await.unwrap();
            }
            for c in [&alice, &bob, &carol] {
                let rid = r.room_id().to_owned();
                md_until(&format!("{label}: three members known"), async || match c.get_room(&rid) {
                    Some(x) => x.members(matrix_sdk::RoomMemberships::JOIN).await.map(|m| m.len() == 3).unwrap_or(false) && x.power_levels().await.is_ok(),
                    None => false,
                })
                .await;
            }
            scopes.push((label, RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(r.room_id().as_str()) }));
        }
        for (label, scope) in &scopes {
            mw_scenario(label, &alice, &bob, &carol, scope).await;
        }

        md_step("cleanup");
        for s in &services {
            s.stop().await;
        }
        for c in [alice, bob, carol] {
            let _ = c.matrix_auth().logout().await;
        }
    });
}
