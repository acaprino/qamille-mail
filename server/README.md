# qamille-server

Server di Qamille: utenti, dispositivi e accesso con username (o email) e password, secondo la [specifica dell'accesso con password](../docs/specs/2026-11-12-accesso-password-edizioni-design.md) e l'[ADR-0012](../docs/adr/ADR-0012-accesso-password-edizioni.md). Il server ha anche la pagina `/admin` sull'ascolto amministrativo e una distribuzione con Docker Compose e Caddy. La registrazione dall'app, il recupero della password per posta, «Continua con Google» e gli abbonamenti non sono qui: sono dell'edizione enterprise (repository privato), costruita sopra questo crate (INV-27: nel core un utente nasce solo per mano dell'amministratore). Il [server di calcolo](../docs/specs/2026-11-07-server-calcolo-locale-remoto-design.md) (embedding con EmbeddingGemma 2, spezzatura, estrazione dei documenti, trascrizione con whisper.cpp) gira in `serve`, dietro le chiavi dei dispositivi (l'app desktop non lo avvia più: è un client); l'inferenza sta nel crate [`compute/`](../compute) (`qamille-compute`).

È un crate a sé (`qamille-server`), separato da `app/` e da `desktop/src-tauri/`: non c'è un workspace Cargo radice. Dipende da `../compute` (ONNX Runtime, tokenizer, lettori di documenti), che l'app non linka (INV-1).

## Avvio

```sh
QAMILLE_DATA_DIR=server/data cargo run --jobs 2 --manifest-path server/Cargo.toml -- serve
```

`serve` ascolta su `bind` (predefinito `0.0.0.0:8787`), in HTTP semplice: in produzione va dietro un reverse proxy che termina TLS (vedi «Distribuzione con Docker Compose»). Il login, il cambio della password provvisoria, la disattivazione con password, il cambio password, la cancellazione dell'account e le rotte amministrative rispondono `400 tls_required` se la connessione non arriva dal loopback né da un proxy di `trusted_proxies` con `X-Forwarded-Proto: https`.

**Proxy sullo stesso computer.** Un reverse proxy che gira sulla stessa macchina e si collega al server da `127.0.0.1` va elencato in `trusted_proxies` (`trusted_proxies = ["127.0.0.1"]`). Altrimenti il server vede tutti i clienti come `127.0.0.1`: un'origine sola per il limite dei tentativi (chi sbaglia chiude fuori tutti) e `X-Forwarded-For` ignorato. Da un proxy fidato la connessione è sicura **solo** con `X-Forwarded-Proto: https`, anche quando il proxy è sul loopback: una richiesta che il proxy ha ricevuto in chiaro resta in chiaro.

### Calcolo in `serve` (modalità remota)

`serve` monta le rotte di calcolo accanto a quelle dell'account (spec D2, D5, A1, wave 3):

- ogni rotta di calcolo vuole `Authorization: Bearer <chiave del dispositivo>` (`spk_dev_…`), verificata come per `GET /v1/account/devices`: un dispositivo revocato, un utente disattivato o cancellato ricevono `401 unauthorized` alla richiesta successiva (EX-15);
- `GET /health` senza chiave risponde solo `{status, version}`; con una chiave valida anche `mode: "remote"`, i modelli (`ready`, `loading`, `downloading` con `percent`, `missing`), il modello di embedding (`embedding: {model, dimensions, quantization}`, che l'app confronta con il suo indice, D4), le capacità e i limiti;
- i modelli li decide chi gestisce il server: si installano **solo sull'ascolto amministrativo**, `POST /admin/api/models/{id}/install` e `GET /admin/api/models` con la chiave amministratore (TLS o loopback, limite dei tentativi per origine, lo stesso `401` per ogni chiave sbagliata). Sull'ascolto pubblico la chiave amministratore non vale nulla (è una chiave sbagliata come le altre, `401`); `POST /qamille/v1/models/{id}/install` non esiste sull'ascolto pubblico (`404 not_found`, con qualsiasi chiave);
- al massimo `compute_max_in_flight_per_user` richieste in volo insieme per **utente**, su tutti i suoi dispositivi (A2, predefinito 2): oltre, `429 rate_limited` con `retry_after_s: 1`, una pausa che l'app aspetta;
- chiavi sbagliate (anche su `/health`) contano per origine: oltre `compute_max_auth_failures_per_origin` al minuto (predefinito 120) le richieste con chiave di quell'origine ricevono `429 rate_limited` senza toccare il database, e `/health` risponde come senza chiave;
- una richiesta rimasta in coda mentre l'amministratore revocava il dispositivo (o disattivava l'utente) riceve `401` quando esce dalla coda, prima di arrivare al modello;
- ogni testo da embeddare è al massimo 32 KiB (rifiutato prima del permesso del modello) e 2048 token del modello (contati sotto il permesso, prima dell'inferenza): oltre, `413 payload_too_large`, senza mettere in pausa il modello per gli altri;
- immagini, documenti e audio si leggono **prima** di prendere il permesso del modello, del lettore o del riconoscitore: al massimo 4 caricamenti letti o tenuti in memoria insieme, ognuno entro 60 s (oltre, `400 invalid_request` «The upload did not arrive in time.»);
- per utente il server tiene solo **contatori** (`compute_usage`: richieste, secondi di audio, token embeddati), visibili in `GET /admin/api/users` (`usage`); mai testi, audio o vettori;
- l'audio di una trascrizione e il suo risultato stanno solo in `<data_dir>/compute-work/` (cartella 0700, file 0600), cancellati alla fine della richiesta e, se il server è stato ucciso, al suo avvio successivo; mai nella cartella temporanea comune;
- il modello si carica alla prima richiesta e si scarica dopo 60 s senza richieste, come sempre; i lettori dei documenti girano nel figlio `--parser-worker`. I file di EmbeddingGemma 2 stanno in `models_dir` (predefinito `<data_dir>/models/embeddinggemma-2/`, copiati lì o installati dall'amministratore), quelli di whisper in `speech_models_dir`.

Per installare il modello di embedding come amministratore, sul computer del server (nel compose l'ascolto amministrativo è pubblicato su `127.0.0.1:8788`):

```sh
curl -X POST -H "Authorization: Bearer $QAMILLE_ADMIN_KEY" http://127.0.0.1:8788/admin/api/models/embeddinggemma-2/install
```

### Assistente (gateway LLM)

Con `llm_provider = "openai-compatible"` `serve` monta anche `POST /v1/chat/completions`: il server inoltra la conversazione a un modello compatibile con OpenAI che scegli tu, **interno** (Ollama, vLLM, llama.cpp già in funzione) o **remoto** (OpenAI). Le chiavi `llm_*` sono nella tabella di «Configurazione»; la chiave del provider sta **solo** nella variabile `QAMILLE_LLM_API_KEY`, mai nel file di configurazione, mai verso le app, mai nei log. `llm_provider = "builtin"` (un modello che gira dentro il server) arriva con una versione successiva.

- **Accesso:** la chiave di un dispositivo attivo, come per il calcolo, il diritto d'uso dell'edizione (nell'enterprise `402 subscription_required` senza abbonamento), il tetto giornaliero e un cancello suo per le richieste in volo per utente; un corpo oltre `llm_max_request_bytes` riceve `413`.
- **Consenso:** `GET /v1/info` dichiara in `llm` il modello, i modelli ammessi, `external`, `label` e `context_tokens`. L'app mostra prima del collegamento, per esempio, «le richieste all'assistente vengono inoltrate a OpenAI, un servizio esterno a questo server» e manda `X-Qamille-Llm-Consent: <external>;<label>`. Se non coincide con il provider attuale il server risponde `409 llm_consent_required` **senza inoltrare nulla**.
- **Che cosa passa:** un sottoinsieme di OpenAI: `model`, `messages` (ruoli `system`, `user`, `assistant`, solo testo), `temperature`, `top_p`, `max_tokens` o `max_completion_tokens` (ridotti al tetto), `stop`, `response_format`, `seed`. Sono rifiutati con `400 invalid_request`: `stream`, `tools` e `functions`, `n > 1`, immagini e audio. Il campo `user` diventa lo SHA-256 dell'id interno. La risposta ha solo `id`, `object`, `created`, `model`, `choices` e `usage`.
- **Errori del provider:** nessuna risposta, tempo scaduto, `5xx`, `401`/`403` (chiave sbagliata, nel log solo «chiave del provider LLM rifiutata»), un redirect o una risposta non valida diventano `503 llm_unavailable`; un `429` resta `429 rate_limited`; una conversazione più lunga del contesto è `400 llm_context_exceeded`. Il testo di errore del provider non arriva mai all'app né al log.
- **Contatori e log:** i token del provider vanno ai contatori per utente (`usage.llm_prompt_tokens` e `usage.llm_completion_tokens` in `GET /admin/api/users`, e nella colonna «Token assistente» della pagina `/admin`, che compare solo con l'assistente acceso). Il log registra rotta, esito, durata, modello e numero di token; mai il contenuto.
- **Nessun redirect:** il server non segue i redirect del provider e non gli manda nessuna intestazione dell'app.

**Provider esterno e consenso.** Con OpenAI le mail e i testi della persona lasciano il tuo server. Prima di accendere un provider esterno per clienti aziendali: un contratto per il trattamento dei dati con il provider, la conservazione zero dei dati dove disponibile, la residenza dei dati in UE se la offre (da verificare sulle condizioni del momento), e un tetto giornaliero di token, perché un dispositivo rubato o un'app che sbaglia consuma i crediti.

Esempio con un modello interno nel compose (l'esempio `ollama` di `compose.yaml` è commentato; togli i commenti e metti in `.env` le tre variabili di `.env.example`):

```sh
docker compose up -d
docker compose exec ollama ollama pull qwen2.5:14b   # una volta, prima del primo uso
```

Esempio con OpenAI: `QAMILLE_LLM_PROVIDER=openai-compatible`, `QAMILLE_LLM_BASE_URL=https://api.openai.com/v1`, `QAMILLE_LLM_MODEL=gpt-4.1-mini`, `QAMILLE_LLM_MAX_TOKENS_PER_DAY_PER_USER=200000` e la chiave in `QAMILLE_LLM_API_KEY`.

Per fermare il server: Ctrl+C o `SIGTERM`.

### Modelli predefiniti

Con `llm_provider = "builtin"` il server non ha bisogno di Ollama né di un servizio esterno: scarica da Hugging Face un modello del catalogo, ne controlla lo SHA-256 e lo esegue da sé con `llama-server` di llama.cpp, un processo figlio che ascolta solo su `127.0.0.1`, con una chiave diversa a ogni avvio che non compare negli argomenti né nei log. Il modello si carica alla prima richiesta e si scarica dalla memoria dopo `llm_builtin_idle_secs` senza richieste.

**Accenderlo.** In `.env` (o in `server.toml`): `QAMILLE_LLM_PROVIDER=builtin` e `QAMILLE_LLM_BUILTIN_MODEL=llm-piccolo` (`llm_provider` e `llm_builtin_model`). Il modello si scarica da solo all'avvio (sotto), o lo si installa a mano. L'app lo vede in `GET /v1/info` come «questo server», non esterno: il consenso è `false;questo server`.

| Modello | Quale | Disco | Memoria (contesto 8192, 1 slot) |
| --- | --- | --- | --- |
| `llm-piccolo` | Gemma 4 E4B, Q4_0 (QAT), Apache-2.0 | 5,4 GB | 6,9 GB |
| `llm-medio` | Gemma 4 12B, Q4_0 (QAT), Apache-2.0 | 7,2 GB | 11,6 GB |
| `llm-grande` | Gemma 4 26B-A4B (esperti misti: 4 miliardi di parametri attivi), Q4_0 (QAT), Apache-2.0 | 14,6 GB | 19,3 GB |

I modelli sono i Gemma 4 di Google, scaricati dai repository ufficiali `google/gemma-4-*-it-qat-q4_0-gguf` (senza accesso riservato: non serve un token). **Licenza:** Gemma 4 è distribuito con licenza Apache-2.0 (la dichiarano il repository, la scheda del modello e il GGUF stesso); le condizioni sono su https://ai.google.dev/gemma/docs/gemma_4_license. `model install` le stampa prima di scaricare e `/admin` le mostra accanto a «Installa»: chi installa il modello le accetta. Le versioni precedenti di Gemma (3 e prima) hanno invece le «Gemma Terms of Use»: non sono nel catalogo. Gemma 4 ragiona solo se il prompt di sistema contiene `<|think|>`, e il server non lo mette: nessuna opzione per spegnere il ragionamento.

Il disco comprende la riserva di 200 MB che il download lascia libera; su Windows il motore (circa 25 MB) si scarica insieme al primo modello. La memoria è quella dei pesi, della cache del contesto e del motore, con il 15% di margine, e **si somma** a EmbeddingGemma 2 e a Whisper se li usi. Se la memoria libera non basta, il server non avvia il modello: risponde `503 llm_unavailable` con il testo di memoria e si riprova più tardi. I modelli sono provvisori finché il proprietario non li conferma, e la velocità non è ancora misurata (`speed_note` del catalogo lo dice).

**Scaricamento automatico.** Con `QAMILLE_LLM_BUILTIN_MODEL` impostata, all'avvio il server controlla se il file del modello c'è e, se manca, lo scarica da solo in background (da Hugging Face, o dallo specchio se c'è `models_mirror_url`; il modello non sta mai nell'immagine Docker, ma nel volume dei dati). Il download riprende da dove si è fermato (richieste `Range`) e, dopo un errore, riprova con attese crescenti (10 s, 30 s, 1 min, 5 min, poi ogni 15 min). Finché il modello non c'è, le richieste all'assistente ricevono `503 llm_unavailable` con il testo «The assistant model is being prepared on the server; try again in a few minutes.» (nel desktop: «Il modello dell'assistente è in preparazione sul server: riprova tra qualche minuto.») e `Retry-After`; il progresso si vede nella pagina `/admin`. **Licenza:** per i modelli Gemma il server non scarica nulla finché l'operatore non scrive `QAMILLE_LLM_ACCEPT_LICENSE=gemma` (`llm_accept_license` nel file di configurazione): con le condizioni di cui sopra, è la sua accettazione esplicita. Senza, il download non parte e la pagina `/admin` dice perché («accept the licence terms by setting QAMILLE_LLM_ACCEPT_LICENSE=gemma»). Se il modello è già installato, non succede nulla.

**Installarlo.** Dalla pagina `/admin`, sezione «Modelli»: la riga mostra memoria e disco richiesti accanto a quelli liberi *prima* di «Installa», poi una barra di avanzamento; «Rimuovi» chiede un secondo clic. Oppure da riga di comando, anche con il server fermo (non apre nessun ascolto):

```sh
qamille-server model list
qamille-server model install llm-piccolo     # aspetta la fine; Ctrl+C lascia il file parziale e si riprende
qamille-server model remove llm-piccolo --yes
```

Con Docker: `docker compose exec server qamille-server model install llm-piccolo`. Si possono installare i modelli anche prima di accendere `builtin`. Un modello con una richiesta in corso non si rimuove (`400`, si riprova a richieste finite); se il figlio è solo inattivo, il server lo ferma da sé prima di cancellare i file.

**Specchio e token.** Un ufficio senza Internet imposta `models_mirror_url` (`QAMILLE_MODELS_MIRROR_URL`): modelli e motore si scaricano da lì e gli SHA-256 restano quelli del catalogo. `QAMILLE_HF_TOKEN` serve solo per modelli di Hugging Face che chiedono di accettare una licenza (repository «gated»); il catalogo predefinito non ne ha bisogno. Sta solo nell'ambiente, va solo a `huggingface.co` e mai allo specchio né dopo un redirect.

**Richieste insieme.** `llm_builtin_slots` (`QAMILLE_LLM_BUILTIN_SLOTS`, da 1 a 4, predefinito 1): `llama-server` divide il contesto tra gli slot, quindi con 2 slot la memoria cresce come se il contesto raddoppiasse. Con 1 una richiesta alla volta usa tutto il contesto; le altre aspettano il loro turno nel limite `llm_max_in_flight_per_user`. Aumentalo solo con memoria da vendere. `llm_builtin_context` è il contesto di ogni richiesta, `llm_builtin_idle_secs` l'attesa prima di scaricare il modello.

**Che cosa succede in una pausa.** Se il figlio cade o non parte, il server lo riavvia con attese di 5, 10, 30 e 60 secondi e intanto risponde `503 llm_unavailable` con `Retry-After`. Una richiesta tagliata dalla caduta è una pausa, non un tentativo. Se il modello viene rimosso, le richieste ricevono lo stesso `503` finché non lo reinstalli.

**Il motore.** In Docker è `/usr/local/bin/llama-server`, compilato nell'immagine dal commit fissato (`LLAMA_TAG`, `LLAMA_COMMIT`, `LLAMA_SHA256` in `server/Dockerfile`), per CPU generica; la build controlla che `llama-server --help` conosca ogni opzione che il server usa. Su Linux fuori da Docker indichi tu il programma con `llm_builtin_program` (percorso assoluto, non scrivibile da altri); niente ricerca nel `PATH`. Su Windows senza Docker il server scarica l'archivio ufficiale di una release di llama.cpp, ne controlla lo SHA-256 e ricontrolla tutti i file a ogni avvio. **La GPU non è ancora supportata** (CUDA e Vulkan sono varianti future dello stesso catalogo): il motore gira su CPU.

**Aggiornare il motore.** Si cambiano insieme tag, commit e SHA-256 in `compute/src/llama.rs` (`LLAMA_TAG`, `LLAMA_COMMIT`, `SOURCE_SHA256`, più l'archivio e i file del pacchetto Windows) e gli `ARG LLAMA_*` di `server/Dockerfile` (e di quello dell'edizione enterprise, nel suo repository); `node compute/tools/llama-lookup.mjs <tag>` stampa i valori. Un test controlla che i tre posti coincidano. **Aggiornare un modello:** nuova revisione, dimensione e SHA-256 nella voce di `CATALOG` in `compute/src/llm_catalog.rs`, ricavati con `node compute/tools/llm-catalog-lookup.mjs <repo> <file>`.

**Non ancora provato** (non c'è un modello vero nei test automatici): la velocità sul CPU di riferimento, la qualità in italiano su un riassunto di 6000 token entro `llm_timeout_secs`, e che `content` di Gemma 4 non contenga i tag del canale di ragionamento (`<|channel>`; se li contenesse, l'assistente dell'app li mostrerebbe). La prova con il modello vero è `cargo test --manifest-path server/Cargo.toml --test llm_real_model -- --ignored --nocapture`, su un PC con rete e circa 6 GB liberi. Non è misurato nemmeno l'effetto di `GGML_OPENMP=OFF` sulla velocità.

### Sincronizzazione delle caselle

Con `QAMILLE_SYNC_KEY` impostata il server conserva le caselle di posta di ogni utente, con le loro credenziali (token OAuth, password per app), e le consegna ai **dispositivi attivi dello stesso utente**: l'app le ritrova su ogni computer dopo il login. Senza la chiave la sincronizzazione è spenta: le rotte rispondono `404` e `GET /v1/info` non ha il campo `sync`.

- **La chiave.** `qamille-server sync keygen` stampa una chiave casuale di 64 cifre esadecimali (con Docker Compose: `docker compose run --rm server sync keygen`). Mettila in `QAMILLE_SYNC_KEY` nel `.env`: **solo nell'ambiente**, mai in `server.toml` (la chiave `sync_key` non esiste) e mai nel repository. Tienila **fuori dal database e dai backup di `server.db`**, in un posto da cui la recuperi: il backup da solo non si apre. Una chiave di formato sbagliato o troppo regolare ferma l'avvio, senza ripeterla nel messaggio.
- **Come sono conservate.** Ogni elemento è cifrato a riposo con ChaCha20-Poly1305 (nonce casuale a ogni scrittura, dati associati utente e id dell'elemento: una riga spostata non si apre). Non è una cifratura da un capo all'altro: finché il server è acceso la chiave è in memoria e chi lo gestisce può leggere le credenziali. Il server non le usa e non le scrive nei log (nemmeno l'id degli elementi).
- **Se perdi la chiave.** Gli elementi non si aprono più: il server resta acceso (puoi entrare in `/admin`), `GET /v1/sync/items` risponde `500` e all'avvio il log dice che la chiave non apre gli elementi. Il rimedio è `sync reset --yes`, che li cancella: al prossimo accesso i computer ricaricano da soli le loro caselle (cambia `epoch` nella risposta, e l'app lo legge come «il server è ripartito da zero», non come «caselle rimosse»).
- **Cambiare la chiave (`sync rekey`).** A server fermo: metti la chiave nuova in `QAMILLE_SYNC_KEY` e la vecchia in `QAMILLE_SYNC_KEY_OLD`, esegui `qamille-server sync rekey` (una transazione: tutto o niente, versioni invariate), poi **togli** `QAMILLE_SYNC_KEY_OLD`. Fallo almeno una volta l'anno e subito se la chiave è uscita dal posto in cui la tenevi.
- **Limiti.** `sync_max_items` (200 caselle per utente, da 1 a 10000; oltre, `409 sync_limit`; la modifica di una casella già presente passa sempre) e `sync_max_item_bytes` (65536 byte per casella, da 1024 a 1048576; oltre, `413 payload_too_large`). Una cancellazione lascia un segnaposto per 90 giorni (al più `sync_max_items` per utente), poi sparisce. Al più 2 richieste insieme per utente (`429 rate_limited`).
- **Rotte** (chiave del dispositivo; solo su `https` o dal computer stesso, come il cambio password): `GET /v1/sync/items?since=N` (`{version, epoch, items}`), `PUT /v1/sync/items/{id}` (`{base_version, item}`; se la versione non è più quella attuale `409 sync_conflict` con l'elemento attuale in `error.current`) e `DELETE /v1/sync/items/{id}?base_version=N` (la versione è facoltativa). L'id sono 64 cifre esadecimali minuscole scelte dall'app. Nell'enterprise serve l'abbonamento (`402 subscription_required`).

## Configurazione

Un file `server.toml` facoltativo, indicato con `--config <file>` oppure cercato in `<data_dir>/server.toml`. Ogni chiave ha una variabile d'ambiente `QAMILLE_<CHIAVE>` che vince sul file.

| Chiave | Predefinito | Significato |
| --- | --- | --- |
| `bind` | `0.0.0.0:8787` | Indirizzo dell'ascolto pubblico. |
| `admin_bind` | vuoto | Indirizzo dell'ascolto amministrativo (`/admin/api/*`). Vuoto: nessun ascolto amministrativo. |
| `data_dir` | `data` | Cartella di `server.db` (relativa alla cartella corrente). |
| `sync_max_items` | `200` | Caselle sincronizzate per utente (da 1 a 10000; `QAMILLE_SYNC_MAX_ITEMS`). Vedi «Sincronizzazione delle caselle». |
| `sync_max_item_bytes` | `65536` | Byte di una casella sincronizzata (da 1024 a 1048576; `QAMILLE_SYNC_MAX_ITEM_BYTES`). |
| (solo variabile) | vuoto | `QAMILLE_SYNC_KEY`: la chiave della sincronizzazione, solo nell'ambiente (mai nel file); `QAMILLE_SYNC_KEY_OLD`: la chiave precedente, solo per `sync rekey`. |
| `default_max_devices` | `3` | Dispositivi attivi per utente; `0` = nessun limite. |
| `trusted_proxies` | `[]` | IP dei proxy fidati: solo da loro valgono `X-Forwarded-For` (ultimo valore) e `X-Forwarded-Proto`. Nella variabile, separati da virgole. |
| `login_max_failures_per_origin` | `30` | Fallimenti per origine prima di `429 rate_limited`… |
| `login_failure_window_secs` | `600` | …dentro questa finestra. |
| `admin_max_failures` | `300` | Chiavi amministratore sbagliate per origine prima di `429`… (alta di proposito: la chiave non si indovina per tentativi, e una soglia bassa permetterebbe a chi raggiunge l'ascolto, nel compose ogni utente dell'host, di chiudere fuori l'amministratore) |
| `admin_failure_window_secs` | `60` | …dentro questa finestra. |
| `temporary_password_days` | `7` | Validità della password provvisoria. |
| `device_inactive_days` | `180` | Giorni senza uso dopo i quali la chiave di un dispositivo non vale più e il dispositivo si revoca (si accede di nuovo); `0` = mai. Variabile: `QAMILLE_DEVICE_INACTIVE_DAYS`. |
| `admin_plaintext_from` | `[]` | IP da cui l'ascolto amministrativo accetta connessioni in chiaro oltre al loopback (nel compose: il gateway della rete, da cui arriva la porta pubblicata su `127.0.0.1` dell'host). Vale solo per l'ascolto amministrativo, e solo se `admin_bind` è un indirizzo di loopback o privato e le sorgenti sono di loopback o private: altrimenti il server non parte. **Rischio:** chiunque possa collegarsi da quelle sorgenti manda la chiave in chiaro sulla rete tra lui e il server; usalo solo dove quella rete è il computer stesso (il ponte Docker) o una rete di cui ti fidi. |
| `plaintext_from` | `[]` | IP da cui l'ascolto pubblico accetta connessioni in chiaro (login e cambio password compresi) oltre al loopback: serve al server sullo stesso computer con Docker, dove la porta pubblicata su `127.0.0.1` arriva dal gateway della rete del compose. Solo indirizzi di loopback o privati (uno pubblico ferma l'avvio); a differenza di `admin_plaintext_from` non richiede un `bind` privato, perché nel container il server ascolta su `0.0.0.0`. Conta solo l'indirizzo del socket, mai `X-Forwarded-*`, e mai un proxy fidato. **Rischio:** chiunque si colleghi da quegli indirizzi manda la password in chiaro; non metterci reti di cui non ti fidi. Variabile: `QAMILLE_PLAINTEXT_FROM`. `plaintext_from` è solo per una porta pubblicata su 127.0.0.1: non usarlo con una porta raggiungibile dalla rete. |
| `models_dir` | `<data_dir>/models` | Modelli di calcolo di `serve` (`embeddinggemma-2/`). Variabile `QAMILLE_MODELS_DIR`. |
| `speech_models_dir` | `<models_dir>/speech` | Modelli ggml di whisper.cpp di `serve`. |
| `transcription_parallelism` | i core | Trascrizioni insieme in `serve`. |
| `compute_max_in_flight_per_user` | `2` | Richieste di calcolo in volo insieme per utente, su tutti i suoi dispositivi; oltre, `429 rate_limited`. |
| `compute_max_auth_failures_per_origin` | `120` | Chiavi di dispositivo sbagliate al minuto per origine sulle rotte di calcolo prima di `429 rate_limited`. |
| `llm_provider` | `none` | Assistente: `none` (spento) o `openai-compatible`; `builtin` si riconosce ma ferma l'avvio finché non arriva con una versione successiva. Variabile `QAMILLE_LLM_PROVIDER`. |
| `llm_base_url` | vuoto | Con `openai-compatible`: l'indirizzo comprensivo di `/v1`, per esempio `http://ollama:11434/v1` o `https://api.openai.com/v1`. `http://` solo verso loopback, indirizzi privati e nomi senza punti; credenziali, query e frammento fermano l'avvio. Variabile `QAMILLE_LLM_BASE_URL`. |
| `llm_model` | vuoto | Il modello predefinito (`qwen2.5:14b`, `gpt-4.1-mini`). Variabile `QAMILLE_LLM_MODEL`. |
| `llm_allowed_models` | `[]` | Altri modelli che le app possono chiedere; vuoto: solo `llm_model`. Nella variabile, separati da virgole (`QAMILLE_LLM_ALLOWED_MODELS`). |
| `llm_max_output_tokens` | `4096` | Tetto ai token di risposta per richiesta (`QAMILLE_LLM_MAX_OUTPUT_TOKENS`). |
| `llm_context_tokens` | vuoto | Lunghezza del contesto del modello, dichiarata alle app (`QAMILLE_LLM_CONTEXT_TOKENS`). |
| `llm_max_request_bytes` | `2097152` | Corpo massimo di una richiesta; oltre, `413` (`QAMILLE_LLM_MAX_REQUEST_BYTES`). |
| `llm_timeout_secs` | `120` | Tempo massimo della chiamata al modello (`QAMILLE_LLM_TIMEOUT_SECS`); la rotta ha 10 secondi in più. |
| `llm_max_in_flight_per_user` | `2` | Richieste all'assistente in volo insieme per utente, con un cancello separato da quello del calcolo (`QAMILLE_LLM_MAX_IN_FLIGHT_PER_USER`). |
| `llm_max_in_flight_total` | `16` | Richieste all'assistente in volo insieme su tutto il server, di tutti gli utenti; oltre, `429 rate_limited` con `retry_after_s: 2` (`QAMILLE_LLM_MAX_IN_FLIGHT_TOTAL`). |
| `llm_max_tokens_per_day_per_user` | `0` | Token in entrata più in uscita al giorno per utente (`0` = nessun tetto); oltre, `429 rate_limited` fino a mezzanotte UTC. Soglia morbida: una richiesta può superarla. Raccomandato con un provider a pagamento (`QAMILLE_LLM_MAX_TOKENS_PER_DAY_PER_USER`). |
| `llm_label` | dall'host | Nome mostrato nel consenso: «OpenAI» per `api.openai.com`, «Azure OpenAI» per `*.openai.azure.com`, altrimenti l'host (`QAMILLE_LLM_LABEL`). ASCII stampabile, 1 a 64 caratteri, senza `;` e senza spazi ai bordi. |
| `llm_external` | dedotto | `false` per loopback, indirizzi privati e nomi senza punti, altrimenti `true`; si può forzare (`QAMILLE_LLM_EXTERNAL`, `1`/`0`). |
| `llm_builtin_model` | vuoto | Con `llm_provider = "builtin"`: il modello del catalogo (`llm-piccolo`, `llm-medio`, `llm-grande`) che il server scarica e accende da sé (`QAMILLE_LLM_BUILTIN_MODEL`). Con `builtin` non si impostano `llm_base_url`, `llm_context_tokens`, `llm_external = true` né altri modelli in `llm_allowed_models`. |
| `llm_builtin_context` | `8192` | Contesto di ogni richiesta del modello interno, da 1024 a 262144 token (`QAMILLE_LLM_BUILTIN_CONTEXT`). |
| `llm_builtin_idle_secs` | `600` | Secondi senza richieste dopo i quali il modello interno si scarica dalla memoria, almeno 10 (`QAMILLE_LLM_BUILTIN_IDLE_SECS`). |
| `llm_builtin_slots` | `1` | Richieste che il modello interno serve insieme, da 1 a 4; il contesto totale è contesto × slot (`QAMILLE_LLM_BUILTIN_SLOTS`). |
| `llm_builtin_program` | percorso fisso | Solo Linux: percorso assoluto di `llama-server`, non scrivibile da altri; vuoto: `/usr/local/bin/llama-server`. Su Windows si usa il pacchetto verificato e la chiave è ignorata (`QAMILLE_LLM_BUILTIN_PROGRAM`). |
| `models_mirror_url` | vuoto | Specchio di `huggingface.co` e `github.com` per modelli e motori: `https`, oppure `http` verso la rete interna; senza credenziali (`QAMILLE_MODELS_MIRROR_URL`). Gli SHA-256 non cambiano. |
| (solo ambiente) | vuoto | `QAMILLE_HF_TOKEN`: token di Hugging Face, mai nel file; va solo a `huggingface.co` e solo per le voci che lo dichiarano. |


### Limite per origine

Un IPv4 è un'origine; un IPv6 conta come la sua /64. I tentativi in corso contano come fallimenti finché non finiscono, così una raffica in parallelo non supera la soglia. La tabella tiene 4096 origini: a tabella piena un'origine nuova non viene mai rifiutata né lasciata passare senza conteggio; per farle posto si tolgono prima le voci senza fallimenti recenti, poi quella con **meno** fallimenti (a parità, la toccata da più tempo). Chi riempie la tabella con migliaia di origini da un fallimento non chiude fuori nessuno, e un'origine vicina alla soglia resta contata. Il rallentamento per account (10 000 voci) a tabella piena toglie prima le voci di identificativi sconosciuti, mai quelle di utenti esistenti; se non c'è nulla da togliere usa una voce comune di riserva che conta tentativi in corso e sbagliati come le altre.

Solo nell'ambiente:

- `QAMILLE_ADMIN_KEY`: chiave amministratore. Almeno 32 caratteri e almeno 8 caratteri distinti, altrimenti il server non parte. Vuota: l'ascolto amministrativo non si apre (anche con `admin_bind` impostato).
- `QAMILLE_EDITION`: `self-hosted` (predefinito); solo informativo, lo riporta `GET /v1/info`.

Esempio di `server.toml`:

```toml
bind = "0.0.0.0:8787"
admin_bind = "127.0.0.1:8788"
trusted_proxies = ["172.18.0.2"]
default_max_devices = 3
```

### Chiave amministratore

```sh
export QAMILLE_ADMIN_KEY=$(openssl rand -hex 32)
```

La pagina `/admin` e le rotte `/admin/api/*` esistono **solo** sull'ascolto di `admin_bind`; su quello pubblico rispondono 404. Tienilo su `127.0.0.1` e raggiungilo con un tunnel SSH:

```sh
ssh -L 8788:127.0.0.1:8788 server
curl -H "Authorization: Bearer $QAMILLE_ADMIN_KEY" http://127.0.0.1:8788/admin/api/users
```

### Sicurezza dell'accesso

- **Connessione sicura.** La chiave di un dispositivo vale come una password: tutte le rotte che la usano (account, dispositivi, calcolo, assistente, sincronizzazione) rispondono `400 tls_required` su una connessione non sicura, con la stessa regola del login (loopback, o un indirizzo di `plaintext_from`, o un proxy fidato che dice `X-Forwarded-Proto: https`). L'ascolto amministrativo guarda solo `admin_plaintext_from`, mai `plaintext_from`.
- **Chiavi sbagliate.** Oltre `compute_max_auth_failures_per_origin` chiavi di dispositivo sbagliate al minuto, un'origine riceve `429 rate_limited` senza interrogare il database, anche sulle rotte dell'account.
- **Un account solo.** Per ogni account (o nome sconosciuto) al più due verifiche della password sono in corso insieme: da quante origini arrivino, un account non occupa più di due dei quattro posti di argon2; oltre, `429 rate_limited` senza verifica. Le attese per account (tre tentativi liberi, poi 0,25 s raddoppiando fino a 4 s) restano. **Contropartita:** chi conosce il nome di un account e tiene occupati i suoi due posti può rendergli difficile entrare finché dura la raffica (l'utente vero riceve `429` e riprova); gli altri account non risentono.
- **Password comuni.** Una nuova password tra le più usate (un elenco breve dentro il programma) si rifiuta con `password_too_common`.
- **Chiavi dei dispositivi.** Una chiave non usata per `device_inactive_days` (180) non vale più e il dispositivo si revoca. Reimpostare la password di un utente (`user reset-password`, `/admin`) e disattivarlo revocano tutti i suoi dispositivi: riattivarlo non li fa rivivere.
- **Chiave amministratore.** Almeno 32 caratteri, 8 diversi e nessuna ripetizione evidente (un carattere oltre un terzo, sei uguali di fila, un blocco di otto o meno che si ripete, entropia sotto 3 bit per carattere): `openssl rand -hex 32` va bene.
- **Avviso all'avvio.** Con `trusted_proxies` vuoto il server avverte che dietro un proxy o un gateway tutte le richieste arrivano dallo stesso indirizzo.
- **Registro di controllo.** Gli eventi con `target = "audit"` (`user_created`, `user_deleted`, `user_disabled`, `user_enabled`, `admin_password_reset`, `device_revoked`, `password_changed`, `login_failed`, `password_check_failed`) riportano l'id dell'utente (o, per un nome sconosciuto, 8 cifre dello SHA-256 del nome) e l'origine dei tentativi falliti; mai password, chiavi, password provvisorie né il nome scritto.
- **Cancellazione dell'account.** `Server::with_deletion_guard` permette a un'estensione di fermare la cancellazione di un utente (da lui stesso o dall'amministratore) prima che avvenga; l'enterprise lo usa per non cancellare chi ha un abbonamento che incassa ancora. Il comando `user delete` da riga di comando non passa dalla guardia.
- **Diritto d'uso.** `Server::with_entitlement` collega un'`Entitlement` che il server interroga con la risorsa richiesta: `Compute` (embedding, estrazione, modelli), `Transcription` (`/v1/audio/transcriptions`, a parte perché una quota di audio finita non fermi la ricerca per significato), `Llm` (assistente) e `Sync`. Senza diritto risponde `402 subscription_required`; un errore `ServerError::Extension` dell'estensione (stato, codice, messaggio, `retry_after_s` e un oggetto `extra` i cui campi finiscono dentro `error`) arriva al client così com'è, su calcolo, trascrizione, assistente e sincronizzazione; qualunque altro errore resta un `500` senza dettaglio.

### Pagina `/admin`

Con il tunnel aperto, apri `http://127.0.0.1:8788/admin` nel browser e scrivi la chiave amministratore. La pagina elenca gli utenti (username, email, stato, dispositivi attivi, massimo) e per ogni riga permette di vedere e revocare i dispositivi, reimpostare la password (la nuova provvisoria si vede una volta sola, con «Copia»), disattivare e riattivare, cambiare il massimo di dispositivi e cancellare (con conferma). «Nuovo utente» crea un utente e mostra la provvisoria una volta.

La chiave resta solo nella memoria della pagina e va nell'intestazione `Authorization`: niente cookie, niente archivio del browser, e ricaricando la pagina va scritta di nuovo. Un `401` la svuota e la richiede. La pagina è statica, incorporata nel binario, con `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; form-action 'none'; base-uri 'none'`, `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` e `Cache-Control: no-store`.

## Primo utente

Sulla macchina del server (stessa configurazione di `serve`):

```sh
qamille-server user create mario --email mario@example.com
```

Lo standard output contiene **solo** la password provvisoria (valida 7 giorni), così si può passare a un altro programma; il resto va sullo standard error. Con la provvisoria `POST /v1/login` risponde `403 password_change_required`: l'utente sceglie la sua password con `POST /v1/login/password`, che registra anche il primo dispositivo.

```sh
curl -X POST http://127.0.0.1:8787/v1/login/password -H 'Content-Type: application/json' \
  -d '{"identifier":"mario","temporary_password":"<provvisoria>","new_password":"<nuova password>","device_name":"Qamille su Windows"}'
```

La risposta contiene `device_key` (`spk_dev_` + 64 cifre esadecimali), da mandare come `Authorization: Bearer` alle rotte dell'account.

## Comandi

| Comando | Effetto |
| --- | --- |
| `user create <username> [--email E] [--max-devices N]` | Crea l'utente e stampa la provvisoria. |
| `user reset-password <username>` | Nuova provvisoria (stampata); revoca tutti i dispositivi dell'utente. |
| `user disable <username>` · `user enable <username>` | Disattiva o riattiva: un utente disattivato non entra e le sue chiavi ricevono `401`. |
| `user delete <username> --yes` | Cancella utente, dispositivi e chiavi. |
| `user list` | Elenco degli utenti. |
| `user devices <username>` | Dispositivi attivi. |
| `user revoke-device <username> <device_id>` | Disattiva un dispositivo. |
| `user max-devices <username> <N\|default>` | Massimo di dispositivi dell'utente (`0` = nessun limite, `default` = quello predefinito). |
| `model list` | Elenco dei modelli con stato, dimensione e memoria richiesta. |
| `model install <id>` | Scarica un modello (per esempio `llm-medio`) e aspetta la fine, con l'avanzamento su stderr. |
| `model remove <id> --yes` | Cancella i file di un modello `llm-*`. |
| `sync keygen` | Stampa una chiave casuale per `QAMILLE_SYNC_KEY` (non apre né configurazione né database). |
| `sync rekey` | Ricifra tutte le caselle con `QAMILLE_SYNC_KEY` aprendole con `QAMILLE_SYNC_KEY_OLD`; a server fermo, tutto o niente. |
| `sync reset --yes` | Cancella tutte le caselle salvate sul server (rimedio a una chiave persa); non vuole la chiave. |

`--config <file>` vale per tutti i comandi. Codici d'uscita: 0 riuscito, 1 errore, 2 uso sbagliato.

## Rotte

Pubbliche: `GET /health`, `GET /v1/info`, `POST /v1/login`, `POST /v1/login/password`, `POST /v1/login/disable-device`, `GET /v1/account`, `PUT /v1/account/password`, `GET /v1/account/devices`, `DELETE /v1/account/devices/{id}`, `DELETE /v1/devices/self`, `DELETE /v1/account` (`{password}`, oppure `{login_code, code_verifier}` di un accesso con Google appena rifatto). Le rotte di registrazione, recupero, Google e abbonamenti sono dell'enterprise.

`GET /v1/info` risponde con `version`, `api` ed `edition` (e `llm` con l'assistente acceso, `sync` con la sincronizzazione accesa); `GET /v1/account` (chiave del dispositivo) con `user_id`, `username`, `email`, `has_password` e `device_id`. I campi in più (registrazione, provider di accesso, abbonamenti) li aggiunge l'enterprise.

Amministrative (solo su `admin_bind`): con il calcolo montato `GET /admin/api/models` e `POST /admin/api/models/{id}/install`; la pagina `GET /admin` (con `/admin/app.css` e `/admin/app.js`) e, con `Authorization: Bearer <chiave amministratore>`, `GET`/`POST /admin/api/users`, `GET /admin/api/users/{id}/devices`, `POST /admin/api/users/{id}/reset-password`, `/disable`, `/enable`, `/max-devices` (`{"max_devices": N | null}`), `DELETE /admin/api/users/{id}`, `DELETE /admin/api/users/{id}/devices/{device_id}`.

Di calcolo (con la chiave del dispositivo; vedi «Calcolo in `serve`»): `GET /health` (con il token: modelli, stato, limiti), `GET /v1/models`, `POST /v1/embeddings` (`{model, input: str | [str], dimensions: 256}`, fino a 64 testi già con i prompt), `POST /qamille/v1/chunks` (`{model, title, text}` → `{chunks}`), `POST /qamille/v1/embeddings/image` (i byte dell'immagine), `POST /qamille/v1/extract` (multipart `name`, `file`; 25 MiB in ingresso, 4 MiB di testo in uscita), `POST /v1/audio/transcriptions` (multipart `file` WAV, `model` `whisper-tiny|base|small`, `language` codice o `auto`, `response_format` `json` predefinito, `verbose_json` con i segmenti in secondi, `text`; 25 MiB), `GET /qamille/v1/models/{id}`; con l'assistente acceso anche `POST /v1/chat/completions` (vedi «Assistente (gateway LLM)») e i suoi modelli in `GET /v1/models`. Una sola inferenza di embedding alla volta per modello, coda di 16; trascrizioni in parallelo fino ai core, coda di 32; oltre `429 rate_limited`. Una richiesta che supera il tempo della sua rotta (300 s, 900 s per la trascrizione) risponde `429 rate_limited` con `retry_after_s: 30`, una pausa che non costa tentativi. Un cliente che chiude la connessione ferma il suo `whisper-cli`. Codici d'errore chiusi: `model_missing`, `model_downloading`, `paused_for_memory`, `paused_after_failure`, `document_unreadable`, `payload_too_large`, `unauthorized`, `rate_limited`, `internal` (più `invalid_request` e `not_found` per le richieste malformate, i modelli si installano solo sull'ascolto amministrativo, `POST /admin/api/models/{id}/install`; sull'ascolto pubblico le rotte di installazione non esistono: `404 not_found`). Il log registra rotta, esito, dimensione e durata, mai testi, nomi di file o vettori.

Gli errori hanno la forma `{"error":{"code","message","retry_after_s"}}`; `409 device_limit` aggiunge `devices`. Ogni risposta porta `Cache-Control: no-store`; i corpi oltre 64 KiB ricevono `413 payload_too_large`.

## Cosa conserva `server.db`

`server.db` (SQLite) nella cartella dei dati contiene solo (INV-2 della specifica):

- **utenti**: username, email facoltativa, hash argon2id della password, se è provvisoria e fino a quando, massimo di dispositivi, data di disattivazione e di creazione;
- **dispositivi**: nome, data di creazione, ultimo accesso (aggiornato al più ogni 5 minuti), data di revoca;
- **chiavi dei dispositivi**: solo lo SHA-256, con il contatore delle password attuali sbagliate di fila;
- **elementi di sincronizzazione** per utente (schema v7, solo con `QAMILLE_SYNC_KEY`): l'id dell'elemento (un hash), la versione, la data e il dispositivo dell'ultima modifica e il contenuto **cifrato** (nonce e testo cifrato), oppure un segnaposto di cancellazione senza contenuto; più un contatore e un'epoca per utente. È l'unica eccezione a «niente testo né chiavi in chiaro»: il contenuto non sta mai in chiaro nel file, nel WAL o nei backup;
- **contatori di calcolo** per utente (schema v4): richieste, secondi di audio, token embeddati, token dell'assistente (entrata e uscita, anche per giorno: solo numeri), data dell'ultimo aggiornamento;

Un `server.db` più vecchio si aggiorna da sé alla prima apertura (migrazioni con nome: `core-001-users`, `core-002-usage`, `core-003-llm-usage`, `core-004-sync`); un database con migrazioni che questo binario non conosce non si apre (`UnknownMigration`, con il nome della migrazione). Le migrazioni `core-003-llm-usage` e `core-004-sync` sono a senso unico: dopo il primo avvio con questa versione un binario precedente rifiuta il database, quindi prima dell'aggiornamento fai una copia di `server.db` (da quella si torna indietro). Le tabelle dell'enterprise (codici per posta, identità esterne, abbonamenti) sono nella documentazione di quell'edizione.

Mai password in chiaro, codici in chiaro, chiavi in chiaro, testo, audio, allegati o vettori. Gli indirizzi di rete stanno solo in memoria, per il limite per origine, e non vanno nei log: il log di accesso registra metodo, rotta, esito e durata.

## Distribuzione con Docker Compose

In `server/` ci sono `Dockerfile` (multi-stage, `linux/amd64`; il contesto è la radice del repository perché il server dipende da `compute/`, con i file ammessi in `Dockerfile.dockerignore`), `compose.yaml`, `Caddyfile` e `.env.example`:

```sh
cd server
cp .env.example .env && chmod 600 .env   # dominio, chiave amministratore
docker compose up -d --build
```

- **`caddy`** pubblica 80 e 443, ottiene da sé il certificato TLS per `QAMILLE_DOMAIN` e inoltra a `server:8787`, sovrascrivendo `X-Forwarded-For` (con l'indirizzo del cliente) e `X-Forwarded-Proto`. Non scrive log di accesso.
- **`server`** non pubblica la porta 8787: si raggiunge solo da Caddy. `QAMILLE_TRUSTED_PROXIES` sono gli indirizzi fissi di Caddy sulla rete del compose (`172.30.87.2` e `fd00:5a7c:87::2`), così il limite per origine vede i clienti veri e non colpisce tutti insieme. Una richiesta che arriva a `server:8787` da un altro container senza passare da Caddy riceve `400 tls_required` sulle rotte con password.
- L'**ascolto amministrativo** (`QAMILLE_ADMIN_BIND=172.30.87.3:8788`, l'indirizzo privato del container) è pubblicato solo come `127.0.0.1:8788` sull'host: dall'esterno si raggiunge con `ssh -L 8788:127.0.0.1:8788 server` e poi `http://127.0.0.1:8788/admin`. Le connessioni dalla porta pubblicata arrivano al container dal gateway della rete (`172.30.87.1`), che `QAMILLE_ADMIN_PLAINTEXT_FROM` autorizza al chiaro solo su questo ascolto. Su `https://<dominio>/admin` risponde 404.
- La rete ha le sottoreti fisse `172.30.87.0/24` e `fd00:5a7c:87::/64`: se sull'host sono già usate, cambia sottoreti e indirizzi insieme in `compose.yaml`.
- Le immagini (`rust`, `debian`, `caddy`) hanno versione e digest fissati nel `Dockerfile` e nel `compose.yaml`; un aggiornamento si fa cambiando quelle righe. Entrambi i container girano con il filesystem in sola lettura, senza capacità (Caddy tiene solo `NET_BIND_SERVICE` per le porte 80 e 443) e con `no-new-privileges`.

### IPv6

La rete del compose ha IPv6 acceso (`enable_ipv6`, sottorete locale `fd00:5a7c:87::/64`) perché Caddy veda l'indirizzo vero dei clienti IPv6. Con una rete solo IPv4, Docker inoltra le connessioni IPv6 alle porte pubblicate con il suo proxy di spazio utente, e Caddy le vede arrivare tutte dal gateway della rete: per il server sarebbero **un'origine sola**, e chi sbaglia la password da IPv6 farebbe scattare il limite per tutti i clienti IPv6.

Serve Docker 27 o più recente, dove `ip6tables` è attivo di base (nelle versioni precedenti va acceso in `/etc/docker/daemon.json` con `"ip6tables": true`), e un host con IPv6 nel kernel. Su un host senza IPv6 la rete non si crea: togli `enable_ipv6`, la sottorete `fd00:…` e le righe `ipv6_address` (e l'indirizzo IPv6 da `QAMILLE_TRUSTED_PROXIES`). Se l'host ha un indirizzo IPv6 pubblico ma non puoi accendere IPv6 sulla rete, non pubblicare le porte di Caddy su IPv6 (`"0.0.0.0:443:443"` al posto di `"443:443"`): meglio nessun cliente IPv6 che tutti dietro un'origine sola.
- I comandi `user` si danno nel container: `docker compose exec server qamille-server user create mario --email mario@example.com`.

### Aggiornamento da Qemille

Un server installato come Qemille gira come progetto `qemille`, con i volumi `qemille_data`, `qemille_caddy_data` e `qemille_caddy_config`, e il suo `.env` usa i nomi `QEMILLE_*`. Compose passa al server solo le variabili `QAMILLE_*`: si rinominano le chiavi e si tengono i volumi.

```sh
cd server
docker compose -p qemille down            # senza -v: i volumi restano
sed -i 's/^QEMILLE_/QAMILLE_/' .env
cat >> .env <<'ENV'
QAMILLE_DATA_VOLUME=qemille_data
QAMILLE_CADDY_DATA_VOLUME=qemille_caddy_data
QAMILLE_CADDY_CONFIG_VOLUME=qemille_caddy_config
ENV
docker compose up -d --build
```

Lo stesso vale per il server enterprise. I servizi di backup si chiamano ora `qamille-backup.service` e `qamille-backup.timer` (`deploy/backup`): `systemctl disable --now qemille-backup.timer` prima di installare i nuovi.

### Aggiornamento da Sparklone

Un server installato prima del cambio di nome gira come progetto `sparklone`, con i volumi `sparklone_data`, `sparklone_caddy_data` e `sparklone_caddy_config`. Il nuovo progetto si chiama `qamille`: senza indicazioni partirebbe con volumi nuovi e vuoti (niente utenti, modelli né certificati). Per tenerli:

```sh
cd server
docker compose -p sparklone down          # senza -v: i volumi restano
cat >> .env <<'ENV'
QAMILLE_DATA_VOLUME=sparklone_data
QAMILLE_CADDY_DATA_VOLUME=sparklone_caddy_data
QAMILLE_CADDY_CONFIG_VOLUME=sparklone_caddy_config
ENV
docker compose up -d --build
```

Il `down` del vecchio progetto libera anche le porte e la sottorete fissa `172.30.87.0/24`. Il `.env` esistente può restare con i nomi `SPARKLONE_*`: valgono quando manca la variabile `QAMILLE_*` corrispondente.

### Backup

Tutto lo stato del server è `server.db` nel volume `qamille_data` (montato su `/data`). Il modo semplice e sicuro è fermare il server per qualche secondo, così il WAL è già dentro il file:

```sh
docker compose stop server
docker run --rm -v qamille_data:/data:ro -v "$PWD":/backup debian:bookworm-slim \
  tar -C /data -czf "/backup/qamille-data-$(date +%F).tar.gz" .
docker compose start server
```

La chiave `QAMILLE_SYNC_KEY` **non** è in questo backup (sta nel `.env`): conservala a parte, e dalla copia di `server.db` da sola le caselle non si leggono.

Per ripristinare, a server fermo, svuota il volume e decomprimi l'archivio con lo stesso comando al contrario (`tar -C /data -xzf …`, senza `:ro`). Il backup contiene email e hash delle password: conservalo cifrato e fuori dal server.
