# Server di calcolo: tutto locale o server remoto

**Data:** 7 novembre 2026 · **Stato:** proposta · **App:** desktop Tauri (`desktop/`) con il motore Rust condiviso (`app/`) e un nuovo binario server · **Decisione:** ADR da redigere all'approvazione · **Milestone:** 1 di 2

> **Aggiornamento 17 novembre 2026:** la modalità **locale** (D1, `serve --local` figlio dell'app) è **rimossa**: l'app è un client e il calcolo sta sempre su un server, anche sul proprio PC con Docker. Vedi [core open source ed enterprise](2026-11-17-core-enterprise-design.md), E10.

> **Aggiornamento 12 novembre 2026:** l'accesso al server remoto passa a username e password, con un'edizione self-hosted e una Professional preimpostata: vedi [accesso con password ed edizioni](2026-11-12-accesso-password-edizioni-design.md). Sono **sostituiti** D5 (token personali e comandi `user add/revoke/list`), EX-5 e ogni «token personale» o «token remoto» in questo documento, che va letto come «chiave del dispositivo della sessione»; `compute_url` di D7 sta nella sessione del vault. Sono **completati** D1 (`serve` e `serve --local`), D7 (consenso prima delle credenziali, sezione Calcolo), D8 (`401` dopo l'ingresso) e il limite «per token» di D2, che vale per utente. Sono **riformulati** INV-2, INV-4, INV-10 ed EX-10.

## Scopo

Qamille oggi esegue tutto il calcolo pesante sulla macchina dell'utente: embedding con EmbeddingGemma 2 per l'indice semantico, trascrizione con whisper.cpp, estrazione del testo dagli allegati. Questa specifica separa quel calcolo in un **server Qamille** che gira in due modi con lo stesso binario:

- **locale**, avviato e governato dall'app desktop come processo figlio su `127.0.0.1`, senza configurazione: è il comportamento predefinito e deve risultare identico a oggi per l'utente;
- **remoto**, in un container Docker su un server dell'azienda, raggiunto via HTTPS con un token personale per ogni utente.

L'utente sceglie dalle Impostazioni: "su questo computer" oppure "server remoto" con indirizzo e token. L'indice, i vettori e tutti i dati restano sulla macchina dell'utente in entrambi i modi.

Richieste del proprietario (7 novembre 2026): "fare indicizzazione con embedding gemma 2 in un server gestito remoto invece che in locale sulla macchina, così come usare modelli di registrazione e trascrizione STT remoti"; "noi dobbiamo creare l'app desktop, e poi un server BE, che in locale può essere avviato bindato all'app desktop come processo locale, oppure lo possiamo deployare con docker in remoto come server nostro"; "l'utente deve poter scegliere, tutto locale o configura un server remoto"; utenti del server remoto: "più utenti dell'azienda se si vuole"; modello di embedding: "embedding gemma 2" sia in locale sia in remoto; sync e push tra dispositivi: "lo farei in una seconda milestone, adesso 1 e 3 ok" (indice locale e calcolo remoto senza stato).

## Obiettivi

1. Un solo binario `qamille-server`, due modalità di esecuzione (figlio locale, container remoto), un solo protocollo HTTP. L'app desktop parla sempre HTTP con "un server", non sa né le importa quale.
2. Il server della milestone 1 è **senza stato**: non conserva testo, audio o vettori dell'utente oltre la durata della richiesta. Conserva solo i file dei modelli e contatori di utilizzo.
3. Indice, vettori, BM25, cache delle trascrizioni e archivio restano in `data/` sulla macchina dell'utente, scritti solo dall'app.
4. Il server remoto riconosce più utenti con token personali e limiti per utente. Nessuno stato per utente oltre a questo.
5. Il server compila e gira su Linux x64 (Docker, CPU) e su Windows x64 (modalità locale).
6. Le protezioni di oggi restano: limite di memoria del processo di calcolo, rilascio del modello dopo inattività, pausa su memoria insufficiente, distinzione tra errori transitori ed errori del contenuto, cancellazione e autorizzazione per ogni richiesta remota di trascrizione.
7. La milestone 2 (server con stato: sync tra dispositivi, push, lavoro ad app chiusa) aggiunge rotte e stato a questo server, non un servizio nuovo.

## Non obiettivi della milestone 1

- Indice o vettori sul server. Restano locali.
- Sincronizzazione tra dispositivi, notifiche push, accesso del server alla casella di posta. Milestone 2.
- Cambiare la configurazione dell'assistente LLM (`ai_provider`, `ai_base_url`, `ai_key`, `ai_model`). Il server non fa da proxy per l'LLM.
- Accelerazione GPU. L'immagine è CPU; la struttura non la impedisce.
- Scelta separata per servizio (embedding su un server, STT su un altro). Una sola scelta copre tutto il calcolo; la struttura la permette dopo.
- Single sign-on / OIDC. Token personali emessi dall'amministratore.
- Puntare l'embedding a un servizio pubblico generico compatibile OpenAI. Il protocollo lo rende possibile in futuro; la milestone 1 richiede un server Qamille perché embedding delle immagini ed estrazione documenti sono estensioni nostre.

## Anti-obiettivi

Il lavoro è fallito, anche se funziona, se:

- la modalità remota cambia in silenzio i risultati della ricerca perché il modello o le dimensioni dei vettori differiscono da quelle dell'indice;
- il server registra, logga o conserva testo delle mail o audio;
- esistono due percorsi di codice per l'inferenza (uno in-process per il locale, uno HTTP per il remoto) che possono divergere;
- il test di stratificazione `app/tests/layers.rs` o l'invariante INV-009 dell'ADR-0007 v3 (una sola superficie di attività) vengono violati;
- il passaggio locale/remoto perde o invalida l'indice senza chiedere.

## Stato del codice e vincoli

Fotografia del 7 novembre 2026 sul ramo `feat/concorrenza-e-reattivita`, che ha modifiche non ancora committate in `app/src/infrastructure/embedding_worker.rs`, `app/src/application/indexing.rs`, `app/src/infrastructure/rag_segments.rs`, `app/src/infrastructure/mail_index_source.rs`. Questa specifica costruisce sulla semantica di quelle modifiche e va implementata dopo che sono state consolidate.

**Embedding oggi.** `ModelClient` (`app/src/infrastructure/embedding_worker.rs:74`) avvia `current_exe --embedding-worker` come processo figlio e gli parla con frame (lunghezza u32 + JSON `Request{kind,text,title,bytes}` + byte grezzi). Cinque operazioni: `chunk_text`, `embed_query`, `embed_document`, `embed_image`, `extract_document`. Il figlio carica EmbeddingGemma 2 ONNX q4f16 (più l'encoder visivo) con `ort =2.0.0-rc.15` su CPU (`Embedder::load`, `app/src/infrastructure/rag.rs:297`), scaricato da Hugging Face `onnx-community/embeddinggemma-2-ONNX` (~288 MB) in `data/models/embeddinggemma-2`. Vettori nativi a 768 dimensioni, troncati e normalizzati L2 a `DIM=256` (`rag.rs:60`, `truncate_normalize :438`). Chunk di 256 token con 20 di sovrapposizione. Prompt `task: search result | query: {q}` per le query e `title: {t} | text: {text}` per i documenti. Protezioni: Job Object Windows a 1024 MB, rilascio dopo 60 s di inattività, pausa di 60 s dopo un guasto, pausa su memoria bassa, cap 25 MiB per allegato, frame max 8 MiB. Le modifiche in corso introducono tre classi di esito: pause transitorie (`PAUSED_AFTER_FAILURE`, `PAUSED_FOR_MEMORY`, riconosciute da `is_transient`), che non contano come tentativo del messaggio, e `DOCUMENT_UNREADABLE`, che salta solo quel file; i panic dei parser diventano errori per richiesta (`catch_panic`).

**Chiamanti.** `app/src/infrastructure/rag_segments.rs:78,82,92` (indicizzazione), `rag.rs search_disk` (vettore della query), `app/src/infrastructure/mail_index_source.rs:412` e `app/src/infrastructure/search_support.rs:18,47` (estrazione), tramite lo statico `shared()` o `rag::shared_embedder()`. Non esiste un port applicativo per l'embedding: sta sotto `SearchGateway` (`app/src/application/ports.rs:269`), molto più grosso.

**Trascrizione oggi.** `TranscribeOptions{provider:"local"|"remote", model, language}` (`app/src/domain/meeting.rs:308`). `speech.rs:278 local_window` lancia `whisper-cli.exe` (whisper.cpp v1.9.2, runtime win-x64 scaricato da GitHub, modelli ggml da Hugging Face, in `data/speech-models/`). `speech.rs:400 remote_window` invia finestre di 15 s (16 kHz mono WAV, 500 ms di sovrapposizione) in multipart a `{ai_base_url}/audio/transcriptions` con `Bearer ai_key`, modello predefinito `whisper-1`, timeout 90 s, solo se `ai_provider == OpenAiCompatible`. Cache per finestra in `<meeting>/.stt-cache` con chiave SHA-256(audio, modello, lingua, endpoint). Un thread per job, checkpoint durevoli, cancellazione, `authorize()` ricontrollato prima di ogni richiesta remota.

**Frontend.** Circa 150 comandi Tauri in `desktop/src-tauri/src/main.rs:346`; eventi `app-event` tipizzati (`app/src/application/events.rs:9`). La CSP (`desktop/src-tauri/tauri.conf.json`, `connect-src 'self' ipc: http://ipc.localhost`) impedisce alla webview di raggiungere server esterni: tutto il traffico verso il server passa dal Rust. Il wrapper `desktop/src/lib/api.ts:11` ha già un interruttore di trasporto (Tauri o anteprima in browser).

**Processi.** Fino a quattro processi condividono una `data/`: UI, MCP (`--mcp`), worker MCP (`--worker`), worker di embedding. Coordinamento con `store.lock`, SQLite WAL, `WorkspaceUiLock`, poll del `change_feed` ogni 2 s.

**Violazioni dei port da chiudere prima.** `app/src/adapters/mcp/tools.rs:714` costruisce `LocalSearch` direttamente; `tools.rs:1715` costruisce `ProcessingEngine` direttamente; `app/src/adapters/mcp.rs:171`, `app/src/adapters/native/ui.rs` e `app/src/main.rs` chiamano `rag::` direttamente; `search_support.rs` e `mail_index_source.rs:412` chiamano `embedding_worker::shared()` direttamente.

**Altri vincoli.**
- `app/tests/layers.rs`: `domain` non nomina `std::fs`, `std::net`, `infrastructure`, `application`, `tauri`; `application` non nomina `infrastructure`, `adapters`, `std::fs`, `tauri`.
- ADR-0007 v3 governa `desktop/src-tauri/src/main.rs`, `runtime.rs`, `app/src/application/events.rs`, `desktop/src/components/IndexingSettings.tsx` (era `.svelte` prima di ADR-0011 v3): ogni attività di fondo, compresi embedding e installazione modelli, passa dall'indicatore unico (INV-009).
- Build release con `--features custom-protocol` e `--jobs 1` (AGENTS.md); cargo va in OOM con il parallelismo predefinito; `ort =2.0.0-rc.15` e `ring =0.17.14` bloccati; edition 2024. Non esiste un workspace Cargo radice: `app/` e `desktop/src-tauri/` sono crate separati.
- Il ramo è condiviso con altre sessioni: niente `git stash`, porta Vite propria per i test.
- Unico client HTTP nel codice: `ureq 3` bloccante. Nessun `reqwest`, `axum`, `tonic`, `tokio` esplicito, WebSocket.

## Decisioni

### D1. Un binario, due modalità

`qamille-server` è un eseguibile Rust separato dall'app desktop. In **modalità locale** l'app lo avvia come figlio alla prima necessità (passo di indicizzazione, ricerca semantica, trascrizione, estrazione), legato a `127.0.0.1` su una porta effimera, con un token casuale per avvio passato via ambiente o stdin, mai su riga di comando. Il processo vive finché vive il genitore: Job Object con terminazione alla chiusura su Windows (già in uso), watchdog sul PID del genitore su Linux. Il **modello** viene scaricato dalla memoria dentro il server dopo 60 s di inattività, come oggi; il processo resta, perché senza modello pesa poco e ripartire costa secondi.

In **modalità remota** lo stesso binario gira in Docker, ascolta su `0.0.0.0:8787` in HTTP semplice dietro un reverse proxy che termina TLS. L'app, con modalità remota attiva, non avvia alcun processo locale di calcolo.

**Scoperta del server locale.** Un file `data/compute-server.json` con `{pid, port, token, version, started_at}` protetto da un lock file. Un secondo processo (il server MCP, il suo worker) controlla che il PID sia vivo e che `GET /health` risponda; se sì lo riusa, altrimenti ne avvia uno e riscrive il file. Così UI e MCP non caricano il modello due volte.

**Memoria.** Il cap di 1024 MB resta applicato dal genitore via Job Object su Windows; in Docker lo applica il limite del container. Il controllo "memoria disponibile prima di caricare il modello" si sposta dentro il server (Windows: API di sistema come oggi; Linux: `/proc/meminfo`) e produce l'errore tipizzato `paused_for_memory`.

### D2. Protocollo HTTP: compatibile OpenAI dove esiste uno standard, estensioni `/qamille/v1/` altrove

| Rotta | Forma | Nota |
| --- | --- | --- |
| `GET /health` | nostra | Senza token: `{status, version}`. Con token: anche modelli (`id`, `state: ready / loading / downloading{percent} / missing`), capacità, limiti. |
| `GET /v1/models` | OpenAI | Elenco modelli disponibili. |
| `POST /v1/embeddings` | OpenAI | `{model, input: str / [str], dimensions: 256}`. Il server tronca e normalizza; il client **rinormalizza sempre e rifiuta** vettori di lunghezza diversa da 256. I prompt `task:` e `title:` li antepone il client, come oggi. Input a lotti: un documento di 20 chunk è una richiesta. |
| `POST /qamille/v1/chunks` | nostra | `{model, title, text}` → `{chunks:[str]}`. Spezzatura a 256 token con 20 di sovrapposizione usando il tokenizer del modello, che resta accanto al modello. |
| `POST /qamille/v1/embeddings/image` | nostra | Byte dell'immagine → vettore 256. Encoder visivo. |
| `POST /qamille/v1/extract` | nostra | multipart `{name, file}` → `{text}`. PDF/Office → testo, senza modello. Cap 25 MiB in ingresso, 4 MiB di testo in uscita, come oggi. |
| `POST /v1/audio/transcriptions` | OpenAI | multipart `{file, model, language, response_format: verbose_json}`. È la rotta che `remote_window` chiama già. Il server la serve con whisper.cpp. |
| `GET /qamille/v1/models/{id}` · `POST /qamille/v1/models/{id}/install` | nostra | Stato e installazione dei modelli (embedding, taglie whisper). In locale l'app la chiama; in remoto richiede il token amministratore, perché i modelli li decide chi gestisce il server. |

**Autenticazione.** `Authorization: Bearer <token>` su ogni rotta tranne `/health` anonimo. Locale: token per avvio. Remoto: token personali (D5).

**Errori.** JSON `{error:{code, message, retry_after_s}}` con codici chiusi: `model_missing`, `model_downloading`, `paused_for_memory`, `paused_after_failure`, `document_unreadable`, `payload_too_large`, `unauthorized`, `rate_limited`, `internal`. Il client classifica: `model_*`, `paused_*`, `rate_limited` ed errori di rete sono **transitori** (si riprova, il messaggio non perde un tentativo); `document_unreadable` e `payload_too_large` riguardano quel contenuto e lo saltano. È la stessa semantica di `is_transient` oggi, portata attraverso HTTP.

**Connessione caduta sulle rotte di contenuto** (`/qamille/v1/extract`, `/qamille/v1/embeddings/image`). Oggi il client riprova una volta con un processo nuovo e, se muore ancora sullo stesso documento, lo salta come illeggibile. La stessa regola attraversa HTTP: una richiesta che termina senza risposta HTTP (connessione chiusa, timeout) viene ripetuta una volta dopo che `/health` risponde di nuovo; una seconda caduta consecutiva **sullo stesso documento** lo marca `document_unreadable` e non consuma tentativi degli altri messaggi. Una caduta con `/health` ancora muto resta transitoria. Lato server, parser di documenti e decodifica delle immagini girano in un **processo figlio sacrificabile** del server (stessa cornice di `catch_panic` di oggi più isolamento del processo), così un PDF malformato non abbatte il modello né il server condiviso da UI e MCP.

**Concorrenza nel server.** Una sola inferenza di embedding alla volta per modello (semaforo), coda limitata, oltre la coda `rate_limited` con `retry_after_s`. Trascrizione: parallelismo pari ai core, configurabile. Per utente in remoto: massimo 2 richieste in volo per token, salvo configurazione.

### D3. Lato app: un port `ComputeGateway`, una sola implementazione HTTP

In `app/src/application/` nasce il port `ComputeGateway` con il perimetro del `ModelClient` di oggi più la trascrizione: `chunk_text`, `embed_query`, `embed_document`, `embed_image`, `extract_document`, `transcribe_window`, `health`, `install_model`. Entra in `ServiceContext` accanto agli altri port. `application` conosce solo il trait.

In `app/src/infrastructure/`:
- `HttpCompute` implementa il port con `ureq` bloccante verso un endpoint `{base_url, token, identity}`. È l'unica implementazione: anche la modalità locale passa di qui.
- **TLS.** `ureq` oggi usa rustls con i soli root pubblici incorporati: un certificato di una CA aziendale interna fallirebbe sempre. Il client abilita il verificatore di piattaforma (archivio certificati di Windows, dove le CA aziendali arrivano via criteri di gruppo) e accetta in più un file PEM di CA personalizzata (`compute_ca_pem`, D7). Niente opzione "ignora certificato".
- `LocalServerSupervisor` avvia, scopre e sorveglia il figlio e fornisce l'endpoint locale.
- Una factory leggendo le Impostazioni sceglie l'endpoint. Il cambio di modalità ha effetto dal passo di indicizzazione successivo, dalla prossima ricerca e dal prossimo job di trascrizione; i job in corso finiscono con l'endpoint con cui sono partiti.

**Cosa si sposta nel server.** `Embedder` e il caricamento ONNX da `rag.rs`, la logica di `embedding_worker::serve`, `speech::local_window` e la gestione del runtime whisper (`meeting_processing/models.rs`), `attach::bytes_to_text`. **Cosa resta nell'app.** `rag::search_disk` (BM25, prodotto scalare sui passaggi, fusione RRF), `rag_segments` (finestre di 200k caratteri, costruzione dei `Passage`, commit con quota e `store.lock`), la mescola audio in finestre di 15 s, la cache per finestra, i job e i checkpoint delle riunioni, i cursori di scansione.

**Dipendenze.** L'eseguibile desktop non linka più `ort` né il tokenizer; vive nel server. Le quattro violazioni dei port elencate sopra si chiudono: MCP, UI nativa e `main.rs` ricevono il gateway dal contesto.

**Installazione dei modelli.** In locale il thread `qamille-search-model` (`main.rs:149`) chiede al server l'installazione e ne legge l'avanzamento da `/health` ogni 2 s, emettendo `ModelProgress` nell'indicatore unico. In remoto l'app non installa nulla: se il modello manca, l'indicizzazione resta in `waiting_model` con il messaggio "il server remoto non ha il modello EmbeddingGemma 2".

**Cache delle trascrizioni.** La chiave oggi include l'URL dell'endpoint. Con la porta effimera del server locale cambierebbe a ogni avvio e la cache non servirebbe mai. La chiave usa l'`identity` dell'endpoint: la costante `local` oppure l'URL remoto configurato.

### D4. Identità del modello nell'indice

Ogni file di passaggi registra `embedding_model_id` (es. `embeddinggemma-2`), `dimensions` (256) e l'etichetta di quantizzazione. All'avvio dell'indicizzazione il client confronta ciò che il server dichiara in `/health` con l'indice: modello o dimensioni diversi **bloccano** l'indicizzazione con un avviso "il server usa un modello diverso dall'indice" e l'offerta di reindicizzare. La quantizzazione è registrata ma non blocca. Nella milestone 1 l'immagine Docker usa gli stessi file ONNX q4f16 e lo stesso runtime della modalità locale, quindi i vettori coincidono e passare da locale a remoto non richiede reindicizzazione.

*Aggiornamento del 19 novembre 2026.* Chi gestisce il server può scegliere dal pannello `/admin` un altro modello di embedding, servito da un servizio compatibile con OpenAI su un indirizzo interno: sempre 256 dimensioni (verificate prima di salvare e a ogni risposta), dichiarato in `/health` come qui sopra, quindi le app con un indice di un altro modello propongono la reindicizzazione. Due casi che il confronto di questo paragrafo da solo non copre sono chiusi dal server: dopo un cambio i vettori si fermano per 11 minuti (`rate_limited`), perché l'app confronta il modello una volta per messaggio e un messaggio a metà finirebbe con vettori di due modelli; e se dietro lo stesso nome i vettori cambiano, il nome dichiarato diventa `nome#n`. Dettagli nel [README del server](../../server/README.md), «Modello di embedding».

### D5. Più utenti nel server remoto

Configurazione `server.toml`: indirizzo, cartella dei modelli, modelli da precaricare, limiti, elenco utenti con nome e hash del token. Comandi `qamille-server user add <nome>` (stampa il token una sola volta), `user revoke <nome>`, `user list`. Un token amministratore distinto abilita le rotte di installazione modelli. Il server tiene per utente solo contatori (richieste, secondi di audio, token embeddati) per quote e diagnostica, mai contenuti. Il log registra codice di esito, durata e dimensione, non testo né audio.

Nell'app il token remoto va nel vault con chiave `compute-token`, come `ai-key` oggi.

### D6. Docker e Linux

Dockerfile multi-stage (build Rust → immagine Debian slim), `linux/amd64`, CPU. I file ONNX di EmbeddingGemma 2 e i modelli ggml di whisper vengono scaricati in un volume al primo avvio con lo stesso codice della modalità locale; un argomento di build permette di includerli nell'immagine. Un `compose.yaml` con il server e un reverse proxy (Caddy) per TLS. Fabbisogno indicativo: 1,5–2 GB di RAM per EmbeddingGemma 2 più whisper `small`.

Le due parti oggi solo Windows diventano moduli per piattaforma: il limite di memoria (Job Object / limite del container) e il runtime whisper (download `whisper-bin-x64` / compilazione di whisper.cpp nell'immagine o crate linkato, a scelta del costruttore purché restino gli stessi file di modello ggml e la stessa finestra di 15 s).

### D7. Impostazioni e interfaccia

In Impostazioni nasce la sezione **Calcolo** con due scelte: **Su questo computer** (predefinita) e **Server remoto** con indirizzo, token, CA personalizzata facoltativa (file PEM) e pulsante **Verifica connessione**, che chiama `/health` e mostra versione, modelli pronti e, se il modello di embedding differisce dall'indice, l'avviso di D4. La sezione mostra lo stato dei modelli (pronto, in download con percentuale, mancante) e in locale il pulsante di installazione già esistente.

**Consenso.** Oggi, per l'indicizzazione, nulla lascia il computer. Con il server remoto escono testo delle mail, allegati fino a 25 MiB e audio delle riunioni, anche se nulla vi viene conservato. La sezione Calcolo lo dice in chiaro accanto alla scelta **Server remoto**, e il primo salvataggio in modalità remota chiede una conferma esplicita con lo stesso testo. L'hook `authorize()` che oggi protegge la trascrizione remota (permessi MCP) si applica a ogni chiamata del gateway verso un endpoint **remoto**, embedding ed estrazione compresi; il server locale non lo richiede, perché nulla esce dalla macchina.

Nelle Riunioni il selettore `provider` di `TranscribeOptions` cambia valori: **Server di calcolo** (predefinito, segue la sezione Calcolo) ed **Esterno compatibile OpenAI** (l'attuale `remote`, che usa `ai_base_url` e `ai_key`; resta perché esiste già e permette un servizio pubblico solo per l'STT).

Settings in `app/src/domain/model.rs` acquista `compute_mode: Local | Remote`, `compute_url` e `compute_ca_pem` (facoltativo); il token sta nel vault.

**Dove compaiono stati ed errori** (coerente con INV-009 di ADR-0007 v3: l'indicatore mostra attività, gli errori sono avvisi separati):
- stati **transitori** (server in attesa, modello in download, pausa per memoria, nuovo tentativo tra N s) sono lo *stato* dell'attività di indicizzazione nell'indicatore unico, con la stessa nota nel pannello indicizzazione come oggi;
- errori **non transitori** (token non valido, modello diverso dall'indice, certificato rifiutato) sono avvisi separati, come ogni altro errore, e la sezione Calcolo li ripete accanto al campo interessato;
- pannello indicizzazione e sezione Calcolo sono superfici di impostazione, non di attività: non duplicano la percentuale né l'attività corrente.

### D8. Comportamento in caso di guasto

Nessun ripiego automatico da remoto a locale: nasconderebbe il guasto, mischierebbe provider e richiederebbe i modelli locali (~288 MB più whisper) anche a chi ha scelto il remoto.

- **Indicizzazione.** Errore transitorio → l'account entra in pausa; l'indicatore unico mostra l'attività nello stato "in attesa del server di calcolo, nuovo tentativo tra N s" e il pannello indicizzazione riporta la stessa nota; i tentativi del messaggio non vengono consumati; al ritorno riprende da dove era.
- **Trascrizione.** La finestra fallisce, il job fallisce come oggi e resta riprovabile; le finestre già trascritte restano in cache.
- **Ricerca.** Se il vettore della query non si ottiene, la ricerca restituisce i soli risultati lessicali BM25 con l'avviso "ricerca per significato non disponibile". È un cambio rispetto a oggi, scelto perché un risultato parziale batte un errore.
- **Token non valido.** `unauthorized` non è transitorio: l'indicizzazione si ferma, compare l'avviso "token del server non valido" e la sezione Calcolo lo ripete accanto al campo token.

### D9. MCP

`qamille-mcp` e il suo `--worker` usano lo stesso `ComputeGateway` e la stessa factory: riusano il server locale della UI tramite il file di scoperta, oppure ne avviano uno proprio se la UI non gira.

## Alternative scartate

- **RA-1. Libreria in-process con due implementazioni** (ort nell'eseguibile desktop per il locale, HTTP per il remoto). Due percorsi che divergono, niente server Linux, l'eseguibile desktop resta pesante. Scartata.
- **RA-2. gRPC / tonic.** Aggiunge tokio e prost a un'app sincrona; HTTP+JSON basta e la compatibilità OpenAI è lo standard di fatto, che un giorno permette server di terzi.
- **RA-3. Tenere `current_exe --embedding-worker` e aggiungere solo il ramo remoto.** Niente Linux, niente multi-utente, due protocolli (frame su stdio e HTTP).
- **RA-4. Indice sul server.** Multi-utente con archivio, autenticazione e quote: un prodotto a sé, con latenza di rete sulla ricerca. Rimandata; il port del servizio di ricerca resta il posto dove potrà entrare.
- **RA-5. Ripiego automatico remoto → locale.** Vedi D8.
- **RA-6. Scelta per servizio** (embedding qui, STT là). Più interfaccia e più stato; il proprietario ha chiesto un solo interruttore. Il gateway è per endpoint, quindi si potrà aggiungere.
- **RA-7. Spezzatura dei chunk nel client.** Richiederebbe il tokenizer del modello (decine di MB) anche in modalità remota; la rotta `/qamille/v1/chunks` lo tiene accanto al modello.

## Esempi

- **EX-1 Installazione pulita.** Nessuna impostazione. Al primo passo di indicizzazione l'app avvia il server locale; il download del modello compare nell'indicatore unico con percentuale; l'indicizzazione parte. Dal punto di vista dell'utente niente cambia rispetto a oggi.
- **EX-2 Passaggio al remoto.** L'utente inserisce indirizzo e token, **Verifica connessione** mostra versione e "EmbeddingGemma 2: pronto, whisper small: pronto". Dal passo successivo l'indicizzazione usa il remoto; il server locale viene fermato.
- **EX-3 Remoto irraggiungibile.** L'indicatore unico mostra l'indicizzazione nello stato "in attesa del server di calcolo, nuovo tentativo tra 60 s" e il pannello indicizzazione riporta la nota; nessun avviso d'errore, perché è una pausa; nessun messaggio perde tentativi; al ritorno l'indicizzazione riprende.
- **EX-9 PDF che abbatte il parser.** In remoto, la richiesta di estrazione cade senza risposta; `/health` risponde subito; il client ripete una volta; cade di nuovo: il file è `document_unreadable`, la mail viene indicizzata senza quel testo, gli altri messaggi non si fermano, il server resta su perché il parser girava in un figlio sacrificabile.
- **EX-10 Primo passaggio al remoto.** L'utente sceglie **Server remoto** e salva: compare la conferma "testo delle mail, allegati e audio delle riunioni verranno inviati a questo server, che non li conserva"; annullando, la modalità resta locale.
- **EX-4 Trascrizione remota.** Finestre di 15 s inviate a `/v1/audio/transcriptions` del server remoto; l'annullamento a metà job funziona; riprovando, le finestre già fatte vengono dalla cache.
- **EX-5 Due utenti.** Token diversi; la terza richiesta oltre il limite per utente riceve `rate_limited` con `retry_after_s` e l'app aspetta; un token revocato riceve `unauthorized` e la sezione Calcolo mostra "token non valido".
- **EX-6 MCP con UI aperta.** Il server MCP trova `compute-server.json`, verifica PID e `/health`, riusa il server: il modello è in memoria una volta sola.
- **EX-7 Modello diverso.** Il remoto dichiara un altro modello di embedding: l'indicizzazione non parte, l'avviso spiega la differenza e offre la reindicizzazione.
- **EX-8 Riavvio dell'app in locale.** Il server locale riparte su un'altra porta; le trascrizioni in cache vengono ancora riconosciute perché la chiave usa l'identità `local`, non la porta.

## Invarianti

- **INV-1.** Nessun codice di inferenza (ort, tokenizer, whisper) è linkato nell'eseguibile desktop; ogni inferenza passa dal `ComputeGateway` via HTTP.
- **INV-2.** Indice, vettori, BM25 e cache sono scritti solo dall'app sulla macchina dell'utente; il server non persiste contenuti dell'utente.
- **INV-3.** La distinzione transitorio / contenuto attraversa HTTP con codici chiusi; una pausa transitoria non consuma mai un tentativo del messaggio.
- **INV-4.** Ogni richiesta porta un token; il token locale è per avvio e non esce da `data/`; il token remoto sta nel vault.
- **INV-5.** Il cambio di modalità non cancella né invalida l'indice; una differenza di modello o dimensioni blocca l'indicizzazione e chiede.
- **INV-6.** Download dei modelli e pause del calcolo sono stati dell'attività nell'indicatore unico (INV-009 di ADR-0007 v3); nessun'altra superficie persistente mostra attività o percentuale. Gli errori non transitori sono avvisi separati; pannello indicizzazione e sezione Calcolo riportano note e stati di configurazione, non attività.
- **INV-10.** Nessuna richiesta a un endpoint remoto parte senza che la modalità remota sia stata confermata esplicitamente una volta e senza che `authorize()` l'abbia approvata; verso il server locale non c'è alcuna di queste due condizioni.
- **INV-11.** Parser di documenti e decodifica immagini non girano nel processo che tiene il modello.
- **INV-7.** `app/tests/layers.rs` resta verde: `application` conosce solo il trait `ComputeGateway`.
- **INV-8.** La chiave della cache delle trascrizioni è stabile tra un avvio e l'altro in modalità locale.
- **INV-9.** Un solo server locale per `data/`, qualunque sia il numero di processi Qamille attivi.

## Lasciato al costruttore

- Disposizione dei crate: workspace Cargo radice o crate separati; nome ed eventuale estrazione di una libreria `qamille-compute` condivisa tra server e test.
- Framework HTTP del server (axum + tokio è la scelta attesa; il motore è sincrono, quindi `spawn_blocking` più semaforo).
- Meccanismo esatto di lock e scoperta del server locale; come consegnare il token al figlio.
- Whisper su Linux: compilazione di whisper.cpp nell'immagine oppure crate linkato.
- Schema di hash dei token e formato di `server.toml`.
- Valori di `retry_after_s` (si parte dai 60 s di oggi), dimensioni di coda e limiti predefiniti.
- Come l'installer e `Build-Qamille.bat` includono `qamille-server.exe` in `release/Qamille/` (sidecar Tauri o copia accanto all'eseguibile).
- Meccanismo del processo figlio sacrificabile per i parser (pool o processo per richiesta) e il suo limite di memoria.
- Come il verificatore di piattaforma e la CA personalizzata convivono in `ureq` (feature `platform-verifier` o `native-tls`).

## Incrementi spedibili

1. **Port e chiusura delle violazioni.** `ComputeGateway` in `application`, implementazione che avvolge il `ModelClient` di oggi, MCP e UI nativa passano dal contesto. Nessun cambio di comportamento. Circa 1 giorno.
2. **Server locale HTTP.** Binario `qamille-server` con le rotte di embedding, chunk, immagini ed estrazione; `HttpCompute` e `LocalServerSupervisor` sostituiscono i frame su stdio; file di scoperta; `ort` esce dall'eseguibile desktop. Solo Windows. Circa 3 giorni.
3. **Trascrizione nel server.** `/v1/audio/transcriptions` servito con whisper.cpp; `speech.rs` usa solo il gateway; chiave della cache stabile; selettore Riunioni aggiornato. Circa 2 giorni.
4. **Modalità remota.** Sezione Calcolo, token nel vault, verifica connessione, codici d'errore e pause transitorie attraverso HTTP, identità del modello nell'indice, ricerca lessicale di ripiego. Circa 2 giorni.
5. **Docker e multi-utente.** Build Linux, moduli per piattaforma, `server.toml`, comandi utente, `compose.yaml` con Caddy, test di contratto locale/container. Circa 3 giorni.

Le stime sono giorni di lavoro dell'agente con revisione, non calendario.

## Verifica

- Test di contratto: le stesse richieste al figlio locale e al container producono gli stessi chunk e vettori uguali entro 1e-5.
- `cargo tree` dell'eseguibile desktop non contiene `ort` né `tokenizers`; `app/tests/layers.rs` verde.
- Passaggio locale → remoto → locale con lo stesso modello: nessuna reindicizzazione richiesta, ricerca con gli stessi risultati.
- Guasto iniettato: server ucciso a metà indicizzazione → pausa transitoria, nessun tentativo consumato; al riavvio riprende.
- Multi-utente: percorsi `unauthorized` e `rate_limited` con due token.
- Riavvio dell'app: cache delle trascrizioni riconosciuta (EX-8).
- `node tests/release-theme-probe.mjs` in `desktop/` ancora verde con il server accanto all'eseguibile in `release/Qamille/`.

## Milestone 2 (anteprima, fuori ambito)

Server con stato: identità degli utenti legata a dati, `change_feed` sulla rete attraverso il port `StoreRepository` (il punto duro è `transaction(&mut dyn FnMut(&mut Store))`, che presuppone lettura-modifica-scrittura atomica locale), eventi `AppEvent` via SSE o WebSocket (già JSON con tag `kind`, validati da `desktop/src/lib/events.ts`), push. La milestone 1 le prepara autenticazione, configurazione, Docker e lo spazio di rotte `/qamille/v1/`.
