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
- **Il file d'avvio del worker** (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\Qamille-worker.vbs`) è uno per utente, condiviso con le copie portatili. La disinstallazione lo toglie a cose fatte (non con `/UPDATE`, e non se si annulla); l'app lo riscrive da sé a ogni avvio quando l'avvio automatico del worker è acceso (Impostazioni → Accesso agenti) e il file manca o nomina un altro eseguibile, mai quando è spento. Così un aggiornamento a mano non spegne l'avvio automatico, e una cartella spostata si corregge al primo avvio.

La prova con l'installatore vero (installare, avviare il worker, disinstallare con la casella spuntata: la cartella non c'è più e all'accesso dopo non compare nessun errore di Windows Script Host) richiede una build con `QAMILLE_SERVER`; finché non è fatta, è da fare.

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
