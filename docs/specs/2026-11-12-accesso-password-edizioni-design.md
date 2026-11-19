# Accesso con username e password, edizione self-hosted ed edizione Professional

**Data:** 12 novembre 2026 · **Versione:** 2, dopo la revisione di sicurezza e di coerenza dello stesso giorno · **Stato:** direzione approvata dal proprietario il 12 novembre 2026; ADR-0012 proposto · **Modifica:** [server di calcolo locale o remoto](2026-11-07-server-calcolo-locale-remoto-design.md): sostituisce D5 ed EX-5, completa D1, D7 e D8, riformula INV-2, INV-4 e INV-10 ed EX-10 · **Decisione:** [ADR-0012](../adr/ADR-0012-accesso-password-edizioni.md) · **Piani:** eseguiti e rimossi il 17 novembre 2026 (restano nella storia di git); i piani in corso sono nell'[indice core ed enterprise](../superpowers/plans/2026-11-17-core-enterprise-indice.md)

Le parti segnate «scelta mia» sono decisioni di chi scrive, da controllare.

> **Aggiornamento del 17 novembre 2026.** Account, password, registrazione e recupero diventano funzioni dell'edizione **enterprise**. Il server open source non ha account: si entra con chiavi per dispositivo create dall'amministratore. Vedi [core open source ed enterprise](2026-11-17-core-enterprise-design.md).

## Perché

La specifica del server di calcolo prevede, per il server remoto, **token personali** emessi dall'amministratore con `qamille-server user add` e incollati nell'app. Va bene per un server aziendale, non per un prodotto venduto: un cliente non deve conservare un token, e due modi di entrare (token per chi installa il server, password per i clienti) sarebbero due logiche da mantenere.

Richiesta del proprietario (12 novembre 2026): Qamille deve avere l'accesso con **utente e password**, da vendere come **Professional** oppure da distribuire **open source self-hosted**. Nel self-hosted le utenze le gestisce l'amministratore e il client si configura a mano per collegarsi al server; nel Professional ci si registra dall'app e l'installer o il portable sono già collegati al server.

## Decisioni del proprietario

1. **Si entra con username (o email) e password**, in tutte le edizioni. Il token personale esce dall'app e dal server.
2. **Due build della stessa app**; la differenza è l'indirizzo del server fissato alla build:
   - **Self-hosted / open source (senza indirizzo preimpostato).** L'utente scrive l'indirizzo del server, poi accede. **Nessuna registrazione dall'app**: gli account li crea solo l'amministratore del server.
   - **Professional (indirizzo preimpostato).** Installer e portable nascono collegati al server del venditore. Si accede oppure si crea l'account dall'app. Niente passo «Server».
3. **Le utenze sono del server**, non dell'app. Nessun utente e nessuna password predefiniti.
4. **Le due edizioni**: self-hosted con utenze dell'amministratore e server scritto a mano, servizio venduto con registrazione dall'app e server preimpostato.
5. **La modalità «Su questo computer» resta in entrambe le edizioni** (12 novembre 2026). Il Professional parte collegato, ma si può tornare a tutto locale e poi ricollegarsi. Il valore venduto sono il calcolo remoto e la sync futura.

## Cosa resta della specifica del server di calcolo

- **La modalità locale resta.** «Su questo computer» non chiede né indirizzo né account. Il server locale ha il suo token per avvio, che non esce da `data/` (D1, INV-4).
- Protocollo, rotte di calcolo, codici d'errore, identità del modello nell'indice, guasti: invariati, salvo dove questa specifica dice altro.
- **Nessun contenuto dell'utente sul server** (INV-2 riformulato sotto).
- La sincronizzazione tra dispositivi resta la milestone 2. Questa specifica le prepara l'identità: un utente con più dispositivi registrati è quello che la sync userà.

## Decisioni

### A1. Credenziali

| Credenziale | Chi la ha | A che serve |
| --- | --- | --- |
| **Password** dell'utente | la persona | Solo a ottenere la chiave di un dispositivo e alle operazioni sull'account (cambio password, disattivare un dispositivo, cancellare l'account). Non si salva mai sul computer. |
| **Sessione del server** | l'app, nel vault, voce `server-session` | Un solo valore JSON `{url, user_id, username, device_id, device_key}`, scritto con un solo `set`. La chiave va **solo** a quell'`url`. |
| **Chiave amministratore** | chi gestisce il server | Solo le rotte `/admin` sull'ascolto amministrativo (A4) e l'installazione dei modelli in modalità remota. |
| **Token per avvio** | app e server locale | Solo la modalità locale (D1), invariato. |

L'impostazione `compute_url` di D7 sparisce: l'indirizzo del server remoto sta nella sessione, accanto alla chiave che vale per lui. `Settings` conserva solo `compute_mode: Local | Remote` e `compute_ca_pem`.

La chiave del dispositivo ha la forma `spk_dev_` + 64 cifre esadecimali (32 byte casuali); il server ne conserva solo lo SHA-256. Il prefisso serve a riconoscerla in un log per sbaglio.

### A2. Rotte dell'account

Campi e codici d'errore in snake_case, corpo d'errore `{"error":{"code","message","retry_after_s"}}` come in D2.

| Rotta | Credenziale | Effetto |
| --- | --- | --- |
| `GET /v1/info` | nessuna | `{version, api, signup, password_reset, edition}`. `edition` è `self-hosted` o `professional`, dichiarato dal server (`QAMILLE_EDITION`) e solo informativo. |
| `POST /v1/login` | password | `{identifier, password, device_name}` → `{user_id, username, device_id, device_key}`. Registra il dispositivo. |
| `POST /v1/login/password` | password provvisoria | `{identifier, temporary_password, new_password, device_name}`: sostituisce la provvisoria e registra il primo dispositivo **nella stessa transazione**. |
| `POST /v1/login/disable-device` | password (anche provvisoria) | `{identifier, password, device_id}`: al massimo di dispositivi ne disattiva uno dello stesso utente. |
| `POST /v1/signup` · `POST /v1/signup/confirm` | nessuna | Solo con `QAMILLE_SIGNUP=1` (A9). Spenta, rispondono 404. |
| `POST /v1/password-reset` · `POST /v1/password-reset/confirm` | nessuna | Solo con SMTP configurato (A10). |
| `PUT /v1/account/password` | chiave del dispositivo + password attuale | `{current_password, new_password, sign_out_other_devices}` (A7). |
| `GET /v1/account/devices` | chiave del dispositivo | Elenco `{id, name, created_at, last_seen_at, current}`. |
| `DELETE /v1/account/devices/{id}` | chiave del dispositivo | Disattiva un altro dispositivo dello stesso utente. |
| `DELETE /v1/devices/self` | chiave del dispositivo | Esce da questo dispositivo. |
| `DELETE /v1/account` | chiave del dispositivo + password | `{password}`: cancella utente, dispositivi e chiavi. |

Le rotte di calcolo remote (`/v1/embeddings`, `/v1/audio/transcriptions`, `/qamille/v1/*` tranne i modelli) si aprono solo con la chiave di un dispositivo attivo di un utente attivo. Le rotte d'installazione dei modelli si aprono, in remoto, solo con la chiave amministratore; in locale con il token per avvio, perché lì è l'app a installarli (D3).

Il limite di D2 «2 richieste in volo per token» diventa **per utente**: più dispositivi dello stesso utente condividono il limite.

### A3. Regole dell'accesso

- **Identificativo:** con `@` è un'email, senza è uno username; maiuscole e spazi ai lati non contano; uno username non contiene `@`; un'email, quando c'è, è di un utente solo (indice unico `NOCASE`).
- **Password:** da 10 a 256 caratteri, nessun'altra regola di composizione. Hash **argon2id** (19 MiB, 2 passate, 1 filo) con sale casuale.
- **Password provvisoria** di 20 caratteri da un alfabeto senza caratteri ambigui, generata dal server, mostrata una volta sola, **valida 7 giorni** (scelta mia). Con essa `POST /v1/login` risponde `403 password_change_required` e non registra nulla. Una provvisoria scaduta si comporta come una password sbagliata.
- **Stessa risposta per chi non può entrare:** utente sconosciuto, disattivato, senza password, provvisoria scaduta e password sbagliata danno lo stesso `401 invalid_credentials` («Username o password non corretti.») dopo lo stesso lavoro: una verifica argon2, contro un hash fittizio quando non c'è un hash vero. `403 password_change_required` e `409 device_limit` arrivano solo dopo la password giusta e non rivelano nulla a chi non la conosce.
- **Rallentamento per identificativo, mai blocco.** Dopo tre tentativi sbagliati in 15 minuti, ogni tentativo aspetta 250 ms, poi il doppio, al più 4 s. L'attesa è prenotata prima della verifica, così tentativi mandati insieme si mettono in fila. Due regole, emerse dalla revisione di sicurezza, impediscono che il rallentamento diventi un blocco:
  - il tetto vale dall'arrivo della richiesta: `start = min(max(next_free, now) + wait, now + 4 s)`; senza tetto, N richieste in parallelo farebbero aspettare la password giusta circa 4·N secondi;
  - a tabella piena (10 000 identificativi) non si toglie una voce ancora dentro la finestra di 15 minuti; un identificativo nuovo che non trova posto aspetta il massimo; togliendo la voce più vecchia, chi inonda il server di identificativi inventati azzererebbe il conteggio della vittima.
- **Limite per origine.** Un'origine che supera `login_max_failures_per_origin` fallimenti (predefinito 30 in 10 minuti, scelta mia) riceve `429 rate_limited` con `retry_after_s` **prima** di prenotare un turno e prima di argon2. È un conteggio separato da quello della chiave amministratore. L'origine è l'indirizzo del socket; dietro un proxy è l'ultimo indirizzo di `X-Forwarded-For` **solo se la connessione arriva da un indirizzo in `trusted_proxies`** (elenco di IP, vuoto di base). Il `compose.yaml` fornito elenca il proxy del compose, così il limite non colpisce tutti i clienti dietro lo stesso proxy.
- **Verifiche argon2:** al più 4 insieme, su `spawn_blocking`. Chi non trova posto entro 2 s riceve `503 server_busy` con `retry_after_s`, invece di accumularsi.
- **Massimo di dispositivi** per utente, **3 di base** (`default_max_devices`; 0 = nessun limite), controllato dentro la transazione `BEGIN IMMEDIATE` che registra il dispositivo; al limite `409 device_limit` con l'elenco, mai una chiave.
- **Nome del dispositivo:** l'app propone «Qamille su Windows» (o `Qamille Professional su Windows`), modificabile prima di accedere; non usa il nome del computer, che può essere un dato personale.
- **Trasporto:** il server rifiuta `POST /v1/login`, le rotte di registrazione e di recupero e le rotte `/admin` su una connessione in chiaro che non viene da loopback, a meno che arrivi da un proxy di `trusted_proxies` con `X-Forwarded-Proto: https`. L'app accetta `http` solo verso il computer locale.

### A4. Gestione delle utenze

- **Riga di comando sulla macchina del server:** `qamille-server user create <username> [--email <email>] [--max-devices N]` stampa sullo standard output solo la password provvisoria; `user reset-password <username>`; `user disable <username>` e `user enable <username>`; `user delete <username>`; `user list`; `user devices <username>`; `user revoke-device <username> <device_id>`; `user max-devices <username> <N>`.
- **Chiave amministratore:** variabile d'ambiente `QAMILLE_ADMIN_KEY`: almeno 32 caratteri e almeno 8 caratteri distinti, altrimenti il server non parte. Vuota: l'ascolto amministrativo non si apre.
- **Ascolto amministrativo separato** (scelta mia): la pagina `/admin` e le rotte `/admin/api/*` stanno su un secondo indirizzo, `admin_bind`, vuoto di base. Il `compose.yaml` lo lega a `127.0.0.1:8788` sull'host, raggiungibile con un tunnel SSH; un ufficio può legarlo alla rete interna. Così il server Professional esposto su Internet non espone `/admin`. Sull'ascolto pubblico `/admin` risponde 404.
- **Pagina `/admin`:** crea utenti, reimposta password, disattiva e riattiva, revoca dispositivi, imposta il massimo, cancella. Pagina statica, CSP stretta con `frame-ancestors 'none'`, niente `innerHTML`, `Cache-Control: no-store`, chiave tenuta solo in memoria della pagina e mandata in `Authorization`, nessun cookie.
- **Utente disattivato:** il login fa comunque la verifica argon2 e risponde `invalid_credentials`; ogni chiave dei suoi dispositivi riceve `401` (l'autenticazione della chiave controlla `users.disabled_at`).
- **Dati del server:** `server.toml` per indirizzi, modelli, limiti e `trusted_proxies`; utenti, dispositivi, hash delle chiavi e contatori in `server.db` (SQLite, rusqlite come l'app) nel volume dei dati. Nessuna riga di utenti in `server.toml`.

### A5. Due modi di avviare il server

Completa D1. Lo stesso binario ha due comandi:

- **`qamille-server serve`** (remoto): ascolta su `bind` (predefinito `0.0.0.0:8787`, dietro il proxy TLS), apre `server.db`, offre le rotte dell'account e l'ascolto amministrativo se configurato. Nel `compose.yaml` la porta 8787 non è pubblicata: si raggiunge solo dal proxy.
- **`qamille-server serve --local`** (figlio dell'app): ascolta solo su `127.0.0.1` con porta effimera, accetta solo il token per avvio, non apre `server.db`, non ha `/v1/login` né `/admin`. È la modalità di D1.

### A6. Le due build

**Una variabile di build**, `QAMILLE_PRESET_SERVER` (nome scelto per non confondersi con le variabili `QAMILLE_*` lette a runtime dal server e con `QAMILLE_DATA`). La legge il crate `app/` in un modulo `edition` con `option_env!`, così la vedono l'interfaccia, il server MCP e il worker. Un `build.rs` in `app/` la valida (`https://…` oppure `http://127.0.0.1`/`localhost` per le prove) e **fa fallire la build** se è impostata ma non valida: niente Professional spedito per sbaglio come self-hosted. Il frontend la legge con un comando Tauri (`app_edition`), non da una variabile Vite.

`Build-Qamille.bat` accetta `/server <url>` in qualunque posizione insieme a `/no-pause`, svuota la variabile quando l'argomento manca e scrive in una cartella propria: `release\Qamille\` per il self-hosted, `release\Qamille-Professional\` per il Professional, così una build non sovrascrive l'altra né il suo `data/`.

**Installer Professional.** I dati stanno sempre in `data/` accanto all'eseguibile (`Store::data_dir`), e l'app si ferma se non può scriverci. L'installer è quindi NSIS **per utente** (`installMode: currentUser`, in `%LOCALAPPDATA%\Programs\Qamille`), prodotto con `tauri build --bundles nsis`, mai MSI e mai in Program Files. Contiene `qamille-server.exe` accanto all'eseguibile, come il portable.

| | Self-hosted (senza `QAMILLE_PRESET_SERVER`) | Professional (con `QAMILLE_PRESET_SERVER`) |
| --- | --- | --- |
| Primo avvio | Come oggi: tutto locale, nessun account. | Schermata «Accedi a Qamille» (A8), con «Usa solo su questo computer». |
| Collegarsi al server | Impostazioni → Calcolo → **Server remoto**: passi «Server», «Prima di collegarti», «Accedi». | Impostazioni → Calcolo → **Server Qamille**: passi «Prima di collegarti», «Accedi»; l'indirizzo non si mostra. |
| Registrazione dall'app | Mai, anche se il server l'avesse accesa (scelta mia, coerente con «utenze gestite dall'amministratore»). | Sì, se `signup: true`. |
| «Cambia server» | Sì. | No. |
| «Su questo computer» | Sì, predefinita. | Sì. Tornarci non esce dall'account. |
| Sessione salvata per un altro indirizzo | È la sessione in uso. | All'avvio: `DELETE /v1/devices/self` verso il vecchio indirizzo se risponde entro 5 s, poi la sessione si toglie e si mostra «Accedi». Il caso nasce passando lo stesso `data/` da un self-hosted a un Professional. |

### A7. Dispositivi, cambio password, uscita e cancellazione

- **«Dispositivi»** in Impostazioni → Calcolo: elenco con «questo dispositivo» e «Disattiva» sugli altri.
- **«Cambia password»**: attuale e nuova, con «Scollega gli altri dispositivi» spuntato di base (scelta mia). Dieci password attuali sbagliate di fila con la stessa chiave la revocano (scelta mia: una chiave rubata non diventa un modo per indovinare la password), e i fallimenti contano anche nel limite per origine.
- **Reimpostazione da amministratore**: nuova provvisoria e revoca di tutti i dispositivi (come il recupero per posta, A10).
- **«Esci da questo dispositivo»**: `DELETE /v1/devices/self`, poi la sessione si toglie dal vault e la modalità torna «Su questo computer» in entrambe le edizioni. Se il server non risponde, la sessione si toglie lo stesso e l'avviso dice che il dispositivo resta nell'elenco del server finché non lo si disattiva da un altro dispositivo o dall'amministratore.
- **Tornare in locale** non tocca la sessione: «Server remoto» si riattiva senza password.
- **«Elimina account»** in fondo a Impostazioni → Calcolo, con password e conferma rossa: `DELETE /v1/account`, poi come «Esci».

### A8. Interfaccia

- **Un solo modulo guidato** per collegarsi, per «Cambia server» e per il primo avvio Professional:
  - **Passo «Server»** (solo self-hosted): prova con `GET /v1/info` circa 400 ms dopo l'ultima battuta e a «Riprova», scadenza 5 s. Esiti: «sto provando», «trovato», «non risponde», «non sembra un server Qamille», versione incompatibile, «serve https». «Avanti» acceso solo con «trovato». Il campo CA personalizzata (`compute_ca_pem`) sta qui.
  - **Passo «Prima di collegarti»** (consenso, D7 ed EX-10): «testo delle mail, allegati e audio delle riunioni verranno inviati a questo server per il calcolo, che non li conserva». Si conferma **prima** di mandare credenziali o creare un account, una volta per indirizzo; «Cambia server» lo richiede di nuovo per il nuovo indirizzo. «Annulla» lascia tutto com'era; nel primo avvio Professional porta a «Su questo computer».
  - **Passo «Accedi»:** username o email, password, nome del dispositivo. «Crea account» solo nel Professional con `signup: true`; «Password dimenticata?» con `password_reset: true`. Password provvisoria → «Nuova password» prima di entrare. Al massimo di dispositivi → elenco da cui disattivarne uno.
- **Primo avvio Professional.** Schermata a tutta finestra con «Accedi», «Crea account» e l'azione secondaria **«Usa solo su questo computer»**. La schermata prova il server da sé e mostra «sto provando», «non risponde» o «versione incompatibile» con «Riprova»: con il server irraggiungibile si può comunque entrare in locale e leggere la posta. La finestra non ha cornice (`decorations: false`): la schermata porta i `WindowControls`. «Primo avvio» è un flag persistente nelle impostazioni (`onboarding_done`), acceso dopo l'accesso o dopo «Usa solo su questo computer».
- **Impostazioni → Calcolo**, quando si è collegati: server (nel self-hosted), utente, «Dispositivi», «Cambia password», «Esci da questo dispositivo», «Elimina account».
- **Cambia server** (solo self-hosted): la sessione vecchia resta finché il nuovo server non ha accettato l'accesso; un errore o «Annulla» non cambiano nulla. Dopo il successo: `DELETE /v1/devices/self` verso il vecchio server (un errore si ignora), poi la sessione nuova sostituisce la vecchia con un solo `set`. L'indice locale non si tocca (INV-5); se il nuovo server dichiara un altro modello vale l'avviso di D4.
- La password resta in memoria solo per la durata del modulo e non si scrive mai nel vault, nei log, negli eventi `AppEvent` o nello stato del frontend dopo l'ingresso.
- **`401` dopo l'ingresso** su una rotta di calcolo (dispositivo o utente disattivato, account cancellato): non è transitorio. L'indicizzazione si ferma senza consumare tentativi, compare l'avviso «Questo dispositivo non è più collegato al server», Impostazioni → Calcolo offre «Accedi di nuovo». La ricerca ricade sui risultati lessicali con l'avviso di D8. Sostituisce il «token del server non valido» di D8. Nessun processo toglie la sessione da solo: solo «Esci», «Elimina account», «Cambia server» o il caso Professional di A6.
- **Modello del server Professional:** il server del venditore usa EmbeddingGemma 2 ONNX q4f16 a 256 dimensioni, come la modalità locale, così passare tra locale e remoto non chiede di reindicizzare (D4). Un server self-hosted con un altro modello resta coperto dall'avviso di D4.

### A9. Registrazione dall'app (Professional)

Per non rivelare chi ha già un account e per non lasciare occupare l'email di un altro, la registrazione passa da un **codice per posta** (scelta mia; richiede SMTP, quindi `QAMILLE_SIGNUP=1` senza SMTP configurato impedisce al server di partire):

1. `POST /v1/signup {email}` risponde sempre `202` «Ti ho scritto un codice». Se l'email è libera, manda un codice; se è già di un account, manda invece «hai già un account: accedi o recupera la password». L'invio avviene in un'attività separata, con risposta a tempo costante.
2. `POST /v1/signup/confirm {email, code, password, username?, device_name}` crea l'utente, registra il dispositivo ed entra (`{user_id, username, device_id, device_key}`). Lo username, se manca, nasce dall'email.

Il codice è come quello di A10. Limiti: per origine su ogni richiesta, anche riuscita (predefinito 5 registrazioni all'ora, scelta mia); per email, 5 codici in 24 ore. Il consenso (A8) si dà prima del passo 1. Quote di calcolo per i nuovi account, prova gratuita e pagamenti restano fuori (Non obiettivi); i limiti per utente di D2 valgono già.

### A10. Password dimenticata

Solo con SMTP configurato (`password_reset: true`); per il Professional è di fatto obbligatorio.

- `POST /v1/password-reset {email}` risponde sempre `202` «Se l'email è di un account, ti ho scritto», con invio in un'attività separata e risposta a tempo costante.
- **Codice:** 8 simboli dall'alfabeto delle provvisorie (circa 40 bit), valido 15 minuti e una volta sola, conservato come hash, 5 tentativi. Al massimo 5 codici per email in 24 ore: oltre, la risposta è la stessa ma nessun codice parte. Un codice nuovo annulla il precedente.
- `POST /v1/password-reset/confirm {email, code, new_password}`: imposta la password e **revoca tutti i dispositivi**; poi si accede come sempre.
- Limite per origine su entrambe le rotte, come il login.
- La password SMTP non va nei log; un invio fallito va nel log senza l'indirizzo del destinatario, e l'utente vede comunque la frase generica.

### A11. MCP e processi

- `qamille-mcp` e il suo worker leggono `server-session` dal vault e mandano la chiave solo al suo `url`. Non chiedono mai la password e non dipendono dalla costante di build: una copia gestita dell'eseguibile per gli assistenti (`assistant_install.rs`) nata da una build self-hosted funziona anche dopo il passaggio al Professional, perché l'indirizzo sta nella sessione.
- Davanti a un `401` ogni processo rilegge il vault e riprova una volta se la sessione è cambiata (l'interfaccia può aver appena fatto «Accedi di nuovo»).
- Con sessione assente o revocata: la ricerca MCP ricade sui risultati lessicali con l'avviso «ricerca per significato non disponibile: Qamille non è collegato al server, accedi dall'app»; l'indicizzazione del worker resta in pausa.
- Il server MCP usa la chiave solo per le rotte di calcolo, mai per quelle dell'account.

### A12. Portable

- La sessione sta nel vault cifrato in `data/` come le altre credenziali: spostando la cartella si resta collegati.
- **Copiare** la cartella su un altro computer copia anche l'identità del dispositivo: le due copie occupano un solo posto, e disattivarne una scollega anche l'altra. `docs/portable.md` lo dice, e dice di usare «Esci da questo dispositivo» prima di dare la cartella a un'altra persona. La milestone 2 (sync) dovrà riconoscere le copie; qui si dichiara il limite.
- Come già per `credentials.key`, chi possiede la cartella `data/` possiede la sessione: una cartella sincronizzata in cloud la espone.

## Non obiettivi

- Pagamenti, piani, prova gratuita, quote commerciali, termini d'uso: sono il progetto «vendita Professional». Qui si costruisce ciò che lo rende possibile.
- Single sign-on / OIDC, codici di invito, autenticazione a due fattori.
- Account centrale tra server diversi: ogni server ha i suoi utenti nel suo `server.db`.
- Sync dei dati tra dispositivi (milestone 2).
- Licenza dell'app desktop o blocchi della build open source: la differenza tra le edizioni è l'indirizzo preimpostato, e chi ricompila il codice aperto può cambiarlo. Il controllo vero sta sul server (INV-16).

## Anti-obiettivi (un fallimento anche se funziona)

- Utente o password predefiniti sul server.
- Registrazione aperta su un server self-hosted senza che l'amministratore l'abbia accesa.
- Una risposta che distingue «utente inesistente» da «password sbagliata», o «email registrata» da «email libera».
- Un account chiuso fuori da chi conosce il suo identificativo, anche con richieste in parallelo.
- Password, chiavi, codici o URL con credenziali nei log, negli eventi `AppEvent`, nel `change_feed` o in `Debug`.
- La password salvata sul computer, anche cifrata.
- La chiave di un server mandata a un altro indirizzo.
- La sessione di un server vecchio tolta prima che il nuovo abbia accettato l'accesso (salvo il caso Professional di A6).
- Credenziali mandate prima del consenso.
- Il primo avvio del self-hosted che chiede un account, o il primo avvio Professional senza via per restare in locale.
- `/admin` raggiungibile dall'indirizzo pubblico.

## Alternative scartate

- **Tenere i token personali accanto alla password:** due modi di entrare, due cose da proteggere.
- **Indirizzo del Professional in un file accanto all'eseguibile:** lo si cambia a mano e le edizioni diventano uguali; per questo l'indirizzo si fissa alla build.
- **Indirizzo nelle Impostazioni e chiave nel vault, separati:** due file non aggiornati insieme, e la chiave può partire verso un indirizzo cambiato.
- **Consenso dopo l'accesso** (versione 1 di questa specifica): le credenziali e, con la registrazione, un account nascevano prima del consenso.
- **Registrazione in un passo solo:** rivela chi ha un account e lascia occupare l'email di un altro.
- **Professional senza modalità locale:** scartata dal proprietario.
- **Sessioni con cookie:** la webview non parla col server.

## Invarianti

Numeri in continuità con quelli della specifica del server di calcolo. L'ADR-0012 li riprende con la sua numerazione e una tabella di corrispondenza.

- **INV-2 (riformulato).** Indice, vettori, BM25 e cache sono scritti solo dall'app sulla macchina dell'utente. Il server conserva solo utenti (username, email facoltativa, hash della password, stato), dispositivi (nome, date, ultimo accesso), hash delle chiavi e dei codici, e contatori; mai testo, audio, allegati o vettori dell'utente oltre la durata della richiesta. Gli indirizzi di rete stanno solo in memoria, per il limite per origine.
- **INV-4 (riformulato).** Ogni richiesta di calcolo porta una credenziale: il token per avvio verso il server locale (che non esce da `data/`), la chiave del dispositivo della sessione verso il remoto, mandata solo all'`url` della sessione. La password non si salva mai.
- **INV-10 (riformulato).** Prima del consenso esplicito per un indirizzo, verso quell'indirizzo partono solo la prova `GET /v1/info`; dopo il consenso, credenziali, nome del dispositivo e, solo dopo l'accesso, contenuti dell'utente. `authorize()` vale per ogni richiesta di calcolo remota come in D7.
- **INV-12.** Password e codici non si scrivono in chiaro da nessuna parte (database, log, errori, risposte, `Debug`).
- **INV-13.** Utente sconosciuto, disattivato, senza password, provvisoria scaduta e password sbagliata sono indistinguibili dalla risposta; registrazione e recupero non rivelano se un'email ha un account.
- **INV-14.** Nessun blocco per identificativo: la password giusta, da un'origine che non ha superato il suo limite, passa dopo al più 4 s dall'arrivo, qualunque cosa facciano gli altri.
- **INV-15.** Con la password provvisoria non nasce nessun dispositivo; cambio e registrazione sono una sola transazione.
- **INV-16.** Nessun utente nasce se non dalla chiave amministratore, dalla riga di comando sul server o dalla registrazione con codice accesa dall'amministratore.
- **INV-17.** Una build con `QAMILLE_PRESET_SERVER` non usa nessun altro server Qamille; una build senza non offre mai la registrazione. È un vincolo d'interfaccia, non di sicurezza; i servizi esterni scelti dall'utente (provider dell'assistente, STT esterno, Gmail) restano.
- **INV-18.** Le rotte `/admin` esistono solo sull'ascolto amministrativo.

## Esempi

Continuano la numerazione degli esempi della specifica del server di calcolo (EX-1…EX-10). EX-5 è superato da EX-14 ed EX-15.

- **EX-11 Self-hosted, primo collegamento.** L'amministratore esegue `qamille-server user create mario` e riceve la password provvisoria. Mario apre Qamille (già funzionante in locale), va in Calcolo → Server remoto, scrive `https://calcolo.ufficio.it`: «trovato». Conferma «Prima di collegarti», accede con `mario` e la provvisoria: l'app chiede la nuova password, poi l'indicizzazione passa al remoto.
- **EX-12 Professional, primo avvio.** Il portable scaricato dal sito si apre su «Accedi a Qamille». «Crea account»: conferma il consenso, scrive l'email, riceve il codice, sceglie la password; l'account nasce e l'app si apre collegata.
- **EX-13 Password sbagliata venti volte.** Venti tentativi distanziati di qualche secondo dalla stessa origine: le risposte rallentano fino a 4 s; al ventunesimo, con la password giusta, si entra.
- **EX-14 Quarto computer oltre il limite di tre.** `409 device_limit`: l'app mostra i tre dispositivi; Mario disattiva il vecchio portatile con la sua password ed entra.
- **EX-15 Dispositivo disattivato dall'amministratore.** La richiesta di embedding successiva riceve `401`: l'indicizzazione si ferma senza consumare tentativi, la ricerca resta lessicale con l'avviso, «Accedi di nuovo» riapre il passo «Accedi».
- **EX-16 Portable Professional spostato.** La cartella si sposta su una chiavetta e si apre altrove: resta collegato con la stessa sessione.
- **EX-17 Registrazione su server self-hosted.** `QAMILLE_SIGNUP` è spento: `POST /v1/signup` risponde 404 e `GET /v1/info` dice `signup: false`; la build self-hosted non mostra «Crea account» in nessun caso.
- **EX-18 Attacco in parallelo.** Cento richieste insieme con password sbagliate per `mario` da origini diverse: la password giusta di Mario, arrivata in mezzo, entra entro 4 s.
- **EX-19 Professional con il server spento.** Primo avvio, server irraggiungibile: la schermata dice «Il server non risponde» con «Riprova»; Mario sceglie «Usa solo su questo computer» e legge la posta.
- **EX-20 Da self-hosted a Professional sullo stesso `data/`.** La sessione punta a `https://calcolo.ufficio.it`: all'avvio il Professional prova a uscire da quel server, toglie la sessione e mostra «Accedi».
- **EX-21 Registrazione con un'email già usata.** La risposta è la stessa di un'email libera; nella casella arriva «hai già un account».

## Wave e ordine

Tre wave, ognuna spedibile da sola e con i controlli al cancello di fine wave:

1. **Server con utenti e accesso.** `server.db`, `serve` e struttura per `serve --local`, chiave amministratore e listener separato, comandi `user`, login, provvisoria, limite di dispositivi, rallentamento con tetto, limite per origine, rotte dell'account, API amministrativa, estrattore `DeviceAuth`. Si usa da solo con `curl`.
2. **L'app si collega, nelle due edizioni.** Sessione nel vault, modulo guidato con consenso prima delle credenziali, Impostazioni → Calcolo, dispositivi, cambio password, «Cambia server», uscita, cancellazione, `401` dopo l'ingresso, MCP; `QAMILLE_PRESET_SERVER` con `build.rs`, primo avvio Professional con «Usa solo su questo computer», `Build-Qamille.bat /server`, installer NSIS per utente. Va costruita **insieme al passo 4 del server di calcolo** (modalità remota).
3. **Registrazione, password dimenticata, `/admin`, distribuzione.** Codici per posta, SMTP, pagina `/admin` sull'ascolto separato, `compose.yaml` con Caddy e proxy fidato.

Ordine con il server di calcolo: i suoi passi 1–3 e la wave 1 in qualunque ordine; il suo passo 4 con la wave 2; la wave 3 e il suo passo 5 (Docker) insieme o in sequenza, estendendo gli stessi file.

## Verifica

- Server: provvisoria `403` poi `200`; provvisoria scaduta come password sbagliata; risposte uguali per sconosciuto, disattivato e password sbagliata; EX-13 ed EX-18; tabella piena che non azzera la vittima; `429` prima di argon2; `503` con argon2 saturo; limite di dispositivi con disattivazione e concorrenza; `trusted_proxies`; login in chiaro rifiutato fuori da loopback; `/admin` 404 sull'ascolto pubblico; la riga di comando stampa solo la password; nessun segreto nei log; registrazione e recupero con risposte uguali per email presenti e assenti.
- App: build con e senza `QAMILLE_PRESET_SERVER` (e build che fallisce con un valore non valido); server raggiungibile, non raggiungibile, non Qamille, incompatibile, `http` remoto; consenso prima delle credenziali; «Annulla» ed errore in «Cambia server» che non toccano la sessione vecchia; chiave mai mandata a un indirizzo diverso da quello della sessione; `401` che ferma l'indicizzazione senza consumare tentativi; EX-19, EX-20; MCP che rilegge il vault dopo un `401`.
- `app/tests/layers.rs` verde: l'account passa da un port in `application`, implementato in `infrastructure` con `ureq`.
- Prove manuali in WebView2 del portable e dell'installer Professional, e di `/admin` in un browser: fatte o dichiarate **non fatte**.
