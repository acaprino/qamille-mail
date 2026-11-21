# Le due edizioni di Qamille: open source e Professional

> **Stato:** la divisione del server è fatta (wave 1 della [specifica core ed enterprise](specs/2026-11-17-core-enterprise-design.md)): `server/` è il core (utenti creati dall'amministratore, accesso con username e password, dispositivi, calcolo, pagina `/admin`), `enterprise/server/` è il crate enterprise (registrazione da sé, recupero per posta, «Continua con Google», abbonamenti). Sono due binari e due immagini dello stesso repository: `qamille-server` e `qamille-server-enterprise`. L'app non avvia più un server locale né installa modelli (wave 2): senza server è un client di posta completo con la ricerca per parole; chi vuole tutto sul proprio PC usa il server core in Docker ([guida](server-sul-pc.md), `server/compose.local.yaml`). Il passaggio dell'enterprise a un repository privato, il gateway LLM e la sincronizzazione delle caselle dopo il login sono nelle wave seguenti.

Una sola app e un solo server, dallo stesso commit. Le edizioni differiscono per **configurazione**, non per codice: l'app sa a quale server parlare (si decide alla build), il server sa che cosa vende (si decide all'avvio). Decisioni e motivi: [specifica «Vendita Professional»](specs/2026-11-16-vendita-professional-design.md) e [ADR-0013](adr/ADR-0013-vendita-professional.md), dopo [ADR-0012](adr/ADR-0012-accesso-password-edizioni.md).

## In breve

| | Open source self-hosted | Professional |
| --- | --- | --- |
| Chi lo usa | chi installa il proprio server (un ufficio, una persona) | i clienti del servizio del venditore |
| App | senza server preimpostato; si usa anche senza account | server del venditore preimpostato; account obbligatorio: senza accesso mostra «Accedi a Qamille» (a ogni avvio e dopo l'uscita) |
| Account | li crea l'amministratore | registrazione dall'app, per posta o con Google |
| Accesso con Google | facoltativo (client dell'organizzazione) | acceso |
| Abbonamenti | spenti: il calcolo è per tutti gli utenti attivi | prova, piani, `402` senza abbonamento |
| Che cosa si paga | niente | il calcolo sul server (e domani la sincronizzazione) |

L'edizione open source funziona anche senza account e senza server: posta, ricerca, riunioni in locale. La Professional chiede sempre l'accesso (decisione del proprietario, 2026-11-19: è il servizio che si vende; chi vuole l'app senza account ha l'open source); con l'account ma senza abbonamento la posta resta in locale e il calcolo sul server chiede l'abbonamento.

## Che cosa si imposta e dove

**App, alla build** (variabili d'ambiente del processo di compilazione):

| Variabile | Open source | Professional |
| --- | --- | --- |
| `QAMILLE_PRESET_SERVER` | vuota | `https://<server del venditore>` (con `QAMILLE_SERVER` impostata per `build-release.bat`) |
| `QAMILLE_GOOGLE_CLIENT_ID`, `QAMILLE_GOOGLE_CLIENT_SECRET` | vuote (ogni utente usa il suo client in Opzioni sviluppatore) | il client **Desktop app** della posta del venditore, verificato da Google ([guida](oauth-google.md)) |

L'app **non** ha bisogno di nulla per «Continua con Google»: l'accesso passa dal server.

**Server, all'avvio** (`server.toml` e variabili `QAMILLE_*`, dettagli in [server/README.md](../server/README.md) per il core e in [enterprise/server/README.md](../enterprise/server/README.md) per l'enterprise). La colonna «Di» dice chi legge la chiave: un core che trova una chiave dell'enterprise la ignora e lo scrive nel log all'avvio.

| Chiave | Di | Open source | Professional |
| --- | --- | --- | --- |
| `QAMILLE_BIND`, `QAMILLE_ADMIN_BIND`, `QAMILLE_ADMIN_KEY`, `QAMILLE_DATA_DIR`, `QAMILLE_TRUSTED_PROXIES`, `QAMILLE_DEFAULT_MAX_DEVICES`, limiti di login e di calcolo, `QAMILLE_MODELS_DIR` | core | come da README | come da README |
| `QAMILLE_EDITION` | core | `self-hosted` | `professional` |
| `QAMILLE_SIGNUP` + `QAMILLE_SMTP_*` | enterprise | assenti | accesi |
| `QAMILLE_PUBLIC_URL` | enterprise | assente | `https://<dominio>` |
| `QAMILLE_GOOGLE_CLIENT_ID`, `QAMILLE_GOOGLE_CLIENT_SECRET`, `google_allowed_domains` | enterprise | assenti | il client **Applicazione web** dell'accesso |
| `QAMILLE_BILLING_PROVIDER`, `QAMILLE_ALLOW_FAKE_BILLING` | enterprise | assenti | `fake` per il collaudo (con `QAMILLE_ALLOW_FAKE_BILLING=1` su un indirizzo pubblico), poi il processore vero |
| `[[plans]]`, `trial_days`, `billing_grace_days`, `billing_currency` | enterprise | — | prezzi e regole commerciali |
| `QAMILLE_BILLING_WEBHOOK_SECRET` | enterprise | — | il segreto del processore |
| `QAMILLE_SIGNUP_*_MAX_PER_ORIGIN`, `QAMILLE_PASSWORD_RESET_*_MAX_PER_ORIGIN` | enterprise | — | limiti per origine di registrazione e recupero |

## Costruire le due app (Windows)

```powershell
# Open source: release\Qamille.exe, installata in D:\Software\Qamille
.\build-release.bat

# Professional: release\, installata in D:\Software\Qamille-Professional, e release\installer\Qamille-Professional-Setup-<versione>.exe
$env:QAMILLE_GOOGLE_CLIENT_ID = "<id del client Desktop della posta>"
$env:QAMILLE_GOOGLE_CLIENT_SECRET = "<segreto dello stesso client>"
$env:QAMILLE_SERVER = "https://qamille.example.com"
.\build-release.bat
```

Le due build scrivono in cartelle diverse e non si toccano. Un valore non valido di `/server` ferma la build: un Professional non esce mai per sbaglio come self-hosted. In una pipeline di rilascio le due credenziali della posta e il certificato di firma vengono dai segreti della pipeline, mai dal repository.

### Disinstallare la Professional

L'installatore è per utente e tiene i dati accanto al programma, in `<cartella d'installazione>\data` (account, posta, `credentials.key`), non in `%APPDATA%`. Gli hook sono in `desktop/src-tauri/windows/installer-hooks.nsh` ([regola 25](aspetto-nativo.md)).

- **La casella per eliminare i dati dell'applicazione** (pagina di conferma della disinstallazione) cancella `data` **per sempre**: account, posta e `credentials.key`, senza la quale le credenziali salvate non si leggono più. Non cancella niente se non è spuntata, e mai con `/UPDATE`. Un aggiornamento fatto a mano non passa `/UPDATE`: sulla pagina «Disinstalla prima di installare» la disinstallazione mostra la stessa casella, e spuntarla lì cancella i dati prima della nuova versione.
- **Il worker** (`Qamille --worker`) avviato dalla cartella d'installazione viene chiuso prima della disinstallazione; le altre copie di Qamille (anche portatili) restano aperte.
- **L'avvio all'accesso** (Impostazioni → Generali → Windows, [regola 27](aspetto-nativo.md)) è il valore «Qamille» sotto `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, con il suo gemello di Gestione attività sotto `…\Explorer\StartupApproved\Run`. La disinstallazione li toglie a cose fatte (non con `/UPDATE`, e non se si annulla). Un aggiornamento a mano li toglie lo stesso: il Professional installato rimette il valore al primo avvio, se l'avvio all'accesso era acceso in Qamille e non era stato spento da Gestione attività. Le copie portatili hanno un nome loro (`Qamille (<cartella>, <codice>)`) e la disinstallazione non le tocca.
- **Il file d'avvio del worker** delle versioni precedenti (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\Qamille-worker.vbs`) non si scrive più: l'avvio all'accesso è Qamille stesso nell'area di notifica, che invia da sé. Al primo avvio di questa versione il file, se nomina questo eseguibile, diventa l'avvio all'accesso e si cancella; se nomina un eseguibile che non c'è più si cancella e basta. La disinstallazione lo toglie ancora, per un'installazione che non ha mai avviato questa versione.

La prova con l'installatore vero (installare, accendere l'avvio all'accesso, disinstallare con la casella spuntata: la cartella non c'è più, in Gestione attività non resta nessuna voce di Qamille e all'accesso dopo non parte niente) richiede una build con `QAMILLE_SERVER`; finché non è fatta, è da fare.

## Firma e aggiornamenti

**Stato:** preparati nell'app e nel workflow di release di `qamille-enterprise`, spenti finché il proprietario non dà le chiavi e un indirizzo pubblico (elenco in fondo). Nessun effetto sull'open source né sulle copie portatili.

**Aggiornamenti nell'app** (`desktop/src-tauri/src/updates.rs`, `desktop/src/components/UpdateNotice.tsx`). Il plugin ufficiale `tauri-plugin-updater` si registra solo se valgono tutte e quattro: l'exe è costruito con le due variabili qui sotto, l'edizione è la Professional e Qamille è installato dal suo installatore (accanto all'exe c'è `uninstall.exe`). Se ne manca una il plugin non si registra e niente controlla aggiornamenti.

| Variabile, alla build | Che cosa |
| --- | --- |
| `QAMILLE_UPDATER_PUBKEY` | la chiave pubblica dell'updater (minisign), quella che stampa `npx tauri signer generate` |
| `QAMILLE_UPDATER_ENDPOINT` | l'indirizzo `https://` di `latest.json` (un indirizzo `http://` spegne l'updater) |

La chiave e l'indirizzo arrivano al plugin da Rust, mai da `tauri.conf.json`; la pagina non ha permessi del plugin (`capabilities/main.json` non cambia), solo i comandi `update_status` e `update_install`. Un minuto dopo l'avvio, poi ogni 24 ore (un'ora dopo un controllo non riuscito, che va in `native.log`), Qamille legge `latest.json`: una versione più nuova si scarica subito, in background, solo da un indirizzo `https://`, con al massimo dieci minuti per il controllo e per lo scaricamento; se ne verifica la firma con la chiave, e la firma deve nominare la versione per cui è stata fatta (`requireSignedVersion`: la build di Tauri la scrive da sé), così un pacchetto vecchio firmato non passa per uno nuovo. A pacchetto pronto compare in basso a destra un avviso che resta finché non si sceglie, con «Riavvia e aggiorna». Il clic chiude Qamille come Esci, attraverso le guardie (bozze non salvate, registrazione, coda di Annulla invio): se una guardia tiene aperta la finestra non succede niente e l'avviso resta; altrimenti, all'uscita, parte l'installatore (modalità passiva, nessuna domanda), che riapre Qamille aggiornato e sullo schermo, senza gli argomenti del primo avvio (né `--autostart` né un link `mailto:` già aperto).

**Artefatti dell'updater.** `tauri.professional.conf.json` non li chiede. `build-release.bat`, con `QAMILLE_SERVER` e `TAURI_SIGNING_PRIVATE_KEY` impostate (più `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` se la chiave ne ha una), aggiunge `--config src-tauri/tauri.updater.conf.json` (`bundle.createUpdaterArtifacts: true`) e Tauri scrive accanto all'installatore la sua firma per l'updater, `app\target\release\bundle\nsis\<installatore>.exe.sig`. Senza la chiave il comando dell'installatore è quello di prima, byte per byte (controllo 27 di `desktop/tests/native-feel.mjs`).

**Firma del codice (Authenticode).** La fa la build stessa, con un certificato in un file PFX:

| Variabile, alla build | Che cosa |
| --- | --- |
| `QAMILLE_SIGNING_PFX` | il percorso del certificato con la sua chiave privata (un file `.pfx`) |
| `QAMILLE_SIGNING_PFX_PASSWORD` | la sua password (può mancare) |
| `QAMILLE_SIGNING_TIMESTAMP_URL` | il server di timestamp RFC 3161 (se manca, quello di DigiCert) |

Con `QAMILLE_SIGNING_PFX` impostata, `build-release.bat` aggiunge `--config src-tauri/tauri.signing.conf.json` (`bundle.windows.signCommand`). Così Tauri firma, con `desktop/src-tauri/windows/sign.ps1` (`signtool sign /fd SHA256 /tr <timestamp> /td SHA256`), il programma che mette nell'installatore, il disinstallatore e l'installatore, prima di scriverne la firma per l'updater: il `.sig` vale per l'installatore firmato. Tauri rimette poi il programma che ha costruito com'era, non firmato; per questo `build-release.bat` firma anche il `Qamille.exe` portatile in `release\`, con lo stesso script. La password resta nell'ambiente: lo script mette il PFX in un archivio certificati suo, dell'utente corrente, firma per impronta e poi toglie l'archivio, il certificato e la sua chiave privata, così la password non arriva mai su una riga di comando né in un log. Senza `QAMILLE_SIGNING_PFX` il comando dell'installatore è quello di prima, byte per byte (controllo 27).

Nel workflow di release di `qamille-enterprise` (`.github/workflows/release.yml`) il certificato viene dai segreti `WINDOWS_CERTIFICATE` (il PFX in base64, scritto in un file temporaneo del runner e cancellato alla fine del job anche se fallisce) e `WINDOWS_CERTIFICATE_PASSWORD`. Senza i segreti uno step scrive un avviso e la release esce come prima, non firmata; con i segreti, dopo la build, uno step controlla che lo zip portatile e l'installatore abbiano una firma valida.

**Che cosa manca al proprietario:**

1. I segreti di GitHub di `qamille-enterprise`: `WINDOWS_CERTIFICATE` e `WINDOWS_CERTIFICATE_PASSWORD` per la firma del codice; `TAURI_SIGNING_PRIVATE_KEY` e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` per l'updater, con la chiave pubblica della stessa coppia come `QAMILLE_UPDATER_PUBKEY`. La coppia si genera una volta sola: una chiave privata nuova fa rifiutare ogni aggiornamento alle copie già installate.
2. Un indirizzo **pubblico** per `latest.json` e per l'installatore (`QAMILLE_UPDATER_ENDPOINT`): gli asset delle release di un repository privato non si scaricano senza token.
3. Pubblicare di nuovo l'installatore: l'updater di Windows installa l'installatore NSIS, mentre oggi la release pubblica solo lo zip, per scelta del proprietario.

Finché 2 e 3 non sono decisi, lo step che scrive `latest.json` nel workflow resta commentato, con la spiegazione di che cosa farà.

**Prove fatte e prove da fare.** Fatte, su un PC con un certificato di prova fatto apposta (mai uno vero): la firma dentro la build di Tauri (`tauri bundle` con `tauri.signing.conf.json`: installatore, programma che contiene e disinstallatore firmati, nessuna password nel log né nello script NSIS, nessun archivio o chiave rimasti), la firma del `Qamille.exe` portatile, il comando dell'installatore identico senza le variabili. Restano al proprietario, o al Task 9 della fase 3: una prova del workflow con «Run workflow» (`workflow_dispatch`) senza «publish», prima senza i segreti (compare l'avviso e la build passa) e poi con il certificato vero; una build vera con `build-release.bat` e il certificato; un aggiornamento vero da una versione installata alla successiva.

## Avviare i due server

**Open source self-hosted** (core): `server/compose.yaml`, come in [server/README.md](../server/README.md), «Distribuzione con Docker Compose»; immagine `qamille-server:local`. Gli utenti li crea l'amministratore: `docker compose exec server qamille-server user create <nome>`.

**Professional** (enterprise): `enterprise/server/compose.yaml` (immagine `qamille-server-enterprise:local`, vedi [enterprise/server/README.md](../enterprise/server/README.md)), con in `.env` (parte da `enterprise/server/.env.example`):

```sh
QAMILLE_EDITION=professional
QAMILLE_SIGNUP=1
QAMILLE_SMTP_HOST=…            # e il resto della posta: serve alla registrazione e al recupero
QAMILLE_PUBLIC_URL=https://qamille.example.com
QAMILLE_GOOGLE_CLIENT_ID=…apps.googleusercontent.com
QAMILLE_GOOGLE_CLIENT_SECRET=…
QAMILLE_BILLING_PROVIDER=fake  # collaudo: nessun incasso
QAMILLE_ALLOW_FAKE_BILLING=1   # solo sul server di collaudo: chiunque si abbona gratis
```

e i piani in `server.toml` nel volume dei dati (esempio nel README dell'enterprise). I due compose usano lo stesso progetto, la stessa rete e lo stesso volume dei dati: se ne avvia uno solo per host, e `server.db` si può aprire a turno con l'uno e con l'altro. Con `fake` l'app mostra «Modalità di prova» e le pagine di pagamento sono finte: non usarlo con clienti veri.

## Provare tutto su un computer

Senza Google e senza posta, con il processore finto:

1. Server enterprise (dalla cartella `enterprise/server/`):

   ```sh
   export QAMILLE_DATA_DIR=./prova QAMILLE_BIND=127.0.0.1:8787 QAMILLE_ADMIN_BIND=127.0.0.1:8788
   export QAMILLE_ADMIN_KEY=$(openssl rand -hex 32)
   export QAMILLE_PUBLIC_URL=http://127.0.0.1:8787 QAMILLE_BILLING_PROVIDER=fake
   cargo run --release -- user create mario --email mario@example.com   # stampa la provvisoria
   cargo run --release -- serve
   ```

2. App Professional che punta a quel server: `set QAMILLE_SERVER=http://127.0.0.1:8787` e `build-release.bat` (`http://` è ammesso solo verso il computer stesso).
3. Primo avvio: «Accedi», consenso, `mario` con la provvisoria, nuova password. Mario non ha la prova (l'ha creato l'amministratore): la modalità resta «Su questo computer» e Calcolo → Abbonamento mostra i piani.
4. «Abbonati»: si apre `http://127.0.0.1:8787/billing/fake/checkout/…`; «Paga (riuscito)». L'app dice «Abbonamento attivo» e passa il calcolo al server.
5. «Gestisci abbonamento» apre la pagina finta di gestione: «Disdici», «Simula rinnovo non riuscito», «Termina subito» mostrano gli altri stati.
6. `http://127.0.0.1:8788/admin` con la chiave: colonna «Abbonamento», concessioni a mano e prove.

Per «Continua con Google» serve un client Applicazione web vero con reindirizzamento `http://127.0.0.1:8787/v1/auth/google/callback` (Google accetta `http://` solo per il loopback) e `QAMILLE_GOOGLE_CLIENT_ID`/`_SECRET`.

## Fuori dal repository

Segreti della pipeline o un repository privato del venditore: client Google della posta, certificato di firma del codice, chiave degli aggiornamenti; sul server chiave amministratore, password SMTP, segreto del client Google dell'accesso, segreto dei webhook, chiavi del processore vero, `server.toml` di produzione e `.env`.

## Prima di vendere

Stato al 18 novembre 2026 (wave 9, Task 8). L'elenco con i motivi è nella specifica, «Prima di vendere davvero».

**Fatto (impianto, senza testo legale):**

- `site/terms.html`: i dodici titoli e una riga «TESTO DA FORNIRE DAL TITOLARE»; `noindex`, non collegata dal sito. `site/legal.test.mjs` (`node --test site/legal.test.mjs`) tiene le pagine dentro il CSP del sito; con `QAMILLE_LAUNCH=1` diventa il **cancello del lancio** e fallisce finché resta un segnaposto, un `TODO`, una sezione vuota, «Price at launch» o un collegamento mancante.
- Prezzi sul sito: oggi «Price at launch» (nessun numero). Con i prezzi confermati si scrivono con `data-plan` e `data-price-cents` (vedi il commento in `site/index.html`); con `QAMILLE_INFO_JSON=<risposta salvata di GET /v1/info del server di collaudo>` la prova controlla che i numeri del sito siano quelli del server. I segnaposto del piano, **da confermare**: mensile 9 €, annuale 90 €, IVA inclusa, prova di 14 giorni.
- `deploy/README.md`: monitoraggio (controlli da fuori e sulla macchina, webhook muto, avvisi di fatturazione, coda ferma), posta transazionale con Resend, procedura di incidente, e una prova che il backup porta le tabelle della vendita.

**Resta al proprietario:**

- I testi: termini di servizio (`site/terms.html`), informativa privacy, accordo sul trattamento dei dati per le aziende, testo del recesso, parere del commercialista sulla fatturazione. Il titolare del trattamento (oggi un `TODO` in `site/privacy.html`) e la data di entrata in vigore.
- Il processore (Task 7), il conto in modalità di prova, le chiavi nell'ambiente, i prezzi e i numeri delle quote in `server.toml` di produzione.
- Le credenziali SMTP di Resend nel `.env` di produzione; verifica Google della posta; firma del codice; immagine scaricabile senza credenziali; backup acceso e **una prova di ripristino** nella tabella di `deploy/README.md`.
- Revisione legale: è il cancello del lancio, non del codice.

**Frasi di `site/privacy.html` che non sono più vere** (da riscrivere a cura del proprietario o del suo consulente; qui solo elencate):

| Dove | Frase | Perché non basta più |
| --- | --- | --- |
| §2 | «Data from Google is transferred directly between Google and your computer. It is not sent to the Qamille Mail developer» | Con la sincronizzazione delle caselle (wave 6-7) la posta, cifrata dal server, sta anche sul server del servizio. Serve anche alla verifica Google (Limited Use) |
| §3 | «forwards the request to an external provider chosen by its administrator» | Va detto quale provider e con quale base per il servizio ospitato (assistente interno oppure OpenAI, secondo il piano) |
| §5 | «it is not stored on the server» | Vale per testo e audio delle richieste di calcolo, **non** per le caselle sincronizzate (restano per 30 giorni dopo la fine dell'abbonamento, poi si cancellano) |
| §5 | «stores your username, email address, a one-way hash of your password, and the list of devices» | Mancano: i contatori di consumo del mese (minuti, token), la versione dei termini accettata e quando, i dati di abbonamento (piano, stato, scadenza, identificativi presso il processore) |
| §5 | «this deletes the account data from the server» | La cancellazione chiude anche l'abbonamento presso il processore; il registro delle prove conserva solo SHA-256 di email e identità Google, anche dopo la cancellazione |
| §5 | «no account data reaches the developer» | Con un server proprio resta vero, ma la frase va riscritta per il servizio ospitato |
| §6 | «Data leaves your computer only to the services you connect: … and the Qamille Mail account server» | Manca il processore di pagamento (dati di fatturazione e carta stanno da lui, mai da noi) e, per l'assistente, il provider esterno |
| Intestazione (riga 27) | `TODO` del titolare del trattamento | Va compilato prima del lancio |
| `site/index.html`, FAQ «Does my mail leave my computer?» | «nothing goes to the developer … deleted when each request ends» | Stesso motivo del §2 e del §5 (sincronizzazione) |
