// Member roles on a real Synapse/Tuwunel (driver: the member_detail_probe script with PROBE_FN=auth::auth_tests::live_member_roles_on_a_real_homeserver).
// alice creates the room; bob and carol join. Role changes go through the engine (`set_member_role`); every outcome is read from the server's
// m.room.power_levels and from the engine (`load_member_rights`). Included by auth_tests; reuses the mw_* helpers of member_writes_tests.rs.
use matrix_domain::role::MemberRole;

/// Power level of `who` as the server holds it (users map, else users_default).
async fn mr_pl(c: &matrix_sdk::Client, rid: &ruma::RoomId, who: &str) -> i64 {
    let pl = mw_pl(c, rid).await;
    pl["users"][who].as_i64().unwrap_or_else(|| pl["users_default"].as_i64().unwrap_or(0))
}

/// `room_version` of the m.room.create event as the server holds it.
async fn mr_version(c: &matrix_sdk::Client, rid: &ruma::RoomId) -> String {
    let req = get_state_event_for_key::v3::Request::new(rid.to_owned(), StateEventType::RoomCreate, String::new());
    let v: serde_json::Value = serde_json::from_str(c.send(req).await.expect("create event").event_or_content.get()).unwrap();
    v["room_version"].as_str().unwrap_or("1").to_owned()
}

#[test]
#[ignore = "requires the member_detail_probe script"]
fn live_member_roles_on_a_real_homeserver() {
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
        md_step("fixtures: room, alice creates, bob and carol join");
        let room = alice.create_room(ruma::api::client::room::create_room::v3::Request::new()).await.unwrap();
        let rid = room.room_id().to_owned();
        for who in [&bob, &carol] {
            room.invite_user_by_id(who.user_id().unwrap()).await.unwrap();
            who.join_room_by_id(&rid).await.unwrap();
        }
        for c in [&alice, &bob, &carol] {
            md_until("three members known", async || match c.get_room(&rid) {
                Some(x) => x.members(matrix_sdk::RoomMemberships::JOIN).await.map(|m| m.len() == 3).unwrap_or(false) && x.power_levels().await.is_ok(),
                None => false,
            })
            .await;
        }
        let scope = RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(rid.as_str()) };
        let (a, b, c) = (alice.user_id().unwrap().to_string(), bob.user_id().unwrap().to_string(), carol.user_id().unwrap().to_string());
        let rights = async |who: &matrix_sdk::Client, target: &str| {
            crate::operations::load_member_rights(who, &crate::room_sync::Timelines::default(), account, &scope, target).await.expect("rights")
        };
        let set = async |who: &matrix_sdk::Client, target: &str, role| crate::operations::set_member_role(who, account, &scope, target, role).await;
        let bob_role = async |viewer: &matrix_sdk::Client| rights(viewer, &b).await.role;
        let version = mr_version(&alice, &rid).await;
        println!("OBS room version {version}");

        md_step("a plain member cannot change roles; nothing changes on the server");
        assert_eq!(set(&carol, &b, MemberRole::Moderator).await, Err(OperationError::Forbidden));
        assert_eq!(set(&carol, &c, MemberRole::Admin).await, Err(OperationError::Forbidden));
        assert_eq!(set(&bob, &a, MemberRole::Member).await, Err(OperationError::Forbidden));
        assert!(!rights(&carol, &b).await.can_change_role);
        assert_eq!(mr_pl(&alice, &rid, &b).await, 0);
        assert_eq!(mr_pl(&alice, &rid, &c).await, 0);

        md_step("Member -> Moderator");
        assert!(rights(&alice, &b).await.can_change_role);
        assert_eq!(set(&alice, &b, MemberRole::Moderator).await, Ok(()));
        assert_eq!(mr_pl(&alice, &rid, &b).await, 50, "server power level");
        md_until("engine reads moderator", async || bob_role(&alice).await == MemberRole::Moderator && bob_role(&bob).await == MemberRole::Moderator).await;

        md_step("a moderator cannot change roles either (m.room.power_levels needs 100)");
        assert_eq!(set(&bob, &c, MemberRole::Moderator).await, Err(OperationError::Forbidden));
        assert_eq!(mr_pl(&alice, &rid, &c).await, 0);
        assert!(!rights(&bob, &c).await.can_change_role);

        md_step("Moderator -> Admin");
        assert_eq!(set(&alice, &b, MemberRole::Admin).await, Ok(()));
        assert_eq!(mr_pl(&alice, &rid, &b).await, 100, "server power level");
        md_until("engine reads admin", async || bob_role(&alice).await == MemberRole::Admin && bob_role(&bob).await == MemberRole::Admin).await;

        md_step("the creator is read-only, even for another admin");
        let r = rights(&bob, &a).await;
        println!("OBS creator seen by an admin: {r:?}");
        assert_eq!((r.role, r.can_change_role), (MemberRole::Admin, false));
        if version == "12" {
            assert!(r.creator, "v12 creator");
        }
        assert_eq!(set(&bob, &a, MemberRole::Member).await, Err(OperationError::Forbidden));
        assert_eq!(set(&bob, &a, MemberRole::Moderator).await, Err(OperationError::Forbidden));
        // v12: the creator has no entry (infinite level); before: 100.
        assert_eq!(mr_pl(&bob, &rid, &a).await, if version == "12" { 0 } else { 100 }, "creator level unchanged on the server");
        let r = rights(&alice, &a).await;
        assert_eq!((r.role, r.can_change_role), (MemberRole::Admin, false), "alice over herself");

        md_step("Admin -> Member");
        assert_eq!(set(&alice, &b, MemberRole::Member).await, Ok(()));
        assert_eq!(mr_pl(&alice, &rid, &b).await, 0, "server power level");
        md_until("engine reads member", async || bob_role(&alice).await == MemberRole::Member && bob_role(&bob).await == MemberRole::Member).await;
        assert_eq!(set(&bob, &c, MemberRole::Moderator).await, Err(OperationError::Forbidden), "demoted bob is refused again");

        md_step("cleanup");
        for s in &services {
            s.stop().await;
        }
        for c in [alice, bob, carol] {
            let _ = c.matrix_auth().logout().await;
        }
    });
}
