# Rubriche e rubrica unificata: studio di fattibilità

**Prodotto:** Qamille · **Stato:** studio, nessun codice scritto · **Data:** 3 novembre 2026 · **Consegna:** unica, senza fasi.

**Richiesta:** gestione delle rubriche e una rubrica unificata, con controllo delle sovrapposizioni, unione, rubrica master, sincronizzazione, aggiunta e modifica.

Questo documento non è una specifica approvata. Ogni scelta marcata **[mia scelta]** è una proposta da confermare; ciò che è marcato **[verificato]** è stato letto nel codice o nella documentazione ufficiale in questa sessione; **[non verificato]** indica un'ipotesi.

## 1. Stato attuale [verificato nel codice]

Qamille non ha una rubrica. Esiste solo una lista derivata dalla posta.

| Fatto | Dove |
| --- | --- |
| I suggerimenti persone sono ricavati da mittenti, A e Cc delle email, chiave = indirizzo in minuscolo | `app/src/application/mail.rs:511` (`search_suggestions`), `normalized_person` a `:115` |
| Li usa solo la barra di ricerca (datalist) | `desktop/src/components/SearchBar.svelte:139` |
| Il tool MCP `search_contacts` li espone con `source: "observed_mail"` e dichiara "not a synced cloud address book" | `app/src/adapters/mcp/tools.rs:101`, `:667` |
| Il compositore ha campi A/Cc/Ccn come testo libero, senza completamento | `desktop/src/components/Composer.svelte:346-358` |
| Account: `AccountKind::{Sample, Gmail}` (IMAP è alias), con `provider_id` e `oauth_client_id` | `app/src/domain/model.rs:294-340` |
| Il consenso OAuth del calendario è separato da quello della posta e salva il refresh token come segreto `calendar:{id}` | `app/src/infrastructure/oauth.rs:21-23`, `app/src/adapters/mcp/calendar.rs:~340-375` |
| Lo stato di sync esterno (sync token) del calendario vive in un file a parte con lock, scrittura temporanea e rename | `app/src/adapters/mcp/calendar.rs:32-105` (`calendar-state.json`) |
| Il file dati principale è un unico aggregato JSON con transazione atomica e lock di file | `app/src/domain/model.rs:451`, `app/ARCHITECTURE.md:30-37` |
| Il dominio non può usare I/O e i servizi non possono usare adapter concreti: lo impone un test | `app/tests/layers.rs` |
| L'annulla esistente copre solo azioni su email e bozze, non snapshot generici | `app/src/domain/undo.rs:6-22` (`UndoOperation`) |
| La barra laterale ha una sezione "strumenti" (Calendario, Meeting, Assistente) dove aggiungere Rubrica | `desktop/src/components/Sidebar.svelte:~30`, `desktop/src/App.svelte:78` |
| L'anteprima nel browser simula il backend: ogni comando nuovo va replicato in `preview.ts` | `desktop/src/lib/preview.ts:265` |

Conseguenza: la funzione è nuova, ma il progetto ha già i tre schemi da copiare: consenso Google separato (calendario), stato di sync fuori dall'aggregato (calendario), porte e servizi con test a memoria (`tests/services.rs`).

## 2. Modello concettuale

Quattro oggetti. La distinzione tra i primi due è la decisione che rende tutto il resto reversibile.

1. **Rubrica (address book)**: una sorgente di contatti. Tipi: `local` (di proprietà di Qamille), `google:{account_id}` (People API), `visti-nella-posta` (virtuale, sola lettura, è la lista di oggi), più avanti `carddav:{account_id}` e import `.vcf`.
2. **Contatto sorgente**: la scheda così com'è in una rubrica. Conserva id remoto, etag e l'ultima copia sincronizzata (la "base" per il confronto a tre vie).
3. **Contatto unificato**: un gruppo di contatti sorgente che rappresentano la stessa persona. È un **collegamento**, non una copia: i sorgenti restano intatti.
4. **Rubrica master = rubrica virtuale unificata** *(definizione data dall'utente il 3 novembre)*: una rubrica che non esiste su nessun server. È la vista che riunisce tutti i contatti di tutte le caselle e di tutte le rubriche, fonde i doppioni in un'unica scheda e su **ogni scheda riporta le origini**. Non è un contenitore con dati propri: si ricalcola dalle sorgenti e dai collegamenti.

**Origini di un contatto.** Ogni scheda unificata elenca da dove viene, con un'etichetta per origine:

| Origine | Esempio di etichetta | Scrivibile |
| --- | --- | --- |
| Rubrica Google di un account | `Google · alfio@gmail.com` | sì, dopo il consenso di scrittura |
| Rubrica locale di Qamille | `Locale` | sì |
| Importata da file | `vCard · famiglia.vcf` | sì (diventa locale) |
| Vista nella posta di una casella (come oggi) | `Posta · lavoro@azienda.it` | no, sola lettura |
| CardDAV (fuori perimetro, §11) | `CardDAV · account` | sì, se aggiunto |

Una persona con scheda Google, scritta più volte a lavoro e già presente nella rubrica locale mostra tre etichette, e il dato conserva per ognuna id remoto, stato di sincronizzazione e data dell'ultimo aggiornamento.

**Contatti nuovi** *(deciso dall'utente il 3 novembre)*: creando un contatto si **sceglie obbligatoriamente una rubrica** tra quelle scrivibili, compresa la rubrica locale portable (`Locale`, salvata in `data/` accanto all'eseguibile). Non esiste una destinazione implicita: il modulo non si salva senza quella scelta. Il modulo propone preselezionata l'ultima rubrica usata, `Locale` la prima volta. **[mia scelta per la preselezione]**

**Ordine di priorità delle origini**: decide quale valore mostrare quando un campo a valore singolo differisce tra le rubriche (§4.2). È un'impostazione, non una rubrica a sé. **[mia scelta]**

Ispirazione: schede unificate/collegate dei sistemi operativi mobili **[non verificato in questa sessione]**. Il principio è che l'unione sia una vista derivata e annullabile, non una fusione distruttiva.

## 3. Controllo delle sovrapposizioni

### 3.1 Normalizzazione (funzioni pure, nel dominio)

- **Email:** minuscolo, trim. Non togliere i punti né il `+tag`: vale per Gmail ma non per ogni provider, e un falso accorpamento costa più di un duplicato mancato. **[mia scelta]**
- **Telefono:** E.164 con regione predefinita ricavata dalla locale (`Settings.locale`, sottotag regione, es. `it-IT` → IT; se vuota, dal sistema). Il dominio oggi non conosce un paese (`app/src/domain/regional.rs`): va aggiunto un piccolo passaggio che lo risolve. Senza regione valida, confronto sulle sole cifre finali, marcato come debole.
- **Nome:** minuscolo, senza diacritici, apostrofi e punteggiatura rimossi, token ordinati ("Rossi Mario" = "Mario Rossi"), titoli e suffissi ignorati.

### 3.2 Livelli di confidenza (regole esplicite, non punteggio opaco)

| Livello | Condizione | Effetto |
| --- | --- | --- |
| A, certo | stesso indirizzo email normalizzato, oppure stesso telefono E.164, oppure stesso `UID` vCard | Una sola scheda nella vista unificata, **e segnalazione** all'utente (§4.1) |
| B, probabile | stesso nome normalizzato e almeno un indizio concordante (stessa azienda, stesso dominio email, nome+cognome identici) | **Segnalazione** all'utente (§4.1); nella vista restano schede separate finché non decide |
| C, possibile | nome simile (distanza di stringa sotto soglia) senza altri indizi | Compare solo nel pannello "Duplicati", senza avviso |

Per semplicità, i livelli non si vedono nell'interfaccia: il pannello mostra una lista ordinata dal più certo al meno certo, e i pulsanti sono gli stessi per tutti.

La vista unificata è una proiezione di sola lettura: mostrare come una sola scheda un gruppo di livello A non scrive nulla da nessuna parte. Ciò che modifica le rubriche è sempre e solo la risposta dell'utente alla segnalazione.

**Gruppi di 2, 3 o più schede.** Lo stesso contatto può stare in due, tre o più rubriche (due account Google, la rubrica locale, un file `.vcf` importato, e così via). Il rilevamento produce quindi **gruppi** di schede, non coppie. Attenzione alla catena: se A coincide con B per email e B con C per telefono, A e C possono essere persone diverse. Perciò il gruppo si forma solo con collegamenti di livello A; con il livello B si propongono coppie, e l'utente vede e può togliere ogni membro del gruppo prima di confermare.

Eccezioni obbligatorie:
- **Indirizzi condivisi o di ruolo** (`info@`, `vendite@`, `famiglia@`, `noreply@`): due persone diverse possono condividerli. Il livello A non scatta su una lista di ruoli, e l'utente può marcare un indirizzo come "condiviso".
- **"Persone diverse":** coppie rifiutate vengono ricordate e non si ripropongono mai.

### 3.3 Prestazioni

Il confronto a coppie su tutta la rubrica non scala: servono chiavi di raggruppamento (blocking): email, telefono, token di nome normalizzato. Una sola chiave perde coppie e non è recuperabile dopo; ne usiamo più di una.

Evidenza: la letteratura sul record linkage (modello Fellegi–Sunter; il blocking scambia richiamo con trattabilità e la scelta non si corregge a valle) è nota **ma in questa sessione ho letto solo estratti di ricerca, non gli articoli**. Le **soglie numeriche** per il livello C sono quindi **non misurate**: vanno tarate con il protocollo di test del §10 su dati sintetici italiani, non assunte.

## 4. Unione

### 4.1 Segnalazione e domande all'utente *(richiesta del 3 novembre: tutto in un'unica consegna)*

**Quando si segnala.** Mai con una finestra bloccante per ogni caso. Tre momenti:

1. **Salvataggio di un contatto** (nuovo o modificato) che coincide con altri: avviso nel modulo prima di salvare, con i contatti simili e le loro origini.
2. **Importazione `.vcf` e sincronizzazione** che portano doppioni: un solo avviso riepilogativo ("12 potenziali duplicati") che apre il pannello. Durante la prima sincronizzazione di migliaia di schede non compare nulla finché non finisce.
3. **Pannello "Duplicati"**, sempre disponibile: gruppi di livello A e B, più il livello C.

**Cosa si chiede.** Una finestra con il gruppo (2, 3 o più schede affiancate, ognuna con la sua origine) e **tre pulsanti**:

| Pulsante | Effetto |
| --- | --- |
| **Stessa persona** | Le schede diventano una sola nella rubrica virtuale. Nessuna rubrica viene toccata. È la risposta sicura, e l'unica possibile senza consenso di scrittura |
| **Unisci…** | Apre la scelta di campi e rubriche (sotto) e scrive sulle rubriche |
| **Persone diverse** | Restano separate; la coppia non viene più proposta |

**Unisci…** mostra una scheda sola, già compilata con la regola di §4.2, dove ogni campo si può cambiare, e la domanda **Dove tenerla?** con una casella per ogni rubrica del gruppo e le scorciatoie **Solo in una** e **In tutte**. Le rubriche spuntate ricevono la scheda (creata se mancava); da quelle non spuntate la copia viene **eliminata**. L'anteprima elenca per rubrica che cosa succede ("Google · alfio@gmail.com: aggiornata · Locale: eliminata") e chiede conferma, perché è l'unica operazione che elimina dati sui server.

Casi particolari:
- **Origini "Posta"** (sola lettura): non sono una casella, non si eliminano, non provocano segnalazioni da sole. Un gruppo fatto di una sola scheda scrivibile più una "Posta" chiede solo "Aggiungi ai contatti?".
- **Rubrica senza consenso di scrittura:** casella disattivata con la spiegazione; il resto procede.
- **Esito parziale:** l'esito è mostrato per rubrica; ciò che fallisce resta in coda e si ritenta senza ripetere le parti riuscite.

Il vecchio "Aggiorna" (copiare i campi mancanti senza eliminare nulla) è **Unisci…** con "In tutte": non serve un pulsante a parte. **[mia scelta, per semplicità]**

### 4.2 Sopravvivenza dei campi

- **Campi a valore singolo** (nome visualizzato, azienda, ruolo, compleanno): vince l'origine più in alto nell'ordine di priorità; a parità, il valore più recente per data di modifica. L'utente può fissare un valore campo per campo (scelta ricordata nel collegamento) e vederne l'origine. Le origini "Posta" hanno priorità minima: il loro nome è quello scritto dal mittente e spesso è sbagliato.
- **Campi multipli** (email, telefoni, indirizzi): unione senza duplicati sul valore normalizzato, con etichetta (casa/lavoro) conservata.
- **Note:** concatenate con indicazione della provenienza, mai scartate.

La specifica vCard 4.0 (RFC 6350 §7.1) conferma l'impianto: le schede con `UID` equivalente DEVONO essere abbinate; altrimenti l'abbinamento è "a discrezione del motore di sincronizzazione"; le proprietà con cardinalità 1 vengono abbinate, quelle multiple per nome e parametro `PID` **[verificato]**. Quindi le euristiche del §3 sono legittime ma restano una nostra scelta.

### 4.3 Annulla

Oggi l'annulla copre solo posta e bozze (`domain/undo.rs:6`). Per le unioni serve un **diario di unione** proprio: prima di Unisci… si salva lo stato delle schede coinvolte; "Annulla unione" le ripristina. "Stessa persona" si annulla togliendo il collegamento, senza diario. Per i sorgenti remoti cancellati, il ripristino è una nuova creazione, con nuovo id remoto: va detto all'utente. Vale per ogni rubrica del gruppo (2, 3 o più) e anche per un'unione riuscita solo in parte: il diario registra l'esito per rubrica, e l'annulla ripristina solo ciò che è stato davvero scritto. **[mia scelta]** Il diario è conservato per un periodo configurabile.

## 5. Sincronizzazione

### 5.1 Google (People API) [verificato su documentazione ufficiale]

| Tema | Fatto | Implicazione |
| --- | --- | --- |
| Lettura incrementale | `people.connections.list` con `requestSyncToken=true`, poi `syncToken`; i cancellati arrivano con `metadata.deleted` | Come il calendario: un token per account |
| Scadenza token | "Sync tokens expire 7 days after the full sync"; token scaduto → errore `EXPIRED_SYNC_TOKEN` | Se l'app resta chiusa più di 7 giorni serve una risincronizzazione completa. Prevedere il caso come normale |
| Pagina | `pageSize` da 1 a 1000, predefinito 100 | Usare 1000 per la prima sincronizzazione |
| Modifica | `updateContact` richiede `metadata.sources.etag` e `updatePersonFields`; l'etag fa fallire l'aggiornamento se il contatto è cambiato | Stesso schema dell'aggiornamento ottimistico delle bozze (`ARCHITECTURE.md:61`) |
| Scritture | Le richieste di modifica per lo stesso utente vanno inviate **in sequenza** | Coda scritture seriale per account |
| Propagazione | "Writes may have a propagation delay of several minutes for sync requests" | Mai rileggere via sync dopo una scrittura: applicare localmente la risposta della scrittura |
| Solo contatti | Solo i contatti veri sono modificabili; i profili (`people/me`) sono sola lettura | Il proprio profilo non entra nella rubrica modificabile |
| "Altri contatti" | Esistono come raccolta separata; `copyOtherContactToMyContactsGroup` li promuove | Utile al pulsante "Aggiungi ai contatti" |
| Scope | `contacts` (lettura, modifica, eliminazione), `contacts.readonly`, `contacts.other.readonly`; elenco confermato | Consenso separato, come `CALENDAR_SCOPE` |
| Quote scrittura | Tabella con costo per richiesta (creazione/aggiornamento singoli = 1 scrittura contatto; batch con costi maggiori) | Il valore del limite giornaliero totale **non** è nella pagina letta: da verificare prima di scrivere in blocco |
| Classificazione scope (sensibile/riservato) e verifica Google | **Non confermata** dalle pagine lette | Controllare in Cloud Console → Accesso ai dati prima di pianificare la distribuzione pubblica |

Il consenso `contacts.readonly` si chiede al collegamento della rubrica; `contacts` (scrittura) solo alla prima operazione che scrive su Google, per esempio la prima modifica o la prima risposta "Unisci" (consenso incrementale **[mia scelta]**). Chi non concede la scrittura mantiene la sola lettura e la casella di quella rubrica resta disattivata nella finestra di unione.

Lacuna in `docs/oauth-google.md` (passo 3): per i contatti va abilitata anche la People API nello stesso progetto, come già per Gmail e Calendar.

### 5.2 Politica dei conflitti

Servono la **base** (ultima copia sincronizzata), la **versione locale** e la **versione remota**. Per ogni campo: se è cambiato solo da una parte, si applica; se è cambiato da entrambe in modo diverso, conflitto da mostrare. Etag diverso al momento della scrittura = riprova dopo nuova lettura, mai sovrascrittura cieca.

### 5.3 Offline

Le modifiche fatte senza rete vanno in una coda persistente (stesso principio dell'`outbox` della posta), con stato visibile per contatto: sincronizzato, in attesa, in conflitto, errore.

### 5.4 Account IMAP non Google

Due strade: **CardDAV**, fuori perimetro (RFC 6352; ogni vCard ha un `UID` univoco nella collezione; i client usano il REPORT `sync-collection` quando il server lo supporta, secondo la documentazione di DAVx⁵ **[verificato come descrizione di un client]**; il testo di RFC 6578 non è stato letto), e **import/export `.vcf`**, che è nel perimetro e funziona con qualunque servizio.

## 6. Dove salvare i dati [mia scelta, da confermare]

| Opzione | Pro | Contro |
| --- | --- | --- |
| A. `contacts` dentro `Store` (con `#[serde(default)]`) | Atomico con il resto, incluso nell'export portabile già esistente | Ogni sync riscrive tutto l'aggregato e tiene il lock che serve anche alla posta; migliaia di schede lo ingrossano |
| B. File separato dietro una porta `ContactStore`, stesso schema lock/temp/rename del calendario | Isola il traffico di sync dalla posta; nessuna contesa sul lock principale | Va incluso nell'export/import portabile; coerenza tra due file da gestire |

**Cosa si salva e cosa si ricalcola.** La rubrica virtuale non è un archivio: si salvano solo i contatti sorgente (con id remoto, etag e ultima copia sincronizzata), i **collegamenti** confermati dall'utente, le coppie "non sono duplicati", i valori fissati a mano e i contatti nascosti. I collegamenti automatici di livello A e la scheda unificata con le sue etichette di origine si ricalcolano a ogni caricamento o sincronizzazione. Salvare anche il risultato creerebbe due fonti di verità che divergono. **[mia scelta]**

Consiglio **B**, ma con la porta nell'applicazione e l'implementazione nell'infrastruttura. Il calendario tiene il proprio stato dentro un adapter MCP, che `ARCHITECTURE.md:55-59` definisce debito tecnico da non ripetere. Da verificare: come l'export portabile enumera i dati (`app/src/application/workspace.rs`, `app/tests/portable_workspace.rs`) per includere il nuovo file.

## 7. Punti di innesto

| Livello | File nuovi / toccati |
| --- | --- |
| Dominio | `domain/contacts.rs`: `Contact`, `AddressBook`, `ContactLink`, normalizzazione, regole §3, sopravvivenza §4 (tutto puro, testabile) |
| Applicazione | `application/contacts.rs`: `ContactService`; porte `ContactStore`, `ContactGateway` in `ports.rs`; riuso `SecretStore`, `Clock` |
| Infrastruttura | `google_people.rs` (modellato su `google_calendar.rs`: `ureq`, Bearer, hook di autorizzazione), `contact_repository.rs` (JSON), `vcard.rs` (lettura/scrittura 3.0 e 4.0); `oauth.rs`: `CONTACTS_SCOPE` |
| Tauri | `desktop/src-tauri/src/commands/contacts.rs`, registrazione in `main.rs:163`, composizione in `runtime.rs` |
| Interfaccia | `Contacts.svelte` (lista, scheda, modifica, pannello duplicati, finestra unione), voce in `Sidebar.svelte`, tipo `Folder` in `types.ts`, `Settings.svelte` (rubriche, ordine di priorità) |
| Integrazioni UI | Compositore A/Cc/Ccn con completamento dalla rubrica unificata; barra di ricerca da `search_suggestions` alla rubrica con ripiego su "visti nella posta"; lettore: "Aggiungi ai contatti" sul mittente |
| Anteprima | `preview.ts`, `sample.json`, testi in `backend-text.ts`, `tr(it, en)` |
| MCP | `search_contacts` con campo `source` reale; `list_address_books`, `get_contact`, `find_duplicates` in sola lettura; creazione/modifica/unione sotto `Access`; aggiornare `docs/mcp-tools.json` |

La rubrica "visti nella posta" resta com'è oggi come sorgente virtuale a priorità minima: nessuna regressione.

## 8. Interfaccia (bozza)

- **Rubrica** nella barra laterale: lista unificata con ricerca, filtro per rubrica di origine, badge "2 sorgenti".
- **Scheda contatto:** campi, **etichette delle origini** sotto il nome (anche nella lista), stato di sincronizzazione per origine, modifica con scelta di dove scrivere.
- **Modifica di un contatto unificato:** non esiste un "record master" da salvare. Si modifica un campo e l'app scrive nelle origini scrivibili che ne hanno bisogno (anteprima: "verrà aggiornato su Google · alfio@gmail.com e su Locale"). Un contatto che esiste solo come "Posta" non è modificabile: il pulsante "Aggiungi ai contatti" apre il modulo di creazione precompilato, con la stessa scelta obbligatoria della rubrica, e lo collega.
- **Eliminazione:** per origine, mai silenziosa. Un contatto "Posta" non si elimina, si **nasconde** dalla vista.
- **Duplicati:** una lista di gruppi, dal più certo al meno certo, con i tre pulsanti di §4.1: Stessa persona · Unisci… · Persone diverse.
- **Unisci…:** schede affiancate sopra, scheda risultante modificabile sotto, caselle delle rubriche, anteprima per rubrica, conferma.
- **Filtro per origine:** "Tutte", oppure una casella o rubrica singola; stesse etichette delle schede.
- **Nuovo contatto:** modulo con la scelta obbligatoria della rubrica in cima (Locale portable o un account Google con scrittura).
- **Impostazioni → Rubriche:** elenco rubriche, interruttore di sincronizzazione, ordine di priorità delle origini, consenso Google dei contatti.

## 9. Privacy, MCP e AI

- I contatti sono dati personali di terzi. Nessun contatto va a un provider AI (`Settings.ai_provider`) senza una scelta esplicita dell'utente. **[mia scelta]**
- Via MCP l'AI può **proporre** unioni (come `classify_email`, che "never applies changes", `tools.rs:256`), non applicarle. Creare e modificare restano dietro `Access`.
- Il consenso Google dei contatti non va confuso con quello della posta: schermata, messaggio e revoca separati.

## 10. Test

Seguire `app/tests/services.rs`: repository a memoria e gateway finti.

- **Dominio:** corpus sintetico italiano con apostrofi ("D'Angelo"), accenti, ordine invertito, omonimi, stesso nome con email diverse, indirizzi condivisi, `+tag`, numeri con e senza prefisso.
- **Protocollo di taratura soglie (eseguito il 3 novembre 2026, `app/tests/contacts_corpus.rs`):** 217 coppie sintetiche etichettate **per costruzione** (nessun annotatore umano). Ipotesi H1 "il livello Probabile non propone mai persone diverse": **confermata** (0 errori su 25 trovate; richiamo 0,217 perché solo 25 positivi hanno la stessa azienda). Ipotesi H2 "precisione del livello Possibile ≥ 0,90": **falsificata** (0,578 a 0,92, 0,56-0,58 a ogni soglia: 30 omonimi di aziende diverse e 38 cognomi quasi uguali su 42 segnalati). Dopo aver visto i numeri ho cambiato la regola (stesso nome ma due aziende diverse = prove contrastanti, non si propone) e formulato H2' "precisione ≥ 0,70, nessun omonimo, richiamo oltre la base": **0,710 a 0,92**, richiamo 0,809 contro 0,235 del confronto esatto. I cognomi quasi uguali ("Ferrari"/"Ferraro") restano segnalati perché la similarità di stringa non distingue un refuso da una persona diversa: il livello Possibile compare solo nel pannello, senza avviso. La soglia cambia il richiamo (0,87 a 0,88; 0,75 a 0,94), non la precisione; resta 0,92. I numeri valgono per questo corpus, non per rubriche reali.
- **Sync:** token scaduto → ripartenza completa; etag obsoleto → nessuna sovrascrittura; scrittura in sequenza; cancellazione remota; coda offline.
- **Unione:** Stessa persona e il suo annulla; Unisci… con "Solo in una" e "In tutte"; Persone diverse ricordato; annulla con ripristino di una scheda remota eliminata; esito parziale.
- **Architettura:** `layers.rs` deve restare verde (dominio senza I/O).

## 11. Consegna unica *(nessuna suddivisione in fasi, su richiesta del 3 novembre)*

Si rilascia una volta sola, quando tutto il perimetro funziona. Perimetro:

- rubrica locale con aggiunta, modifica, eliminazione, import ed export `.vcf`;
- Google: consenso di lettura e scrittura, sincronizzazione incrementale con ripartenza dopo la scadenza del token, "altri contatti", scrittura con etag in sequenza, coda offline;
- rubrica virtuale con le origini su ogni scheda e filtro per origine;
- rilevamento dei gruppi di duplicati (2, 3 o più schede) e segnalazione nei tre momenti del §4.1;
- pulsanti Stessa persona / Unisci… / Persone diverse, con scelta delle rubriche in cui tenere il contatto, anteprima per rubrica, esito parziale e ritentativi;
- **fuori perimetro, per semplicità:** gruppi ed etichette dei contatti (Famiglia, Clienti), foto del contatto. I dati non vanno persi: la sincronizzazione li conserva così come arrivano e li riscrive invariati; solo non si vedono né si modificano **[mia scelta]**;
- diario delle operazioni e annulla;
- completamento degli indirizzi nel compositore, ricerca che usa la rubrica, "Aggiungi ai contatti" dal lettore;
- tool MCP (lettura sempre, scrittura sotto `Access`), anteprima nel browser, testi in italiano e inglese, guida Google aggiornata.

Fuori perimetro salvo tua indicazione: **CardDAV** per gli account non Google. Motivo: non posso verificarlo senza un server reale e aggiunge un protocollo intero; l'import `.vcf` copre il caso minimo.

Stima mia, non misurata: l'insieme equivale a circa 6-8 sessioni di lavoro con revisione. L'ordine interno di costruzione (dominio → archivio → servizio → Google → interfaccia → integrazioni) è un dettaglio del piano di realizzazione, non una consegna a tappe.

**Verifica:** i test automatici usano un archivio in memoria e un gateway Google finto. L'accesso reale a Google non è verificabile senza il progetto OAuth e il consenso dell'utente (come già per posta e calendario in `docs/oauth-google.md`): questa parte va provata da te sul tuo account.

## 12. Decisioni aperte (servono a te)

1. ~~Cosa significa "master"~~ **Risolta dall'utente (3 novembre):** rubrica virtuale unificata che fonde i doppioni e riporta le origini su ogni scheda (§2, punto 4).
2. ~~Unione distruttiva sul server~~ **Risolta dall'utente (3 novembre):** inclusa. L'utente sceglie per ogni gruppo se tenere il contatto in una sola rubrica o in tutte (§4.1), anche con 2, 3 o più rubriche.
2a. ~~Significato di "Aggiorna" e "Unisci"~~ **Semplificato (3 novembre):** tre pulsanti, Stessa persona / Unisci… / Persone diverse; "Aggiorna" è Unisci… con "In tutte" (§4.1).
2b. ~~Dove nascono i contatti nuovi~~ **Risolta dall'utente (3 novembre):** scelta obbligatoria della rubrica a ogni creazione, Locale portable compresa (§2).
3. **Sorgenti nella consegna unica:** locale + `.vcf` + Google. CardDAV è fuori perimetro salvo tua indicazione (§11).
4. **Dove salvare:** B (file separato) consigliato, oppure A (dentro `Store`).
5. **Distribuzione:** uso personale (modalità Testing, token a 7 giorni già documentati in `docs/oauth-google.md`) o pubblica (verifica Google, da chiarire).

## 13. Evidenze consultate e limiti

- **Archivio ricerche del progetto:** `docs/research/` non esiste; nessuna ricerca riusabile. Esito del controllo scientifico: ricerca nuova ma **non pubblicata** come record (nessun permesso di scrittura in `docs/research/` concesso); le evidenze stanno solo in questo studio.
- **Documentazione ufficiale letta:** People API (sincronizzazione, creazione/aggiornamento, "altri contatti", `connections.list`), elenco scope OAuth Google, RFC 6350 sezione 7 e definizioni UID/REV/PRODID/KIND/MEMBER. Il testo di RFC 6350 è stato letto fino a circa 100.000 caratteri su 139.895: la sezione 7.2.4 (modifica simultanea) è tagliata.
- **Letteratura sul record linkage:** solo estratti di ricerca (Fellegi–Sunter; blocking). **Nessun articolo aperto**: le soglie del §3 non hanno supporto empirico locale.
- **Non verificato:** classificazione degli scope contatti come sensibili/riservati; limite giornaliero totale di scritture; testo di RFC 6578; comportamento delle schede unificate dei sistemi operativi mobili; come l'export portabile enumera i dati.

### Fonti

- Google People API, contatti: https://developers.google.com/people/v1/contacts
- Google People API, `people.connections.list`: https://developers.google.com/people/api/rest/v1/people.connections/list
- Google People API, altri contatti: https://developers.google.com/people/v1/other-contacts
- Scope OAuth 2.0 per le API Google: https://developers.google.com/identity/protocols/oauth2/scopes
- RFC 6350, vCard 4.0: https://www.rfc-editor.org/rfc/rfc6350.txt
- RFC 6352, CardDAV: https://www.greenbytes.de/tech/specs/rfc6352.html
- DAVx⁵, informazioni tecniche: https://manual.davx5.com/technical_information.html
- Record linkage, stato dell'arte (non aperto): https://pubs.dbs.uni-leipzig.de/dc/node/754
