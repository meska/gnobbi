# Gnobbi

Client desktop Matrix open source scritto in Rust, con interfaccia GPUI.

Stato: sviluppo iniziale, nessuna release.

Build: `cargo build --locked` (workspace: `matrix-domain`, `matrix-engine`, `matrix-desktop`).

## Accesso

Il primo accesso chiede homeserver, ID Matrix e password; la chiave che protegge l'archivio locale è custodita nel portachiavi di sistema (Secret Service su Linux), senza frase da inventare. Dopo l'accesso un passo facoltativo permette di inserire la chiave di recupero Matrix esistente (non la frase locale) per recuperare dal backup la cronologia cifrata precedente; non tutti i vecchi messaggi sono garantiti nel backup. Una chiave errata o un errore di rete lasciano il backup intatto e si può riprovare; si può sempre proseguire senza recupero. Un account senza archivio segreto Matrix lo scopre quando si prova una chiave: a quel punto Gnobbi offre «Crea chiave di recupero», che genera una chiave e il backup delle chiavi di cifratura, mostra la chiave una sola volta (copia esplicita, poi conferma di averla salvata) e solo allora prosegue; se la creazione fallisce, il server resta com'era e si può riprovare. Gnobbi crea il recupero soltanto quando mancano sia l'archivio segreto sia il backup: un account con backup ma senza archivio (o l'inverso) viene rifiutato senza modifiche, e chiavi o backup esistenti non vengono mai sostituiti. Il backup protegge i messaggi cifrati scambiati da questo dispositivo dopo la creazione: la cronologia precedente, mai salvata in un backup, non diventa leggibile e non è promessa. Se il portachiavi non è disponibile o è bloccato, Gnobbi lo dice e non modifica il profilo. Compilando senza `linux-vault` resta il vecchio modulo con frase locale.

Avvio: `matrix-desktop --signin <cartella-profilo>` (sessione Wayland).

## Licenza

Doppia licenza [MIT](LICENSE-MIT) oppure [Apache-2.0](LICENSE-APACHE), a scelta. Le dipendenze di terze parti hanno licenze proprie: vedi THIRD_PARTY_NOTICES.
