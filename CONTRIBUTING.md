# Contribuire a Gnobbi

Grazie dell'interesse. Il progetto è in sviluppo iniziale: l'ambito attuale è il client desktop Linux (Wayland prima), con build per macOS; Windows e mobile arriveranno più avanti.

## Prima di iniziare

- Apri una *issue* che descrive il problema o la proposta prima di una PR di una certa entità: così eviti lavoro che non entra nell'ambito attuale.
- Le funzioni Matrix passano prima di tutto dal [Matrix Rust SDK](https://github.com/matrix-org/matrix-rust-sdk) ufficiale: codice scritto a mano al suo posto, in generale, non viene accettato.
- Niente telemetria, niente dati personali, niente segreti o credenziali nei commit.
- I testi dell'interfaccia sono in inglese nel codice e tradotti nei cataloghi `locales/`: vedi [TRANSLATING.md](TRANSLATING.md).

## Compilare e testare

```sh
cargo build --locked -p matrix-desktop --features linux-vault
cargo test --locked -p matrix-domain
cargo test --locked -p matrix-desktop
cargo clippy --locked --workspace --all-targets --features matrix-engine/sdk,matrix-desktop/linux-vault -- -D warnings
```

`Cargo.lock` è versionato: usa sempre `--locked` e aggiorna il lock solo quando cambi davvero le dipendenze.

Le PR devono compilare, passare i test e non introdurre warning di clippy. Per le modifiche all'interfaccia allega uno screenshot.

## Segnalazioni

- Bug e proposte: apri una *issue* con passi per riprodurre, comportamento atteso e osservato, versione/commit.
- Vulnerabilità: vedi [SECURITY](SECURITY.md) — non aprire issue pubbliche.
