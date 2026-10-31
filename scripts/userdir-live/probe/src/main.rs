//! Probe usa e getta per documentare `POST /_matrix/client/v3/user_directory/search`
//! tramite matrix-sdk 0.19.1 (`Client::search_users`) su Synapse e Tuwunel di test.
//! Non fa parte dell'applicazione: serve ai test live.
//!
//! Comandi:
//!   userdir-probe seed   <homeserver_url> <server_name>
//!   userdir-probe search <homeserver_url> <server_name> <localpart> [altro_server_name]
//!
//! seed: registra alice/bob/carol (display name + avatar mxc), alice crea una stanza
//! privata invitando bob che entra; carol non entra in nessuna stanza.
//! search: login come <localpart> e cerca la matrice fissa di termini, stampando
//! una riga JSON per ricerca (risultati, flag limited, errori).

use matrix_sdk::ruma::api::client::room::{Visibility, create_room::v3::{Request as CreateRoomRequest, RoomPreset}};
use matrix_sdk::ruma::api::client::account::register;
use matrix_sdk::ruma::api::client::uiaa::{AuthData, Dummy};
use matrix_sdk::ruma::{OwnedMxcUri, OwnedUserId, UserId};
use matrix_sdk::Client;
use serde_json::{json, Value};

const PASSWORD: &str = "probe-userdir-pass";

fn out(ev: Value) {
    println!("{ev}");
}

fn user(server: &str, localpart: &str) -> OwnedUserId {
    UserId::parse(format!("@{localpart}:{server}")).expect("user id valido")
}

async fn client(homeserver: &str) -> Client {
    Client::new(homeserver.parse().expect("url homeserver valida"))
        .await
        .expect("creazione client")
}

async fn register_and_profile(hs: &str, server: &str, localpart: &str, display: &str) -> Option<Client> {
    let c = client(hs).await;
    let mut req = register::v3::Request::new();
    req.username = Some(localpart.to_owned());
    req.password = Some(PASSWORD.to_owned());
    if let Err(e) = c.matrix_auth().register(req).await {
        // Riprova con m.login.dummy: con registrazione aperta entrambi i server
        // presentano solo il flusso dummy ma lo chiedono comunque via UIA.
        let mut req2 = register::v3::Request::new();
        req2.username = Some(localpart.to_owned());
        req2.password = Some(PASSWORD.to_owned());
        req2.auth = Some(AuthData::Dummy(Dummy::new()));
        if let Err(e2) = c.matrix_auth().register(req2).await {
            out(json!({"ev": "register_error", "server": server, "user": localpart, "error": e2.to_string(), "first_error": e.to_string()}));
            return None;
        }
    }
    out(json!({"ev": "registered", "server": server, "user_id": user(server, localpart).to_string()}));
    if let Err(e) = c.account().set_display_name(Some(display)).await {
        out(json!({"ev": "display_name_error", "user": localpart, "error": e.to_string()}));
    }
    let mxc: OwnedMxcUri = OwnedMxcUri::from(format!("mxc://probe.test/{localpart}").as_str());
    if let Err(e) = c.account().set_avatar_url(Some(&mxc)).await {
        out(json!({"ev": "avatar_error", "user": localpart, "error": e.to_string()}));
    }
    Some(c)
}

async fn seed(hs: &str, server: &str) {
    let alice = register_and_profile(hs, server, "alice", "Alice Rossini").await;
    let bob = register_and_profile(hs, server, "bob", "Bob Verdi").await;
    let (alice, bob) = match (alice, bob) {
        (Some(a), Some(b)) => (a, b),
        _ => return,
    };
    // carol: registrata con profilo ma nessuna stanza (non indicizzata di default).
    register_and_profile(hs, server, "carol", "Carol Blu").await;

    let mut req = CreateRoomRequest::new();
    req.invite = vec![user(server, "bob")];
    req.preset = Some(RoomPreset::PrivateChat);
    req.visibility = Visibility::Private;
    let room = match alice.create_room(req).await {
        Ok(r) => r,
        Err(e) => {
            out(json!({"ev": "create_room_error", "server": server, "error": e.to_string()}));
            return;
        }
    };
    let room_id = room.room_id().to_owned();
    match bob.join_room_by_id(&room_id).await {
        Ok(_) => out(json!({"ev": "room_shared", "server": server, "room_id": room_id.to_string()})),
        Err(e) => out(json!({"ev": "join_error", "server": server, "error": e.to_string()})),
    }
}

/// Attende che la user directory sia popolata (Synapse la indicizza in background):
/// cerca "bo" come alice finché non trova bob, max ~60s.
async fn wait_ready(hs: &str, server: &str) {
    let c = client(hs).await;
    if let Err(e) = c.matrix_auth().login_username("alice", PASSWORD).await {
        out(json!({"ev": "login_error", "server": server, "user": "alice", "error": e.to_string()}));
        return;
    }
    for _ in 0..30 {
        if let Ok(r) = c.search_users("bo", 10).await {
            let n = r.results.iter().filter(|u| u.user_id.localpart() == "bob").count();
            out(json!({"ev": "ready", "server": server, "bob_visible": n > 0}));
            if n > 0 {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn search(hs: &str, server: &str, localpart: &str, other: Option<&str>) {
    let c = client(hs).await;
    if let Err(e) = c.matrix_auth().login_username(localpart, PASSWORD).await {
        out(json!({"ev": "login_error", "server": server, "user": localpart, "error": e.to_string()}));
        return;
    }
    let mut terms = vec![
        "alice".to_owned(),
        "ross".to_owned(),
        "Rossini".to_owned(),
        "verd".to_owned(),
        "bo".to_owned(),
        "carol".to_owned(),
        "zz".to_owned(),
        format!("@bob:{server}"),
    ];
    if let Some(other) = other {
        terms.push(other.to_owned());
        terms.push(format!("@alice:{other}"));
    }
    for term in terms {
        match c.search_users(&term, 10).await {
            Ok(r) => {
                let results: Vec<Value> = r
                    .results
                    .iter()
                    .map(|u| {
                        json!({
                            "user_id": u.user_id.to_string(),
                            "display_name": u.display_name,
                            "avatar_url": u.avatar_url.as_ref().map(|a| a.to_string()),
                        })
                    })
                    .collect();
                out(json!({
                    "ev": "search", "server": server, "as": localpart, "term": term,
                    "limited": r.limited, "results": results,
                }));
            }
            Err(e) => {
                out(json!({"ev": "search_error", "server": server, "as": localpart, "term": term, "error": e.to_string()}));
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match (args.get(1).map(String::as_str), args.get(2..)) {
        (Some("seed"), Some([hs, server])) => seed(hs, server).await,
        (Some("ready"), Some([hs, server])) => wait_ready(hs, server).await,
        (Some("search"), Some([hs, server, local])) => search(hs, server, local, None).await,
        (Some("search"), Some([hs, server, local, other])) => search(hs, server, local, Some(other)).await,
        _ => {
            eprintln!("uso: userdir-probe seed|ready|search <homeserver_url> <server_name> [localpart] [altro_server]");
            std::process::exit(2);
        }
    }
}
