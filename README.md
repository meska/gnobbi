# Dime

Client desktop Matrix open source scritto in Rust, con interfaccia GPUI.

Stato: sviluppo iniziale, nessuna release.

Build: `cargo build --locked` (workspace: `matrix-domain`, `matrix-engine`, `matrix-desktop`).

## Accesso

Il primo accesso chiede homeserver, ID Matrix e password; la chiave che protegge l'archivio locale è custodita nel portachiavi di sistema (Secret Service su Linux), senza frase da inventare. Dopo l'accesso un passo facoltativo avvisa che la cronologia cifrata precedente può non essere leggibile; si può proseguire senza recupero. Se il portachiavi non è disponibile o è bloccato, Dime lo dice e non modifica il profilo. Compilando senza `linux-vault` resta il vecchio modulo con frase locale.

Avvio: `matrix-desktop --signin <cartella-profilo>` (sessione Wayland).

## Licenza

Doppia licenza [MIT](LICENSE-MIT) oppure [Apache-2.0](LICENSE-APACHE), a scelta. Le dipendenze di terze parti hanno licenze proprie: vedi THIRD_PARTY_NOTICES.
