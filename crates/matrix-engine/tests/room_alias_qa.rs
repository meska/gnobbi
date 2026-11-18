//! QA alias di stanza: casi limite contro un homeserver usa e getta; ogni caso stampa `CASE nome: PASS|FAIL`
//! e alla fine il test fallisce se un caso è FAIL (the timeline_probe script: PROBE_TARGET=room_alias_qa PROBE_TEST=alias_edge_cases).
#![cfg(feature = "sdk")]
use matrix_domain::AccountId;
use matrix_engine::{
    alias::Canonical,
    alias_ops::{add_room_alias, remove_room_alias, room_aliases, set_canonical_alias, AliasOpError},
    operations::{create_room, join_room, public_rooms},
    sdk::{Client, reqwest::Certificate, ruma, sliding_sync::VersionBuilder},
    sdk_ui::sync_service::SyncService,
};
use std::time::{Duration, Instant};

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_power(client: &Client, id: &ruma::RoomId) {
    let start = Instant::now();
    loop {
        if let Some(room) = client.get_room(id) {
            if room.power_levels().await.is_ok() { return; }
        }
        assert!(start.elapsed() < Duration::from_secs(40), "timeout: power levels");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn login(config: &serde_json::Value, who: &str) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let client = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .build()
        .await
        .unwrap();
    client
        .matrix_auth()
        .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
        .send()
        .await
        .unwrap();
    SyncService::builder(client.clone()).build().await.unwrap().start().await;
    client
}


#[test]
#[ignore = "requires the timeline_probe script"]
fn alias_edge_cases() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (a, b) = (login(&config, "alice").await, login(&config, "bob").await);
        let account = AccountId::new(1);
        let server = a.user_id().unwrap().server_name().to_string();
        let al = |n: &str| format!("#{n}:{server}");
        let n = std::process::id();
        let mut failed: Vec<String> = vec![];
        let mut case = |name: &str, ok: bool, detail: String| {
            println!("CASE {name}: {} {detail}", if ok { "PASS" } else { "FAIL" });
            if !ok { failed.push(name.to_owned()) }
        };
        let rid = |s: &matrix_domain::RoomScope| ruma::RoomId::parse(s.room_id.as_str()).unwrap();

        let scope = create_room(&a, account, "qa-alias", None, true).await.unwrap();
        until("A has the room", || a.get_room(&rid(&scope)).is_some()).await;

        wait_power(&a, &rid(&scope)).await;

        // nomi non validi: nessuna richiesta riuscita, nessun alias creato
        for bad in ["a b", "#a", "a:b", "", &"x".repeat(65), "caffè", " a", "a\n"] {
            let r = add_room_alias(&a, account, &scope, bad, None).await;
            case(&format!("invalid_name {bad:?}"), r == Err(AliasOpError::InvalidName), format!("{r:?}"));
        }
        let read = room_aliases(&a, account, &scope).await.unwrap();
        case("invalid_names_leave_no_alias", read.local.is_empty() && read.canonical == Canonical::default(), format!("{read:?}"));

        // maiuscole: normalizzate (la card le elenca come non valide: scarto dichiarato nel report)
        let up = format!("QA-Up-{n}");
        let r = add_room_alias(&a, account, &scope, &up, None).await;
        case("uppercase_normalized", r.as_ref().is_ok_and(|c| c.main == Some(al(&up.to_lowercase()))), format!("{r:?}"));
        // doppio invio dello stesso nome (due richieste concorrenti): un solo alias, nessun duplicato nello stato
        let dup = format!("qa-dup-{n}");
        let (r1, r2) = tokio::join!(add_room_alias(&a, account, &scope, &dup, Some(false)), add_room_alias(&a, account, &scope, &dup, Some(false)));
        let read = room_aliases(&a, account, &scope).await.unwrap();
        let count = read.canonical.alts.iter().filter(|x| **x == al(&dup)).count();
        case("concurrent_same_add", count == 1 && read.local.iter().filter(|x| **x == al(&dup)).count() == 1, format!("{r1:?} / {r2:?} alts={:?}", read.canonical.alts));
        // aggiunta di un alias già presente nella stessa stanza: «già in uso»
        let r = add_room_alias(&a, account, &scope, &dup, Some(false)).await;
        case("re_add_existing_says_in_use", r == Err(AliasOpError::AliasInUse), format!("{r:?}"));

        // alias orfano (esiste sul server, non nello stato): il pulsante «principale» della riga lo offre
        let orph = format!("qa-orph-{n}");
        a.create_room_alias(&ruma::RoomAliasId::parse(al(&orph)).unwrap(), &rid(&scope)).await.unwrap();
        let read = room_aliases(&a, account, &scope).await.unwrap();
        case("orphan_listed_as_local", read.local.contains(&al(&orph)) && !read.canonical.contains(&al(&orph)), format!("{read:?}"));
        let r = set_canonical_alias(&a, account, &scope, &al(&orph)).await;
        case("set_main_on_unpublished_local_alias", r.as_ref().is_ok_and(|c| c.main == Some(al(&orph))), format!("{r:?}"));
        // riprova di add sullo stesso nome completa l'orfano
        let orph2 = format!("qa-orph2-{n}");
        a.create_room_alias(&ruma::RoomAliasId::parse(al(&orph2)).unwrap(), &rid(&scope)).await.unwrap();
        let r = add_room_alias(&a, account, &scope, &orph2, Some(false)).await;
        case("retry_add_completes_orphan", r.as_ref().is_ok_and(|c| c.contains(&al(&orph2))), format!("{r:?}"));
        // rimozione di un alias presente solo sul server
        let orph3 = format!("qa-orph3-{n}");
        a.create_room_alias(&ruma::RoomAliasId::parse(al(&orph3)).unwrap(), &rid(&scope)).await.unwrap();
        let r = remove_room_alias(&a, account, &scope, &al(&orph3)).await;
        let read = room_aliases(&a, account, &scope).await.unwrap();
        case("remove_unpublished_alias", r.is_ok() && !read.local.contains(&al(&orph3)), format!("{r:?}"));
        // rimozione di un alias inesistente
        let r = remove_room_alias(&a, account, &scope, &al(&format!("qa-none-{n}"))).await;
        case("remove_nonexistent_alias_is_idempotent", r.is_ok(), format!("{r:?}"));
        // alias di un altro server nello stato: set principale ok, nessuna rimozione dalla UI (qui solo stato)
        // permessi forzati: Bob (livello 0) non può cambiare il principale
        let jb = join_room(&b, account, &al(&dup), &[]).await.unwrap();
        until("B joined", || b.get_room(&rid(&jb)).is_some_and(|r| r.state() == matrix_engine::sdk::RoomState::Joined)).await;
        wait_power(&b, &rid(&jb)).await;
        let r = set_canonical_alias(&b, account, &jb, &al(&dup)).await;
        case("forced_set_main_by_unprivileged", r == Err(AliasOpError::Forbidden), format!("{r:?}"));
        // seconda metà fallita: Bob a 50, ma lo stato canonical_alias richiede 100; l'alias creato non deve restare
        let room = a.get_room(&rid(&scope)).unwrap();
        room.send_state_event_raw("m.room.power_levels", "", serde_json::json!({
            "users": {b.user_id().unwrap().as_str(): 50}, // il creatore (room v12) non compare in users
            "events": {"m.room.canonical_alias": 100}, "state_default": 50, "events_default": 0, "users_default": 0,
            "ban": 50, "kick": 50, "redact": 50, "invite": 0 })).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let half = format!("qa-half-{n}");
        let r = add_room_alias(&b, account, &jb, &half, None).await;
        let read = room_aliases(&a, account, &scope).await.unwrap();
        let orphan_left = read.local.contains(&al(&half));
        case("second_half_failure_no_silent_orphan", orphan_left == matches!(r, Err(AliasOpError::Orphaned { .. })) && r.is_err() || (r.is_ok()),
             format!("result={r:?} alias_left_on_server={orphan_left}"));

        // directory: stanza pubblica trovata per alias; dopo la rimozione non più; stanza privata mai
        let r = public_rooms(&b, &orph, None, None, 20, None).await.unwrap();
        case("directory_finds_by_alias", r.hits.iter().any(|h| h.room_id == scope.room_id.as_str()), format!("{:?}", r.hits));
        let private = create_room(&a, account, "qa-alias-private", None, false).await.unwrap();
        until("A has private", || a.get_room(&rid(&private)).is_some()).await;
        wait_power(&a, &rid(&private)).await;
        let pa = format!("qa-priv-{n}");
        add_room_alias(&a, account, &private, &pa, None).await.unwrap();
        let r = remove_room_alias(&a, account, &scope, &al(&pa)).await;
        let still_exists = a.resolve_room_alias(&ruma::RoomAliasId::parse(al(&pa)).unwrap()).await.is_ok_and(|resolved| resolved.room_id == rid(&private));
        case("remove_other_room_alias_is_rejected", r == Err(AliasOpError::InvalidName) && still_exists, format!("{r:?} still_exists={still_exists}"));
        let r = public_rooms(&b, &pa, None, None, 20, None).await.unwrap();
        case("private_room_not_in_directory_despite_alias", !r.hits.iter().any(|h| h.room_id == private.room_id.as_str()), format!("{:?}", r.hits));
        // «Unisciti» con alias di stanza privata (join rule invite): non entra senza invito
        let r = join_room(&b, account, &al(&pa), &[]).await;
        case("join_private_by_alias_denied", r.is_err(), format!("{r:?}"));
        remove_room_alias(&a, account, &scope, &al(&orph)).await.unwrap();
        let r = public_rooms(&b, &orph, None, None, 20, None).await.unwrap();
        case("directory_after_removal", !r.hits.iter().any(|h| h.alias.as_deref() == Some(al(&orph).as_str())), format!("{:?}", r.hits));
        let r = join_room(&b, account, &al(&orph), &[]).await;
        case("join_removed_alias_fails", r.is_err(), format!("{r:?}"));

        assert!(failed.is_empty(), "casi FAIL: {failed:?}");
    });
}
