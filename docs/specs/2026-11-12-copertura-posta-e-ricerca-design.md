# Copertura completa della posta, ricerca sul server e indicizzazione che regge la casella intera

**Data:** 12 novembre 2026 · **Stato:** proposta · **App:** motore Rust condiviso (`app/`), shell Tauri (`desktop/src-tauri/`), interfaccia React (`desktop/src/`) · **Riferimenti:** [posta online e indice locale](../posta-online-specifica.md), [indicizzazione completa](../indicizzazione-completa.md), [ricerca email](../ricerca-email-specifica.md), ADR-0005 (archivio unico per account), ADR-0007 v3 (concorrenza e reattività), ADR-0010 v2 (conversazioni nella lista) · **Decisione:** ADR da redigere all'approvazione · **Piani:** eseguiti e rimossi il 17 novembre 2026 (restano nella storia di git)

Le parti segnate «scelta mia» sono decisioni di chi scrive, da controllare.

## Problema riportato

Il proprietario, 12 novembre 2026: «la ricerca per adesso cerca solo sulle mail scaricate, non va indietro, e poi l'indicizzazione e il download lo fa solo per le mail più recenti, io non vedo mail precedenti».

## Diagnosi

Rilievi sul codice del ramo `main` al commit `2b33f18`. I numeri di riga sono quelli di quel commit.

### D-1. La sincronizzazione conosce solo gli ultimi 200 messaggi della Posta in arrivo

- `app/src/infrastructure/mail_cache_download.rs:234-261` (`fetch_policy_extended`): per ogni cartella fa `UID SEARCH ALL`, ordina dal più recente e, se l'account non è «tutto offline», tronca a `fetch_count.clamp(10, 500)`. `fetch_count` vale 200 per i nuovi account (`app/src/application/accounts.rs:59`, `app/src/infrastructure/mail.rs:33`).
- `folder_plan` (`:34-90`) aggiunge le altre cartelle (Archivio, Inviata, Cestino, Spam, Bozze) **solo** con `keep_all_offline`. Senza, la sincronizzazione legge solo `INBOX`.
- Non esiste un cursore verso il passato: ogni sincronizzazione rilegge gli stessi ultimi 200. I messaggi più vecchi non entrano mai dalla sincronizzazione.
- `reconcile_known` (`:526`) conserva i messaggi già noti fuori dalla finestra (test `old_inbox_outside_window_and_unverified_legacy_messages_are_left_intact`): quello che entra per altre vie resta.

### D-2. La posta vecchia entra solo dall'indicizzazione, e l'indicizzazione è lentissima

L'indicizzazione percorre tutte le cartelle IMAP dal più recente al più vecchio (`app/src/domain/indexing.rs:73-120`, `FolderProgress::plan` e `advance`) e, quando incontra un messaggio sconosciuto, lo aggiunge all'archivio locale (`app/src/application/indexing.rs:653`, `store.emails.push`). È l'unica strada da cui oggi compare la posta precedente. Il percorso costa:

1. **Un messaggio per giro e 2 secondi di pausa fissa.** Il filo `qamille-mail-index` chiama `indexer.step()` e poi dorme 2 s in ogni caso, anche dopo un giro con lavoro (`desktop/src-tauri/src/main.rs:142-153`). `step` torna dopo il primo account che ha fatto progressi (`indexing.rs:370`), e `step_account` tratta un messaggio (`indexing.rs:552-556`).
2. **Due connessioni IMAP complete per messaggio.** `GmailIndexSource::next` apre e chiude una sessione (TLS e login OAuth) per leggere un'intestazione (`app/src/infrastructure/mail_index_source.rs:161`); `content` ne apre un'altra per scaricare il corpo (`:265`). Ogni `next` rifà anche `UID SEARCH` su una finestra di 2048 UID per prenderne uno solo (`:196-205`).
3. **Manutenzione e letture dell'archivio a ogni giro.** `step` chiama `reconcile_storage()` (`indexing.rs:338`), che legge e riscrive l'archivio intero in una transazione (`app/src/infrastructure/sqlite_store.rs:667-673`); poi `read()` dell'archivio intero (`indexing.rs:340`, `:411`); poi almeno due `update` per messaggio (`indexing.rs:620`, `:713`), ognuno dei quali rilegge e copia tutto (D-3).
4. **Cartelle in fila e account a turno.** `step_account` prende la prima cartella non completa (`indexing.rs:528-533`): la Posta in arrivo va finita prima di passare alle altre. Gli account si alternano a ogni giro (`indexing.rs:341-347`).
5. **Fermo senza modello.** Se il modello di ricerca (EmbeddingGemma 2, circa 288 MB) non è installato, ogni account resta in `waiting_model` e non si scansiona nulla (`indexing.rs:348-361`): nemmeno le intestazioni, quindi niente posta vecchia.

Stima (scelta mia, non misurata): da 4 a 6 secondi per messaggio nel caso migliore, meno di mille messaggi l'ora, e il costo cresce con la dimensione dell'archivio (D-3). Una casella di 30.000 messaggi richiede giorni con l'app aperta.

### D-3. Ogni lettura e ogni scrittura costano quanto l'intera casella

ADR-0007 v3 (decisione 1) ha portato l'archivio in SQLite, ma il port `StoreRepository` (`app/src/application/ports.rs:9-11`) è ancora un aggregato: `read() -> Store` e `transaction(FnMut(&mut Store))`.

- `SqliteStoreRepository::read` legge tutte le tabelle e costruisce l'intero `Store` (`sqlite_store.rs:614-624`, `read_store :1350`).
- `write_attempt` rilegge l'intero `Store` dentro la transazione, lo copia due volte (`before`, `store`), esegue l'operazione e confronta tutto (`sqlite_store.rs:538-551`); `write_store` costruisce le righe di **ogni** email per confrontarle con le precedenti (`sync`, `:1446-1490`) e ripassa i figli di ogni email (`write_children`, `:1599-1602`); `portable()` copia l'archivio ancora una volta per svuotare `body_html` (`:497-504`).
- `MailService::list` legge l'intero archivio e applica `undo::project` a ogni pagina (`app/src/application/mail.rs:405-406`); `WorkspaceService::snapshot` fa lo stesso per contare le cartelle (`app/src/application/workspace.rs:40-58`).
- `usage_with_storage_bytes` serializza in JSON ogni email per misurarne i metadati (`app/src/domain/storage.rs:276-292`), a ogni sincronizzazione (`mail.rs:1686-1693`).
- La ricerca lessicale chiede all'indice, **per ogni email**, se la sua voce è aggiornata (`app/src/infrastructure/lexical_index.rs:288-293`, `Reader::current`): una query SQLite per messaggio a ogni ricerca.

Con 200 messaggi questo non si vede. Con la casella intera (ADR-0007 v3 cita caselle reali da 30.000 messaggi, circa 2 KB ciascuno) ogni clic diventa decine o centinaia di millisecondi, e l'indicizzazione rallenta man mano che procede. Portare tutta la posta nell'archivio (D-1, D-2) senza correggere questo peggiora l'app.

### D-4. «Cerca sul server» ignora il testo e scarica la casella

- `MailService::search_remote` passa al provider solo l'intervallo di date della ricerca (`mail.rs:808-812`, `parsed.range()`), non il testo né i filtri.
- `mail::search_account` (`app/src/infrastructure/mail.rs:712-770`) costruisce la ricerca IMAP con `server_search_keys` (`:693-698`): senza date è `UNDRAFT`, cioè **tutti** i messaggi. Per ognuno, in Posta in arrivo, Inviata, Cestino, Spam e Archivio, scarica il messaggio intero (`BODY.PEEK[]`), lo analizza, estrae il testo degli allegati e tiene tutto in memoria. Archivio contiene già Posta in arrivo e Inviata: molti messaggi passano due volte.
- Fallisce se Gmail non espone Inviata, Cestino o Spam via IMAP (`:701-707`) e si interrompe del tutto se la casella cambia durante la ricerca (`:744`).
- I risultati abbinati restano solo in memoria (`server_payloads`, `mail.rs:413-436`) e spariscono alla ricerca successiva.

Su una casella vera la ricerca sul server non termina: il pulsante esiste, ma non recupera la posta vecchia.

### D-5. La ricerca locale vede solo quello che c'è sul computer

È il comportamento progettato (`docs/ricerca-email-specifica.md:277`), e l'avviso «Risultati locali: parte della posta sul server potrebbe mancare» c'è già (`mail.rs:726`). Con D-1, D-2 e D-4 però la parte mancante è quasi tutta la casella, e l'avviso non dice quanta posta c'è sul computer né porta a una ricerca sul server che funzioni.

## Obiettivi

1. **Tutta la posta in elenco.** Ogni messaggio Gmail fuori da Spam e Cestino compare nell'elenco della sua cartella con mittente, oggetto, data, stato ed etichette, senza scaricare corpi o allegati. Spam e Cestino si recuperano con la stessa logica quando l'utente li apre o li cerca (wave 3).
2. **Tempi.** Su una casella di 50.000 messaggi e una rete di 20 Mbit/s, il recupero delle intestazioni finisce in meno di 15 minuti con l'app aperta; i primi risultati vecchi compaiono entro un minuto dall'avvio (scelta mia).
3. **Ricerca sul server vera.** «Cerca sul server» manda a Gmail il testo e i filtri, riceve le prime 50 corrispondenze in meno di 5 secondi su una casella di 100.000 messaggi, le mostra subito con «Altri risultati dal server», e le conserva come normali righe dell'archivio.
4. **Indicizzazione veloce.** Il costo IMAP e d'archivio per messaggio scende sotto i 100 ms in media; il resto è il modello. Nessuna pausa quando c'è lavoro.
5. **Scala.** Con 100.000 messaggi nell'archivio: pagina dell'elenco ≤ 150 ms, un'azione su un messaggio (letto, stella, archivia) ≤ 50 ms di scrittura, un lotto dell'indicizzazione o del recupero ≤ 250 ms di scrittura, `workspace_snapshot` ≤ 100 ms, memoria residente aggiuntiva ≤ 300 MB. Misure su una macchina di sviluppo con il test di scala della wave 2.
6. **Copertura dichiarata.** L'utente vede per ogni account quanta posta è in elenco rispetto al server e quanta è indicizzata per il contenuto, e la ricerca locale dice se sta cercando su una casella incompleta.

## Non obiettivi

- Provider diversi da Gmail: oggi solo Gmail ha `sync` tra le capacità (`app/src/infrastructure/gateways.rs:163`). Il codice resta per cartelle IMAP generiche dove costa poco, ma le prove sono su Gmail.
- Copia offline dei corpi: resta la scelta esplicita di ADR-0005 e della specifica della posta online.
- Riscrivere il port `StoreRepository` come un insieme completo di query SQL. La wave 2 lascia l'aggregato e lo rende economico sui percorsi caldi.
- Sync tra dispositivi o server di calcolo: sono le specifiche del server di calcolo e dell'accesso con password.

## Anti-obiettivi (un fallimento anche se funziona)

- Il recupero dello storico scarica corpi o allegati.
- La ricerca sul server scarica messaggi interi per decidere se corrispondono.
- Un messaggio presente in più cartelle Gmail compare due volte.
- L'elenco salta, perde righe caricate o muove lo scroll mentre arriva lo storico (INV-007 di ADR-0007 v3).
- Il recupero o l'indicizzazione rallentano le azioni dell'utente: un clic aspetta un lotto in scrittura.
- Una seconda superficie d'attività con percentuali accanto all'indicatore unico (INV-009 di ADR-0007 v3).
- Il conteggio «non letti» della Posta in arrivo cresce di colpo di migliaia senza che l'utente capisca perché.
- Un messaggio cancellato sul server che resta in elenco come se esistesse, o uno esistente cancellato in locale perché una ricerca non l'ha restituito.

## Decisioni

Le decisioni sono divise per wave. Ogni wave è autonoma: si può spedire da sola e lascia l'app funzionante.

### Wave 1. Ricerca sul server vera e indicizzazione a lotti

#### W1.1 Ricerca sul server con la ricerca di Gmail

- **Una cartella.** La ricerca si fa nella cartella con il ruolo `\All` (Archivio, «Tutti i messaggi»), che contiene tutto tranne Spam e Cestino. Spam e Cestino si interrogano solo se la query lo chiede (`in:spam`, `in:trash`, `in:anywhere`). Senza `\All` esposta via IMAP la ricerca dice «Attiva "Tutti i messaggi" in Gmail → Impostazioni → Etichette → Mostra in IMAP» e non prova altre cartelle. Non si richiedono più Inviata, Cestino e Spam (fine di `validate_search_folders`).
- **Traduzione della query.** `ParsedQuery` (`app/src/domain/search.rs:112-150`) si traduce in una stringa della ricerca Gmail, mandata con `UID SEARCH CHARSET UTF-8 X-GM-RAW <stringa>`:

  | Nodo o atomo di Qamille | Ricerca Gmail | Note |
  | --- | --- | --- |
  | `Text(t, exact)` | `"t"` se `exact`, altrimenti `t` | Virgolette interne raddoppiate come `\"` |
  | `Person("from", v)`, `to`, `cc` | `from:v`, `to:v`, `cc:v` | |
  | `Subject(v)` | `subject:(v)` | |
  | `Body(v)` | `v` | Gmail non distingue corpo e intestazioni: il filtro esatto si riapplica in locale dopo l'apertura |
  | `Date{start, end}` | `after:<secondi>` `before:<secondi>` | Secondi Unix: Gmail li accetta e così non conta il fuso del server |
  | `Folder("inbox"/"sent"/"spam"/"trash"/"anywhere")` | `in:inbox`, `in:sent`, `in:spam`, `in:trash`, `in:anywhere` | `archive` → `-in:inbox -in:sent`; `tasks`, `snoozed` sono locali: si omettono e si filtra in locale |
  | `Label(v)` | `label:"v"` | |
  | `Has("attachment")` | `has:attachment` | `document`, `spreadsheet`, `presentation` → `has:document`, `has:spreadsheet`, `has:presentation`; `reminder` è locale |
  | `Filename(v)` | `filename:v` | |
  | `FileType(v)` | `filename:v` | Estensione |
  | `State("read"/"unread"/"starred")` | `is:read`, `is:unread`, `is:starred` | Gli altri stati sono locali |
  | `Category`, `Account` | — | Locali; `Account` sceglie a quali account mandare la ricerca |
  | `Not(n)` | `-(n)` | Solo se `n` si traduce; altrimenti si omette e si filtra in locale |
  | `And(a, b)` | `a b` | |
  | `Or(a, b)` | `{a b}` | Solo se entrambi si traducono; altrimenti l'intero `Or` si omette e si filtra in locale |

  Ciò che non si traduce si omette dalla stringa e si riapplica in locale sui risultati (`ParsedQuery::matches` sui metadati). Una query che si riduce a nulla (per esempio solo `is:task`) non va al server e la ricerca dice «Questa ricerca riguarda dati che esistono solo su questo computer». La funzione di traduzione è pura, nel dominio (`app/src/domain/search_remote.rs`), con test per riga della tabella.
- **Trasporto.** Il crate `imap` 2.x non gestisce i letterali IMAP per testo non ASCII: il comando va sul canale grezzo di `app/src/infrastructure/mail_extensions.rs` (`Extensions::run`), che già invia comandi Gmail che il crate non sa leggere. Un testo ASCII senza virgolette viaggia come stringa quotata; altrimenti come letterale `{n}` con l'attesa della continuazione `+`. Se il canale grezzo non regge la continuazione, il costruttore la aggiunge lì (è l'unico posto che parla IMAP a mano).
- **Solo metadati.** Per gli UID trovati: ordine decrescente (UID più alto = più recente nella cartella `\All`), primi 50, poi `UID FETCH` di `UID FLAGS RFC822.SIZE INTERNALDATE BODYSTRUCTURE BODY.PEEK[HEADER.FIELDS (FROM TO CC BCC SUBJECT DATE MESSAGE-ID IN-REPLY-TO REFERENCES LIST-UNSUBSCRIBE CONTENT-TYPE)]`, più `X-GM-MSGID`, `X-GM-THRID` e `X-GM-LABELS` dal canale grezzo. Nessun `BODY.PEEK[]`. Il corpo si scarica solo quando l'utente apre il messaggio, come per ogni riga `remote_only` (`Job::ReadBody`).
- **Etichette e cartella logica da `X-GM-LABELS`.** `\Inbox` → `Inbox`; `\Sent` → `Sent`; `\Trash` → `Trash`; `\Spam` → `Spam`; `\Draft` → si scarta; nessuna delle precedenti → `Archive`; le etichette utente vanno in `labels`; `\Starred` e `\Important` diventano stato e non etichette. La funzione è pura (`app/src/domain/gmail_labels.rs`) e serve anche alla wave 3. `X-GM-LABELS` restituisce nomi in UTF-7 modificato e con virgolette: la lettura usa il decodificatore UTF-7 già usato per i nomi delle cartelle se c'è, altrimenti uno nuovo nello stesso file.
- **Paginazione.** La ricerca restituisce `{results, total_on_server, next_cursor}`; `next_cursor` è l'ultimo UID mostrato più la `UIDVALIDITY`. «Altri risultati dal server» chiede i 50 successivi senza rifare la ricerca se la sessione è ancora aperta, altrimenti la rifà e salta fino al cursore. Una `UIDVALIDITY` cambiata ricomincia dalla prima pagina.
- **Risultati come righe vere.** I messaggi trovati entrano nell'archivio come righe `remote_only` con identità verificata (`remote_global_id`), deduplicati con `mail_identity::same_message` come fa oggi l'indicizzazione. Non c'è più `server_payloads` in memoria: un risultato trovato sul server resta in elenco nella sua cartella, come se l'avesse portato la sincronizzazione. Una riga già nota si arricchisce di etichette, posizione remota e stato, senza perdere lo stato locale (attività, promemoria, assegnazioni).
- **Errori.** «La casella è cambiata durante la ricerca» non interrompe più tutto: si scartano gli UID spariti e si continua. Rete persa → risultati già ricevuti restano e compare «Riprova»; annullamento come oggi (`cancel_remote_search`).
- **Interfaccia.** L'avviso della ricerca locale (`mail.rs:723-731`) aggiunge la copertura: «Cerchi in 1.240 messaggi su 48.300 di questo account. Cerca sul server». Dopo «Cerca sul server» i risultati del server si fondono con quelli locali nell'ordine scelto e l'avviso diventa «Risultati dal server: 50 di 312. Altri risultati dal server». Il numero di messaggi sul server viene dal conteggio della wave 3 quando c'è; prima della wave 3, dal `EXISTS` della cartella `\All` letto dalla ricerca stessa.

#### W1.2 Indicizzazione a lotti con una sessione riusata

- **Port.** `IndexSource` (`app/src/application/indexing_ports.rs`) passa da `next` e `content` per messaggio a una sessione: `open(account) -> Box<dyn IndexSession>`, con `next_batch(folder, max) -> IndexBatch{progress, emails}` e `contents(emails) -> Vec<(id, AppResult<IndexContent>)>`. Una sessione vive al più 10 minuti o 500 messaggi (scelta mia), poi si chiude e se ne apre un'altra: un account non tiene una connessione aperta per sempre e il token OAuth si rinnova.
- **Lotti.** `next_batch` fa un solo `UID SEARCH` per finestra di UID e restituisce fino a `max` = 25 messaggi della finestra (non uno), con intestazioni e struttura in un solo `UID FETCH`. `contents` scarica i corpi del lotto in un solo `UID FETCH (UID RFC822.SIZE)` e poi `UID FETCH BODY.PEEK[]` per gruppi il cui totale dichiarato sta sotto 32 MiB, rispettando il limite per messaggio di 32 MiB di oggi (`mail_index_source.rs:16`). Il corpo resta solo in memoria, come oggi.
- **Una scrittura per lotto.** `IndexerService::process` diventa `process_batch`: un solo `update` aggiunge o arricchisce le righe del lotto, poi l'incorporamento gira fuori da ogni lock, poi un solo `update` registra gli esiti (`search_index` di ogni messaggio, cursori della cartella). La deduplicazione tra messaggi dello stesso lotto e con l'archivio resta `same_message`.
- **Cadenza.** Il filo dell'indicizzatore dorme solo quando `step` non ha fatto lavoro; dopo un giro con lavoro riparte subito. Tra un lotto e l'altro cede comunque il passo con un `yield` breve (50 ms, scelta mia), così le scritture dell'utente passano per prime. `reconcile_storage` esce da `step`: lo fa già il servizio di manutenzione (ADR-0007, decisione 2) e, all'avvio, una volta sola.
- **Equità.** Gli account si alternano per lotto, non per messaggio. Dentro un account la Posta in arrivo ha la precedenza solo per la posta nuova (`after_uid`); per lo storico si avanza nella cartella `\All` invece che cartella per cartella, perché contiene tutto una volta sola (le altre cartelle Gmail sono viste della stessa posta). Spam e Cestino si indicizzano dopo `\All`. Per account non Gmail resta l'ordine per cartella di oggi.
- **Modello assente.** Senza modello l'indicizzazione semantica aspetta come oggi; la parte lessicale (BM25, `lexical_index::write`) e l'inserimento delle righe proseguono, così la posta vecchia compare e si trova per parole anche senza il modello. Quando il modello arriva, i messaggi già scansionati vengono incorporati da una seconda passata che legge i corpi solo per loro (stato `search_index.semantic_pending`).
- **Attività.** Il progresso resta nell'indicatore unico come oggi (`announce_progress`); niente nuove superfici.

### Wave 2. L'archivio regge la casella intera

Obiettivo 5. Prima si misura, poi si interviene solo dove la misura supera il bilancio.

#### W2.1 Test di scala

`app/tests/scale.rs`, con `#[ignore]` (si lancia a mano e al cancello di fine wave): genera in una cartella temporanea un archivio SQLite con 3 account e 100.000 email realistiche (etichette, 1–3 allegati descritti, posizioni remote, `remote_only`, nessun corpo), poi misura e stampa: `read()`, `MailService::list` di una pagina di 60 righe in Posta in arrivo con conversazioni e senza, `workspace_snapshot`, `apply_actions` su un messaggio (letto), un `update` che aggiunge 25 righe, una ricerca lessicale di una parola, la memoria residente prima e dopo. Le soglie sono quelle dell'obiettivo 5; il test fallisce se una misura le supera di oltre il 50%, così regge macchine diverse ma prende le regressioni vere.

#### W2.2 Istantanea condivisa per le letture

- `StoreRepository` guadagna `read_shared() -> AppResult<Arc<Store>>`, con un'implementazione predefinita che avvolge `read()`.
- `SqliteStoreRepository` tiene l'ultima istantanea in un `Arc<Store>` insieme a `PRAGMA data_version` della sua connessione e alla coda del `change_feed`. `read_shared` restituisce l'istantanea se nessuno ha scritto nel frattempo (stesso `data_version`: lo cambiano le scritture di altre connessioni; le scritture di questo processo aggiornano l'istantanea dopo il commit). Altrimenti la ricostruisce.
- I lettori caldi passano a `read_shared`: `MailService::list`, `search_info`, `search_suggestions`, `WorkspaceService::snapshot`, `IndexerService::step` e `step_account`, `sync_snapshot`, la ricerca lessicale. `undo::project` lavora su un riferimento e restituisce solo ciò che serve alla pagina invece di copiare l'archivio. `read()` resta per chi deve modificare una copia.
- Il processo MCP e il suo worker condividono il database ma non la memoria: ognuno ha la propria istantanea, invalidata da `data_version` come sopra.

#### W2.3 Scritture che costano quanto le righe toccate

- **Base della transazione dall'istantanea.** Dentro `BEGIN IMMEDIATE`, se `data_version` è uguale a quello dell'istantanea, `write_attempt` parte dall'istantanea invece di rileggere tutto (`read_store`). La garanzia di oggi resta: nessuna scrittura di un altro processo va persa, perché un `data_version` diverso forza la rilettura.
- **Confronto per struttura, non per riga serializzata.** `Email` e i suoi figli (`Attachment`, `RemoteLocation`, stato dell'indice) derivano `PartialEq`. `sync` confronta prima il record vecchio con il nuovo e costruisce i valori della riga solo se differiscono; `write_children` gira solo per email nuove o cambiate. Il `seq` che conserva l'ordine resta com'è (`sync :1466-1471`).
- **Niente copie inutili.** `portable()` svuota `body_html` e persiste i percorsi solo sulle email cambiate; `before` non si copia più per intero ma si tiene come riferimento all'istantanea.
- **Inserimento in blocco.** Per recupero, indicizzazione e ricerca sul server: `StoreRepository::upsert_remote_metadata(account_id, rows: Vec<Email>) -> AppResult<Vec<u64>>` inserisce o arricchisce righe `remote_only` in una transazione mirata (SQL diretto sulle tabelle `emails`, `email_labels`, `remote_locations`, `attachments`), con deduplicazione per `remote_global_id` (indice nuovo `emails_by_global_id (account_id, remote_global_id)`) e poi per `message_id`, scrive il `change_feed` come ogni altra scrittura, e aggiorna l'istantanea. Assegna gli id con lo stesso contatore `next_id`. La regola d'arricchimento è quella di oggi in `IndexerService::process` (`indexing.rs:627-651`), spostata in una funzione del dominio usata da entrambi i percorsi.
- **Misure dell'archivio.** `metadata_bytes` e `serialized_bytes` di `storage.rs` non serializzano più ogni email a ogni misura: la dimensione dei metadati diventa una colonna `metadata_bytes` calcolata alla scrittura della riga, sommata con SQL. La regola di quota di ADR-0005 non cambia.
- **Ricerca lessicale.** `Reader::current` diventa una sola lettura di `(id, hash)` dell'account in una mappa per ricerca (`lexical_index.rs:288-293`).
- **Conteggi delle cartelle.** `snapshot` conta con l'istantanea condivisa in una passata sola invece di sei filtri (`workspace.rs:43-58`).

Se il test di scala resta sopra soglia dopo W2.2 e W2.3, la wave 2 aggiunge l'elenco paginato in SQL per la sola vista senza conversazioni (`ORDER BY date DESC LIMIT/OFFSET` su `emails_by_folder`) e lo dichiara nel messaggio di consegna; non si va oltre senza una nuova decisione.

### Wave 3. Tutto lo storico in elenco

#### W3.1 Recupero delle intestazioni verso il passato

- **Lavoro nuovo dell'account worker.** `Job::Backfill` (`app/src/application/account_worker.rs:85`) con priorità più bassa di `Sync`, `Act`, `ReadBody` e `Prefetch`: un'azione dell'utente passa sempre prima. Lo accoda il servizio di sincronizzazione dopo ogni sincronizzazione riuscita finché il recupero dell'account non è completo, e all'avvio.
- **Cartella.** Su Gmail si percorre la cartella `\All` dal più recente al più vecchio, per lotti di 500 UID; poi Spam e Cestino, solo se l'utente ha scelto «Includi Spam e Cestino» (spento di base) oppure quando li apre (W3.3). Le cartelle logiche vengono da `X-GM-LABELS` con la funzione di W1.1, quindi un messaggio è una riga sola qualunque sia il numero delle sue etichette.
- **Cosa si legge.** Le stesse voci della ricerca sul server (W1.1): intestazioni selezionate, flag, dimensione, data interna, struttura, `X-GM-MSGID`, `X-GM-THRID`, `X-GM-LABELS`. Niente corpi, niente allegati, niente file.
- **Cursore.** Nell'account, `history: HistoryProgress{uid_validity, before_uid, highest_seen, total, fetched, complete, last_error}` (colonna JSON nella tabella `accounts`, come `search_index`). `before_uid` scende a ogni lotto; una `UIDVALIDITY` diversa ricomincia dall'alto senza cancellare righe (la deduplicazione per `remote_global_id` le ritrova). Il cursore si scrive nella stessa transazione delle righe del lotto: un'interruzione riprende dall'ultimo lotto scritto.
- **Scrittura.** `upsert_remote_metadata` della wave 2, un lotto per transazione.
- **Tempi.** Un lotto di 500 intestazioni selezionate pesa circa 300–500 KB (stima: 600–1000 byte per messaggio); a 20 Mbit/s il trasferimento è sotto il secondo. Il worker non pausa tra un lotto e l'altro se la coda è vuota, ma rilascia la connessione e ne apre una nuova ogni 10 minuti come W1.2.
- **Coordinamento con l'indicizzazione.** Il recupero porta le righe; l'indicizzazione porta i contenuti. Con W3.1 attivo, l'indicizzazione smette di scoprire righe nuove nella cartella `\All` e legge il contenuto delle righe esistenti non ancora indicizzate, dalla più recente: niente doppia scansione IMAP. Per account non Gmail l'indicizzazione resta com'è dopo la wave 1.
- **Quota.** I metadati contano nella quota dell'account come oggi (ADR-0005). Raggiunto il limite, il recupero si ferma con «Spazio dell'account esaurito: la posta più vecchia di <data> non è in elenco. Aumenta il limite in Impostazioni → Archivio locale» e la data del messaggio più vecchio presente. Stima per la scelta del limite: 100.000 messaggi ≈ 150–250 MB di metadati.

#### W3.2 La finestra recente resta veloce

- La sincronizzazione (`fetch_policy_extended`) continua a leggere i messaggi recenti della Posta in arrivo con `fetch_count` per avere la posta nuova in pochi secondi, ma il troncamento non significa più «il resto non esiste»: è il recupero a portarlo.
- Con il recupero completo, la sincronizzazione aggiunge `CONDSTORE`/`X-GM-LABELS` sulla finestra recente della cartella `\All` per accorgersi di messaggi archiviati o rietichettati altrove (un messaggio uscito da `\Inbox` sul web passa in `Archive` anche in Qamille). Se il server non offre `CONDSTORE`, si rilegge la finestra di 500 UID più recenti di `\All` a ogni sincronizzazione.
- `reconcile_known` resta l'unico punto che dichiara un messaggio cancellato, con le prove di oggi; il recupero non cancella mai nulla.

#### W3.3 Interfaccia e copertura

- **Indicatore unico.** Il recupero è un'attività dell'account nell'indicatore unico: «Recupero della posta precedente · 12.400 di 48.300». Niente percentuali altrove (INV-009).
- **Impostazioni → Ricerca** (`desktop/src/components/IndexingSettings.tsx`): per account, «In elenco: 48.300 di 48.300 · Indicizzati per contenuto: 12.100», stato del recupero (in corso, completo, fermo per spazio, errore con motivo), «Includi Spam e Cestino».
- **Elenco.** Le righe arrivate dallo storico entrano nella loro posizione per data; INV-007 vale: un ricaricamento in background non riduce le righe già caricate né muove lo scroll. Le righe `remote_only` aprono il corpo dal server come oggi.
- **Non letti.** I messaggi recuperati arrivano con il loro stato del server. Il contatore della Posta in arrivo conta i non letti veri, come Gmail. Per non far comparire migliaia di non letti senza spiegazione, la prima volta che il recupero porta più di 100 non letti in Posta in arrivo compare un avviso una sola volta: «Sono arrivati 3.214 messaggi non letti più vecchi dalla tua Posta in arrivo di Gmail.» con «Segna come letti quelli più vecchi di 30 giorni» (azione annullabile, che scrive anche sul server) e «Lascia così» (scelta mia).
- **Spam e Cestino.** Aprire Spam o Cestino, se non sono inclusi, avvia il recupero di quella cartella con lo stesso cursore e un messaggio «Sto recuperando i messaggi di questa cartella dal server».
- **Ricerca locale.** L'avviso di W1.1 usa i conteggi del recupero. A recupero completo e indicizzazione completa l'avviso sparisce e la ricerca locale si dichiara completa per metadati e contenuto; con recupero completo e indicizzazione parziale dice «Mittente, oggetto e date: tutta la posta. Testo: 12.100 messaggi su 48.300».

## Esempi

- **EX-1 Casella grande, primo avvio dopo l'aggiornamento.** Account Gmail con 48.300 messaggi, 200 in archivio. Entro un minuto compaiono nell'elenco dell'Archivio messaggi di anni prima; l'indicatore mostra «Recupero della posta precedente · 2.000 di 48.300»; dopo meno di 15 minuti il recupero è completo e la Posta in arrivo ha tutti i suoi messaggi.
- **EX-2 Ricerca sul server.** L'utente cerca `fattura enel` prima che il recupero arrivi al 2021. La ricerca locale avvisa «Cerchi in 9.800 messaggi su 48.300»; «Cerca sul server» restituisce in 2 secondi le prime 50 di 212 corrispondenze, tra cui una mail del 2019, che resta poi nell'Archivio.
- **EX-3 Ricerca non traducibile.** `is:task from:anna` → al server va solo `from:anna`; i risultati si filtrano in locale per `is:task`, che esiste solo sul computer.
- **EX-4 Ricerca solo locale.** `is:snoozed` → nessuna richiesta al server, avviso «Questa ricerca riguarda dati che esistono solo su questo computer».
- **EX-5 Messaggio con tre etichette.** Una mail in Posta in arrivo con le etichette «Clienti» e «2024» è una riga sola in `Inbox` con due etichette, sia dal recupero sia dalla ricerca sul server.
- **EX-6 Interruzione.** L'app si chiude durante il recupero al lotto 37. Alla riapertura riparte dal lotto 38; nessun messaggio doppio.
- **EX-7 `UIDVALIDITY` cambiata.** Gmail rigenera la cartella: il recupero ricomincia dall'alto e ritrova le righe esistenti per `X-GM-MSGID`; nessuna riga doppia, nessuna cancellazione.
- **EX-8 Spazio esaurito.** Limite di 100 MB raggiunto a 40.000 messaggi: il recupero si ferma, Impostazioni → Ricerca dice da quale data manca la posta, l'elenco funziona.
- **EX-9 Indicizzazione senza modello.** Modello non installato: le righe dello storico arrivano, la ricerca per parole funziona sui messaggi indicizzati lessicalmente, la ricerca per significato aspetta il modello.
- **EX-10 Azione durante il recupero.** L'utente archivia un messaggio mentre il recupero scrive lotti: l'archiviazione è visibile entro 100 ms; il lotto successivo aspetta.
- **EX-11 Scala.** Con 100.000 messaggi la pagina della Posta in arrivo si apre in meno di 150 ms e segnare come letto scrive in meno di 50 ms (test di scala).

## Invarianti

- **INV-1.** Il recupero dello storico e la ricerca sul server non scaricano mai corpi o allegati; il corpo arriva solo con l'apertura, la stampa, l'esportazione, l'indicizzazione o la copia offline scelta.
- **INV-2.** Un messaggio Gmail è una sola riga per account, qualunque sia il numero di cartelle ed etichette; l'identità è `X-GM-MSGID` quando c'è.
- **INV-3.** Il recupero e la ricerca sul server non cancellano righe; solo `reconcile_known` dichiara un messaggio sparito, con le prove di oggi.
- **INV-4.** Ogni cursore (indicizzazione, recupero) si scrive nella stessa transazione delle righe che descrive.
- **INV-5.** Un'azione dell'utente non aspetta più di un lotto in scrittura; i lotti cedono il passo tra uno e l'altro.
- **INV-6.** La pagina dell'elenco, un'azione su un messaggio e `workspace_snapshot` rispettano i bilanci dell'obiettivo 5 con 100.000 messaggi, verificati dal test di scala.
- **INV-7.** Il recupero e l'indicizzazione sono attività dell'indicatore unico; Impostazioni → Ricerca mostra conteggi e stati, non attività (INV-009 di ADR-0007 v3).
- **INV-8.** Nessuna scrittura di un altro processo va persa: una base di transazione presa dall'istantanea è valida solo con lo stesso `data_version`.

## Alternative scartate

- **Alzare `fetch_count` o togliere il troncamento.** Ogni sincronizzazione rileggerebbe migliaia di intestazioni e scriverebbe tutto nell'archivio aggregato: più lento a ogni giro, e comunque legato alla sola Posta in arrivo.
- **«Tutto offline» come soluzione.** Porta anche i corpi e gli allegati di tutta la casella: contrario alla posta online (ADR-0005) e allo spazio scelto dall'utente.
- **Scorrere ogni cartella Gmail.** Le cartelle Gmail sono viste della stessa posta: si scaricherebbe ogni messaggio tante volte quante sono le sue etichette.
- **Ricerca IMAP standard (`TEXT`, `FROM`, `SUBJECT`) invece di `X-GM-RAW`.** Su Gmail è più lenta, non conosce le etichette né `has:attachment`, e il risultato non coincide con quello che l'utente vede sul web.
- **Riscrivere subito `StoreRepository` in query SQL.** Tocca ogni servizio; la wave 2 misura prima e interviene sui percorsi caldi con un'istantanea condivisa e scritture mirate.
- **Lasciare lo storico all'indicizzazione semantica.** Lega l'elenco al modello e al costo dell'incorporamento: senza modello, nessuna posta vecchia.

## Ordine e dipendenze

- **Wave 1** non dipende dalle altre: corregge la ricerca sul server e la velocità dell'indicizzazione. Lascia l'archivio aggregato com'è; con la wave 1 la posta vecchia entra dalla ricerca sul server e, più velocemente di oggi, dall'indicizzazione.
- **Wave 2** non dipende dalla 1 ma serve prima della 3: senza, portare tutta la casella nell'archivio rallenta ogni azione (D-3).
- **Wave 3** dipende da entrambe: usa la traduzione delle etichette della 1 e l'inserimento in blocco della 2.

Con il lavoro dell'accesso con password non c'è sovrapposizione di file, salvo `desktop/src-tauri/src/main.rs` (filo dell'indicizzatore qui, comandi là): chi arriva secondo aggiorna il punto di conflitto.

## Verifica

Alla fine di ogni wave, non per singolo passo:

- **Wave 1:** test unitari della traduzione della query e delle etichette; test dell'indicizzatore a lotti con una sorgente finta (una sessione per lotto, una scrittura per lotto, nessuna pausa con lavoro, prosecuzione lessicale senza modello); test di `search_account` con un server IMAP finto (risposte `X-GM-RAW`, `X-GM-LABELS`, letterali); `cargo test` di `app/` e `desktop/src-tauri/`; `npm run build`; prova a mano su un account Gmail vero: «Cerca sul server» trova una mail di anni prima in pochi secondi; l'indicizzazione avanza visibilmente più veloce. La prova a mano si dichiara **non fatta** se non è possibile.
- **Wave 2:** test di scala sotto soglia (stampa delle misure nel messaggio di consegna, prima e dopo); `cargo test` di `app/` completo, compresi i test di concorrenza tra processi (`portable_workspace.rs`, `mail_locks.rs`, `account_worker.rs`); `app/tests/layers.rs` verde.
- **Wave 3:** test del recupero con un server IMAP finto (lotti, cursore, `UIDVALIDITY`, interruzione, quota, deduplicazione per etichette); test Playwright dell'anteprima per l'avviso dei non letti, Impostazioni → Ricerca e l'elenco che non salta; test di scala con 100.000 righe dopo un recupero simulato; prova a mano su un account Gmail vero (EX-1). Revisione del codice sul diff di wave.
