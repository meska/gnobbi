//! «Alias stanza»: add / set principale / remove / già in uso / permessi, contro un homeserver usa e getta
//! (the timeline_probe script: PROBE_TARGET=room_alias PROBE_TEST=alias_round_trip).
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
fn alias_round_trip() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (a, b) = (login(&config, "alice").await, login(&config, "bob").await);
        let account = AccountId::new(1);
        let server = a.user_id().unwrap().server_name().to_string();
        let al = |n: &str| format!("#{n}:{server}");
        let n = std::process::id();
        let (first, second, third) = (format!("gnobbi-al1-{n}"), format!("gnobbi-al2-{n}"), format!("gnobbi-al3-{n}"));

        let scope = create_room(&a, account, "gnobbi-alias", None, true).await.unwrap();
        let id = ruma::RoomId::parse(scope.room_id.as_str()).unwrap();
        until("A has the room", || a.get_room(&id).is_some()).await;
        wait_power(&a, &id).await;
        let main_of = |c: &Canonical| c.main.clone();

        // 1. primo alias = principale; secondo = alternativo; stato letto dal server
        let s = add_room_alias(&a, account, &scope, &first, None).await.unwrap();
        assert_eq!(main_of(&s), Some(al(&first)));
        let s = add_room_alias(&a, account, &scope, &second, None).await.unwrap();
        assert_eq!((main_of(&s), s.alts.clone()), (Some(al(&first)), vec![al(&second)]));
        let read = room_aliases(&a, account, &scope).await.unwrap();
        assert!(read.can_edit_aliases);
        assert!(read.local.contains(&al(&first)) && read.local.contains(&al(&second)), "{read:?}");

        // 2. cambio principale
        let s = set_canonical_alias(&a, account, &scope, &al(&second)).await.unwrap();
        assert_eq!((main_of(&s), s.alts.clone()), (Some(al(&second)), vec![al(&first)]));
        until("client sees the new main", || a.get_room(&id).unwrap().canonical_alias().is_some_and(|x| x.as_str() == al(&second))).await;

        // 2b. la stanza pubblica compare nella directory con il nuovo principale, trovata anche per alias
        let page = public_rooms(&b, &second, None, None, 20, None).await.unwrap();
        assert!(page.hits.iter().any(|h| h.room_id == scope.room_id.as_str() && h.alias.as_deref() == Some(al(&second).as_str())), "{:?}", page.hits);

        // 3. già in uso, da un'altra stanza: AliasInUse, nessun cambiamento di stato
        let other = create_room(&a, account, "gnobbi-alias-other", None, true).await.unwrap();
        until("A has the other room", || a.get_room(&ruma::RoomId::parse(other.room_id.as_str()).unwrap()).is_some()).await;
        wait_power(&a, &ruma::RoomId::parse(other.room_id.as_str()).unwrap()).await;
        assert_eq!(add_room_alias(&a, account, &other, &first, None).await, Err(AliasOpError::AliasInUse));
        // nome non valido: respinto prima della rete
        assert_eq!(add_room_alias(&a, account, &scope, "a b", None).await, Err(AliasOpError::InvalidName));

        // 4. permessi: Bob (livello 0) non può né aggiungere né togliere
        let joined = join_room(&b, account, &al(&second), &[]).await.unwrap();
        let bid = ruma::RoomId::parse(joined.room_id.as_str()).unwrap();
        until("B joined", || b.get_room(&bid).is_some_and(|r| r.state() == matrix_engine::sdk::RoomState::Joined)).await;
        wait_power(&b, &bid).await;
        assert!(!room_aliases(&b, account, &joined).await.unwrap().can_edit_aliases);
        assert_eq!(add_room_alias(&b, account, &joined, &third, None).await, Err(AliasOpError::Forbidden));
        assert_eq!(remove_room_alias(&b, account, &joined, &al(&first)).await, Err(AliasOpError::Forbidden));
        assert!(room_aliases(&a, account, &scope).await.unwrap().local.contains(&al(&first)), "alias di Alice intatto");
        // l'alias del tentativo respinto non è rimasto orfano
        assert!(!room_aliases(&a, account, &scope).await.unwrap().local.contains(&al(&third)));

        // 5. rimozione del principale: subentra l'alternativo; poi dell'ultimo: nessun principale
        let s = remove_room_alias(&a, account, &scope, &al(&second)).await.unwrap();
        assert_eq!((main_of(&s), s.alts.clone()), (Some(al(&first)), vec![]));
        let s = remove_room_alias(&a, account, &scope, &al(&first)).await.unwrap();
        assert_eq!(s, Canonical::default());
        let read = room_aliases(&a, account, &scope).await.unwrap();
        assert!(read.local.is_empty(), "{read:?}");
        assert!(a.resolve_room_alias(&ruma::RoomAliasId::parse(al(&first)).unwrap()).await.is_err());
    });
}
