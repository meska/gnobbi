//! «Volta Softair Team» on a disposable homeserver: a space and its child room with the SAME name,
//! 11 m.space.child events (add/remove repeated), m.space.parent on the child, an empty space and a
//! space invite. The real sliding-sync adapter feeds the real Sidebar. Driven by
//! the space_projection_qa script (FIXTURE_AUTH_CONFIG, test accounts only). Prints `QA <name> PASS|FAIL`.
#![recursion_limit = "512"]
use matrix_desktop::Sidebar;
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListEvent, bounded,
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{Request as CreateRoom, RoomPreset},
            assign,
            events::room::message::RoomMessageEventContent,
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sliding,
};
use std::{num::NonZeroUsize, time::Duration};

const HS: &str = "matrix-desktop.test";
const ICON: &str = "mxc://matrix-desktop.test/volta-icon";

async fn login(config: &serde_json::Value, who: &str) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .sliding_sync_version_builder(VersionBuilder::None)
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(
            config[format!("{who}_mxid")].as_str().unwrap(),
            config[format!("{who}_password")].as_str().unwrap(),
        )
        .send()
        .await
        .unwrap();
    client
}

fn req(name: &str, space: bool, invite: &Client) -> CreateRoom {
    assign!(CreateRoom::new(), {
        preset: Some(RoomPreset::PrivateChat),
        name: Some(name.to_owned()),
        creation_content: space.then(|| Raw::new(&serde_json::json!({"type": "m.space"})).unwrap().cast_unchecked()),
        invite: vec![invite.user_id().unwrap().to_owned()],
    })
}

fn report(failed: &mut Vec<String>, name: &str, ok: bool, detail: String) {
    eprintln!("QA {name} {} {detail}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        failed.push(name.to_owned());
    }
}

fn shown(s: &Sidebar) -> Vec<(String, bool)> {
    s.visible_indices().iter().map(|&i| (s.rooms()[i].name.clone(), s.rooms()[i].info.is_space)).collect()
}

#[test]
#[ignore = "requires the space_projection_qa script"]
fn volta_softair_team_space_and_same_name_room_project_correctly() {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut failed = Vec::new();
    rt.block_on(async {
        let (alice, bob) = (login(&config, "alice").await, login(&config, "bob").await);
        let name = "Volta Softair Team";
        let space = bob.create_room(req(name, true, &alice)).await.unwrap();
        space.send_state_event_raw("m.room.avatar", "", serde_json::json!({"url": ICON})).await.unwrap();
        let room = bob.create_room(req(name, false, &alice)).await.unwrap();
        let (ids, idr) = (space.room_id().to_string(), room.room_id().to_string());
        room.send_state_event_raw("m.space.parent", &ids, serde_json::json!({"via": [HS], "canonical": true})).await.unwrap();
        // 11 events on the same state key: add, remove, add... ends added. The space gets no message.
        for i in 0..11 {
            let content = if i % 2 == 0 { serde_json::json!({"via": [HS]}) } else { serde_json::json!({}) };
            space.send_state_event_raw("m.space.child", &idr, content).await.unwrap();
        }
        room.send(RoomMessageEventContent::text_plain("ciao")).await.unwrap();
        room.send(RoomMessageEventContent::text_plain("secondo")).await.unwrap();
        let empty = bob.create_room(req("Vuoto", true, &alice)).await.unwrap();
        let invited = bob.create_room(req("Invito", true, &alice)).await.unwrap();
        for r in [&space, &room, &empty] {
            alice.join_room_by_id(r.room_id()).await.unwrap();
        }
        let (idv, idi) = (empty.room_id().to_string(), invited.room_id().to_string());

        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(AccountId::new(9), alice.clone());
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let mut sidebar = Sidebar::connected(AccountId::new(9));
        let mut invite_seen = false;
        let ready = |s: &Sidebar, seen: bool| {
            seen && s.rooms().len() == 3
                && s.rooms().iter().filter(|r| r.name == name).count() == 2 // names arrive after the first diff
                && s.spaces().any(|r| r.scope.room_id.as_str() == ids && r.info.children.iter().any(|c| c.as_str() == idr))
        };
        let waiting = async {
            while let Some(ev) = rx.recv().await {
                if let RoomListEvent::Diff(d) = ev {
                    if let matrix_engine::RoomListDiff::Upsert(r) = &d {
                        invite_seen |= r.scope.room_id.as_str() == idi && r.info.invited;
                    }
                    sidebar.apply_diff(d);
                    if ready(&sidebar, invite_seen) {
                        return;
                    }
                }
            }
        };
        tokio::select! {
            _ = &mut run => eprintln!("adapter ended early"),
            _ = tokio::time::timeout(Duration::from_secs(60), waiting) => {}
        }
        // wait() may have ended on timeout: report it, then assert on whatever was projected.
        report(&mut failed, "projection_complete", ready(&sidebar, invite_seen), format!("rooms={} invite_seen={invite_seen}", sidebar.rooms().len()));

        let only_room = |s: &Sidebar| shown(s) == [(name.to_owned(), false)];
        sidebar.set_query("");
        report(&mut failed, "tutte_only_room", only_room(&sidebar), format!("{:?}", shown(&sidebar)));
        for q in ["Volta", "softair", "volta softair team"] {
            sidebar.set_query(q);
            report(&mut failed, &format!("search_{q}_only_room"), only_room(&sidebar), format!("{:?}", shown(&sidebar)));
        }
        sidebar.set_query("Vuoto");
        report(&mut failed, "search_empty_space_name_shows_nothing", shown(&sidebar).is_empty(), format!("{:?}", shown(&sidebar)));
        sidebar.set_query("");
        sidebar.set_unread_only(true);
        report(&mut failed, "unread_filter_no_space", shown(&sidebar).iter().all(|(_, sp)| !sp), format!("{:?}", shown(&sidebar))); // shortcut: the room is read for alice (joined after the messages), so the list may be empty
        sidebar.set_unread_only(false);
        sidebar.set_show_empty(true);
        report(&mut failed, "show_empty_no_space", only_room(&sidebar), format!("{:?}", shown(&sidebar)));
        sidebar.set_show_empty(false);

        let rail: Vec<_> = sidebar.navigation_entries().into_iter().map(|e| (e.name, e.avatar)).collect();
        let has = |n: &str| rail.iter().filter(|(x, _)| x == n).count();
        report(&mut failed, "rail_has_space_with_icon", rail.contains(&(name.to_owned(), Some(ICON.to_owned()))) && has(name) == 1, format!("{rail:?}"));
        report(&mut failed, "rail_has_empty_space", has("Vuoto") == 1 && has("Tutte") == 1, format!("{rail:?}"));
        report(&mut failed, "invite_not_in_rail_or_list", has("Invito") == 0 && !sidebar.rooms().iter().any(|r| r.scope.room_id.as_str() == idi), format!("{rail:?}"));
        report(&mut failed, "no_orphan_space_rows", sidebar.rooms().iter().filter(|r| r.info.is_space).count() == 2, format!("{}", sidebar.rooms().len()));
        report(&mut failed, "room_not_a_space", sidebar.rooms().iter().any(|r| r.scope.room_id.as_str() == idr && !r.info.is_space), String::new());

        let scope = |id: &str| sidebar.spaces().find(|r| r.scope.room_id.as_str() == id).map(|r| r.scope.clone());
        let (sv, se) = (scope(&ids), scope(&idv));
        report(&mut failed, "select_space_lists_child", sidebar.set_space(sv) && only_room(&sidebar), format!("{:?}", shown(&sidebar)));
        report(&mut failed, "select_empty_space_lists_nothing", sidebar.set_space(se) && shown(&sidebar).is_empty(), format!("{:?}", shown(&sidebar)));
        report(&mut failed, "empty_space_still_in_rail", sidebar.navigation_entries().iter().any(|e| e.name == "Vuoto" && e.selected), String::new());
        sidebar.set_space(None);
        report(&mut failed, "back_to_tutte_only_room", only_room(&sidebar), format!("{:?}", shown(&sidebar)));
    });
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}
