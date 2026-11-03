# userdir-live: user directory su Synapse e Tuwunel (usa e getta)

Script usa e getta per verificare dal vivo cosa restituisce la ricerca utenti
(`POST /_matrix/client/v3/user_directory/search`, via `Client::search_users`
di matrix-sdk 0.19.1 vendored) su un Synapse e un Tuwunel in container, senza
API admin. Serve ai test live del dialogo «Nuova chat». Risultati documentati in `the user-directory-search notes`.

## Uso

```sh
./run.sh build    # una volta sola: compila probe/ in un container rust:1
./run.sh default  # Synapse e Tuwunel con configurazione di default
./run.sh all      # Synapse user_directory.search_all_users + Tuwunel show_all_local_users
./run.sh down     # spegni ed elimina i volumi
```

Ogni scenario ricrea i server con volumi freschi, registra tre utenti per
server — alice («Alice Rossini») e bob («Bob Verdi») condividono una stanza
privata, carol («Carol Blu») non entra in nessuna stanza — attende che la
user directory sia stabile (Synapse la popola con un background update che
può tornare vuoto a intermittenza durante il ripopolamento) e poi cerca
come alice e come carol una matrice fissa di termini (localpart, parziale
del display name, 2 caratteri, ID completo, nome e utente dell'altra
istanza). Output: `out/<scenario>.jsonl`, una riga JSON per evento.

Porti locali: Synapse `18008`, Tuwunel `18009`. Nessuna federazione tra i due:
serve proprio a verificare che la ricerca non esce dall'istanza.

## Contenuto

- `docker-compose.yml` — i due homeserver usa e getta.
- `run.sh` — build + scenari `default`/`all` + teardown.
- `tuwunel-runtime.toml` — generato da `run.sh` (non committato).
- `probe/` — binario Rust standalone che dipende dal matrix-sdk 0.19.1
  vendored nel repo; è il mezzo con cui si chiamano le API, non un client
  HTTP scritto a mano.
