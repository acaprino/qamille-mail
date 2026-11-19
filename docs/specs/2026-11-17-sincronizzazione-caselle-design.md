# Sincronizzazione delle caselle tra i computer della stessa persona

**Data:** 17 novembre 2026 · **Versione:** 2 (dopo la revisione dello stesso giorno: basta il login, niente codice di collegamento, le credenziali le tiene il server) · **Stato:** proposta, da implementare · **Fa parte di:** [core open source ed enterprise](2026-11-17-core-enterprise-design.md) (E2, E9 passo 4) · **Decisione:** [ADR-0014](../adr/ADR-0014-core-open-source-ed-enterprise.md) · **Revisione:** [rapporto del 17 novembre](../../reports/2026-11-17-revisione-spec-e-piani.md)

Le parti segnate «scelta mia» sono decisioni di chi scrive, da controllare.

> **Che cosa è cambiato dalla versione 1.** La versione 1 collegava i computer con un codice di 16 caratteri e cifrava le credenziali da un capo all'altro, e attribuiva entrambe le cose al proprietario. Non erano sue decisioni. Il proprietario vuole che sul secondo PC basti il login, come in un normale servizio in abbonamento. Gruppi, inviti, prove, buste e cambi di chiave non esistono più.

## Perché

Richiesta del proprietario (17 novembre 2026): se una persona collega la sua casella di posta sul PC, deve **ritrovarla sul secondo PC** con Qamille. Le credenziali delle caselle vanno quindi sul server, per arrivare agli altri computer.

## Decisioni del proprietario

1. Si sincronizzano le caselle collegate nell'app, con le loro credenziali, tra i computer della stessa persona.
2. Passano dal server.
3. **Sul secondo PC basta il login**: stesso utente, stesse caselle. Vale nell'open source (utenti creati dall'amministratore) e nell'enterprise.
4. Le credenziali le tiene il server, come in un normale servizio in abbonamento; non serve che il server non le possa leggere.

## S1. Chi vede che cosa

Le credenziali di una casella (il token di Google, o la password per app di un IMAP) danno accesso a tutta la posta, e per Google anche a calendario e rubrica. Il server le conserva e le consegna ai dispositivi dello stesso utente.

- **In viaggio**: sulla connessione della sessione, `https` o il computer stesso.
- **A riposo**: cifrate dal server con una **chiave del server** che sta solo nel suo ambiente (`QAMILLE_SYNC_KEY`), mai nel database né nei backup (INV-35). Chi ruba `server.db` o un backup trova dati che non apre.
- **Chi può leggerle**: i dispositivi attivi di quell'utente, e chi controlla il server mentre è acceso. Il consenso dell'app lo dice (S6). Il venditore può dire ai clienti: «le tue credenziali sono cifrate sul nostro server; chi ruba i nostri dati o un backup non le legge».
- **Il server non usa le credenziali**: non legge la posta e non lavora sulle caselle mentre i computer sono spenti. Se un giorno servisse (invii programmati a PC spento, versione web), sarà un'altra specifica, con un consenso suo.

Senza `QAMILLE_SYNC_KEY` la sincronizzazione è spenta e `GET /v1/info` non la dichiara.

## S2. Che cosa si sincronizza

| Si sincronizza | Non si sincronizza |
| --- | --- |
| Elenco delle caselle: indirizzo, nome, colore, fornitore (`provider_id`), server e porte IMAP e SMTP, utente, cartelle di archivio e cestino, client Google usato (`oauth_client_id`) | Le mail: ogni computer le scarica dal suo provider |
| Credenziali: token di aggiornamento di Google, password per app IMAP, il segreto del client Google se è uno personalizzato | Indice di ricerca, cache, allegati |
| Firma della casella (il testo, non il riferimento locale) | Bozze, riunioni, registrazioni, rubrica (versioni future) |
| | Impostazioni del computer: cartella dei dati, limiti di archivio, modalità di calcolo, provider dell'assistente; il numero interno della casella |

**Con Gmail funziona così:** tutte le copie dell'app usano lo stesso client Google, quindi il token rilasciato al primo computer vale anche sugli altri. Si evita anche il limite di Google sul numero di token attivi per account. Se la persona revoca l'accesso dal suo account Google, tutti i computer lo perdono insieme: basta ricollegare la casella su uno, e il token nuovo arriva agli altri.

Ogni casella ha l'interruttore **«Sincronizza questa casella»**, acceso di base (scelta mia: è ciò che il proprietario si aspetta dopo il login). Spento, la casella resta solo su quel computer.

## S3. Come funziona

- **A chi appartengono i dati**: all'utente del server. Ogni dispositivo attivo di quell'utente legge e scrive le sue caselle. Non ci sono gruppi né collegamenti tra computer.
- **Un elemento per casella**, con un id che dipende dalla casella e non dal computer: SHA-256 di fornitore e indirizzo in minuscolo. Così la stessa casella collegata su due PC è **un solo elemento**, senza doppioni.
- **Contenuto** di un elemento: `{schema: 1, kind: "mail_account", updated_at, credential_updated_at, account: {...}, credential: {...}, mail_signature}`.
- **Quando**: all'avvio, subito dopo il login, ogni 5 minuti (scelta mia) e subito dopo una modifica locale. Prima si scarica, poi si invia.
- **Conflitti**: ogni scrittura porta la versione da cui parte; se nel frattempo è cambiata, il server risponde `409 sync_conflict` con l'elemento attuale. L'app unisce: per la credenziale vince `credential_updated_at` più recente, per il resto `updated_at` più recente. Così una firma cambiata su un PC non sovrascrive un token appena rinnovato su un altro.
- **Una casella aggiunta, ricollegata o cambiata** compare sugli altri computer, che cominciano a scaricarne la posta.
- **Una casella rimossa** su un computer: sugli altri compare «La casella X è stata rimossa su un altro computer», con «Rimuovi anche qui» e «Tienila solo qui» (scelta mia: niente cancellazioni silenziose della posta locale). Il server tiene un segnaposto della cancellazione per 90 giorni; una casella che un computer aveva già sincronizzato e che non trova più vale come rimossa, non come nuova.

**Limiti del server** (`sync_max_items`, `sync_max_item_bytes`): 200 elementi per utente, 64 KiB l'uno (scelta mia).

## S4. Il secondo PC

1. Sul computer nuovo la persona fa il login: nell'open source indirizzo del server, username e password; nell'enterprise l'indirizzo è preimpostato e può usare «Continua con Google».
2. Il passo «Prima di collegarti» dice che le caselle collegate vengono salvate sul server per portarle sugli altri computer (S6).
3. Dopo il login l'app scarica gli elementi dell'utente, crea le caselle e comincia a scaricare la posta. Nessun codice e nessuna conferma sul primo PC, che può essere spento.

## S5. Togliere un computer (perso, rubato, venduto)

1. Da un altro computer: Impostazioni → Calcolo → «Dispositivi» → «Disattiva» (esiste già). Oppure lo fa l'amministratore da `/admin`. Dalla richiesta successiva quel computer non riceve più nulla dal server (INV-37).
2. **Le credenziali che aveva già** restano su quel computer. «Disattiva» propone quindi anche **«Revoca gli accessi alle caselle»**: per Gmail l'app chiede a Google di revocare il token (che smette di valere ovunque), poi fa ricollegare la casella su questo computer, e il token nuovo arriva agli altri. Per le password IMAP l'app dice di revocarle o cambiarle presso il fornitore della casella (per Gmail: Account Google → Password per le app).
3. Se la revoca presso Google non riesce, l'app lo dice come **non fatta**, con «Riprova» e il collegamento alla pagina dei permessi dell'account Google.

**Eliminare l'utente** elimina anche le sue caselle dal server. Rigenerare o cambiare la password non le tocca.

## S6. Consenso

- Il passo «Prima di collegarti» dell'app, se il server dichiara `sync`, dice: «Le caselle che colleghi, con le loro credenziali, vengono salvate su questo server per ritrovarle sugli altri tuoi computer. Chi gestisce il server può accedervi.»
- Prima del consenso e del login l'app non manda nessuna casella (INV-35).
- In Impostazioni → Account c'è l'interruttore generale «Sincronizza le caselle tra i miei computer» (acceso di base dopo il consenso) e, in ogni casella, il suo.

## S7. Rotte (core)

| Rotta | Credenziale | Effetto |
| --- | --- | --- |
| `GET /v1/info` | nessuna | Aggiunge `sync: {available: true}` quando la sincronizzazione è accesa |
| `GET /v1/sync/items?since=` | dispositivo | Elementi dell'utente cambiati dopo una versione, segnaposto di cancellazione compresi |
| `PUT /v1/sync/items/{id}` | dispositivo | Crea o aggiorna, con la versione di partenza; `409 sync_conflict` se è cambiata |
| `DELETE /v1/sync/items/{id}` | dispositivo | Segnaposto di cancellazione |

Tutte passano da `Entitlement` con `Resource::Sync`: nell'enterprise senza abbonamento attivo rispondono `402 subscription_required` e l'app dice che la sincronizzazione riprende con l'abbonamento (scelta mia). Le caselle già presenti sui computer continuano a funzionare.

Il server non scrive mai nei log il contenuto degli elementi; tiene i dati in `server.db`, nella tabella `sync_items`, cifrati con ChaCha20-Poly1305 (nonce casuale per ogni scrittura, dati associati: utente e id dell'elemento).

**La chiave del server.** `QAMILLE_SYNC_KEY`, 32 byte, solo nell'ambiente. Si cambia con `qamille-server sync rekey`, che legge la vecchia da `QAMILLE_SYNC_KEY_OLD` e ricifra tutto. Se la chiave si perde, gli elementi sul server non si aprono più: `qamille-server sync reset` li cancella e i computer, che hanno le caselle in locale, le ricaricano da soli al primo allineamento.

## S8. Che cosa protegge, e che cosa no

**Protegge da:**
- furto del database o dei backup del server, finché la chiave del server resta fuori da entrambi;
- gli altri utenti dello stesso server;
- un dispositivo disattivato, dalla richiesta successiva.

**Non protegge da:**
- **Chi gestisce il server o lo compromette mentre è acceso**: ha la chiave. È la scelta del proprietario, dichiarata nel consenso.
- **Un computer compromesso**, che ha già tutto.
- **Le copie della cartella `data/`**: chi la copia copia anche la sessione del server e le credenziali, come già succede (`docs/portable.md`).

**Da verificare con Google** prima del lancio dell'enterprise: un server che conserva e può leggere i token di Gmail entra nella valutazione di sicurezza degli scope con restrizioni.

## S9. Nell'app

- Impostazioni → Account → **«Sincronizzazione delle caselle»**: stato, ultimo allineamento, interruttore generale.
- In ogni casella, l'interruttore «Sincronizza questa casella».
- Impostazioni → Calcolo → «Dispositivi»: «Disattiva» con la casella «Revoca gli accessi alle caselle».
- Avvisi delle caselle rimosse su un altro computer.
- Senza server, o con un server che non dichiara `sync`, la sezione spiega che la sincronizzazione passa da un server Qamille.

## Non obiettivi

- Sincronizzare le mail, l'indice, le bozze, le riunioni o la rubrica.
- Notifiche istantanee dal server (si usa il controllo periodico).
- Cifratura da un capo all'altro e collegamento dei computer con un codice.
- L'uso delle credenziali da parte del server.

## Invarianti

- **INV-35.** Le credenziali delle caselle lasciano un computer solo dopo il consenso e il login, solo verso il server della sessione; il server le conserva cifrate con una chiave che non sta nel database, nei backup né nei log.
- **INV-36.** Le caselle di un utente le leggono e le scrivono solo i dispositivi attivi di quell'utente.
- **INV-37.** Un dispositivo disattivato, o di un utente disattivato o eliminato, non riceve più nulla dalla richiesta successiva; chi lo disattiva dall'app riceve la proposta di revocare gli accessi alle caselle.
- **INV-38.** Il server limita dimensioni e numero degli elementi per utente e non scrive mai nei log il loro contenuto.

## Esempi

- **EX-36 Due PC.** Giulia collega due Gmail sul fisso. Sul portatile appena installato fa il login con lo stesso utente: dopo pochi secondi il portatile ha le due caselle e comincia a scaricare la posta. Il fisso può essere spento.
- **EX-37 Ricollegamento.** Google chiede di riaccedere a una casella. Giulia lo fa sul portatile; il fisso riceve il token nuovo al controllo successivo, senza fare nulla.
- **EX-38 Portatile rubato.** Dal fisso: «Dispositivi» → «Disattiva» su «Portatile» con «Revoca gli accessi alle caselle». Il portatile non entra più nel server, Google invalida i token e Giulia ricollega le due caselle sul fisso.
- **EX-39 Stessa casella su due PC.** Giulia aveva già collegato la stessa Gmail su entrambi i computer prima di fare il login sul secondo: l'elemento è uno solo, e resta il token più recente.

## Verifica

- Server: elementi per utente, versioni e conflitti, segnaposto, limiti, cifratura a riposo (il database non contiene il testo in chiaro), `rekey` e `reset`, isolamento tra utenti, dispositivo disattivato, `402` nell'enterprise senza abbonamento, niente nei log.
- App: due istanze con lo stesso utente contro un server di prova: una casella IMAP e una Gmail compaiono sulla seconda dopo il login; modifica e conflitto per campi; rimozione remota con avviso; stessa casella su entrambe senza doppioni; revoca di un token Gmail con un finto Google, anche fallita.
- Prova manuale fra due computer Windows veri: **da fare**.

## Differenze emerse nella wave 6 (server)

Scritte dopo aver realizzato il server; l'app (wave 7) parte da qui.

- **`epoch`.** `GET /v1/sync/items` risponde `{version, epoch, items}`. `sync reset` incrementa `epoch` (per chi aveva dati) e non azzera mai `version`. Quando l'app vede un `epoch` diverso da quello che ricorda, il server è ripartito da zero: ricarica le proprie caselle e **non** mostra «rimossa su un altro computer» per ognuna (S3: una casella già sincronizzata che non c'è più vale come rimossa solo se `epoch` è lo stesso).
- **`DELETE` con `base_version` facoltativa.** Senza, la cancellazione è incondizionata; con, `409 sync_conflict` se l'elemento è cambiato. Una cancellazione già fatta risponde `200` con la versione del segnaposto, senza scrivere; un id mai scritto `404`.
- **Conflitto senza elemento.** Se `base_version` si riferisce a un elemento che non c'è (mai scritto, o segnaposto già scaduto), `409 sync_conflict` ha `error.current: null`: l'app riparte da `base_version: 0`.
- **Segnaposto.** Al più `sync_max_items` per utente (oltre, si scartano i più vecchi) e al più 90 giorni; chi non li ha visti trova l'elemento «mancante», come dopo i 90 giorni.
- **Connessione.** Come il cambio password: `https` (o `X-Forwarded-Proto` da un proxy fidato) oppure dal computer stesso; altrimenti `400 tls_required` prima di leggere la chiave o il corpo. Un limite per origine sulle chiavi sbagliate (`429`).
- **Corpo.** `PUT`: `{"base_version": n, "item": {…}}`, campi in più rifiutati; `item` è un oggetto JSON che il server conserva byte per byte. Il corpo ammesso è `sync_max_item_bytes` più 1 KiB (non i 64 KiB delle altre rotte del core); oltre, `413`.
- **Chiave sbagliata o persa.** Il server parte comunque (l'amministratore deve poter entrare); `GET` risponde `500` finché gli elementi non si aprono, e il log lo dice all'avvio senza id né contenuti. Rimedio: la chiave giusta, oppure `sync reset --yes`.
- **Id nei log.** Anche l'id dell'elemento (SHA-256 di fornitore e indirizzo) non va nei log, perché con un dizionario di indirizzi si indovina; nei log c'è la rotta come schema.
