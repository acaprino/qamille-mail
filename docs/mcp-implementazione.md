# MCP Qamille — implementazione e uso

Riferimento: [specifica approvata come ambito](mcp-specifica.md). Data: 2 novembre 2026.

## Avvio

Per aggiungere server e skill ai client AI dall’app, vedere [Installare Qamille negli assistenti](assistenti-installazione.md).

1. Chiudere eventuali versioni già aperte e avviare `artifacts/mcp/qamille-desktop.exe`; aprire **Impostazioni → Accesso agenti**.
2. Impostare il livello generale e salvare. Definire il profilo dell'assistente, gli account/calendari consentiti e le capacità aggiuntive; salvare i permessi del profilo.
3. Copiare la configurazione mostrata nella sezione MCP nel proprio client. Il desktop supporta `--mcp`; il binario separato `qamille-mcp.exe` avvia direttamente il trasporto stdio. Per selezionare un profilo aggiungere `--mcp-profile nome` agli argomenti.
4. Per gli invii con client e finestra chiusi, avviare il processo background. L'avvio all'accesso Windows si abilita esplicitamente nelle impostazioni; non è stato attivato durante l'implementazione.
5. Per Google Calendar aprire **Calendario → Collega calendario** con un account Google OAuth. Il consenso calendario è distinto da quello della posta. Le bozze cloud richiedono anch'esse OAuth e le API Google abilitate nel progetto del client.

Gli eseguibili consegnati sono in `artifacts/mcp/`: `qamille-desktop.exe` e `qamille-mcp.exe`. È pronta una [configurazione del client per questo PC](mcp-client.json); il [catalogo JSON](mcp-tools.json) contiene gli 81 strumenti esposti dall'eseguibile verificato. Nessuna porta HTTP viene esposta dal server MCP. La compilazione di collaudo usa `checks/mcp-target/debug/`; la sua cache fisica è stata spostata sul disco C: e collegata al percorso originale per non saturare D:.

## Funzioni implementate

| Area | Implementazione |
| --- | --- |
| Protocollo e accesso | Trasporto stdio limitato a 1 MiB per richiesta, negoziazione legacy e richieste moderne, catalogo paginato, validazione input, risultati strutturati, profili, isolamento account, risorse con handle, registro metadati e job oltre 10 secondi. |
| Posta e bozze | Ricerca locale/remota/semantica con copertura, lettura e thread, allegati, export EML/MBOX, etichette reali Gmail, firme, risposte/inoltri, Ccn, revisioni, bozze cloud e rilevamento dei conflitti. |
| Invio | Outbox durevole comune a desktop e MCP, messaggio e allegati congelati all'accodamento, chiavi di idempotenza, claim tra processi, programmazione, annullamento, controllo delle revoche, invio della bozza Gmail tramite il provider e stati incerti distinti dagli invii riusciti. |
| Calendario | Calendari locali e Google, consenso separato, cache/sync e proiezione desktop, eventi con ETag, invitati/RSVP/Meet, disponibilità con orari lavorativi e margini, ricorrenze Google, import/export ICS con file gestiti. |
| Meeting e attività | Lettura/ricerca di fonti e trascrizioni versionate, note con revisioni, elaborazione/import/export di audio salvato, job, attività locali da fonti e preferenze con recupero delle modifiche interrotte. Nessuna inizializzazione del microfono nel runtime MCP. |

Il catalogo combina `adapters/mcp/tools.rs`, `adapters/mcp/calendar.rs` e `application/mcp_outbox.rs`. Le porte mail espongono capacità per account e operazioni su bozze/etichette/invio indipendenti dal provider. `ProviderRegistry` seleziona un adattatore per account usando `provider_id`, conservato nell'archivio; gli account legacy mantengono la propria identità. Le credenziali dei nuovi provider hanno chiavi distinte per provider/account. L'adattatore Gmail è il primo concreto; gli altri provider restano connettori da realizzare, come concordato.

## Persistenza e recupero

| Dati | Posizione relativa alla cartella dati |
| --- | --- |
| Posta, bozze e outbox | `store.json`, con lock e sostituzione atomica del repository esistente. Gli snapshot dell'outbox e i relativi byte sono protetti e contabilizzati per account. |
| Snapshot degli allegati in uscita | `outbox-files/`; il messaggio programmato non cambia se viene modificato il file originale. |
| Configurazione MCP | `mcp-config.json`; modificabile dalle impostazioni desktop, non dagli strumenti MCP. Le revoche vengono ricontrollate durante letture, commit e operazioni provider. |
| Calendari e attività | `calendar-state.json`, `mcp-tools.json`; revisioni, identità remote, ricevute di creazione e intenti recuperabili. |
| Job, file e diagnostica | `mcp-jobs/`, `mcp-files/`, `mcp-activity.jsonl`. I risultati dei job lenti sono temporanei (24 ore dopo il completamento); i file hanno handle con scadenza e vincoli di account/profilo. Il registro diagnostico non contiene corpi o credenziali. |

Gli esiti `unknown` e `sent_not_saved` non vengono rispediti automaticamente. Le richieste fallite prima del contatto con il provider possono essere riprese esplicitamente. Le programmazioni in ritardo oltre 15 minuti richiedono revisione. La sospensione o lo spegnimento del PC non consentono un invio puntuale.

I risultati generici delle operazioni lente si leggono con `get_operation` usando l'ID `job:…`; lo stato degli invii è nell'outbox. Le operazioni di separazione di una serie calendario hanno anche `get_calendar_operation`. Un job interrotto durante un effetto remoto richiede verifica dello stato prima di ripetere la richiesta.

La UI nativa legacy usa snapshot completi: un lock di compatibilità impedisce di aprirla insieme ai nuovi processi MCP/background. La UI desktop corrente usa invece i servizi transazionali condivisi.

## Limiti dichiarati

| Caso | Comportamento |
| --- | --- |
| Account reali | Non sono stati collegati, letti o modificati nel collaudo di implementazione. Invii, consenso OAuth e operazioni calendario reali richiedono il successivo collaudo con l'account dell'utente. |
| Ricorrenze | Google gestisce le ricorrenze. La separazione «questa e successive» supporta una singola RRULE; insiemi avanzati con più regole/eccezioni vengono rifiutati esplicitamente. Il calendario locale dichiara l'assenza di ricorrenze native. |
| Bozze Gmail concorrenti | Il confronto della revisione avviene prima della sostituzione remota; Gmail non fornisce qui una transazione atomica con l'archivio locale. Conflitti e risultati incerti restano visibili. |
| Annullamento | È disponibile prima dell'effetto remoto, per gli invii in coda e per i job meeting controllabili. Un job generico con operazione remota già iniziata dichiara `can_cancel: false`. Una mail già accettata dal server non può essere richiamata da questo flusso. |
| Archiviazione | Resta il repository JSON di compatibilità. La migrazione completa SQLite, l'accesso HTTP remoto e i connettori aggiuntivi non sono inclusi. Gli snapshot delle consegne e le chiavi sono mantenuti; la crescita dell'outbox rientra nella quota dell'account. |

## Verifiche

Le prove usano dati sintetici, account simulati e cartelle `checks/` isolate. La build dell'interfaccia e 10 scenari browser hanno superato i controlli, inclusi Ccn salvato/riaperto, calendario locale, permessi in anteprima e regressioni delle schede di composizione.

| Controllo concluso | Esito |
| --- | --- |
| Build web | Riuscita, zero errori e zero avvisi Svelte. Log: `checks/mcp-web-build.log`. |
| Build Windows e server autonomo | Entrambe riuscite. Log: `checks/mcp-desktop-build.log`, `checks/mcp-server-build.log`. Per Tauri è stato ridotto il picco di memoria con `profile.dev.package.tauri-utils.codegen-units=256`. |
| Protocollo reale | Handshake dell'eseguibile desktop e del server autonomo riuscito; 75 strumenti con nomi unici e catalogo paginato esportato in JSON. |
| Binario consegnato | Invio sintetico una sola volta e ripetizione della chiave, organizzazione locale senza permesso Send, revoca durante la connessione e processo separato di estrazione IPC passati. Log: `checks/mcp-binary-smoke.log`. |
| Suite Rust completa | **339 test riusciti, 1 fallito, 6 ignorati**. Log: `checks/mcp-tests.log`. I test MCP, calendario, outbox, protocolli, registro provider, allegati e confini architetturali sono passati. |

Il test non riuscito è `background_indexing::failed_content_keeps_retry_state_without_stalling_the_scan` in `app/tests/support/indexing.rs`: dopo la richiesta di riprova attende `(1,1)` e osserva `(1,2)`. È emerso durante il lavoro parallelo sul censimento e sull'indice BM25; questa consegna ne conserva l'esito senza modificare quel lavoro. La suite generale quindi non è interamente superata. I test su provider e servizi reali rimangono separati e richiedono gli account di prova autorizzati.

Sono passati anche i tre scenari nuovi di `desktop/tests/mcp-ui.mjs` e i sette scenari di `desktop/tests/composer-tabs.mjs`, usando l'anteprima sintetica. La configurazione e i cataloghi sono inclusi anche in `artifacts/mcp/`; le impronte degli eseguibili sono in `artifacts/mcp/SHA256.txt`.

Il controllo di compilazione della UI precedente (`native-ui`) è riuscito; log in `checks/mcp-native-check.log`. Conserva gli avvisi numerici già presenti nel rendering legacy. Entrambi gli eseguibili consegnati sono stati ricompilati dopo l'introduzione dei nuovi campi dell'indice, per mantenere il modello dati allineato al progetto.

Per riprodurre i controlli Rust su questo PC, indicare la cartella della libreria ONNX già installata con `ORT_LIB_LOCATION` e usare `QAMILLE_DATA` per una cartella di prova. È stata usata una cartella di compilazione separata per evitare le altre compilazioni presenti nel progetto, con un solo compilatore attivo dopo gli errori di memoria del sistema.
