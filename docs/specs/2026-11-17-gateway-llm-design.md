# Gateway LLM sul server: modello interno o remoto (OpenAI)

**Data:** 17 novembre 2026 · **Versione:** 5 (dopo la wave 4: `llm_builtin_slots`, la prontezza del figlio con `POST /tokenize`, il motore dell'immagine da un commit scaricato con git, i modelli scelti) · 4 (dopo la revisione dello stesso giorno: uguale nel self-host e nell'enterprise, consenso verificato dal server, tetto giornaliero, processo figlio più chiuso) · **Stato:** proposta, da implementare · **Fa parte di:** [core open source ed enterprise](2026-11-17-core-enterprise-design.md) (E2, E9 passo 3) · **Decisione:** [ADR-0014](../adr/ADR-0014-core-open-source-ed-enterprise.md)

Le parti segnate «scelta mia» sono decisioni di chi scrive, da controllare.

## Perché

Richieste del proprietario (17 novembre 2026): il server, self-host o enterprise, deve poter usare **servizi LLM interni propri** oppure **remoti tramite OpenAI**; il backend deve essere **predisposto per scaricare e installare modelli predefiniti da Hugging Face**, senza chiedere all'amministratore di installare a parte Ollama o altro.

Oggi l'assistente dell'app (risposte, riassunti, punti d'azione delle riunioni, domande su posta e riunioni) chiama direttamente Anthropic, un endpoint compatibile con OpenAI oppure Ollama, con una chiave che ogni persona configura sul suo computer (`app/src/infrastructure/ai.rs`). Va bene per una persona sola; non va bene per un ufficio che vuole un modello interno per tutti, né per un servizio venduto in cui il cliente non deve avere una chiave OpenAI.

## Decisioni

### L1. Un solo adattatore: l'API compatibile con OpenAI

Il server parla con il modello attraverso l'API **Chat Completions compatibile con OpenAI**. La parlano OpenAI, Azure OpenAI, Mistral, Groq e i server dei modelli interni: Ollama, vLLM, llama.cpp, LM Studio, LocalAI. Tre modi di avere un modello, un solo adattatore:

| `llm_provider` | Il modello | Chi lo installa |
| --- | --- | --- |
| `builtin` | Un modello del **catalogo predefinito**, scaricato da Hugging Face ed eseguito dal server stesso con llama.cpp (L8) | Il server, su richiesta dell'amministratore |
| `openai-compatible` interno | Ollama, vLLM o simili già in funzione nella rete dell'ufficio | L'amministratore, a parte |
| `openai-compatible` remoto | OpenAI o un altro servizio esterno | Nessuno: serve la chiave |

Un adattatore per l'API di Anthropic è un passo successivo (non obiettivo): l'app lo ha già per chi usa la propria chiave.

### L2. Configurazione (core)

| Chiave | Predefinito | Significato |
| --- | --- | --- |
| `llm_provider` | `none` | `none` (spento), `builtin` (modello del catalogo, L8) o `openai-compatible`. |
| `llm_builtin_model` | nessuno | Con `builtin`: l'id del catalogo, per esempio `llm-medio`. |
| `llm_base_url` | nessuno | Con `openai-compatible`: per esempio `http://ollama:11434/v1` (interno, nel compose) o `https://api.openai.com/v1`. `http://` è ammesso solo verso loopback, indirizzi privati e nomi senza punti (i servizi del compose); altrove serve `https://`. |
| `llm_model` | nessuno | Con `openai-compatible`: il modello predefinito, per esempio `qwen2.5:14b` o `gpt-4.1-mini`. Con `builtin` vale l'id del catalogo. |
| `llm_allowed_models` | `[]` | Altri modelli che le app possono chiedere; vuoto: solo `llm_model`. |
| `llm_max_output_tokens` | `4096` | Tetto ai token di risposta per richiesta, qualunque cosa chieda l'app. |
| `llm_context_tokens` | nessuno | Con `openai-compatible`: la lunghezza del contesto del modello, dichiarata alle app come `context_tokens`. Con `builtin` vale `llm_builtin_context`. |
| `llm_max_request_bytes` | 2 MiB | Corpo massimo di una richiesta (una conversazione con il testo delle mail). |
| `llm_timeout_secs` | `120`; `600` con `builtin` | Tempo massimo di una risposta del modello. Su CPU un riassunto lungo chiede minuti. |
| `llm_max_in_flight_per_user` | `2` | Richieste LLM insieme per utente. |
| `llm_max_tokens_per_day_per_user` | `0` (nessun tetto) | Token in entrata più in uscita al giorno per utente; oltre: `429 rate_limited` con `retry_after_s` fino a mezzanotte UTC. Raccomandato con un provider a pagamento (scelta mia). |
| `llm_label` | dal nome dell'host | Il nome mostrato nel consenso: «OpenAI» per `api.openai.com`, altrimenti l'host. |
| `llm_external` | dedotto | `false` se l'host è loopback, un indirizzo privato o un nome senza punti (un servizio del compose); altrimenti `true`. Si può forzare. |
| `QAMILLE_LLM_API_KEY` | nessuno | Nell'ambiente, mai nel file di configurazione né verso le app; il pannello `/admin` può impostarne un'altra (INV-32). |
| `llm_builtin_slots` | `1` | Con `builtin`: richieste servite insieme dal motore, da 1 a 4 (scelta mia: 1, perché il contesto si divide tra gli slot e la memoria cresce con loro). |
| `llm_builtin_context` | `8192` | Con `builtin`: lunghezza del contesto in token (più lungo, più memoria). |
| `llm_builtin_idle_secs` | `600` | Con `builtin`: dopo questi secondi senza richieste il modello si scarica dalla memoria. |
| `llm_builtin_program` | vuoto | Con `builtin` fuori da Docker e da Windows: il percorso di `llama-server`. In Docker è fisso, su Windows è il pacchetto verificato (L8). |
| `models_mirror_url` | vuoto | Indirizzo di uno specchio interno da cui scaricare modelli e motori invece che da Hugging Face e GitHub (uffici senza Internet). Gli SHA-256 restano quelli del catalogo. |
| `QAMILLE_HF_TOKEN` | nessuno | Solo nell'ambiente: per i modelli di Hugging Face che chiedono di accettare una licenza. Il catalogo predefinito non ne ha bisogno (L8). Non va mai allo specchio e non segue un redirect verso un altro host. |

### L3. Rotte

| Rotta | Credenziale | Effetto |
| --- | --- | --- |
| `GET /v1/info` | nessuna | Aggiunge `llm: null` oppure `{model, models, external, label, context_tokens}`. |
| `GET /v1/models` | dispositivo | Elenca anche i modelli LLM ammessi, oltre a quello di embedding. |
| `POST /v1/chat/completions` | dispositivo | Inoltra al modello e restituisce la risposta. |

**Che cosa accetta `POST /v1/chat/completions`** (sottoinsieme di OpenAI, scelta mia per la prima versione): `model` (assente: quello predefinito; fuori dall'elenco: `400`), `messages` (ruoli `system`, `user`, `assistant`; contenuto testo o elenco di parti di testo), `temperature`, `top_p`, `max_tokens` o `max_completion_tokens` (ridotti al tetto), `stop`, `response_format` (`text`, `json_object`, `json_schema`), `seed`.

Rifiutati con `400 invalid_request`: `stream: true`, `tools` e `functions`, `n > 1`, immagini e audio nei messaggi. Il campo `user` si sostituisce con lo SHA-256 dell'id interno dell'utente, così il provider non riceve email o nomi.

**Che cosa restituisce**: la risposta del provider con `id`, `model`, `choices` e `usage`, senza altri campi.

**Errori**, nella forma del server:

| Dal provider | Risposta all'app | Per l'app |
| --- | --- | --- |
| Nessuna risposta, tempo scaduto, `5xx` | `503 llm_unavailable` con `retry_after_s` | pausa: riprova o usa le regole senza modello |
| `429` | `429 rate_limited` con il suo `retry_after_s` | pausa |
| `401`, `403` (chiave del provider sbagliata) | `503 llm_unavailable`; nel log del server «chiave del provider LLM rifiutata» | pausa; il problema è dell'amministratore |
| `400` | `400 invalid_request` con un messaggio fisso | errore della richiesta |
| `400` perché la richiesta è più lunga del contesto del modello | `400 llm_context_exceeded` | l'app accorcia o divide il testo |

Senza diritto all'uso (enterprise senza abbonamento): `402 subscription_required`, come per il calcolo.

### L4. Accesso, limiti, contatori

- Stesso percorso delle rotte di calcolo: chiave del dispositivo della sessione, limite per origine delle credenziali sbagliate, `Entitlement` con `Resource::Llm` (`402`, oppure `429 quota_exceeded` nell'enterprise), richieste in volo per utente.
- Tetto giornaliero di token per utente, `llm_max_tokens_per_day_per_user` (scelta mia): senza, un dispositivo rubato o un'app che sbaglia consuma i crediti del provider fino alla disattivazione.
- I token di `usage` (in entrata e in uscita) vanno a `UsageSink`. Nell'enterprise sono la base delle **quote per piano**, che arrivano con il processore vero.
- Il server non scrive mai il contenuto delle richieste o delle risposte su disco né nei log: solo rotta, esito, durata, modello e numero di token (INV-33).

### L5. Privacy e consenso

- `GET /v1/info` dichiara se il server inoltra a un servizio esterno (`external`) e a chi (`label`).
- Il passo «Prima di collegarti» dell'app lo dice per nome: «Le richieste all'assistente vengono inoltrate a OpenAI, un servizio esterno a questo server.» Con un modello interno: «L'assistente usa un modello che gira su questo server.» (INV-34)
- L'app ricorda il provider per cui ha avuto il consenso e lo manda in ogni richiesta (`X-Qamille-Llm-Consent: <external>;<label>`). Se non coincide con il provider attuale, il server risponde `409 llm_consent_required` **senza inoltrare nulla**, e l'assistente «Server Qamille» si ferma finché la persona non conferma di nuovo (scelta mia: così non c'è una finestra in cui le mail vanno a un provider nuovo senza consenso).
- Raccomandazione per chi usa OpenAI con clienti aziendali: un contratto per il trattamento dei dati, la conservazione zero dei dati dove disponibile, e la residenza dei dati in UE se il provider la offre. Da verificare sulle condizioni del momento.

### L8. Modelli predefiniti scaricati da Hugging Face

Il server sa **scaricare, verificare ed eseguire** un catalogo di modelli LLM predefiniti, come fa già per EmbeddingGemma e Whisper. Solo il server: l'app non esegue modelli ([core ed enterprise](2026-11-17-core-enterprise-design.md), E10).

**Motore: llama.cpp (`llama-server`, licenza MIT).** Esegue modelli quantizzati in formato GGUF su CPU (e su GPU dove disponibile) ed espone la stessa API compatibile con OpenAI: il gateway lo usa con l'adattatore di L1, senza un secondo percorso. Il server lo avvia come **processo figlio** che ascolta solo su `127.0.0.1`, con una porta casuale e una chiave casuale per ogni avvio (INV-40):
- si avvia alla prima richiesta e si ferma dopo `llm_builtin_idle_secs` senza richieste, così la memoria torna libera;
- prima di avviarsi controlla la memoria disponibile; se non basta, `503 llm_unavailable` con l'avviso di memoria, la stessa pausa del calcolo;
- se cade si riavvia con attesa crescente; una richiesta in corso fallisce come pausa e non costa tentativi;
- al più `llm_builtin_slots` richieste insieme sul motore (predefinito 1, massimo 4: scelta mia). In `llama-server` il contesto si divide tra gli slot: `--ctx-size` vale `llm_builtin_context` per il numero di slot, e il controllo della memoria usa quel valore (verificato sul tag fissato);
- la chiave del figlio passa dall'ambiente o da un file leggibile solo dal server, **mai dagli argomenti**, che ogni utente della macchina può leggere; il figlio parte con un ambiente vuoto, senza la chiave dell'amministratore né quella del provider;
- «pronto» vuol dire che il figlio è vivo **e** risponde a una richiesta con la chiave: un altro processo che occupa la porta non riceve nulla. Sul tag fissato `/health` e `/v1/models` sono pubblici, quindi la prova è `POST /tokenize` con un contenuto fisso: senza chiave `401`, con la chiave `200`;
- la sua uscita non finisce nel log del server, salvo avvio ed errori senza contenuti (INV-33).

**Da dove arriva il motore:**
- **Docker e Linux** (il caso normale, anche per chi usa Docker sul proprio PC): incluso nell'immagine del server, compilato dal commit fissato (scaricato con git per identificatore: il commit di 40 cifre fissa il contenuto, l'SHA-256 dell'archivio resta annotato nel Dockerfile e nel crate `compute`), e usato da un percorso fisso. Su Linux fuori da Docker l'amministratore indica il programma con `llm_builtin_program`. Niente ricerca nel `PATH` né in cartelle scrivibili.
- **Un server Windows senza Docker**: l'archivio ufficiale di una release di llama.cpp da GitHub, a versione fissata, verificato con SHA-256 e scompattato come oggi `whisper-cli` (l'elenco esatto dei file attesi, dimensione massima, manifesto con licenza e versione; SHA-256 del programma ricontrollato a ogni avvio).
- Prima versione su CPU (scelta mia). Le build per GPU (CUDA, Vulkan) sono varianti successive dello stesso catalogo.

**Il catalogo.** Ogni voce è fissata nel codice: repository di Hugging Face, **revisione** (il commit, mai «l'ultima»), nome del file GGUF, dimensione, SHA-256, licenza, memoria richiesta, contesto consigliato (INV-39). Il formato GGUF contiene il modello di conversazione, che `llama-server` usa da sé.

| Id (scelta mia) | Per | Disco, circa | Memoria, circa |
| --- | --- | --- | --- |
| `llm-piccolo` | Docker su un PC da 16 GB, risposte brevi, classificazione | 2–3 GB | 4–6 GB |
| `llm-medio` | Docker su un PC da 32 GB e server piccoli, riassunti e risposte | 5–6 GB | 8–10 GB |
| `llm-grande` | server con 32 GB o con GPU, assistente sulle conversazioni lunghe | 9–15 GB | 16–24 GB |

Le taglie dei PC tengono conto che Docker Desktop su Windows dà al container al più metà della memoria, e che nello stesso container stanno EmbeddingGemma e Whisper.

**Quali modelli** si decidono quando si implementa, perché cambiano di mese in mese. I criteri:
1. licenza che permette l'uso commerciale, **anche come servizio** (Apache-2.0 o MIT; altre licenze solo se lette e accettate dal proprietario);
2. GGUF pubblicato dall'autore del modello o da una fonte affidabile, **senza** la necessità di accettare una licenza su Hugging Face (niente token per il catalogo predefinito);
3. buona qualità **in italiano** e in inglese su riassunti, risposte e punti d'azione, provata su un piccolo insieme di mail e trascrizioni di prova;
4. quantizzazione Q4_K_M o simile (anche la QAT q4_0 dell'autore);
5. velocità misurata su una CPU di riferimento, sufficiente a riassumere 6000 token entro `llm_timeout_secs`.

**Modelli scelti (18 novembre 2026) — PROVVISORI, DA CONFERMARE DAL PROPRIETARIO.** Decisione del proprietario: i modelli predefiniti sono **Gemma** (prima erano Qwen3), con le stesse tre taglie. Dati letti dall'API del Hub con `compute/tools/llm-catalog-lookup.mjs` (revisione, dimensione, SHA-256 dall'oid LFS, licenza dai tag, dalla scheda del modello e dal GGUF, cifre dell'intestazione GGUF). La famiglia più recente è **Gemma 4** (taglie E2B, E4B, 12B, 26B-A4B a esperti misti, 31B); il motore fissato (llama.cpp `b12081`) la conosce: `LLM_ARCH_GEMMA4` è in `src/llama-arch.cpp` a quel tag, con `src/models/gemma4.cpp` e il ramo degli esperti. Tutti e tre: repository dell'autore (`google/`), non protetti da accettazione (`gated` = no, nessun token), build QAT `q4_0` pubblicata dall'autore (addestrata per quella quantizzazione: qualità migliore di un Q4_0 ottenuto dopo; il nome del file non dice la quantizzazione in modo uniforme, perciò la voce del catalogo ha un campo `quantization`). Memoria = `(ram_mb + kv_mb_per_1k × contesto / 1024) × 1,15` con contesto 8192 e uno slot.

| Id | Repository, file | Revisione | Byte | SHA-256 | Licenza | `ram_mb` | `kv_mb_per_1k` | Memoria con 8192 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `llm-piccolo` | `google/gemma-4-E4B-it-qat-q4_0-gguf`, `gemma-4-E4B_q4_0-it.gguf` | `4b4a2c1d584be7264f87aac328a1bc739ce81b6c` | 5 154 941 280 | `676c35070db6dbe52f93e9c864ee0fba4eddea94b9c875d9cb10daff453fbaee` | Apache-2.0 | 5708 | 56 | circa 6,9 GB |
| `llm-medio` | `google/gemma-4-12B-it-qat-q4_0-gguf`, `gemma-4-12b-it-qat-q4_0.gguf` | `29d097773436b69ff9feafd636ab4cf873786537` | 6 975 879 296 | `93567e57a8fe10b23569b9d9ec38cd005deedf71e29477c421a4b83f418a538b` | Apache-2.0 | 7618 | 336 | circa 11,6 GB |
| `llm-grande` | `google/gemma-4-26B-A4B-it-qat-q4_0-gguf`, `gemma-4-26B_q4_0-it.gguf` | `d1c082be9cf3c8a514acf63b8761f4b41935842e` | 14 439 363 584 | `3eca3b8f6d7baf218a7dd6bba5fb59a56ee25fe2d567b6f5f589b4f697eca51d` | Apache-2.0 | 15448 | 220 | circa 19,3 GB |

**Licenza.** Il proprietario si aspettava le «Gemma Terms of Use» (Gemma 3 e precedenti). Per Gemma 4 non è così: i tre repository, la scheda del modello e i metadati del GGUF (`general.license`) dicono **Apache-2.0**, con `license_link` a https://ai.google.dev/gemma/docs/gemma_4_license, che oggi pubblica il testo della Apache License 2.0. Il catalogo registra quindi `Apache-2.0` e, in `license_url`, quell'indirizzo; `model install` stampa «licenza … condizioni: …» prima di scaricare e la pagina `/admin` mostra licenza e indirizzo accanto a «Installa»: chi installa le accetta. Se il proprietario sceglierà una famiglia con termini propri (Gemma 3), basta un'altra `license` e un altro `license_url` (un testo che comincia per «accettata dal proprietario: » supera `licenses_are_open`); nessun repository del catalogo richiede un token, e `QAMILLE_HF_TOKEN` resta per i repository «gated» (va solo a `huggingface.co`).

Perché questi: stessa famiglia e stesso modello di conversazione nelle tre taglie (un solo comportamento da provare); autore, non terzi; Apache-2.0; la più recente con GGUF d'autore. Scelte di taglia: `llm-piccolo` è E4B (4 miliardi di parametri «effettivi»; il file è 5,2 GB perché porta anche le tabelle per strato); `llm-medio` è 12B denso; `llm-grande` è 26B-A4B, un modello a esperti misti che consulta circa 4 miliardi di parametri per token, quindi su CPU è molto più veloce di un denso da 31B pur occupando 14,4 GB. Scartati: 31B denso (17,7 GB e lento su CPU), E2B (2,8 GB: qualità più bassa, e la serie ha già l'E4B come taglia piccola), le build di terzi (`unsloth`, `lmstudio-community`) e le Q4_0 non QAT di `ggml-org` (stessa licenza, qualità inferiore). Il disco è quello della colonna «Byte» più il margine di 200 MB. `kv_mb_per_1k` conta ogni strato a contesto pieno (anche quelli a finestra scorrevole di 512 o 1024 token, e gli strati che condividono la cache nell'E4B sono contati una volta): è un limite superiore, non una misura.

**Scaricamento automatico del modello (decisione del proprietario, 18 novembre 2026).** Se `QAMILLE_LLM_BUILTIN_MODEL` (`llm_builtin_model`) è impostata e il file non c'è, all'avvio il server lo scarica da solo, in background, da Hugging Face o dallo specchio configurato (`models_mirror_url`); il modello non sta mai nell'immagine Docker, solo nel volume dei dati. Regole:
1. **Ripresa e nuovi tentativi.** Stesso scaricamento dell'installazione a mano (richieste `Range`, file parziale, SHA-256 controllato); dopo un errore il server aspetta 10 s, 30 s, 1 min, 5 min e poi 15 min tra un tentativo e il successivo, finché ci riesce o il processo si ferma. L'ultimo errore si vede in `/admin` (stato «downloading» con il messaggio), mai un indirizzo o un token.
2. **Nel frattempo.** Le richieste all'assistente ricevono `503 llm_unavailable` con `Retry-After` e il testo «The assistant model is being prepared on the server; try again in a few minutes.» (`LLM_PREPARING`), senza tentare di avviare il motore; il desktop lo mostra come «Il modello dell'assistente è in preparazione sul server: riprova tra qualche minuto.». Il segnale è un flag condiviso tra il gestore dei modelli e il supervisore (`BuiltinSettings::preparing`), alzato per tutta la durata dello scaricamento.
3. **Licenza.** Il catalogo dice, per ogni voce, quale riconoscimento chiede (`accept_license`: `gemma` per i Gemma). Finché `QAMILLE_LLM_ACCEPT_LICENSE` (`llm_accept_license`) non ha quel valore, il server **non scarica nulla**: lo stato del modello in `/admin` è «missing» con «accept the licence terms by setting QAMILLE_LLM_ACCEPT_LICENSE=gemma», e le richieste all'assistente restano `503 llm_unavailable` generico. È l'accettazione esplicita delle condizioni (`license_url`) da parte di chi gestisce il server; l'installazione a mano dalla pagina `/admin` o con `model install` resta possibile, e le condizioni vi sono mostrate prima.
4. **Quando non parte.** Nessun modello configurato, modello non fissato nel catalogo, file già presente con la dimensione giusta, o un'altra installazione o rimozione in corso: non succede nulla. Lo scaricamento si ferma se il gestore viene distrutto (chiusura del server).
5. **Prova.** `server/tests/llm_autoinstall.rs` (con il finto Hugging Face): scarica in background, riprova dopo un errore, riprende un parziale con `Range`, non parte senza riconoscimento e dice perché, non parte se il file c'è o se non c'è un modello configurato, il supervisore risponde `Preparing` e non avvia il figlio, la distruzione del gestore lo ferma; `llm_builtin_route.rs` per il `503` della rotta.

**Ragionamento.** Gemma 4 ragiona solo se il prompt di sistema contiene `<|think|>`, e il server non lo mette; a ragionamento spento i modelli diversi da E2B ed E4B emettono comunque un blocco vuoto `<|channel>thought
<channel|>`, che il modello di conversazione di `--jinja` dovrebbe togliere da `content` (da provare). Nessuna gestione speciale nel server (nessuna opzione `--reasoning-budget`); la prova con il modello vero (`llm_real_model`) controlla che `content` non contenga i tag.

**Non ancora misurato:** la velocità sul CPU di riferimento (`speed_note` dice «not measured»), la qualità in italiano sull'insieme di prova, e che `content` non contenga i tag del canale di ragionamento (la prova con il modello vero non è stata eseguita). `llm-piccolo` è stato riscaricato per intero (5 154 941 280 byte) e il suo SHA-256 coincide con quello del Hub; `llm-medio` e `llm-grande`: SHA-256 dal Hub, non riscaricato. `ram_mb` è la dimensione del file più il 10% e 300 MiB, una stima, non una misura.

Candidati valutati con questi criteri: le famiglie Gemma 4 (Apache-2.0), Qwen3 (Apache-2.0, prima scelta, sostituita), Phi-4-mini (MIT) e un Mistral Apache-2.0 per `llm-grande`.

**Installazione.**
- **Server remoto**: dalla pagina `/admin` (sezione «Modelli», accanto a EmbeddingGemma e Whisper) o da riga di comando (`qamille-server model install llm-medio`), con le rotte che esistono già (`/admin/api/models/{id}/install`). Il download riprende da dove si era fermato e la percentuale si vede come oggi. Un modello si può anche rimuovere per liberare spazio.
- **Sul proprio PC**: con il server in Docker la pagina `/admin` su `127.0.0.1:8788` è la stessa; la pagina mostra memoria e spazio richiesti prima di installare.
- Senza Internet: `models_mirror_url` punta a uno specchio interno con gli stessi file; gli SHA-256 del catalogo li controllano comunque.

**Nell'enterprise.** Il servizio ospitato può usare `builtin` sui propri server con GPU, oppure OpenAI, oppure entrambi in futuro (instradamento per piano). Il catalogo è del core, quindi è lo stesso per tutti.

### L6. Che cosa non passa dal gateway

- **Embedding**: restano su EmbeddingGemma 2 sul server. Cambiare modello di embedding obbliga a reindicizzare tutto, anche gli indici nati con la vecchia modalità locale (D4 del server di calcolo). Costano poco e restano privati.
- **Trascrizione**: sul server resta whisper.cpp; una trascrizione remota fatta dal server (per esempio tramite OpenAI) è un'opzione futura con una sua chiave di configurazione. L'app sa già trascrivere con un provider esterno e la chiave della persona, e continua a farlo.

### L7. Nell'app

- Un nuovo provider dell'assistente: **«Server Qamille»**. Usa l'indirizzo e la credenziale della sessione del server, senza chiedere chiavi. Compare solo se il server dichiara `llm` e la persona ha dato il consenso per quel provider.
- Diventa il **predefinito** quando l'app è collegata a un server che lo offre. I provider con la chiave della persona (Anthropic, OpenAI, Ollama chiamati dall'app) restano, sotto «Usa una tua chiave» ([core ed enterprise](2026-11-17-core-enterprise-design.md), E10, scelta mia).
- La richiesta è quella che l'app fa già verso un endpoint compatibile con OpenAI, con indirizzo del server e credenziale della sessione al posto della chiave dell'utente.
- L'app accorcia o divide il testo in base a `context_tokens`, e lo rifà se riceve `llm_context_exceeded`.
- Errori: `409 llm_consent_required` → nuovo consenso; `402` → «L'abbonamento non è attivo»; `503` → pausa e, dove esistono, le regole senza modello che l'app usa già quando è offline; `401` → «Questo dispositivo non è più collegato al server», come per il calcolo.

## Non obiettivi

- Risposte in streaming (si aggiungono dopo, con un trasporto adatto nel server e nell'app).
- Strumenti e chiamate di funzione, immagini, audio verso il modello.
- Un adattatore nativo per Anthropic o Gemini.
- Più provider insieme con instradamento per piano (l'enterprise può aggiungerlo dopo, sempre come configurazione).

## Invarianti

- **INV-32.** La chiave del provider LLM sta nell'ambiente del server o, scelta dal pannello `/admin` (dal 2026-11-19, richiesta del proprietario), in `llm-settings.json` nella cartella dei dati (permessi 0600); non va mai alle app, nei log né nelle risposte dell'API amministrativa, che ne dicono solo la provenienza e le ultime quattro lettere.
- **INV-33.** Il server non conserva né scrive nei log il contenuto delle richieste e delle risposte LLM.
- **INV-34.** Prima del consenso l'app dice se il server inoltra le richieste all'assistente a un servizio esterno, e a quale. Il server non inoltra una richiesta il cui consenso non coincide con il provider attuale.
- **INV-39.** Ogni modello e ogni motore scaricato è fissato a una revisione o versione e verificato con SHA-256 prima dell'uso; mai «l'ultima versione».
- **INV-40.** Il motore interno ascolta solo su `127.0.0.1`, con una chiave casuale per ogni avvio che non compare negli argomenti né nei log, e riceve richieste solo dal gateway.

## Esempi

- **EX-33 Ufficio con Ollama.** `llm_provider = "openai-compatible"`, `llm_base_url = "http://ollama:11434/v1"`, `llm_model = "qwen2.5:14b"`. Il consenso dice «modello che gira su questo server». Mario chiede un riassunto: la richiesta resta in ufficio.
- **EX-34 Servizio con OpenAI.** `llm_base_url = "https://api.openai.com/v1"`, `llm_model = "gpt-4.1-mini"`, la chiave in `QAMILLE_LLM_API_KEY`. Il consenso dice «inoltrate a OpenAI». I token di ciascun cliente si contano per le quote.
- **EX-35 Provider giù.** OpenAI risponde `503`: l'app mostra la pausa e riprova; le funzioni con regole senza modello continuano.
- **EX-40 Ufficio senza altro software.** L'amministratore avvia il server core con `llm_provider = "builtin"` e `llm_builtin_model = "llm-medio"`, poi dalla pagina `/admin` fa «Installa»: il server scarica motore e modello, li verifica e da lì l'assistente risponde dal server. Nessun Ollama da installare.
- **EX-41 Tutto sul portatile, con Docker.** Anna avvia il server open source sul suo portatile con `compose.local.yaml`, installa `llm-piccolo` dalla pagina `/admin` (circa 3 GB di disco e 6 GB di memoria) e collega l'app a `http://127.0.0.1:8787` con la sua utenza: assistente, ricerca e trascrizione funzionano senza Internet.

## Verifica

- Catalogo: ogni voce con revisione, dimensione e SHA-256; un file alterato o di dimensione diversa è rifiutato; download ripreso; specchio interno.
- Motore interno: chiave mai negli argomenti, ambiente vuoto, impostore sulla porta ignorato, niente prompt nel log; avvio solo su loopback con chiave, fermata per inattività, controllo della memoria, riavvio dopo una caduta, con un finto `llama-server` nei test e quello vero in una prova facoltativa.
- Un finto provider compatibile con OpenAI su loopback: inoltro, tetto ai token, modello non ammesso, `stream` rifiutato, `429` e `5xx` tradotti, chiave del provider mai nella risposta né nei log, contenuto mai nei log.
- `401` senza credenziale, `402` senza diritto (enterprise), limite delle richieste in volo, tetto giornaliero, `409` con un consenso diverso dal provider attuale, `llm_context_exceeded`.
- App: provider «Server Qamille» visibile solo con `llm` dichiarato e consenso dato; nuovo consenso se il provider cambia.

## Rischi residui

- Impostore sulla porta scelta: pronto solo se il figlio è vivo dopo una breve attesa (500 ms) e il controllo con chiave regge due volte; il pid del processo in ascolto non è verificato (residuo, loopback, stesso utente).
- `/data` scrivibile: chi può scrivere nel volume può sostituire il modello tra il controllo e l'avvio (il controllo si ripete a ogni avvio, ma non è atomico).
- I thread in attesa di un avvio non contano per `max_in_flight_total`: molte richieste in coda all'avvio occupano thread senza essere limitate.
- Il download segue i redirect verso qualsiasi host https quando non c'è un mirror; l'integrità è garantita solo dallo SHA-256 fissato nel catalogo.
