# Gestione account da riga di comando

La CLI desktop usa lo stesso catalogo, gli stessi profili e il portachiavi dell'app. Serve una build con il supporto al portachiavi della piattaforma (`linux-vault`, `mac-vault` o `windows-vault`); il portachiavi deve essere disponibile anche nei job senza interfaccia. Gli esempi presuppongono `gnobbi` nel `PATH` e un account Matrix già esistente. La CLI non crea account sul server.

## Uso interattivo

```sh
gnobbi account login --user '@alice:example.org'
gnobbi account status
gnobbi account keys update --account '@alice:example.org'
gnobbi account logout --account '@alice:example.org'
```

Il login chiede la password senza mostrarla; `keys update` chiede la **chiave di recupero Matrix** esistente, non la password. Senza sorgente esplicita serve un terminale. Il server è ricavato dall'ID; usare `--homeserver https://matrix.example.org` per specificarlo. `--account` accetta l'ID Matrix oppure l'ID numerico nel catalogo. `status --account ...` limita la verifica a un account; senza selettore verifica tutti gli account. Chiudere l'app se il profilo è occupato.

Il login di un account già presente con sessione valida, o non verificabile per rete, non crea un nuovo dispositivo (`changed: false`): non garantisce che la password appena fornita sia corretta. Con sessione scaduta elimina il profilo locale esistente e ripete il login. Il logout normale revoca la sessione sul server e poi rimuove dati locali, cache e voci del portachiavi. Un account non presente è un successo senza modifiche. In caso di errore di rete la rimozione locale non procede; `--local` salta la revoca remota, quindi il dispositivo può restare attivo sul server. Revocalo separatamente.

`keys update` verifica la chiave sul server e importa i segreti; non ruota la chiave. Le chiavi delle stanze possono essere scaricate successivamente alla decrittazione. Per creare la prima chiave di recupero usa l'app: la CLI non ha un comando di inizializzazione.

## Installazioni e CI

`GNOBBI_USER`, `GNOBBI_HOMESERVER` e `GNOBBI_ACCOUNT` sostituiscono rispettivamente `--user`, `--homeserver` e `--account`; l'opzione esplicita prevale. Password e chiave si possono fornire con `GNOBBI_PASSWORD` e `GNOBBI_RECOVERY_KEY`, oppure tramite stdin o file. Per ogni segreto scegli **una sola** fonte: una variabile presente, anche vuota, è in conflitto con l'opzione stdin/file. Le variabili vuote non sono segreti validi. Non passare segreti come argomenti `--password` o `--recovery-key`: sono rifiutati.

Esempio: il sistema CI rende disponibili due file segreti regolari, accessibili solo all'utente del job. Non attivare `set -x` e non stampare i file nei log:

```sh
export GNOBBI_USER='@alice:example.org'
export GNOBBI_ACCOUNT="$GNOBBI_USER"
unset GNOBBI_PASSWORD GNOBBI_RECOVERY_KEY
gnobbi account login --password-stdin --json < /run/secrets/matrix-password
gnobbi account keys update --recovery-key-stdin --json < /run/secrets/matrix-recovery-key
gnobbi account status --account "$GNOBBI_ACCOUNT" --json
```

In alternativa: `--password-file /percorso/password` e `--recovery-key-file /percorso/chiave`. I file devono essere regolari; su Unix un file leggibile da altri utenti produce un avviso. Stdin e file sono UTF-8, al massimo 4096 byte, non vuoti; viene rimosso un solo newline finale. Le variabili d'ambiente possono essere esposte ad altri processi o strumenti del job: preferisci file protetti o stdin, secret manager e runner isolati. Non inserire segreti in repository, cronologia shell, log o artefatti CI. La CLI rimuove le due variabili segrete dal proprio ambiente, non da quello della shell chiamante: rimuovile anche lì dopo l'uso.

Per CA privata, ogni comando accetta `--ca-file /percorso/assoluto/ca.pem`: file regolare, massimo 1 MiB. Non disabilitare la verifica TLS.

### Rotazione della chiave

La rotazione cambia la chiave sul server: **la vecchia non funzionerà più**. Prepara prima un archivio sicuro e assicurati che la sessione sia attiva e verificata; la rotazione richiede un'identità cross-signing completa e il recupero/backup pronto. Su Unix, per esempio, scegli una cartella privata già esistente e un nome assoluto non ancora utilizzato:

```sh
umask 077
gnobbi account keys rotate --account '@alice:example.org' --output /percorso/privato/recovery-nuova.txt --json
```

Il file è creato senza sovrascrivere file esistenti, con permessi 0600 su Unix, prima della richiesta al server; durante l'operazione può essere vuoto. Salva la nuova chiave nel secret manager prima di eliminare il file, verifica l'accesso e aggiorna le installazioni che usano la vecchia chiave. Un errore di scrittura dopo la rotazione può lasciare inutilizzabile la vecchia chiave senza salvare la nuova: il messaggio indica di ripetere la rotazione. Anche un'interruzione del processo richiede di verificare server e file prima di riprovare.

`--output -` scrive la chiave su stdout; con `--json` la inserisce nel campo `recovery_key`. Usalo solo con un canale protetto, **mai** nei log CI. Con output su file il JSON contiene il percorso, non la chiave.

## Exit code

| Codice | Significato |
| --- | --- |
| 0 | Successo, anche se nessuna modifica necessaria |
| 1 | Errore interno |
| 2 | Uso, argomento, CA o sorgente del segreto non validi |
| 3 | Account non trovato o catalogo vuoto per `status` |
| 4 | Credenziali rifiutate: non ritentare automaticamente |
| 5 | Sessione scaduta o revocata: ripetere il login |
| 6 | Rete/server temporaneamente non disponibile; per `status`, sessione non verificata |
| 7 | Homeserver non supportato |
| 8 | Portachiavi non disponibile o bloccato, o build senza supporto |
| 9 | Errore di archiviazione locale |
| 10 | Profilo occupato: chiudere l'altro processo e riprovare |
| 11 | Chiave di recupero errata |
| 12 | Recupero o backup non pronti |

Per `status` su più account il codice è quello del primo stato non valido nell'ordine del catalogo; un esito 6 non dimostra che la sessione sia scaduta.

## Output JSON

Con `--json`, stdout contiene un oggetto JSON per comando. In caso di errore ordinario:

```json
{"ok":false,"command":"status","error":{"code":"not_found","exit":3,"message":"no accounts"}}
```

`login` restituisce `ok`, `command`, `changed` e `account` (`id`, `user_id`, `homeserver`, `device_id`). `logout` restituisce `changed` e `account`; quando rimuove un account aggiunge `remote_revoked`. Per account assente, `account` contiene solo `user_id` con il selettore richiesto. `keys update` restituisce `account` e `verified`; `keys rotate` restituisce `account` e `output`, oppure `recovery_key` se `--output -`. Alcuni campi account possono essere `null`.

`status` restituisce `accounts`, con `id`, `user_id`, `homeserver`, `device_id`, `session` e `verified`. I valori di `session` sono `valid`, `unverified`, `expired`, `in_use`, `keyring_unavailable`, `broken`; `verified` può essere `true`, `false` o `null` se non accertabile. Se almeno una sessione non è valida, l'oggetto ha `ok: false` e `exit` alla radice, **non** un campo `error`. Un `valid` con `verified: false` ha comunque exit 0.

Senza `--json` i risultati vanno su stdout e gli errori su stderr. Gli avvisi, ad esempio sui permessi dei file segreti, possono comparire su stderr anche con `--json`; non unirlo a stdout prima di analizzare il JSON.
