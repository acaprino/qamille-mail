---
adr_id: ADR-0007
version: 2
title: Processi indipendenti per account, archivio SQLite e interfaccia reattiva
status: Superseded
author: Alfio Caprino
scope:
  paths:
    - app/src/infrastructure/repository.rs
    - app/src/infrastructure/mail_reader.rs
    - app/src/application/context.rs
    - app/src/application/mail.rs
    - app/src/application/mail_sync.rs
    - app/src/application/undo.rs
    - app/src/application/scheduler.rs
    - app/src/application/events.rs
    - desktop/src-tauri/src/main.rs
    - desktop/src-tauri/src/runtime.rs
    - desktop/src/App.svelte
    - desktop/src/lib/activity.ts
    - desktop/src/components/ActivityIndicator.svelte
    - desktop/src/components/IndexingSettings.svelte
    - desktop/src/components/UndoStack.svelte
    - desktop/src/components/MailList.svelte
  modules:
  domains:
    - storage
    - mail-sync
    - concurrency
    - ui-events
    - activity
governs_symbols:
depends_on:
  - ADR-0005
created_from_task: review-concorrenza-2026-11-04
proposed_on: 2026-11-04
proposed_at_sha: 201d3261d7fc1bc1bcb8113d5dc4e20504c9aba0
accepted_on: 2026-11-04
accepted_at_sha: 16221ef6807cedd2a8faf7381324b89bc23ba191
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: ADR-0007 v3
changes:
  - EX-007
  - EX-008
  - INV-009
  - RA-006
---

# ADR-0007: Processi indipendenti per account, archivio SQLite e interfaccia reattiva

<!-- adr-header:start -->
Version 2 · Superseded by ADR-0007 v3 · proposed 2026-11-04 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

La review `docs/reviews/2026-11-04-concorrenza-e-reattivita.md` ha trovato 46 punti in cui un lavoro aspetta un altro o fa aspettare l'interfaccia, riconducibili a sette cause di fondo:

- tutti i dati della posta (account, email, bozze, azioni in sospeso, coda di invio, impostazioni) vivono in un unico `store.json` protetto da un lock di file esclusivo anche in lettura; ogni lettura analizza tutto il file e ogni `context.update()` lo rilegge tre volte e lo riscrive per intero con `fsync` (`app/src/infrastructure/repository.rs:112-164,448-560`, `app/src/application/context.rs:27-44`);
- un solo mutex `MailService.operation` copre azioni utente e sincronizzazione di tutti gli account ed è tenuto durante le chiamate IMAP (`app/src/application/mail.rs:211`); misurato: 20,5 s all'avvio, 3–6 s per notifica IDLE, un "Fatto" in coda fino a 47 s;
- un thread per tipo di lavoro (undo, scheduler, live sync, indicizzatore), sequenziale e a polling (250 ms, 1 s, 2 s, 5 s);
- eventi verso la UI con payload vuoto e un watcher sulla data di `store.json`, per cui ogni modifica produce più ricaricamenti completi;
- la UI aspetta la conferma del backend prima di aggiornarsi (`busy` globale dopo "Fatto", stato "letto" ritardato, lettore svuotato a ogni apertura, pagine di "Carica altri" perse);
- una connessione IMAP/SMTP/HTTP nuova per quasi ogni operazione (0,2–1,2 s di handshake);
- nessuna superficie unica mostra cosa sta facendo l'app: la sincronizzazione della posta ha un pulsante che gira e un avviso temporaneo (`desktop/src/components/MailSyncFeedback.svelte`), le azioni in conferma una riga in basso a sinistra (`desktop/src/components/UndoStack.svelte:39-47`), l'avanzamento dell'indicizzazione e dell'embedding è visibile solo in Impostazioni con polling ogni 2,5 s (`desktop/src/components/IndexingSettings.svelte:59`), e l'installazione del modello di ricerca e la sincronizzazione dei contatti non hanno alcun segnale.

Domanda: con quale architettura i processi di Qamille diventano indipendenti tra loro e dall'interfaccia, senza lock globali né code uniche?

## Decision Drivers

- Richiesta del proprietario (2026-11-04): "i processi devono essere async e reactive, anche rispetto alla UI; senza blocchi, mono-queue ecc."
- Richiesta del proprietario (2026-11-04): manca un indicatore di quando si sta facendo la sincronizzazione, anche intesa come indicizzazione/embedding. Il proprietario ha confermato che una sola superficie di attività è preferibile a più segnali sparsi (meno posti da ricordare e da controllare; un segnale fisso e silenzioso interrompe meno di avvisi che compaiono e scompaiono).
- Il costo attuale cresce con la dimensione della casella: ~2 KB per email in `store.json`, caselle reali da 30.000 messaggi.
- Il processo MCP (`qamille-mcp`) legge e scrive gli stessi dati: la soluzione deve essere sicura tra processi, non solo tra thread.
- Gmail ammette al massimo 15 client IMAP simultanei per account; una sessione OAuth dura circa quanto il token.
- Invarianti esistenti da preservare: finestra di annullamento 5–30 s; un invio o spostamento il cui esito è andato perso non viene mai ritentato (`app/src/application/undo.rs:162-182`); ADR-0004 e ADR-0005 restano validi.

## Decision

1. **Archivio in SQLite con WAL.** `store.json` viene sostituito da un database SQLite in modalità `journal_mode=WAL` con tabelle per account, email, bozze, azioni in sospeso, coda di invio, impostazioni. Le letture non prendono lock esclusivi e sono concorrenti anche con il processo MCP; ogni scrittura è una transazione breve sulle sole righe toccate. Gli altri file (`contacts.json`, `calendar-state.json`, `session.json` delle riunioni, `rag_index.bin`) sono fuori da questa decisione. La migrazione da `store.json` avviene al primo avvio ed è idempotente; il JSON originale resta sul disco finché la migrazione non è verificata.
2. **Manutenzione fuori dalle scritture.** `reconcile_storage` (recupero indici, misura dello spazio, pulizia della cache) diventa un job di background con il proprio intervallo e la propria coda; nessuna scrittura utente lo esegue in linea.
3. **Un lavoratore per account.** Ogni account Gmail ha un `AccountWorker` con il proprio pool di connessioni IMAP (massimo 4 per account, sotto il limite di 15), una coda con priorità (azioni utente > lettura corpi > sincronizzazione > prefetch/indicizzazione) e il proprio stato di errore. Nessuna operazione di rete viene eseguita tenendo un lock condiviso con altri account o con la UI. Il mutex `MailService.operation` e il pool unico di `MailReader` vengono rimossi.
4. **Scadenze invece di polling.** Undo, scheduler e risvegli dei posticipati si attivano su timer calcolato sulla prossima scadenza o su notifica di nuovo lavoro, non su loop a intervallo fisso. Un commit lento o fallito non blocca gli altri: le azioni in sospeso vengono confermate dal worker dell'account a cui appartengono.
5. **Eventi con contenuto, emessi una volta.** Il backend emette eventi tipizzati dal punto che scrive: `email-changed {account_id, ids}`, `counts-changed {account_id}`, `undo-changed {action}`, `drafts-changed {ids}`, `sync-progress {account_id, phase}`, `index-progress {account_id, percent}`. Il watcher sulla data del file e l'evento generico `workspace-changed` vengono eliminati.
6. **Interfaccia ottimistica e mirata.** Le azioni utente aggiornano subito lo stato locale (riga rimossa, "letto", fissato) e lo riconciliano all'arrivo dell'evento; `busy` è per elemento, non globale; il lettore conserva il messaggio precedente finché arriva il nuovo; la lista applica patch per `ids` e ricarica per intero solo su cambio di vista o filtro, con al massimo un ricaricamento in volo e uno in attesa.
7. **Una sola superficie di attività.** Sincronizzazione per account, indicizzazione/embedding, installazione del modello di ricerca, sincronizzazione dei contatti e azioni in conferma compaiono in un solo indicatore nel piè della barra laterale, alimentato dagli eventi tipizzati della decisione 5 (più `model-progress` e `contacts-sync`). L'indicatore è assente quando non c'è lavoro, mostra l'attività principale con percentuale se nota e un contatore "+N" per le altre, apre un elenco per attività al clic, non mostra attività sotto i 400 ms e marca "ferma" un'attività senza eventi da 60 s. Nessun'altra superficie persistente duplica lo stato di queste attività. Gli errori restano avvisi separati, perché devono farsi notare.
8. **Incrementi spedibili in quest'ordine:** (0) correzioni rapide UI e coalescenza; (1) lock della posta per account e pool di lettura senza lock durante l'I/O; (2) eventi con contenuto e indicatore unico di attività; (3) worker per account e scadenze; (4) SQLite. Ogni passo è misurato con la copia strumentata in `checks/done-latency` e con il test di latenza di EX-001.

## Verified Interview Examples

- **EX-001** (nominal case): Input: sincronizzazione dell'account 2 in corso (download di 200 intestazioni), l'utente preme "Fatto" su un'email dell'account 1 -> Expected: la riga sparisce entro 100 ms; "Salvataggio di 1 azione…" scompare entro 3 s dalla fine della finestra di annullamento, senza aspettare la fine della sincronizzazione.
- **EX-002** (nominal case): Input: l'utente apre un'email non letta -> Expected: grassetto e contatore cambiano subito; il corpo arriva senza che il lettore mostri "Apertura messaggio…" se un messaggio era già aperto; nessun ricaricamento completo della lista.
- **EX-003** (boundary case): Input: l'utente ha premuto "Carica altri" due volte (180 righe), poi arriva una sincronizzazione che aggiorna 3 email -> Expected: la lista resta a 180 righe, le 3 righe vengono aggiornate in posizione, lo scroll non si muove.
- **EX-004** (rejection case): Input: l'account 2 restituisce errore di rete durante la sincronizzazione -> Expected: l'account 1 continua a sincronizzare e a confermare azioni; l'errore compare una sola volta, non a ogni ciclo.
- **EX-005** (boundary case): Input: il processo MCP scrive un'etichetta mentre l'app desktop legge la lista -> Expected: la lettura non aspetta la scrittura; l'app riceve l'evento `email-changed` e aggiorna la riga.
- **EX-007** (nominal case): Input: sincronizzazione dell'account 2, indicizzazione dell'account 1 al 40% e installazione del modello di ricerca in corso insieme -> Expected: un solo indicatore nel piè della barra laterale mostra la sincronizzazione con "+2"; al clic l'elenco ha tre righe (account, fase, percentuale, durata); a lavoro finito l'indicatore sparisce entro 1,5 s; "Salvataggio di N azioni…" non compare altrove.
- **EX-008** (boundary case): Input: un'attività dura 200 ms, oppure non riceve eventi per 60 s -> Expected: nel primo caso nulla compare; nel secondo la riga è marcata "ferma" e non resta attiva senza segnalarlo.
- **EX-006** (boundary case): Input: primo avvio con un `store.json` esistente da 67 email e 2 account -> Expected: il database contiene le stesse 67 email e 2 account; `store.json` resta sul disco; un secondo avvio non duplica nulla.

## Rejected Alternatives

- **RA-001**: store in memoria con un proprietario unico e salvataggio differito su `store.json`; rifiutata perché non risolve la condivisione con il processo MCP e la scrittura resta "tutto il file".
- **RA-002**: ritocchi al JSON attuale (lock condiviso in lettura, manutenzione fuori dalle scritture); rifiutata perché il costo resta proporzionale alla casella e la fila di scrittura resta unica.
- **RA-003**: riscrittura completa in Rust async (tokio + client IMAP async) come primo passo; rifiutata per ora perché il client IMAP in uso è sincrono e il guadagno viene dall'isolamento per account, ottenibile con thread e canali; async resta possibile dentro un worker senza cambiare questa decisione.
- **RA-004**: un lock globale per account ma tenuto durante la rete; rifiutata perché un'azione utente aspetterebbe ancora la sincronizzazione dello stesso account.
- **RA-006**: tenere i segnali separati dove sono oggi (pulsante di sincronizzazione, riga delle azioni, avanzamento in Impostazioni) e aggiungerne uno per l'embedding; rifiutata perché lo stato dell'app resterebbe sparso in più posti da ricordare, e ogni nuovo processo di fondo aggiungerebbe un'altra superficie.
- **RA-005**: mantenere `workspace-changed` come evento unico e debounce lato UI; rifiutata perché la UI non saprebbe cosa è cambiato e dovrebbe comunque ricaricare tutto.

## Enforceable Invariants

- **INV-001**: nessuna chiamata di rete (IMAP, SMTP, HTTP) viene eseguita tenendo un lock del database, di `MailService` o condiviso tra account.
- **INV-002**: una lettura dal database non attende mai una scrittura in corso; nessun lock di file esclusivo protegge le letture.
- **INV-003**: ogni operazione di rete di un account passa dal suo `AccountWorker`; nessun thread serve più di un account; le connessioni IMAP aperte per account sono al massimo 4.
- **INV-004**: ogni evento verso la UI porta `account_id` e gli identificatori toccati; nessun evento con payload vuoto; nessun watcher sulla data dei file.
- **INV-005**: nessun loop di background a intervallo fisso per undo, scheduler e risvegli: si attivano su scadenza calcolata o su notifica.
- **INV-006**: un'azione utente aggiorna lo stato visibile prima della risposta del backend, e nessun flag `busy` disabilita elementi diversi da quello su cui si agisce.
- **INV-007**: un ricaricamento in background non riduce il numero di righe già caricate né muove lo scroll.
- **INV-009**: ogni attività di fondo (sincronizzazione, indicizzazione/embedding, installazione del modello, sincronizzazione dei contatti, azioni in conferma) è visibile nell'indicatore unico e in nessun'altra superficie persistente; l'indicatore è assente senza lavoro e non mostra attività sotto i 400 ms; gli errori non passano dall'indicatore.
- **INV-008**: la finestra di annullamento e la regola "mai ritentare un invio o uno spostamento con esito perso" restano invariate.

## Consequences

- Nuovi moduli: `infrastructure/sqlite_store.rs` (repository SQLite), `application/account_worker.rs` (coda e pool per account), `application/events.rs` (tipi degli eventi), migrazione `store.json` → SQLite.
- Rimossi: `JsonStoreRepository` come repository di produzione (resta per i test e la migrazione), `MailService.operation`, il pool unico di `MailReader`, il watcher mtime in `desktop/src-tauri/src/main.rs`, i thread `qamille-undo` e `qamille-scheduler` nella forma attuale.
- Frontend: `App.svelte` perde `refresh()` come risposta agli eventi e acquista handler per evento; `busy` diventa per elemento; `UndoStack` smette di girare quando non ci sono notifiche visibili e perde la riga "Salvataggio di N azioni…", assorbita dall'indicatore.
- Nuovi: `desktop/src/lib/activity.ts` (registro delle attività, funzioni pure), `desktop/src/components/ActivityIndicator.svelte`; `IndexingSettings.svelte` smette di fare polling e legge il registro; eventi aggiuntivi `ModelProgress` e `ContactsSync` in `app/src/application/events.rs`.
- Il processo MCP apre lo stesso database SQLite in WAL; `store.lock` e `legacy-writer.lock` perdono la funzione di serializzare le letture.
- ADR-0005 resta valido: la politica di archiviazione per account non cambia; cambia solo dove e quando viene applicata (job di manutenzione, decisione 2).
- Costo: 3–5 settimane di lavoro in 5 incrementi; il passo 4 (SQLite) è il più grande e l'unico che tocca il formato dei dati. L'indicatore unico (decisione 7) aggiunge circa un giorno, nell'incremento 2.

## Confirmation

- Test Rust: lettura concorrente alla scrittura su SQLite WAL (EX-005, INV-002); migrazione idempotente (EX-006); nessuna rete sotto lock verificata con un gateway finto che registra lo stato dei lock (INV-001); errore di un account che non ferma l'altro (EX-004, INV-003).
- Test desktop (Playwright): "Carica altri" seguito da evento in background (EX-003, INV-007); "Fatto" con sincronizzazione simulata lenta (EX-001, INV-006); apertura di una non letta (EX-002); indicatore unico con più attività, durata minima e attività ferma (EX-007, EX-008, INV-009) in `desktop/tests/activity.mjs`.
- Misura: la copia strumentata in `checks/done-latency` ripetuta dopo ogni incremento; obiettivo per EX-001 ≤ 3 s dalla fine della finestra di annullamento con sync in corso.
- Revisione: ogni wave verifica INV-001…INV-009 sui file in scope.
