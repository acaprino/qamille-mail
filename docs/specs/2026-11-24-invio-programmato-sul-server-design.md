# Invio programmato sul server (solo Enterprise)

**Data:** 24 novembre 2026 · **Versione:** 1 · **Stato:** Bozza — da approvare · **Continua:** [sincronizzazione delle caselle](2026-11-17-sincronizzazione-caselle-design.md) (S1: «Se un giorno servisse (invii programmati a PC spento, versione web), sarà un'altra specifica, con un consenso suo»: è questa) · **Collegate:** [accesso con password ed edizioni](2026-11-12-accesso-password-edizioni-design.md) (A1, A2, A11), [posta online](../posta-online-specifica.md) (DRF-05 e «Invio programmato nella prima consegna», riga 105), [MCP](../mcp-specifica.md) (§5.3 e §6.1), [edizioni](../edizioni.md)

Le parti segnate «scelta mia» sono decisioni di chi scrive, da controllare. Le sezioni di questa specifica si chiamano P1…P11; S1…S9 sono quelle della sincronizzazione, A1…A12 quelle dell'accesso. Dove il codice non risponde è scritto **da verificare**, con il file da guardare.

## Perché

Oggi l'invio programmato è locale: il messaggio parte solo se all'ora scelta Qamille è aperto, il computer è acceso, c'è rete e la casella è collegata. Con Gmail non c'è alternativa dal lato del provider: «Pianifica invio» esiste solo nel sito e nelle app di Google e non si raggiunge da API, SMTP o IMAP, quindi qualunque client deve tenere lui la coda. Il server Enterprise è sempre acceso e ha già le credenziali delle caselle (S1): può tenere lui la coda e inviare all'ora giusta.

## Decisioni del proprietario

Solo i suoi messaggi, alla lettera:

1. «possiamo spostarla sul server?» (l'invio programmato).
2. «con una coda di job schedulati?»
3. «solo funzione enterprise».

Ha anche osservato che la sincronizzazione salva già le credenziali delle caselle sul server (S1 e decisione 4 di quella specifica). Tutto il resto di questo documento è proposta di chi scrive.

## Lo stato di oggi, letto nel codice

| Che cosa | Dove | Che cosa fa |
| --- | --- | --- |
| Programmazione dall'interfaccia | `desktop/src-tauri/src/commands/undo.rs:16` → `app/src/application/undo.rs:278-305` → `account_worker.rs:113-117`, `609-629` | Mette `send_at` sulla bozza, da 30 secondi a 180 giorni avanti (`domain/rules.rs:16`) |
| Invio all'ora | `app/src/application/scheduler.rs:184-268` | Alla scadenza toglie `send_at` (il «claim»), poi chiama `DraftService::send`. Un fallimento disarma la bozza e manda `AppEvent::ScheduledSendFailed` (`events.rs:49-50`), che diventa una notifica di Windows (`desktop/src-tauri/src/notify_rules.rs:125-131`, commit 9341b62). Con la cassaforte chiusa l'invio resta armato e riprova ogni 60 s (`scheduler.rs:8-11`, `37-41`, commit 63f1a18) |
| Coda durevole (MCP e invio immediato) | `app/src/application/mcp_outbox.rs:49-176` (accoda), `178-292` (esegue), `294-327` (passata) e `domain/outbox.rs:14-54` | `Delivery` con stati `queued`, `scheduled`, `sending`, `sent`, `sent_not_saved`, `unknown`, `failed`, `cancelled`, `needs_review`; chiave di idempotenza e hash della richiesta; oltre 15 minuti di ritardo → `needs_review` (`:248-254`); un claim più vecchio di 10 minuti → `unknown` (`:306-324`) |
| Strumenti MCP | `mcp_outbox.rs:329-399` (catalogo e permessi), `401-647` (chiamate); riesportati in `app/src/adapters/mcp.rs:22` | `schedule_draft`, `cancel_scheduled_send`, `list_outbox`, `get_operation`, `retry_operation` |
| Trasporto | `app/src/infrastructure/mail.rs:1040-1059` | Sempre SMTP con `lettre`: password per app, oppure XOAUTH2 con il token di Google (`oauth.rs:414-433`, rinnovo in `oauth.rs:226-228`). Lo scope è l'intero `https://mail.google.com` (`oauth.rs:22`) |
| Bozza collegata a Gmail | `app/src/infrastructure/gmail_drafts.rs:343-394` | Controlla la revisione, sostituisce la bozza cloud e la invia con l'API (`drafts/send`); SMTP non si usa |
| Server | `server/src/db.rs:27-32`, `server/src/sync/` | Tabelle `users`, `devices`, `device_keys`, `compute_usage`, `llm_usage_day`, `sync_state`, `sync_items`. Dipendenze di rete: solo `ureq` (`server/Cargo.toml:41`); nessun client SMTP, nessun rinnovo di token |
| Attività periodiche del server | `server/src/sync/mod.rs:313-327`, `server/src/compute/mod.rs:824`, avviate in `server/src/lib.rs:531-537` | Esistono già (pulizia dei segnaposto ogni ora, rilascio dei modelli inattivi ogni 10 s). Non c'è una coda di lavori |
| Estensioni | `server/src/ext.rs`, `server/src/lib.rs:195-282` | L'enterprise è un crate sopra il core, in un repository privato (`lib.rs:126`); aggiunge rotte (`with_public`), campi di `/v1/info` (`with_info`), un diritto d'uso (`with_entitlement`), ganci d'avvio (`with_on_start`) e osservatori (`with_observer`) |

Quattro cose che il codice dice e che questa specifica deve rispettare:

- **I percorsi locali sono due**, e non hanno le stesse regole: quello dell'interfaccia (`scheduler.rs`) non ha il limite di ritardo che la riga 105 della posta online chiede («Invio scaduto, da confermare»); lo ha solo la coda MCP.
- **Il pulsante «Programma invio» c'è**, in `desktop/src/components/Composer.tsx`: righe 654-682 (data, ora, «Per inviare, Qamille deve restare aperto.») e 693-702 (il pulsante). Durante la stesura di questo documento (24 novembre 2026) l'albero di lavoro ha avuto per qualche minuto una modifica non registrata che lo toglieva (`Composer.tsx`, `styles.css`, `desktop/tests/compatto-composer.mjs`); a fine stesura non c'è più e il ramo corrente è `sigrid-6-3`. Chi l'ha fatta e se tornerà: **da verificare** con il proprietario; pesa sulla scelta di P1.
- **«Enterprise» nel codice** è il crate enterprise (`qamille-server-enterprise`), che gira con `QAMILLE_EDITION=professional`; l'app lo chiama Professional. In questo repository il crate non c'è: `server/README.md:3` e `docs/edizioni.md:97` lo dicono in un repository privato. La specifica «core ed enterprise» e l'ADR-0014, citati dalle altre specifiche, non sono in `docs/` qui.
- **Il server non può rinnovare il token di una casella Gmail collegata con il client del venditore**: l'elemento sincronizzato porta il segreto del client solo se è personalizzato (`app/src/domain/sync.rs:35-40`, S2).

## Obiettivi

1. Un messaggio programmato parte all'ora scelta anche con Qamille chiuso e il computer spento.
2. La coda è dell'utente, non del computer: ogni suo dispositivo vede gli stessi invii programmati e può annullarli.
3. Nessun messaggio parte due volte per un guasto, e nessun esito incerto è presentato come certo: stesse regole di DRF-05 e di §6.1 dell'MCP.
4. Il server usa le credenziali solo per inviare ciò che l'utente ha programmato, solo dopo un consenso separato da quello della sincronizzazione.
5. Solo sul server Enterprise con la sincronizzazione accesa; il core open source non cambia comportamento.
6. L'utente vede che cosa il server ha fatto a suo nome (registro) e l'amministratore può fermare tutto.

## Non obiettivi

- Leggere, scaricare o indicizzare la posta sul server; versione web.
- Invii ricorrenti, invio «quando il destinatario è in ufficio», annullamento dopo la consegna.
- Sincronizzare le bozze tra i computer (resta fuori, S2).
- Notifiche istantanee dal server ai computer (si usa il controllo periodico, come la sincronizzazione).
- Spostare sul server l'invio immediato e la coda di «Annulla invio».
- Promettere SMTP «esattamente una volta» (§6.1 dell'MCP non lo promette).

## P1. Ambito ed edizioni

L'invio programmato sul server esiste quando valgono tutte: l'app ha una sessione con un server che dichiara `scheduled_send` in `GET /v1/info` (P7); la sincronizzazione delle caselle è accesa e ha il consenso; l'utente ha dato il consenso di P3; la casella del mittente ha «Sincronizza questa casella» acceso (altrimenti il server non ne ha la credenziale).

**Che cosa fa l'app senza queste condizioni.** Tre strade:

| | Che cosa | Pro | Contro |
| --- | --- | --- | --- |
| (a) | L'invio programmato esiste solo sul server Enterprise; l'app senza server non lo offre | Una sola promessa, vera («parte anche a PC spento»); una sola coda da mantenere; coerente con «solo funzione enterprise» | L'open source perde una funzione che aveva; gli strumenti MCP devono rifiutare senza server |
| (b) | Resta tutto com'è, il server non si usa per chi non lo ha | Nessuna regressione | Due motori con regole diverse; l'utente Enterprise vede due comportamenti |
| (c) | Con il server la coda è sul server; senza, resta la coda locale | Nessuna funzione persa | Due motori da mantenere e da provare; il locale conserva i limiti di oggi e la differenza col limite di ritardo |

**Raccomando (a)** (scelta mia): è la lettura più semplice di «solo funzione enterprise», e la coda locale non mantiene la promessa che l'utente si aspetta. Il motore locale non si cancella subito: serve all'invio immediato (`drafts.rs:182-184` passa dalla stessa coda) e a esaurire i programmati già esistenti (P11). Se il proprietario intendeva «solo la coda sul server è enterprise», la strada è (c) e cambia solo P8: è la decisione aperta 1.

## P2. Chi vede che cosa

Passa da «il server custodisce le credenziali» (S1) a **«il server usa le credenziali»**, e per la prima volta il server conserva un contenuto dell'utente: il messaggio. INV-2 dell'accesso («mai testo, audio, allegati… oltre la durata della richiesta») va riformulato con questa eccezione (se sia già stato riformulato per le caselle sincronizzate: **da verificare**, la specifica della sincronizzazione non lo cita).

**Che cosa arriva al server**, per ogni invio programmato:

| Dato | Come | Perché |
| --- | --- | --- |
| Il messaggio completo già composto (RFC 5322: intestazioni, testo, HTML, allegati), al più 36 MiB | Lo compone l'app, come oggi (`mail.rs`, `build_message`); il server non lo interpreta e non lo modifica | Il server spedisce byte, non scrive mail: nessun secondo compositore, e ciò che parte è ciò che l'utente ha visto |
| La busta: mittente, tutti i destinatari e, a parte, quelli in Ccn | Cifrata | Il Ccn non sta nelle intestazioni del messaggio composto (`mail.rs:993`): SMTP lo prende dalla busta, l'API di Gmail no (P5) |
| Oggetto, destinatari da mostrare, nomi e dimensioni degli allegati | Cifrati | Per l'elenco sugli altri dispositivi |
| Ora di invio in UTC e fuso scelto (nome IANA) | In chiaro | Il lavoratore deve leggerla senza aprire nulla |
| La casella: l'id dell'elemento di sincronizzazione (SHA-256 di fornitore e indirizzo, S3) | In chiaro | Collega il lavoro alla credenziale; non è il numero interno della casella, che cambia da un computer all'altro |
| `Message-ID`, SHA-256 e dimensione del messaggio | L'id cifrato, hash e dimensione in chiaro | Idempotenza e riconciliazione (P4) |
| Riferimento alla bozza: id e revisione locali, id e revisione della bozza Gmail se c'è | Cifrato | Sbloccare la bozza sul computer d'origine; fermarsi se la bozza è cambiata in Gmail (P8) |

**A riposo.** Il messaggio sta in un file cifrato nella cartella dei dati (`outbox/<utente>/<lavoro>`), non nel database; busta e riepilogo stanno nel database, cifrati. Motivo (scelta mia): `server.db` ha una sola connessione dietro un mutex (`db.rs:160-162`), e una scrittura di 36 MiB fermerebbe login e sincronizzazione; i backup del database restano piccoli. Cifratura ChaCha20-Poly1305 come `sync::crypto`, dati associati: utente e id del lavoro; il file è cifrato a blocchi per non tenere tutto in memoria (formato da fissare nel piano).

**Quale chiave.** Tre possibilità: la stessa `QAMILLE_SYNC_KEY`; una seconda variabile; una chiave derivata dalla prima. **Raccomando la derivata** (HKDF-SHA256 da `QAMILLE_SYNC_KEY` con l'etichetta `qamille-outbox-v1`, scelta mia): il lavoratore deve comunque aprire le credenziali con `QAMILLE_SYNC_KEY`, quindi una seconda variabile non protegge da chi ha il processo, ma è un segreto in più da perdere; la derivazione separa i due usi senza un secondo segreto. `sync rekey` deve ricifrare anche la coda, e `sync reset` deve annullarla (i lavori senza credenziale non possono partire). È la decisione aperta 6.

**Chi può leggere.** I dispositivi attivi dell'utente (riepilogo sempre; messaggio intero finché esiste, P7); chi controlla il server mentre è acceso, come per le credenziali (S8). Chi ruba database, file o backup senza la chiave non legge nulla.

**Quanto resta.** Il file del messaggio si cancella appena il lavoro è `sent` o `cancelled`. Per `failed`, `unknown` e `needs_review` resta finché l'utente decide, al più 30 giorni; poi si cancella il file e resta la riga, senza contenuto. Le righe terminali restano 30 giorni (§6.1 punto 5 dell'MCP) e poi si tolgono, tranne quelle incerte non risolte.

## P3. Consenso, revoca, dispositivi e abbonamento

**Il consenso è dell'utente, non del computer, e sta sul server** (scelta mia): è il server che agisce, e il secondo PC non deve richiederlo. L'app lo ricorda anche in locale per indirizzo del server, come `mailbox_sync_consent` (`app/src/domain/model.rs:546`, `application/sync.rs:365-378`).

Testo proposto, mostrato la prima volta che si preme «Programma invio» e in Impostazioni → Account:

> **Invio programmato dal server.** Per inviare un messaggio all'ora che scegli anche a computer spento, Qamille lo consegna a questo server insieme ai destinatari e agli allegati. All'ora scelta il server usa le credenziali della tua casella, che conserva già per la sincronizzazione, per inviarlo a tuo nome. Chi gestisce il server può accedere ai messaggi in attesa. Dopo l'invio il messaggio viene cancellato dal server. Puoi vedere ogni invio fatto dal server e revocare questo consenso in Impostazioni → Account.

> **Scheduled send from the server.** To send a message at the time you choose even when your computer is off, Qamille hands it to this server together with its recipients and attachments. At that time the server uses your mailbox credentials, which it already keeps for synchronization, to send it on your behalf. Whoever runs the server can access the messages that are waiting. After sending, the message is deleted from the server. You can see every send made by the server and withdraw this consent in Settings → Accounts.

Pulsanti: «Consento» / «Non ora». Senza consenso il server rifiuta (`403 consent_required`) e l'app non manda nulla (INV-41).

| Evento | Che cosa succede ai lavori |
| --- | --- |
| **Revoca del consenso** | I lavori `scheduled` diventano `cancelled` (motivo `consent_revoked`) e i loro file si cancellano subito; il consenso torna assente. L'app avvisa quanti sono prima di confermare; le bozze restano sui computer d'origine (P8) |
| **Sincronizzazione spenta per una casella, o casella rimossa** | Il server non ha più la credenziale: all'ora il lavoro diventa `needs_review` (`mailbox_unavailable`). L'app avvisa quando si spegne l'interruttore con lavori in attesa |
| **Dispositivo disattivato** | I lavori restano: sono dell'utente. «Disattiva» (S5) mostra quanti invii ha programmato quel dispositivo e offre «Annulla anche i suoi invii programmati» (scelta mia: un portatile rubato può aver programmato messaggi) |
| **«Revoca gli accessi alle caselle»** (S5) | Il token non vale più: il lavoro va in `needs_review` (`reconnect_required`). Dopo il ricollegamento non riparte da solo: lo conferma l'utente |
| **Utente disattivato** | Il lavoratore salta i suoi lavori; riattivato in ritardo, valgono le regole del ritardo (P4) |
| **Utente eliminato** | Righe cancellate con l'utente (`ON DELETE CASCADE`), file cancellati dall'osservatore (`ext.rs:32-35`) e da una pulizia degli orfani all'avvio |
| **Abbonamento finito** | Creare un lavoro risponde `402 subscription_required`. I lavori già accettati non partono: `needs_review` (`subscription_required`) all'ora dell'invio. Elenco, annullamento e registro restano aperti (P7). È la decisione aperta 5 |
| **Interruttore dell'amministratore** | In pausa, i lavori non partono e non scadono in silenzio: alla ripresa valgono le regole del ritardo |

## P4. Il lavoro

**Tabelle** (enterprise; nomi da allineare a quel repository, **da verificare**):

`outbox_jobs`:

| Campo | Che cos'è |
| --- | --- |
| `id` | UUID scelto dal server |
| `user_id` | Proprietario, con `ON DELETE CASCADE` |
| `key` | Chiave di idempotenza scelta dall'app; unica per utente, quindi contiene l'id del dispositivo o una parte casuale (gli id delle bozze sono locali e si ripetono da un computer all'altro) |
| `request_hash` | SHA-256 di messaggio, busta, casella e ora: la stessa chiave con altro contenuto è `409 idempotency_conflict` |
| `mailbox_id` | Id dell'elemento di sincronizzazione |
| `send_at`, `time_zone` | Istante UTC (fa fede) e fuso scelto, solo da mostrare. L'istante non si ricalcola se le regole del fuso cambiano (scelta mia) |
| `status`, `reason` | Stato e motivo, da un elenco chiuso (mai il testo del provider) |
| `version` | Contatore per utente, come `sync_state.version`: serve a `since` e ai conflitti |
| `message_sha256`, `message_bytes` | Del messaggio caricato |
| `sealed`, `nonce` | Busta, riepilogo, `Message-ID` e riferimenti alla bozza, cifrati |
| `attempts`, `next_attempt_at` | Tentativi falliti prima di ogni effetto, e quando riprovare |
| `claimed_by`, `claim_expires_at` | Chi lo sta lavorando e fino a quando |
| `effect_started_at` | Scritto **prima** di contattare il provider con i dati del messaggio: da qui in poi non si riprova più da soli |
| `provider_ref` | Id del messaggio presso il provider, se lo dà |
| `created_by_device`, `source` | Dispositivo che l'ha creato; `app` o `mcp` |
| `created_at`, `updated_at`, `finished_at`, `acknowledged_at` | Date; `acknowledged_at` quando un dispositivo ha mostrato l'esito all'utente |

`outbox_consent` (utente, versione del testo, data, dispositivo), `outbox_audit` (P10), `outbox_usage_day` (invii del giorno per utente, come `llm_usage_day`).

**Stati.**

```text
uploading -> scheduled -> claimed -> sending -> sent
                 |           |          |
                 |           |          -> unknown
                 |           -> scheduled (riprova) | failed | needs_review
                 -> cancelled
scheduled in forte ritardo -> needs_review
needs_review | failed | unknown -> scelta dell'utente (P7)
```

Sono quelli di §6.1 dell'MCP, con due aggiunte: `uploading` (il messaggio non è ancora arrivato intero; non può partire; scade dopo un'ora) e `claimed` (preso, ma nessun effetto remoto iniziato: si può rimettere in coda). `sent_not_saved` non serve sul server: la copia locale è affare dell'app (P8). `unknown` resta: è l'unico stato onesto dopo un guasto tra «consegnato al provider» e «segnato».

**Idempotenza.** Stessa `key` e stesso `request_hash`: il server restituisce il lavoro già creato. Il `Message-ID` lo calcola l'app, come oggi (`domain/outbox.rs:6-12`), e non cambia mai: è ciò che permette di cercare il messaggio presso il provider.

**Tentativi.** Si riprova da soli **solo** ciò che è fallito prima di `effect_started_at`: rete assente, token non rinnovato per un errore temporaneo, provider che risponde `5xx` o `429` prima di accettare dati. Attese 1, 5, 15, 60 minuti (scelta mia), finché non si supera la tolleranza del ritardo; poi `needs_review` (`late`). Un rifiuto definitivo prima dell'effetto (credenziale non valida, casella assente, destinatario rifiutato in modo permanente) non si riprova: `needs_review` o `failed` con il motivo.

**Dopo `effect_started_at` non si riprova mai da soli.** Se il processo cade o la risposta non arriva, il lavoro resta `sending` con il claim scaduto: alla ripresa si **riconcilia** (P5) e diventa `sent` se il provider lo conferma, altrimenti `unknown`. Vale anche con un solo processo.

**Forte ritardo.** Se all'ora dell'invio il server era spento, in pausa o senza rete, e il ritardo supera la tolleranza, il lavoro non parte: `needs_review` (`late`), e l'app mostra «Invio scaduto, da confermare» (posta online, riga 105). Oggi la tolleranza locale è 15 minuti (`mcp_outbox.rs:248`). Sul server propongo **60 minuti**, configurabili (`outbox_late_tolerance_secs`): qui il ritardo è colpa del server, e chi ha il PC spento non può confermare (decisione aperta 3).

**Limiti** (scelte mie, tutte configurabili):

| Limite | Valore | Da dove viene |
| --- | --- | --- |
| Anticipo | da 60 secondi a 180 giorni | 180 giorni come oggi (`rules.rs:16`); oggi il minimo è 30 s (`undo.rs:293`) |
| Messaggio | 36 MiB | 25 MiB di allegati come oggi (`rules.rs:21`), più la codifica |
| Lavori in attesa per utente | 100, e 500 MiB in tutto | |
| Invii dal server per utente al giorno | 200 | Sotto i limiti di Gmail; ferma un abuso |
| Caricamenti insieme per utente | 1 | |
| Invii insieme | 1 per casella, 4 in tutto | L'ordine in una casella è per `send_at`, poi per creazione |

**Un solo invio per lavoro.** Il claim è un `UPDATE … WHERE id = ? AND status = 'scheduled'` dentro `BEGIN IMMEDIATE`: lo vince uno solo, anche se due processi aprono per errore lo stesso database (`docs/edizioni.md:126` dice di avviarne uno per host). Il claim scade dopo 10 minuti, come oggi (`mcp_outbox.rs:311`).

## P5. Il lavoratore del server

- **Come parte.** Un'attività tokio dentro `serve`, avviata da un gancio `with_on_start` (`lib.rs:281`, `520-522`), sul modello di `spawn_purge` (`sync/mod.rs:313-327`). Non parte se la funzione è spenta (`QAMILLE_OUTBOX` assente, scelta mia: accenderla è un atto dell'amministratore, non una conseguenza della chiave di sincronizzazione) o senza `QAMILLE_SYNC_KEY`.
- **Ogni quanto.** Ogni 15 secondi (scelta mia) cerca i lavori `scheduled` con `next_attempt_at` passato. La regola dell'app «niente intervalli fissi» (INV-005, `scheduler.rs:13-16`) riguarda il computer dell'utente, non il server. Un messaggio parte entro un minuto dall'ora scelta.
- **All'avvio.** Rimette in `scheduled` i `claimed` scaduti, riconcilia i `sending` scaduti, applica la regola del ritardo, cancella i file orfani e gli `uploading` scaduti.
- **Un passo.** Prende il lavoro; controlla utente attivo, consenso, diritto d'uso, pausa, limite del giorno; apre l'elemento di sincronizzazione della casella; se è una Gmail con OAuth rinnova il token; scrive `effect_started_at` e passa a `sending`; consegna; scrive l'esito e una riga di registro; cancella il file. La rete sta fuori dalle transazioni, su `spawn_blocking` come il gateway LLM.

**Gmail con OAuth.** Il server chiede un token di accesso a `https://oauth2.googleapis.com/token` con il token di aggiornamento dell'elemento. Il token di accesso resta in memoria, mai su disco né nei log. **Che cosa manca oggi:**

- Il segreto del client. Per un client personalizzato è nell'elemento; per il client del venditore serve una variabile nuova del server, `QAMILLE_MAIL_GOOGLE_CLIENT_ID` e `QAMILLE_MAIL_GOOGLE_CLIENT_SECRET` (il client **Desktop app** della posta, quello della build, `docs/edizioni.md:27`). Non è `QAMILLE_GOOGLE_CLIENT_ID` del server, che è il client web dell'accesso (`docs/edizioni.md:39`). Se `oauth_client_id` dell'elemento non corrisponde a nessun segreto noto: `needs_review` (`client_unknown`).
- Se Google restituisce un token di aggiornamento nuovo, il server lo scrive nell'elemento (versione e `credential_updated_at` nuovi, `updated_by_device = "server"`), così arriva ai computer. Se Google lo faccia per questo tipo di client: **da verificare**.
- `invalid_grant` (accesso revocato): `needs_review` (`reconnect_required`), senza riprovare.

**Consegna a Gmail: con l'API, non con SMTP** (scelta mia). Il server ha già `ureq` e non ha un client SMTP; lo scope `https://mail.google.com` copre l'API; Gmail mette da sé il messaggio in «Inviata» e nel thread. Se il lavoro porta una bozza Gmail si fa come oggi (`gmail_drafts.rs:343-394`): si controlla la revisione, si sostituisce la bozza e la si invia, e Gmail la toglie. Sostituire la bozza ne cambia la revisione: il server scrive nel lavoro la revisione nuova **prima** di chiedere l'invio, così dopo un guasto riconosce la propria scrittura e non la scambia per una modifica dell'utente. Senza bozza: invio del messaggio grezzo.

**Ccn con l'API.** L'API non riceve una busta: legge i destinatari dalle intestazioni. Verso l'API il server antepone quindi al messaggio l'intestazione `Bcc:` con gli indirizzi in Ccn della busta, come fa oggi l'app (`gmail_drafts.rs:359`, `374-378`), e Gmail la toglie dalla copia consegnata. È l'unica aggiunta che il server fa ai byte caricati (INV-43); via SMTP non aggiunge nulla. Il limite di dimensione dell'invio con l'API e il tipo di caricamento per 36 MiB: **da verificare** nella documentazione di Google.

**Riconciliazione con Gmail.** Dopo un esito incerto il server cerca il messaggio per `Message-ID` (ricerca `rfc822msgid:`): trovato → `sent`; non trovato dopo 10 minuti → `unknown`. Come dice §6.1, l'assenza non dimostra che non sia partito: decide l'utente. Con una bozza Gmail c'è una prova in più: se la bozza esiste ancora con la revisione scritta dal server, l'invio non è avvenuto, e «Invia di nuovo» riparte da quella revisione senza fermarsi su `draft_changed`.

**Caselle con password (IMAP/SMTP).** Serve un client SMTP nel server enterprise (TLS implicito sulla 465, STARTTLS altrimenti, come `mail.rs:1049-1053`): quale libreria usa già l'enterprise per la posta di registrazione è **da verificare** nel suo repository. SMTP non ha idempotenza né ricerca: un esito incerto resta `unknown`. **Copia in «Inviata»:** Gmail la crea da sé anche via SMTP; per gli altri fornitori l'app oggi tiene una copia locale (`drafts.rs:228-233`), e una ricerca di `APPEND` in `app/src/infrastructure/` non ha trovato un salvataggio in «Inviata» via IMAP (da confermare leggendo `mail.rs`), quindi un invio dal server non lascerebbe traccia su nessun computer se non quello d'origine. Quali fornitori non Gmail l'app supporta davvero oggi: **da verificare** in `app/src/infrastructure/provider_registry.rs`. È la decisione aperta 4.

## P6. Dove sta il codice

Nel **crate enterprise** (scelta mia, decisione aperta 2): tabelle, rotte, lavoratore, pagina di amministrazione. Il core cambia solo per dare gli appigli:

- una variante `ScheduledSend` in `ext::Resource` (`ext.rs:10-17`), chiesta dalle rotte e dal lavoratore;
- il nome della migrazione enterprise in `TOLERATED` (`db.rs:44-52`), altrimenti il core rifiuta il database (`db.rs:222-231`); il numero libero (dopo `ent-007-alert-kinds`) è **da verificare**;
- un modo pubblico, per le estensioni, di aprire e aggiornare l'elemento di sincronizzazione di un utente con la chiave del server, senza passare da un dispositivo, e di derivare la chiave della coda; oggi `sync::store` apre gli elementi solo per un `Who` con dispositivo (`sync/mod.rs:210-219`);
- `sync rekey` e `sync reset` (`server/src/cli.rs`, **da verificare**) devono poter chiamare l'estensione;
- `AccountObserver` (`ext.rs:32-35`) per utente eliminato e dispositivo revocato: se ne accetta più di uno, **da verificare** in `lib.rs:240`.

Nel core la funzione non esiste: nessuna rotta, `GET /v1/info` non la dichiara, l'app non la mostra.

## P7. Rotte

Formato, errori (`{"error":{"code","message","retry_after_s"}}`) e accesso come le rotte di sincronizzazione (`sync/routes.rs:59-84`), nello stesso ordine: connessione sicura, limite per origine sulle chiavi sbagliate, chiave di un dispositivo attivo, posto tra le richieste in volo dell'utente.

**Due cancelli diversi** (scelta mia). Il diritto d'uso (`Resource::ScheduledSend`) e il consenso servono solo a ciò che fa partire posta: creare, caricare, riprogrammare, `send_now`, `send_again`. Leggere e fermare (elenco, un lavoro, il messaggio, annullare, `discard`, `mark_sent`, `ack`, registro, lettura e revoca del consenso) chiedono solo un dispositivo attivo: chi ha perso l'abbonamento o ha revocato il consenso deve poter vedere la sua coda, annullarla e leggere il registro.

| Rotta | Effetto |
| --- | --- |
| `GET /v1/info` | Aggiunge `scheduled_send: {available, max_message_bytes, max_days, max_jobs, late_tolerance_s}` (con `with_info`) |
| `GET` · `PUT` · `DELETE /v1/outbox/consent` | Legge, dà (`{text_version}`) o revoca il consenso; la revoca annulla i lavori in attesa e risponde con il loro numero |
| `POST /v1/outbox/jobs` | Crea un lavoro in `uploading` dal corpo qui sotto (al più 64 KiB). Con una `key` già usata restituisce il lavoro esistente |
| `PUT /v1/outbox/jobs/{id}/message` | Il messaggio, `Content-Type: message/rfc822`, con limite e durata propri (36 MiB, 10 minuti; le altre rotte restano a 64 KiB e 30 s, `http.rs:45-47`). Dimensione e SHA-256 devono essere quelli dichiarati: allora il lavoro passa a `scheduled`. Ripetibile finché è `uploading` |
| `GET /v1/outbox/jobs?since=` | `{version, jobs}`: i lavori dell'utente cambiati dopo una versione, con riepilogo aperto |
| `GET /v1/outbox/jobs/{id}` | Un lavoro |
| `GET /v1/outbox/jobs/{id}/message` | Il messaggio, finché il file esiste (per «Salva una copia» da un altro computer; scelta mia) |
| `POST /v1/outbox/jobs/{id}/cancel` | `{base_version}`. Solo da `uploading`, `scheduled`, `needs_review`, `failed`; altrimenti `409 cannot_cancel` |
| `POST /v1/outbox/jobs/{id}/reschedule` | `{base_version, send_at, time_zone}`: cambia solo l'ora, da `scheduled` o `needs_review` |
| `POST /v1/outbox/jobs/{id}/resolve` | `{base_version, action}`: `send_now` (da `needs_review` e `failed`), `mark_sent` o `send_again` (da `unknown`, scelta esplicita dell'utente), `discard` |
| `POST /v1/outbox/jobs/{id}/ack` | Un dispositivo ha mostrato l'esito: gli altri non notificano più |
| `GET /v1/outbox/audit?since=` | Il registro dell'utente (P10) |
| `/admin/api/outbox` (ascolto amministrativo) | Conteggi per stato e per utente, pausa generale e per utente; mai contenuti |

**Modificare il contenuto** è annullare e ricreare con una chiave nuova (scelta mia): un lavoro è uno snapshot immutabile, come in §6.1.

Corpo di `POST /v1/outbox/jobs`:

```json
{
  "schema": 1,
  "key": "<device_id>:draft:8842:1795000000",
  "mailbox_id": "<64 cifre esadecimali>",
  "send_at": "2026-12-01T08:00:00Z",
  "time_zone": "Europe/Rome",
  "message": {"id": "<qamille-…@qamille.local>", "sha256": "…", "bytes": 184220},
  "envelope": {"from": "giulia@example.com", "recipients": ["a@example.com", "ccn@example.com"], "bcc": ["ccn@example.com"]},
  "summary": {"subject": "Offerta", "to": ["a@example.com"], "attachments": [{"name": "offerta.pdf", "bytes": 120044}]},
  "origin": {"draft_id": "8842", "draft_revision": 1795000000, "remote_draft": null},
  "source": "app"
}
```

Risposta di un lavoro: `{id, version, status, reason, send_at, time_zone, mailbox_id, summary, message: {sha256, bytes, available}, source, created_by_device, created_at, updated_at, finished_at, acknowledged_at}`.

**Errori nuovi:** `403 consent_required`; `409 idempotency_conflict`, `409 outbox_conflict` (con il lavoro attuale in `error.current`, come `sync_conflict`), `409 cannot_cancel`, `409 mailbox_not_synced`, `409 outbox_full`; `400 invalid_request` per ora fuori dai limiti o hash che non corrisponde; `413` oltre le dimensioni; `429 rate_limited` al limite del giorno; `402 subscription_required`; `503` con la funzione in pausa. La versione dell'API resta 1 (`http.rs:49`): tutto è aggiunto, e l'app guarda la capacità dichiarata.

## P8. Nell'app

**Programmare.** «Programma invio» nel compositore, con giorno e ora, compare solo se valgono le condizioni di P1. Alla conferma l'app: salva la bozza; compone il messaggio e ne prende uno snapshot con gli allegati (come `mcp_outbox.rs:82-85`); crea il lavoro e carica il messaggio; solo quando il server risponde `scheduled` mostra **«Programmato sul server per martedì 1 dicembre alle 09:00. Parte anche a computer spento.»**, con «Annulla». Se il caricamento non riesce, nulla è programmato e la bozza resta com'è: mai «programmato» senza la conferma del server (INV-42).

**Senza le condizioni** (strada (a) di P1): il pulsante non c'è. Con un server Enterprise ma senza consenso o con la casella non sincronizzata, il pulsante c'è e il primo clic spiega che cosa manca e porta al consenso o all'interruttore della casella. In Impostazioni → Account, senza server: «L'invio programmato passa da un server Qamille.»

**La bozza locale** resta sul computer d'origine, bloccata e segnata «Programmata sul server» (lo stesso blocco di oggi, `drafts.rs:93-95`), e si toglie quando il lavoro è `sent`. «Modifica» annulla prima il lavoro sul server: se il server non risponde la bozza resta bloccata, perché non si può garantire che non parta. Sugli altri computer non c'è bozza: si vede il riepilogo, si può annullare, cambiare l'ora, salvare una copia.

**Stato tra i dispositivi.** L'app legge `GET /v1/outbox/jobs?since=` all'avvio, dopo il login, ogni 5 minuti come la sincronizzazione (`application/sync.rs:25`), dopo ogni sua azione e un minuto dopo l'ora di un lavoro in attesa. Tiene una copia locale del riepilogo per mostrarla senza rete, segnata «ultimo controllo alle…». Una vista **«Programmati»** elenca i lavori di tutte le caselle con stato, ora e casella.

**Fallimenti.** Il server non può notificare un computer. Quando l'app, al controllo, trova un lavoro passato a `failed`, `needs_review` o `unknown` senza `acknowledged_at`, mostra l'avviso nella finestra e la notifica di Windows di oggi (preferenza «Invii programmati non riusciti», `desktop/src/components/NotificationSettings.tsx:54`), con un testo nuovo: «Invio programmato non riuscito · «Oggetto» non è partito dal server. Apri Qamille per decidere.» Il clic apre «Programmati», non le Bozze (`desktop/src/lib/notification-open.ts`). Mostrato l'esito, l'app chiama `ack`. A computer spento nessuno avvisa: per questo propongo che il server enterprise, che ha già la posta transazionale, scriva all'indirizzo dell'account (fase 4, P11; il messaggio non contiene oggetto né destinatari).

**Che cosa può scegliere l'utente** su un lavoro fermo: `needs_review` → «Invia ora», «Riprogramma», «Annulla»; `failed` → «Riprova», «Annulla»; `unknown` → «Controlla in Inviata», poi «È partito» oppure «Invia di nuovo», con l'avviso che potrebbe arrivare due volte.

**Conflitti.**

| Caso | Regola |
| --- | --- |
| Due computer annullano o riprogrammano insieme | `base_version`: il secondo riceve `409 outbox_conflict` con lo stato attuale e lo mostra |
| Annullo mentre parte | `409 cannot_cancel`: l'app dice «Sta partendo: non si può più annullare» |
| La bozza cambia o sparisce in Gmail prima dell'ora | Il lavoro porta id e revisione della bozza Gmail; il lavoratore controlla prima di inviare: revisione diversa o bozza assente → `needs_review` (`draft_changed`, `draft_missing`), mai il testo nuovo spedito in silenzio (posta online, riga 105). Cancellare la bozza da Gmail è quindi un modo di fermare l'invio da un telefono |
| Stessa bozza programmata da due computer | Le bozze non si sincronizzano, quindi sono due lavori distinti; `key` diversa. Con una bozza Gmail in comune, il secondo trova la bozza già inviata → `needs_review` (`draft_missing`) |
| Casella ricollegata su un altro computer | Il token nuovo arriva al server con la sincronizzazione: i lavori in attesa lo usano |

**Interfaccia.** Solo componenti di Sigrid (regola del proprietario del 23 novembre 2026): il campo del giorno esiste (`desktop/src/ui/DateField`, usato oggi dal compositore); il campo dell'ora è un `Input` con una classe propria (`time-field`, `schedule-row` in `styles.css`), cioè debito. Se a Sigrid manca un campo dell'ora o un elenco con stato, si aggiunge a Sigrid, generico. Che cosa offre oggi Sigrid: **da verificare** in `desktop/vendor/sigrid/`.

**Strati.** Un port in `application`, implementato in `infrastructure` con `ureq`, come la sincronizzazione (`app/tests/layers.rs`); la chiave del dispositivo va solo all'indirizzo della sessione (INV-4).

## P9. MCP

Gli strumenti sono in `app/src/application/mcp_outbox.rs`. Con la coda sul server:

- **`schedule_draft`** (`:403-439`): stessi argomenti (`id`, `expected_revision`, `idempotency_key`, `date`). Crea un lavoro sul server con `source: "mcp"` e la stessa chiave; il risultato aggiunge `queue: "server"` e l'id del lavoro (una stringa: lo schema ammette già stringa o intero, `:354`). Senza le condizioni di P1 risponde `unsupported_capability` con il motivo (`no_server`, `consent_required`, `mailbox_not_synced`): un assistente non dà mai il consenso al posto della persona.
- **`cancel_scheduled_send`**, **`cancel_operation`** (`:600-629`): accettano l'id di un lavoro del server; `cannot_cancel` come oggi.
- **`list_outbox`**, **`list_operations`**, **`get_operation`** (`:561-599`): uniscono la coda locale (invii immediati, residui) e quella del server, con `queue: "local" | "server"` e, per il server, `checked_at`. Senza rete restituiscono l'ultima copia nota e lo dicono (`coverage: partial`).
- **`retry_operation`** (`:630-643`): su un lavoro del server vale solo da `failed`; `unknown` richiede la persona, come oggi.
- **Permesso.** Oggi il permesso di invio dell'MCP si ricontrolla al momento dell'invio (`:190-207`, `:242-247`); a computer spento non si può. Quindi (scelta mia): si controlla alla creazione, e quando la persona toglie il permesso di invio l'app annulla i lavori `source: "mcp"` ancora in attesa, subito se c'è rete, altrimenti al primo collegamento; l'avviso lo dice.
- **A11 dell'accesso** («Il server MCP usa la chiave solo per le rotte di calcolo») va esteso alle rotte `/v1/outbox`. Come `qamille-mcp` raggiunge oggi il port della sincronizzazione: **da verificare** in `app/src/adapters/mcp/server.rs`.

`docs/mcp-specifica.md` §5.3 e §6.1 si aggiornano con `queue` e con i motivi di `needs_review`.

## P10. Sicurezza e riservatezza

**Che cosa cambia nel rischio.** Già oggi chi compromette il server acceso ha le credenziali (S8). La novità è che il server le **usa** di suo: connessioni dal server a Google sono ora normali, quindi un abuso si nota meno, e un difetto del lavoratore può inviare posta a nome di una persona.

| Misura | Che cosa copre | Che cosa non copre |
| --- | --- | --- |
| Il server invia solo byte caricati da un dispositivo dell'utente, con hash dichiarato | Un difetto che componga o alteri messaggi | Chi controlla il server |
| Limite di invii al giorno per utente, nel codice | Abuso di un account o di un dispositivo rubato | Chi controlla il server |
| Registro visibile all'utente (`GET /v1/outbox/audit`): creato, annullato, credenziale usata, inviato, fallito, con ora, casella, dispositivo o «server» | Un invio che l'utente non riconosce | Un registro cancellato da chi controlla il server |
| Interruttore dell'amministratore, generale e per utente; `QAMILLE_OUTBOX` spento di base | Fermare tutto durante un incidente | |
| Posta all'account a ogni esito non riuscito (e, se il proprietario vuole, a ogni invio) | Accorgersi senza aprire l'app | |
| Token di Google con il solo scope di invio | Limiterebbe ciò che il lavoratore tocca | Il token completo è comunque sul server per la sincronizzazione: il guadagno è piccolo, e servono un secondo consenso di Google e una seconda credenziale. Non nella prima versione (scelta mia); **da verificare con Google** |

**Log.** Mai oggetto, destinatari, mittente, `Message-ID`, id della casella (è un hash indovinabile, come per la sincronizzazione), nomi di allegati, contenuto, risposte testuali del provider, token. Nei log: id del lavoro, id dell'utente, stato, motivo dall'elenco chiuso, dimensione, durata. Gli eventi vanno anche in `target: "audit"`, come quelli degli account (`accounts.rs:805`).

**Che cosa vede l'amministratore.** Dalla pagina: quanti lavori per stato e per utente, dimensioni, ore. Con la chiave e l'accesso alla macchina: tutto, come dichiara il consenso.

**Da segnalare a chi cura la parte legale** (non è una consulenza):

- Il gestore del server conserva e tratta il contenuto di messaggi e dati di terzi (i destinatari) per conto dell'utente: per i clienti azienda è un trattamento da responsabile (art. 28 GDPR), con l'accordo sul trattamento già in elenco in `docs/edizioni.md:166`; durata di conservazione e luogo dei dati vanno scritti.
- `site/privacy.html` va riscritta anche per questo: alle frasi già elencate in `docs/edizioni.md:171-183` si aggiunge che il server conserva i messaggi in attesa e invia a nome dell'utente.
- Google: un server che conserva **e usa** token con scope ristretti entra nella verifica (uso limitato, valutazione di sicurezza), già segnalata in S8. Se i token rilasciati a un client «Desktop app» si possano usare da un server: **da verificare con Google** prima del lancio.

## P11. Migrazione, fasi e verifica

**I programmati locali già esistenti** sono di due tipi: bozze con `send_at` (interfaccia) e `Delivery` in `scheduled` (MCP). Quando compaiono le condizioni di P1 l'app chiede una volta: «Hai N invii programmati su questo computer. Vuoi spostarli sul server?» «Sposta» li carica uno per uno e toglie quello locale solo dopo `scheduled` dal server; «Lascia qui» li lascia al motore locale, che continua a eseguirli alle regole di oggi finché finiscono (al più 180 giorni). Niente si sposta da solo: il consenso vale per ciò che l'utente programma dopo averlo dato. Il campo `send_at` della bozza resta nel formato (`desktop/src/lib/types.ts:170`); l'app non lo scrive più per lavori nuovi. Un'app vecchia con un server nuovo ignora `scheduled_send` in `/v1/info` (che `ServerInfo`, `app/src/domain/server_account.rs:30`, tolleri campi sconosciuti: **da verificare**); un'app nuova con un server senza la capacità non mostra la funzione.

**Fasi.** Tutto il Rust si compila e si prova solo in CI; nell'anteprima del browser (`desktop/src/lib/preview.ts`, che fa già da server finto per la sincronizzazione) si prova l'interfaccia.

| Fase | Che cosa | Giorni | Rischio | Dove si verifica |
| --- | --- | --- | --- | --- |
| 1 | Core: `Resource::ScheduledSend`, migrazione tollerata, apertura delle credenziali per le estensioni, chiave derivata, `rekey`/`reset`. Enterprise: tabelle, consenso, rotte, limiti, cifratura, registro. Nessun invio | 6–8 | Medio | CI |
| 2 | Lavoratore: claim, tentativi, ritardo, ripresa; rinnovo del token; Gmail con l'API e riconciliazione; SMTP per le password (se deciso) | 8–10 | Alto: è qui che un errore invia due volte o non invia | CI con provider finti; una prova con una Gmail vera, da fare a mano |
| 3 | App: port e servizio, controllo periodico, consenso, «Programma invio», «Programmati», bozza bloccata, notifiche, migrazione dei locali | 7–9 | Medio | Interfaccia nell'anteprima; servizio in CI |
| 4 | MCP; pagina e interruttore dell'amministratore; posta all'account; vista del registro | 4–5 | Basso | CI; anteprima per il registro |
| 5 | Prova tra due computer veri e server di collaudo, guasti provocati; documenti (`server/README.md`, `docs/edizioni.md`, MCP, posta online, privacy); verifica con Google | 3–5, più le attese esterne | Medio | A mano: fatta o dichiarata **non fatta** |

In tutto 28–37 giorni di lavoro, senza contare le risposte di Google e i testi legali.

**Test da scrivere**, nello stile di quelli che ci sono:

- **Server** (crate enterprise, come `server/tests/sync_*.rs` con `TestServer` e `ManualClock` di `server/tests/support/mod.rs`): `outbox_store` (stati, versioni, conflitti, idempotenza con stessa chiave e contenuto diverso, isolamento tra utenti, cifratura: né database né file contengono il testo); `outbox_access` (ordine dei controlli, `402`, `403 consent_required`, dispositivo disattivato); `outbox_limits` (dimensione, numero, giorno, anticipo); `outbox_worker` con un provider finto sul modello di `support/fake_openai.rs` (scenari: accetta, rifiuta prima dei dati, risponde tardi, cade dopo aver accettato); **crash tra invio e segno** (il provider ha accettato, l'esito non è scritto: alla ripresa `sent` se la ricerca lo trova, `unknown` altrimenti, e mai un secondo invio: il finto conta le consegne); **doppio claim** (due lavoratori sullo stesso database: una sola consegna); ritardo oltre la tolleranza; token: rinnovo riuscito, `invalid_grant`, client sconosciuto, token nuovo scritto nell'elemento; bozza Gmail cambiata o assente; revoca del consenso; utente eliminato (righe e file); `rekey` e `reset`; `outbox_secrets_in_logs` (come `sync_secrets_in_logs.rs`).
- **App** (`app/tests/`, come `sync_service.rs`, `sync_http.rs`, `sync_two_apps.rs` con `support/http_fake.rs`): programmare (nulla risulta programmato prima della conferma), caricamento interrotto e ripreso con la stessa chiave, bozza bloccata e sbloccata, annullamento con conflitto, due istanze che vedono la stessa coda, notifica una sola volta e `ack`, migrazione dei locali, server senza la capacità; `layers.rs` verde.
- **MCP** (`app/tests/mcp_outbox.rs`): `schedule_draft` verso il server, rifiuto senza condizioni, elenco unito, annullamento, permesso tolto.
- **Browser** (`desktop/tests/`, Playwright come `mailbox-sync-ui.mjs`, con `window.__previewServer`): consenso; programmare e vedere «Programmato sul server»; «Programmati» con ogni stato; annullare; lavoro fermo con le sue scelte; pulsante assente senza server; `only-sigrid.mjs` e `ui-usage.mjs` verdi; `native-feel.mjs` se cambia un CSS globale.

## Invarianti

Continuano dopo INV-40 (l'ultimo nelle specifiche di questo repository; i documenti dell'enterprise non sono stati controllati).

- **INV-41.** Nessun messaggio lascia un computer verso la coda del server prima del consenso di P3, del login e della capacità dichiarata da quel server; il server rifiuta comunque senza consenso registrato.
- **INV-42.** L'app dice «programmato sul server» solo dopo che il server ha confermato il lavoro con il messaggio intero e l'hash giusto.
- **INV-43.** Il server usa una credenziale solo per inviare un lavoro creato da un dispositivo di quell'utente, e invia solo i byte caricati; l'unica aggiunta ammessa è l'intestazione `Bcc:` verso l'API di Gmail, presa dalla busta caricata.
- **INV-44.** Dopo l'inizio dell'effetto remoto nessun lavoro si riprova da solo; un esito che non si può stabilire è `unknown`, mai `sent` né `failed`.
- **INV-45.** Un lavoro lo prende un solo lavoratore; un lavoro in ritardo oltre la tolleranza non parte senza la conferma dell'utente.
- **INV-46.** Messaggio, busta e riepilogo stanno sul server solo cifrati, con una chiave che non è nel database, nei file, nei backup né nei log; il messaggio si cancella all'esito finale.
- **INV-47.** I log non contengono contenuti, indirizzi, oggetti, id delle caselle né token; ogni uso di una credenziale lascia una riga nel registro che l'utente può leggere.
- **INV-48.** Senza abbonamento, senza consenso, con l'utente disattivato o con la funzione in pausa il server non invia nulla.

## Esempi

- **EX-42 PC spento.** Giulia programma dal fisso un messaggio per le 8:00 di lunedì e spegne. Alle 8:00 il server lo invia con la sua Gmail; alle 9:00 apre il portatile: «Programmati» è vuota, il messaggio è in «Inviata» perché l'ha scaricato da Gmail.
- **EX-43 Annullo dal secondo PC.** Domenica, dal portatile, Giulia annulla. Lunedì il fisso, al primo controllo, sblocca la bozza.
- **EX-44 Server fermo all'ora.** Il server riparte 20 minuti dopo le 8:00: entro la tolleranza, il messaggio parte. Riparte dopo due ore: `needs_review`, e Giulia trova «Invio scaduto, da confermare».
- **EX-45 Crash a metà.** Il server cade dopo che Gmail ha accettato il messaggio. Alla ripresa lo trova per `Message-ID`: `sent`, un solo messaggio.
- **EX-46 Bozza cambiata dal telefono.** Giulia corregge la bozza nell'app Gmail. All'ora il server vede un'altra revisione e si ferma: `needs_review` (`draft_changed`).
- **EX-47 Accesso revocato.** Giulia revoca Qamille dal suo account Google. Il lavoro va in `needs_review` (`reconnect_required`); dopo aver ricollegato la casella sceglie «Invia ora».

## Decisioni aperte per il proprietario

1. **L'app senza server Enterprise.** (a) niente invio programmato, (b) tutto com'è, (c) coda locale senza server. **Raccomando (a)**, con i programmati locali esistenti portati a termine (P1, P11).
2. **Dove sta il codice.** Crate enterprise con pochi appigli nel core, oppure nel core dietro un interruttore. **Raccomando il crate enterprise** (P6): è la lettura di «solo funzione enterprise» che non si aggira ricompilando il core.
3. **Tolleranza del ritardo sul server.** 15 minuti come oggi, oppure di più. **Raccomando 60 minuti**, configurabili.
4. **Quali caselle nella prima versione.** Solo Gmail (OAuth con l'API; password per app via SMTP di Gmail, che salva in «Inviata» da sé), oppure anche gli altri IMAP/SMTP. **Raccomando solo Gmail**; gli altri quando sarà deciso come lasciare la copia in «Inviata» (un `APPEND` IMAP dal server, cioè un secondo uso delle credenziali da dichiarare).
5. **Abbonamento finito con lavori in attesa.** Non inviare e chiedere (`needs_review`), oppure onorare i lavori già accettati. **Raccomando di non inviare**, con posta all'account e 30 giorni per decidere, come per le caselle sincronizzate.
6. **Chiave di cifratura della coda.** La stessa, una seconda variabile, o una derivata. **Raccomando la derivata** da `QAMILLE_SYNC_KEY` (P2).

## Da verificare

| Domanda | Dove guardare |
| --- | --- |
| Chi ha tolto per qualche minuto «Programma invio» dall'albero di lavoro il 24 novembre, e se quella modifica tornerà | Il proprietario; `git log` di `desktop/src/components/Composer.tsx` |
| Numero della prossima migrazione enterprise, libreria SMTP, forma di `Entitlement`, pagina `/admin`, nome dei comandi, uso di `with_on_start` | Repository privato dell'enterprise (qui assente) |
| Specifica «core ed enterprise» e ADR-0014 | Citati in `docs/edizioni.md:3` e nella specifica della sincronizzazione, ma non in `docs/` |
| Più osservatori insieme | `server/src/lib.rs:240` |
| `sync rekey` e `sync reset`: come estenderli | `server/src/cli.rs` |
| Limite e modo di caricamento dell'invio con l'API di Gmail per 36 MiB; affidabilità della ricerca per `Message-ID`; rotazione dei token di aggiornamento | Documentazione di Google |
| Uso da un server di token rilasciati a un client «Desktop app»; verifica degli scope ristretti | Google, prima del lancio (già in S8) |
| Fornitori non Gmail davvero supportati; copia in «Inviata» per loro | `app/src/infrastructure/provider_registry.rs` (da riga 60), `mail.rs` |
| Il percorso dell'interfaccia non ha il limite di ritardo chiesto dalla posta online | `app/src/application/scheduler.rs:193-266` contro `docs/posta-online-specifica.md:105` |
| `ServerInfo` tollera campi nuovi in `/v1/info` | `app/src/domain/server_account.rs:30` |
| Come `qamille-mcp` usa la sessione del server oltre il calcolo | `app/src/adapters/mcp/server.rs`, A11 dell'accesso |
| Componenti di Sigrid per ora, elenco con stato, avviso | `desktop/vendor/sigrid/` (generata) |
| Dove l'elenco delle bozze scrive «· Programmata» | `desktop/src/App.tsx:2042` |
| Prova manuale tra due computer Windows veri | **Da fare** |
