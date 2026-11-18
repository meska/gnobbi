//! Alias di una stanza esistente: aggiunta, rimozione, cambio del principale, lettura.
//! Solo API SDK 0.19.1. Ordine: alias sul server, poi stato `m.room.canonical_alias`;
//! in rimozione l'inverso. Mai un alias creato e dimenticato: vedi `AliasOpError::Orphaned`.
pub use crate::alias::{AliasAction, AliasOpError, RoomAliases};
use crate::alias::{compose_alias, normalize_alias_local, Canonical};
use matrix_sdk::ruma::{
    api::{client::room::aliases, error::ErrorKind},
    events::StateEventType, OwnedRoomAliasId, RoomAliasId,
};


/// Stessa distinzione di rete di `directory_error`; il resto è tipizzato qui.
pub fn alias_error(status: Option<u16>, kind: Option<&ErrorKind>) -> AliasOpError {
    use crate::search::SearchError;
    match kind {
        // Synapse answers 409 M_UNKNOWN "Room alias already exists" (spec: 409, no fixed errcode).
        Some(ErrorKind::RoomInUse) => AliasOpError::AliasInUse,
        _ if status == Some(409) => AliasOpError::AliasInUse,
        Some(ErrorKind::Forbidden) => AliasOpError::Forbidden,
        Some(ErrorKind::InvalidParam) => AliasOpError::InvalidName,
        _ => match crate::operations::directory_error(status, kind, false) {
            SearchError::Network => AliasOpError::Network,
            _ => AliasOpError::ServerUnreachable,
        },
    }
}

fn from_sdk(e: &matrix_sdk::Error) -> AliasOpError {
    alias_error(e.as_client_api_error().map(|a| a.status_code.as_u16()), e.client_api_error_kind())
}

fn from_http(e: &matrix_sdk::HttpError) -> AliasOpError {
    alias_error(e.as_client_api_error().map(|a| a.status_code.as_u16()), e.client_api_error_kind())
}


fn joined_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
) -> Result<matrix_sdk::Room, AliasOpError> {
    if scope.account_id != account {
        return Err(AliasOpError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| AliasOpError::WrongScope)?;
    client.get_room(&id).filter(|r| r.state() == matrix_sdk::RoomState::Joined).ok_or(AliasOpError::Unavailable)
}

async fn editable_room(client: &matrix_sdk::Client, account: matrix_domain::AccountId, scope: &matrix_domain::RoomScope) -> Result<matrix_sdk::Room, AliasOpError> {
    let room = joined_room(client, account, scope)?;
    let me = client.user_id().ok_or(AliasOpError::Unavailable)?;
    let levels = room.power_levels().await.map_err(|_| AliasOpError::Unavailable)?;
    if !levels.user_can_send_state(me, StateEventType::RoomCanonicalAlias) {
        return Err(AliasOpError::Forbidden);
    }
    Ok(room)
}

/// Stato letto dal server, non dalla cache sincronizzata: dopo una modifica appena fatta la
/// cache è ancora vecchia e la modifica successiva la cancellerebbe. 404 = nessun evento.
async fn current(client: &matrix_sdk::Client, room: &matrix_sdk::Room) -> Result<Canonical, AliasOpError> {
    use matrix_sdk::ruma::api::client::state::get_state_event_for_key::v3::Request;
    let request = Request::new(room.room_id().to_owned(), StateEventType::RoomCanonicalAlias, String::new());
    let sent = tokio::time::timeout(std::time::Duration::from_secs(20), client.send(request)).await.map_err(|_| AliasOpError::Network)?;
    let response = match sent {
        Ok(r) => r,
        Err(e) if e.as_client_api_error().is_some_and(|a| a.status_code.as_u16() == 404) => return Ok(Canonical::default()),
        Err(e) => return Err(from_http(&e)),
    };
    let json: serde_json::Value = serde_json::from_str(response.event_or_content.get()).map_err(|_| AliasOpError::ServerUnreachable)?;
    let text = |v: &serde_json::Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    Ok(Canonical {
        main: text(&json["alias"]),
        alts: json["alt_aliases"].as_array().map(|a| a.iter().filter_map(text).collect()).unwrap_or_default(),
    })
}

fn parse(alias: &str) -> Result<OwnedRoomAliasId, AliasOpError> {
    RoomAliasId::parse(alias).map_err(|_| AliasOpError::InvalidName)
}

async fn timed<T>(f: impl std::future::Future<Output = Result<T, AliasOpError>>) -> Result<T, AliasOpError> {
    tokio::time::timeout(std::time::Duration::from_secs(20), f).await.map_err(|_| AliasOpError::Network)?
}

async fn publish(room: &matrix_sdk::Room, state: &Canonical) -> Result<(), AliasOpError> {
    let main = state.main.as_deref().map(parse).transpose()?;
    let alts = state.alts.iter().map(|a| parse(a)).collect::<Result<Vec<_>, _>>()?;
    timed(async { room.privacy_settings().update_canonical_alias(main, alts).await.map_err(|e| from_sdk(&e)) }).await
}

pub async fn room_aliases(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
) -> Result<RoomAliases, AliasOpError> {
    let room = joined_room(client, account, scope)?;
    let can_edit_aliases = match (room.power_levels().await, client.user_id()) {
        (Ok(levels), Some(me)) => levels.user_can_send_state(me, StateEventType::RoomCanonicalAlias),
        _ => false,
    };
    let local = timed(async {
        let request = aliases::v3::Request::new(room.room_id().to_owned());
        Ok(client.send(request).await.map_err(|e| from_http(&e))?.aliases.iter().map(ToString::to_string).collect())
    })
    .await?;
    let server = client.user_id().map(|u| u.server_name().to_string()).unwrap_or_default();
    Ok(RoomAliases { canonical: current(client, &room).await?, local, can_edit_aliases, server })
}

/// Crea `#local_part:server` e lo pubblica nello stato. Riprovabile dopo `Orphaned`.
pub async fn add_room_alias(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    local_part: &str,
    make_canonical: Option<bool>,
) -> Result<Canonical, AliasOpError> {
    let room = editable_room(client, account, scope).await?;
    let server = client.user_id().ok_or(AliasOpError::Unavailable)?.server_name().to_string();
    let alias = compose_alias(local_part, &server).map_err(|_| AliasOpError::InvalidName)?;
    let id = parse(&alias)?;
    let state = current(client, &room).await?;
    let created = timed(async { client.create_room_alias(&id, room.room_id()).await.map_err(|e| from_http(&e)) }).await;
    match created {
        Ok(()) => {}
        // Riprova dopo un orfano: l'alias punta già qui ma lo stato non lo contiene.
        Err(AliasOpError::AliasInUse) if !state.contains(&alias) && client.resolve_room_alias(&id).await.is_ok_and(|r| r.room_id == room.room_id()) => {}
        Err(e) => return Err(e),
    }
    let next = state.with_added(&alias, make_canonical);
    if let Err(e) = publish(&room, &next).await {
        // Annulla l'alias appena creato; se non ci riesce lo dichiara.
        return Err(match timed(async { client.remove_room_alias(&id).await.map_err(|e| from_http(&e)) }).await {
            Ok(()) => e,
            Err(_) => AliasOpError::Orphaned { alias },
        });
    }
    Ok(next)
}

/// Toglie l'alias dallo stato, poi lo cancella dal server. Idempotente: se un tentativo
/// precedente ha tolto solo lo stato, il riprova cancella l'alias rimasto.
pub async fn remove_room_alias(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    alias: &str,
) -> Result<Canonical, AliasOpError> {
    let room = editable_room(client, account, scope).await?;
    let id = parse(alias)?;
    let state = current(client, &room).await?;
    if !state.contains(alias) {
        match client.resolve_room_alias(&id).await {
            Ok(found) if found.room_id != room.room_id() => return Err(AliasOpError::InvalidName),
            Ok(_) => {},
            Err(e) if e.as_client_api_error().is_some_and(|a| a.status_code.as_u16() == 404) => return Ok(state),
            Err(e) => return Err(from_http(&e)),
        }
    }
    let next = state.without(alias);
    if next != state {
        publish(&room, &next).await?;
    }
    timed(async {
        match client.remove_room_alias(&id).await {
            Ok(()) => Ok(()),
            Err(e) if e.as_client_api_error().is_some_and(|a| a.status_code.as_u16() == 404) => Ok(()),
            Err(e) => Err(from_http(&e)),
        }
    }).await?;
    Ok(next)
}

pub async fn set_canonical_alias(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    alias: &str,
) -> Result<Canonical, AliasOpError> {
    let room = editable_room(client, account, scope).await?;
    let state = current(client, &room).await?;
    let next = if let Some(next) = state.with_canonical(alias) {
        next
    } else {
        let id = parse(alias)?;
        let local = timed(async {
            let request = aliases::v3::Request::new(room.room_id().to_owned());
            client.send(request).await.map_err(|e| from_http(&e))
        }).await?;
        if !local.aliases.contains(&id) { return Err(AliasOpError::InvalidName); }
        state.with_added(alias, Some(true))
    };
    publish(&room, &next).await?;
    Ok(next)
}


/// Esegue l'azione e rilegge l'elenco anche dopo un errore (un orfano compare tra i locali).
pub async fn alias_action(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &matrix_domain::RoomScope,
    action: &AliasAction,
) -> (Result<(), AliasOpError>, Option<RoomAliases>) {
    let done = match action {
        AliasAction::Load => Ok(()),
        AliasAction::Add { local, make_canonical } => add_room_alias(client, account, scope, local, *make_canonical).await.map(drop),
        AliasAction::Remove { alias } => remove_room_alias(client, account, scope, alias).await.map(drop),
        AliasAction::SetCanonical { alias } => set_canonical_alias(client, account, scope, alias).await.map(drop),
    };
    match room_aliases(client, account, scope).await {
        Ok(fresh) => (done, Some(fresh)),
        Err(e) => (done.and(Err(e)), None),
    }
}

/// Valida solo la parte locale (anteprima nel campo di testo).
pub fn check_local_part(input: &str) -> Result<String, AliasOpError> {
    normalize_alias_local(input).map_err(|_| AliasOpError::InvalidName)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_typed() {
        assert_eq!(alias_error(Some(409), Some(&ErrorKind::RoomInUse)), AliasOpError::AliasInUse);
        assert_eq!(alias_error(Some(409), Some(&ErrorKind::Unknown)), AliasOpError::AliasInUse);
        assert_eq!(alias_error(Some(403), Some(&ErrorKind::Forbidden)), AliasOpError::Forbidden);
        assert_eq!(alias_error(Some(400), Some(&ErrorKind::InvalidParam)), AliasOpError::InvalidName);
        assert_eq!(alias_error(None, None), AliasOpError::Network);
        assert_eq!(alias_error(Some(502), None), AliasOpError::ServerUnreachable);
    }
}
