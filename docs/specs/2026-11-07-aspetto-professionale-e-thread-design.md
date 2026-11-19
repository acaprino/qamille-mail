# Aspetto professionale, colpo d'occhio e thread nella lista

**Stato:** bozza da approvare, 2026-11-07. Nessun file dell'app è stato modificato e nulla è stato committato.
**Riferimento visivo (da guardare, non da copiare):** anteprima locale in `checks/claude-design/preview/` (la cartella `checks/` è ignorata da git): `glance.css`, `glance.js`, `server.mjs`, `varianti.html`. Gira sopra una build di `desktop/` ed è legata a una build precisa. Il codice dell'app va scritto seguendo questo documento, non portando i file dell'anteprima.
**Convenzione:** **[tua scelta]** = l'ha deciso l'utente, con le sue parole. **[mia scelta]** = l'ho deciso io; l'utente ha visto il risultato nell'anteprima ma non ha scelto quel punto.
**Revisione:** il documento è stato riletto da un revisore indipendente che non conosceva la conversazione. Le sue 9 domande e i 4 errori sul codice che ha trovato sono risolti nel testo e riassunti nel §12.

## 1. Intento

L'utente legge posta tutta la giornata e ha l'ADHD: si perde nel mare di informazioni, non legge tutto e ha bisogno di capire una schermata con un colpo d'occhio. Parole sue: «più gerarchia visiva d'impatto per chi soffre di adhd e si perde nel mare di informazioni o non legge tutto e ha bisogno con un colpo d'occhio di capire». In più vuole un aspetto professionale, pulito e moderno, «come Outlook o Spark Mail».

Quando i due criteri confliggono vince il colpo d'occhio. La finitura estetica viene dopo.

## 2. Obiettivi, non-obiettivi, anti-obiettivi

**Obiettivi**
1. In Posta in arrivo, guardando solo la lista, si capisce: quante cose richiedono me (priorità), quante sono da leggere, quale messaggio leggere per primo.
2. Nelle righe esistono due soli pesi: ciò che riguarda me (non letto, priorità) è scuro e in grassetto, il resto arretra.
3. La conversazione si apre dentro la lista come righe, e ogni risposta che ne ha altre sotto si può chiudere.
4. Un messaggio con molti destinatari non allarga né allunga nessuna riga e non spinge il testo fuori vista.
5. Aspetto professionale: cornice grigia con un solo foglio bianco, un solo blu, testo più scuro e più grande.

**Non-obiettivi**
- Ridisegnare Calendario, Rubrica, Meeting, Assistente, Impostazioni: ereditano solo colori e caratteri.
- Nuove funzioni di posta.
- Un'impostazione per scegliere il carattere (idea per dopo, vedi §7).
- Leggere l'intestazione `References` (vedi §5, fuori ambito).
- Conversazioni che attraversano account diversi.

**Anti-obiettivi** (sarebbe un fallimento anche se tutto funziona)
- Nascondere qualcosa senza lasciare una traccia visibile.
- Righe che cambiano altezza, o gruppi e conversazioni che si aprono, si chiudono o cambiano ordine da soli *mentre l'utente le sta guardando*. L'ordinamento della lista resta quello di oggi (per data, con i blocchi già esistenti per le righe appena spostate); una conversazione aperta resta aperta e mantiene il suo ordine interno quando arriva posta. Questo progetto non aggiunge alcun riordinamento.
- Un colore con due significati.
- Un'azione sulla conversazione (Archivia, Fatto, Fissa, Elimina) che tocca messaggi che la vista non mostra, per esempio quelli inviati.
- Un testo non tradotto: ogni stringa nuova ha italiano e inglese (`tr(it, en)`).

## 3. Cosa ha deciso l'utente

- **[tua scelta]** «ok» alla direzione vista nell'anteprima (cornice e foglio, palette, gerarchia delle righe, gruppi minori chiusi).
- **[tua scelta]** Thread: «dovrebbe essere il thread gerarchico proprio nella lista di email», non nel pannello di lettura. Poi «lo vorrei a righe e non ad albero». Poi, visti 5 livelli di rientro: «è confusionario… fammi vedere le varianti, e poi devo poter collassare un nodo». Tra le 4 varianti ha scelto la **D**: nessun rientro, righe tutte sulla stessa linea, il legame scritto accanto al nome («a Maria Rossi»).
- **[tua scelta]** «Attenzione quando in "to" ci sono tanti elementi.»
- **[tua scelta]** Riga principale di una conversazione: **il messaggio più recente**, come oggi.
- **[tua scelta]** «Seleziona caricati»: **nascosta finché non si spunta una riga** (scelta «B»). Quella barra serve solo a stampare o scaricare in `.mbox` i messaggi spuntati; non archivia né cancella.
- **[tua scelta]** Usare UI Pro Max e il taste skill come guida di stile.

## 4. Il progetto

### 4.1 Cornice e stile

- **[mia scelta]** Sidebar e barra superiore su un fondo grigio (`--ground`), senza linea tra loro. Il lavoro sta su un solo foglio bianco con 1px di bordo, angoli di 14px (`--radius-sheet`), staccato di 8px dal bordo destro e da quello in basso. Nessuna ombra a riposo (`--shadow-sm` diventa nulla).
- **[mia scelta]** Un solo blu per agire. Le voci attive della sidebar sono una pillola neutra, non blu.
- **[mia scelta]** Testi più scuri e più grandi: scala 12/13/14/15/16/18/22/26px al posto di 11/12/13/14/16/18/20/24px. Valori e contrasti nell'Appendice A.
- **[mia scelta]** Caratteri: **Source Sans 3** per tutto ciò che si scansiona, **Lexend** solo per i titoli e il marchio. Vedi §7: nessuno studio indipendente dimostra che Lexend aiuti i lettori con ADHD; è una scelta di aspetto e di densità.
- **Vincolo:** l'app è offline e la CSP ha `font-src 'self'` (`desktop/src-tauri/tauri.conf.json:15`). L'anteprima caricava il carattere da Google Fonts, cosa che l'app non può fare. Source Sans 3 va incluso come file, variabile, con la sua licenza OFL, come Lexend in `desktop/public/fonts/lexend/` (file, `OFL.txt`, `README.txt` con la fonte e la data).
- `backgroundColor` della finestra (`tauri.conf.json:13`, oggi `#f8f9fc`) va allineato al nuovo fondo per non far lampeggiare il vecchio colore.
- **Regola ADR-0009:** ogni colore, ombra, raggio, livello e dimensione di testo viene da un token di `theme.css`. `desktop/tests/style-tokens.mjs` deve restare verde. L'anteprima usa qualche valore letterale (per esempio il bordo `1.5px` e il testo `11px` dell'avatar di risposta): nel codice vero diventano token.

### 4.2 La lista di Posta in arrivo

- **Titolo.** Sotto «Posta in arrivo» la riga di stato dice **«N priorità · N da leggere»** (in inglese «N priority · N to read») al posto del totale. Vale solo per Posta in arrivo, in modalità Email e Conversazioni, nelle viste Intelligente, Tutti e Non letti. Le altre cartelle e la ricerca mantengono il testo di oggi («N messaggi» o «N conversazioni»). Zero priorità: si omette quella parte. Zero da leggere: un testo coerente tipo «Tutto letto» (lasciato al costruttore). **[mia scelta]**
  - **«Da leggere»** è il numero dei messaggi non letti della Posta in arrivo, lo stesso del badge della sidebar (`counts["Inbox"]`, `app/src/application/workspace.rs:48`): un numero solo, ovunque. Comprende anche i non letti dei gruppi chiusi (Notifiche, Newsletter), che il titolo dei gruppi segnala con il pallino blu. **[mia scelta]**
  - **«Priorità»** è il numero di righe del gruppo Priorità, come la lista le raggruppa, lette o no (sono comunque in grassetto). **[mia scelta]**
  - **Dove si calcolano:** dal backend, sullo stesso ambito della lista (account scelto compreso), non sulle righe caricate: una pagina ne contiene 60 e il numero sarebbe sbagliato oltre. Il badge della sidebar oggi conta tutti gli account; se la lista è filtrata su un account, il titolo mostra il conteggio di quell'account e il costruttore decide se allineare anche il badge. Servono due campi nuovi nella pagina restituita da `list_mail` (`MailPage`, `mail.rs:77-83`).
- **Due pesi.** Non letto, o riga nel gruppo Priorità: mittente e oggetto in grassetto e in `--text`. Tutto il resto: peso normale e colore attenuato. **[mia scelta]**
- **Priorità.** Bordo sinistro di 3px ambra (`--priority-mark`), sfondo appena ambrato (`--priority-wash`, nuovo token), puntino ambra di 8px nell'intestazione del gruppo. **[mia scelta]**
- **Gruppi minori chiusi.** Notifiche, Newsletter e Già letti partono chiusi. Se un gruppo chiuso contiene non letti, l'intestazione resta scura e in grassetto, con un pallino blu e il numero in blu: la traccia non si perde mai. **[mia scelta]** Lo stato aperto/chiuso scelto dall'utente resta finché l'app è aperta e cambiando cartella; non si riapre da solo all'arrivo di nuova posta. **[mia scelta]**
- **Nelle righe di «Già letti»** non si mostra l'anteprima di una riga di testo. **[mia scelta]**
- **Caselle di selezione.** Compaiono solo al passaggio del mouse, con il focus da tastiera o se spuntate, e prendono il posto dell'avatar (senza disco bianco). **[mia scelta]**
- **«Seleziona caricati».** La barra (oggi sempre in cima alla lista, `MailList.svelte:197-232`) compare solo quando almeno una riga è spuntata, con il conteggio, «Cancella», «Stampa selezionati» e «Scarica .mbox». **[tua scelta]** Conseguenza: per selezionare tutto si spunta prima una riga e poi la casella grande della barra.
- **Date.** Restano come oggi (`shortDate`, `regional.ts:99`: ora se è oggi, altrimenti giorno, mese e anno). L'anteprima mostrava «2 nov» senza anno: **non va portato**, perché `docs/ricerca-email-specifica.md` §1.3 chiede date estese. Se a 1000px manca spazio per l'anno, si accorcia prima il destinatario (§4.4). **[mia scelta]**
- **Tastiera.** La lista resta un solo punto di Tab con le frecce che spostano tra le righe (`MailList.svelte:98-103`, `App.svelte:1064`). Le righe di risposta entrano in `visibleOrder`. Freccia destra/sinistra su una riga con risposte sotto la apre/chiude. **[mia scelta, non provata nell'anteprima]**

### 4.3 La conversazione nella lista (variante D)

Si apre con la freccia nella riga del messaggio principale (accanto al numero totale di messaggi). Sotto compaiono le risposte, **una riga ciascuna, tutte sulla stessa linea, senza rientri né linee di collegamento**.

- **Dove vale.** Solo nella modalità Conversazioni (`threads`; senza preferenza salvata l'app parte in modalità Email, `App.svelte:62`), in tutte le cartelle e le etichette. La ricerca resta una lista piatta come oggi. La modalità Email non cambia. **[mia scelta]**
- **Riga principale.** È **il messaggio più recente** della conversazione tra quelli della vista, come oggi: mittente, oggetto e anteprima vengono da lui (`mail.rs:478-509`). **[tua scelta]** Le righe sotto sono **tutti gli altri messaggi**, primo messaggio compreso, senza ripetere quello già in cima. Un clic sulla riga principale apre il più recente.

- **Riga di risposta, due righe di testo:** avatar piccolo (22px), nome e «a …» e data sopra; l'inizio del testo sotto. Il filetto di selezione e di passaggio del mouse copre solo quella riga.
- **Il legame.** Dopo il nome: «a *Persona*», e «+N» se ci sono altri destinatari in «A». **[mia scelta]** *Persona* è l'autore del messaggio a cui si risponde, se è tra i destinatari di «A»; altrimenti il primo destinatario. «+N» conta gli altri destinatari di «A», mai Cc. Se la persona è l'utente si scrive **«a te»** (in inglese «to you»), in scuro: si confronta con gli indirizzi dell'account.
- **Ordine.** Sotto il messaggio principale, le altre in ordine d'albero a partire dal primo messaggio: ogni risposta seguita dalle sue, a pari livello per data crescente. L'ordine serve a chiudere in gruppo; non si vede alcun rientro. Con la stessa data decide l'identificativo (lasciato al costruttore).
- **Oltre il tetto.** Se le risposte superano il tetto di caricamento (§5.2.5), in fondo alla conversazione c'è una riga **«Mostra altre N risposte»** che carica le successive. Il numero totale sulla freccia non cambia. **[mia scelta]**
- **Chiudere un nodo.** Ogni risposta che ne ha altre sotto ha una freccia (chiusa: mostra il numero delle nascoste, in blu se tra loro c'è un non letto). Chiuderla nasconde tutto ciò che sta sotto di lei. La freccia del messaggio principale chiude l'intera conversazione, con il numero totale e la stessa regola per il blu. Le frecce sono piccole (18px): da non ridurre.
- **Stato iniziale.** Tutte le conversazioni partono chiuse. **[mia scelta]** Aprire o chiudere resta scelta dell'utente; nuova posta non cambia lo stato (§2). Nell'anteprima la prima era aperta solo per mostrarla.
- **Non letti.** Ogni risposta non letta ha nome e testo in grassetto e il pallino blu sempre nella stessa colonna a sinistra. Aprirla la segna letta (stesso percorso di oggi per `markAsSeen`, `App.svelte:586-610`: lì una riga di conversazione «non può dire quale messaggio è non letto»; con le righe separate invece sì, e il codice va aggiornato).
- **Aprire una risposta.** Clic (o Invio): la risposta si apre nel pannello di lettura. Clic sul messaggio principale: torna al principale.
- **Cosa possono le righe di risposta.** Nessuna casella di selezione, nessuna puntina, nessun trascinamento: la selezione, la puntina e il trascinamento restano sulla riga principale e agiscono su tutti i messaggi della vista (`message_ids`), come oggi. **[mia scelta]**
- **Azioni nel pannello di lettura.** I pulsanti del lettore e il tasto `E` agiscono oggi su un solo messaggio (`App.svelte:837`, `:1060`). Su un messaggio mostrato ma non della vista (per esempio una risposta inviata, che sta in Inviati) **Archivia, Fatto, Posticipa, Fissa, Segna come non letto ed Elimina sono disattivati**, con la spiegazione «Messaggio inviato: si gestisce da Inviati». Rispondi e Inoltra restano attivi. Sui messaggi della vista tutto resta come oggi. **[mia scelta]**
- **Mouse e puntina.** Sul messaggio principale la puntina (Fissa) sta sulla riga dell'anteprima, il numero/freccia sulla riga dell'oggetto: non si toccano. Passando sul principale si evidenzia solo il principale; sulla risposta, solo la risposta.
- **Pannello di lettura.** Quando la lista mostra le risposte (modalità Conversazioni), la sezione «Messaggi della conversazione» del lettore (`Reader.svelte:231-266`) non serve più e si nasconde. Il lettore la mostra oggi solo in modalità Conversazioni (`App.svelte:1325`: `conversation={threads ? threadEmails : []}`): in modalità Email non c'è nulla da conservare. Conseguenza accettata: i messaggi in Cestino o Spam, che oggi quella sezione mostra, non compaiono più nella lista (§5.2.4). **[mia scelta]**

### 4.4 Molti destinatari

- **Nella lista:** la riga mostra «a *Persona* +N» e **non cambia mai altezza**. Se manca spazio si accorcia per primo il destinatario, mai il «+N» né la data. Il mittente si accorcia solo se supera il 46% della riga. Il nome «te» è in scuro. Il titolo (tooltip) elenca al massimo 12 nomi e «e altri N».
- **Nomi o indirizzi.** `to` e `cc` contengono oggi solo indirizzi: il parser scarta i nomi (`mail.rs:448-450`, `p.address()`), e il lettore stampa gli indirizzi (`Reader.svelte:274`). Il nome mostrato è quello già noto per quell'indirizzo: il nome con cui quell'indirizzo compare come mittente o destinatario in un messaggio salvato (`search_suggestions`, `domain::contact_view::observed_people`), altrimenti l'indirizzo stesso, che si accorcia con i puntini come un nome lungo. Nessuna colonna nuova e nessun riempimento di dati vecchi. **[mia scelta]** **La Rubrica non è consultata** in questa versione: `MailService` non la vede (verificato leggendo `application/mail.rs`). Un lavoro successivo può aggiungerla.
- **Nel pannello di lettura** (`Reader.svelte:268-280`, che oggi scrive tutti i destinatari di seguito): tre nomi, l'utente per primo, poi **«e altri N»** cliccabile. «A» e «Cc» stanno su righe separate, ognuna col suo conteggio. Aprendolo, la lista parte dall'inizio, con l'etichetta, in un riquadro di al massimo 96px che scorre da solo; sotto c'è «Mostra meno». Il messaggio scende al massimo di circa 100px.
- Un indirizzo o un nome molto lungo non allarga mai il pannello.
- Il difetto del pannello di lettura esiste già oggi: i dati di esempio hanno un solo destinatario, quindi non si è mai visto.

### 4.5 Pannello di lettura

- Titolo in Lexend a 26px. Un'etichetta «Priorità» in ambra davanti al percorso. L'azione principale (Rispondi) resta l'unico pulsante blu; «Fatto» diventa un pulsante con solo il bordo.
- «Aggiungi ai contatti» resta visibile (non solo al passaggio del mouse). **[mia scelta]**
- Il riassunto AI è più vicino al titolo e a 15px. Il corpo del messaggio è a 16px, interlinea 1,6, larghezza massima 68 caratteri.
- Con la finestra stretta (1000px, minimo di `tauri.conf.json`) la barra degli strumenti tiene «Fatto» dentro il pannello (misurato a 1000, 1100, 1200 e 1440px).

## 5. Dati e backend

### 5.1 Cosa c'è oggi (verificato leggendo il codice)

- **`thread_id` della posta ricevuta** è il valore di In-Reply-To, altrimenti il Message-ID del messaggio (`app/src/infrastructure/mail.rs:505-513`). Per una risposta ricevuta è già il **genitore diretto**.
- **Per una risposta inviata** da Qamille, `thread_id` copia il `thread_id` del messaggio a cui si risponde (`app/src/application/drafts.rs:261`): corretto se si risponde alla radice, un livello troppo in alto se si risponde a una risposta. L'In-Reply-To viene mandato (`drafts.rs:243-246`) ma non salvato sull'`Email`.
- **Archivio.** Le email sono colonne scalari di SQLite (`email_row`, `sqlite_store.rs:954`, con destrutturazione esaustiva: un campo nuovo in `Email` non compila finché non lo si gestisce). `SCHEMA_VERSION = 3` (`:69`); le migrazioni sono solo `ALTER TABLE ADD COLUMN` (`migrate`, `:269`).
- **La conversazione è un insieme, non un albero:** `conversation_ids` unisce per uguaglianza di `message_id`/`thread_id` (`app/src/domain/mail_export.rs:15`), con copie in `desktop/src/lib/conversations.ts:4` e `desktop/src/lib/preview-export.ts:58`.
- **`list_mail`** raggruppa solo i messaggi della cartella aperta (`app/src/application/mail.rs:426-499`, raggruppamento `:478`; il filtro della Posta in arrivo, `:437`, esclude anche completati e posticipati). Le risposte inviate stanno in un'altra cartella e oggi non compaiono nella riga.
- **`message_ids`** di una riga è l'elenco su cui agiscono la puntina della riga, la barra di selezione e il trascinamento (`MailList.svelte:75-79`, `:327`; `App.svelte:1313-1317`). I pulsanti del pannello di lettura e il tasto `E` agiscono invece su **un solo messaggio** (`App.svelte:837`, con `ids = [id]`, e `:1060`).
- **L'eco della risposta inviata** (la copia che arriva dalla cartella Inviati) ha due percorsi: la copia locale viene **sostituita** da un messaggio letto dal file originale (`app/src/infrastructure/mail_cache_download.rs:566-578`) oppure **unita sul posto**, tenendo la copia locale e azzerando solo `local_sent` (`app/src/application/mail.rs:1696-1699`).
- **`EmailSummary`** (`desktop/src/lib/types.ts:76`) non ha i destinatari.
- **`app/src/adapters/mcp.rs:185`** considera «dello stesso thread» i messaggi con `thread_id` uguale.

### 5.2 Cosa serve

1. **Genitore diretto per ogni messaggio.** **[mia scelta]** Aggiungere a `Email` un campo `in_reply_to` (nuova colonna, `SCHEMA_VERSION` 4, migrazione `ADD COLUMN`).
   - *Riempimento:* alla ricezione dall'intestazione (`mail.rs:505`); **all'invio da `draft.reply_to`**, nello stesso punto di `drafts.rs:261`. Servono entrambi, perché dell'eco esistono due percorsi (§5.1): se la copia locale viene sostituita, il valore arriva dall'intestazione del file originale; se viene unita sul posto, resta quello salvato all'invio.
   - *Posta già salvata:* solo per la **posta ricevuta** (cartella diversa da Inviati e senza `local_sent`) il genitore si ricava da `thread_id` quando è diverso da `message_id`. Per i messaggi già salvati in Inviati il genitore è **ignoto** e non va ricavato da `thread_id`, che lì può puntare un livello troppo in alto (§5.1).
   - *Genitore ignoto:* la riga si appende sotto il messaggio più vecchio. Con le righe piatte l'errore cambia solo cosa si chiude insieme e il nome dopo «a»: accettabile.
   - *Perché un campo nuovo e non correggere `thread_id`:* non cambia il significato di `thread_id`, che usano `mcp.rs:185` e `conversation_ids`.
2. **Destinatari nella riga di risposta.** L'elenco `to` (e il conteggio) per le righe di risposta. Il messaggio principale non li mostra. Nomi: §4.4.
3. **Membri da mostrare ≠ membri su cui agire.** **[mia scelta, invariante]** Le risposte che non appartengono alla vista (per esempio quelle inviate) si aggiungono alla riga solo per la visualizzazione. `message_ids` resta formato dai soli messaggi della vista aperta: la puntina, la barra di selezione e il trascinamento non toccano mai i messaggi in Inviati. Nel lettore, i pulsanti e il tasto `E` su un messaggio solo mostrato sono disattivati (§4.3).
4. **Provenienza dei membri.** Le righe sotto la principale vengono da tutte le cartelle dello stesso account, comprese Archivio e messaggi completati o posticipati, **tranne Cestino e Spam**. Questo filtro è **lavoro nuovo**: `domain::mail_export::thread` (`mail_export.rs:42-54`) filtra solo per account e conversazione e non lo applica; la sezione «Messaggi della conversazione» del lettore mostra oggi anche i messaggi in Cestino e Spam.
5. **Peso del payload.** Le liste sono paginate (pagine da 60, `desktop/tests/refresh-coalesce.mjs`). Un thread molto lungo non deve gonfiare la pagina: il costruttore decide e dichiara il tetto di risposte caricate (suggerimento: 30); oltre, la riga «Mostra altre N risposte» (§4.3). Il contatore totale resta sempre visibile.
6. **Stato di lettura.** `Email.read` esiste già. Il `read` della riga di conversazione, i conteggi del titolo (§4.2) e quelli dei gruppi dipendono solo dai **messaggi della vista**, non da quelli solo mostrati. Il segno blu di una risposta chiusa conta invece i non letti dei messaggi che nasconde, qualunque sia la cartella.

**Fuori ambito:** `References`, risposte il cui genitore non è stato scaricato (si appendono sotto il più vecchio), conversazioni tra account.

## 6. Cosa l'anteprima toglie o cambia

L'utente ha visto l'anteprima e ha detto «ok» all'insieme. Questi punti però tolgono qualcosa che oggi c'è, e alcuni toccano un principio scritto nel progetto (`docs/ricerca-email-specifica.md` §1.3: riconoscere anziché ricordare; nomi completi, date estese, filtri sempre visibili). Ogni riga dice cosa si costruisce.

| Cosa | Oggi | Nell'anteprima | Si costruisce |
|---|---|---|---|
| Percorso nella barra superiore («Posta / Tutti gli account») | visibile | nascosto (l'account attivo si legge dalla sidebar) | come l'anteprima **[mia scelta]** |
| Indirizzo e-mail sotto il nome dell'account | visibile | nascosto (resta nel titolo al passaggio del mouse) | come l'anteprima **[mia scelta]**; tocca §1.3.4 (mostrare l'account) |
| Suggerimenti di scorciatoia nei pulsanti (N, R) | visibili | nascosti (resta la riga «Premi R…» in fondo) | come l'anteprima **[mia scelta]** |
| Caselle di selezione per riga | sempre | al passaggio del mouse, o col focus | come l'anteprima **[mia scelta]**; nessuna evidenza a favore (§7) |
| «Seleziona caricati» | sempre | dopo la prima spunta | come l'anteprima **[tua scelta]** |
| Anno nella data | «2 nov 2026» | «2 nov» | **resta come oggi** (§1.3.1 date estese) |
| Anteprima di una riga in «Già letti» | sì | no | come l'anteprima **[mia scelta]** |

## 7. Evidenza e suoi limiti

Un controllo di letteratura (17 ricerche, 8 articoli letti, 7 a testo pieno, fatto da un agente di ricerca il 2026-11-07, non pubblicato in `docs/research/`) non ha trovato **nessuno studio su interfacce con adulti ADHD** per nessuna di queste scelte. Quello che esiste è psicologia di laboratorio; il passaggio al design è inferenza.

- **Salienza.** Zhang, Abagis, Steeby, Jonides 2024 (*Visual Cognition*, PMC12330872; 36 adulti con ADHD, 46 senza): la prima cattura dello sguardo da parte di un elemento fuori posto è uguale nei due gruppi (39% contro 40%); chi ha ADHD ci resta più a lungo (seconda fissazione 30,7% contro 24,7%). Forster et al. 2014 (*Neuropsychology*, PMC3906797; 17 contro 16): il costo di un distrattore raro era maggiore anche a carico alto. Inferenza: pochi elementi evidenti e utili, non decorazione.
- **Meno non è sempre meglio.** Lo stesso risultato di Forster non permette di assumere che «più vuoto» aiuti sempre.
- **Nascondere.** Nessun esperimento controllato su controlli rivelati al passaggio del mouse o su elementi chiusi. Fuermaier et al. 2013 (PLoS One, PMC3590133): nella memoria prospettica gli adulti con ADHD ricordano bene il piano ma pianificano e passano da un compito all'altro peggio. Inferenza: ciò che si chiude deve lasciare una traccia visibile (per questo i conteggi blu).
- **Carattere.** Wallace et al. 2022 (*ACM TOCHI*, 352 adulti): il carattere più veloce per una persona è molto più veloce del più lento, ma non c'è un effetto medio, e chi dichiarava condizioni di lettura o neurologiche era escluso. **Non esiste un test indipendente su Lexend** né su Source Sans 3. Source Sans 3 è quindi una scelta di aspetto. Strada più solida per dopo: un'impostazione per scegliere il carattere.
- **Anteprima e smistamento.** Weaver et al. 2011 (MobileHCI, 16 persone, mobile): 2-3 righe di anteprima riducono i passaggi tra panoramica e dettaglio. Sarrafzadeh et al. 2019 (CHIIR): lo smistamento dipende soprattutto dall'importanza del lavoro, non solo dal mittente. Nessuno studio controllato sui riassunti AI.
- **Movimento.** Nessuno studio trovato: restano il principio di `ricerca-email-specifica.md` §1.3.5 (niente animazioni decorative) e questo progetto non ne aggiunge.

Affermazioni **senza** supporto che il progetto non fa: «Lexend aiuta l'ADHD», «gli adulti con ADHD sono catturati di più dai colori vivi», «i controlli rivelati dal mouse calmano», «il mittente decide lo smistamento».

## 8. Cosa è stato verificato nell'anteprima e cosa no

**Verificato** (con clic e misure vere, 1440×900 e 1000×640): 9 passaggi di apertura di una risposta; passaggio del mouse sul principale, su una risposta e sulla casella; chiudere e riaprire ogni nodo e l'intera conversazione; 46 controlli con 40 destinatari, nomi e indirizzi lunghi (nessuna sovrapposizione, altezza fissa a 47px, data sempre dentro); apertura/chiusura dell'elenco destinatari; «Fatto» visibile a 1000, 1100, 1200 e 1440px. 26 coppie di colori misurate nei due temi: tutte sopra le soglie WCAG tranne il bordo forte su superficie (1,58:1 chiaro, 1,89:1 scuro, già sotto 3:1 oggi con 1,45 e 1,85).

**Non verificato:** tastiera e lettore di schermo; Calendario, Rubrica, Meeting, Assistente e Impostazioni con la nuova scala di testo (visti una volta, a riposo, a 1440px: le altezze fisse in px potrebbero non reggere); tema scuro con l'elenco destinatari aperto e in passaggio del mouse; thread con più di 8 risposte o più di 3 livelli; scala di Windows diversa dal 100%; dati reali (le risposte e i destinatari dell'anteprima sono inventati; i dati di esempio dell'app non hanno thread). Il controllo automatico del taste skill sull'anteprima dava 4 errori: carattere (suggeriva IBM Plex Sans, scelto Source Sans 3), indirizzi `example.com` nei dati di esempio, e due che riguardano 390px, sotto il minimo di 1000px dell'app.

### 8.1 Dopo l'implementazione (branch `feat/aspetto-professionale-e-thread`, 2026-11-07)

**Verificato con test automatici** (backend Rust; frontend nell'anteprima del browser, Edge senza interfaccia, dati di esempio):
- **Backend:** migrazione 3→4 che riempie solo la posta ricevuta; `in_reply_to` alla ricezione, all'invio e all'eco; ordine d'albero con orfani, cicli, copie dello stesso messaggio e due account con gli stessi Message-ID; tetto di 30 risposte con il resto contato e `read_replies`; Cestino e Spam esclusi; `read` e conteggi che dipendono solo dai messaggi della vista; conteggi del titolo su tutta la vista e non su una pagina; JSON senza campi vuoti. Suite del crate `app`: 937 test verdi.
- **Cornice e tipo:** Source Sans 3 e Lexend serviti dall'app, foglio, nessuna ombra a riposo, percorso e indirizzo nascosti, token chiari e scuri, a 1440, 1200, 1100 e 1000px.
- **Lista:** gruppi minori chiusi con puntino e numero blu, stato che sopravvive al cambio di schermata, due pesi, riga Priorità, casella al passaggio del mouse, barra di selezione dopo la prima spunta, titolo «N priorità · N da leggere».
- **Destinatari:** tre nomi con l'utente per primo, «e altri N», riquadro da 96px che scorre, 40 destinatari più un indirizzo di 60 caratteri senza scorrimento laterale, a 1440 e 1000px.
- **Lettore:** etichetta di priorità davanti al percorso, «Fatto» con solo il bordo e dentro il pannello a 1000, 1100, 1200 e 1440px, testo a 16px e 68 caratteri, riassunto a 15px.
- **Righe di risposta:** totale sulla freccia, «a *Persona* +N» secondo §4.3, altezza uguale con 12 destinatari, nodo chiudibile con numero (blu se c'è un non letto), apertura che segna letto, tastiera (frecce, destra e sinistra), nuova posta che non chiude la conversazione, tetto di 30 e «Mostra altre N risposte».
- **Azioni:** su un messaggio solo mostrato (risposta inviata) restano disattivati pulsanti, voci del menu e tasto `E`; su quelli della vista tutto funziona; il lettore non ripete più la conversazione.
- `svelte-check` senza errori né avvisi; `style-tokens.mjs` senza violazioni oltre le 9 già presenti in `Calendar.svelte` prima dei lavori.
- **Regressione:** i test esistenti del browser sono stati rieseguiti sul codice nuovo e, dove falliva, sul commit precedente. Sette sembravano rotti anche prima: `mail-sync-feedback`, `review-recovery`, `calendar-flow`, `mcp-ui`, `mcp-settings`, `assistant-installers`, `google-sign-in`. **Correzione del 2026-11-07:** cinque non erano rotti, cercavano soltanto un server sulla porta 5187 (ora leggono `QAMILLE_PREVIEW_URL`, e `npm run test:browser` avvia da solo il server e li esegue tutti); `mcp-ui` e `review-recovery` cercavano un pulsante che dal commit `6cef682` è una casella di spunta, e sono stati corretti.

**Ancora non verificato (aggiornato il 2026-11-07, dopo i lavori sull'eseguibile, sui dati veri e sul modulo `ds`):**
- **Un lettore di schermo vero** (Narrator non si pilota da un test): provati i nomi, i ruoli e gli stati nell'albero di UI Automation dell'eseguibile (`a11y-native.mjs`, 102 controlli attivabili tutti con nome) e il percorso con la sola tastiera nell'anteprima (`a11y-keyboard.mjs`); non provato come viene letto ad alta voce.
- **L'app online sui dati veri** (Gmail, IMAP, sincronizzazione): mai avviata sui dati dell'utente. Provate soltanto, su copie in cartelle temporanee e con stampa dei soli conteggi, la migrazione 3→4 di `store.sqlite` e l'elenco delle conversazioni su `store.json` (`app/tests/real_data_migration.rs`, `#[ignore]`).
- **Scala di Windows reale** (125% e 200% sull'eseguibile): provati scala 1, 1,25, 1,5 e 2 nell'anteprima, e 1,5 sull'eseguibile con `--force-device-scale-factor` (che apre la finestra sotto il minimo di 1000px).
- Calendario, Rubrica, Meeting, Assistente e Impostazioni: confrontati pixel per pixel prima e dopo la migrazione al modulo `ds` (61 schermate identiche), non provati in uso oltre ai test del browser esistenti (46 verdi).
- I nomi dei destinatari non vengono dalla Rubrica (§4.4): `MailService` non la vede.
- **Punti rimandati:** (le frecce della conversazione erano 30×20 e 22×18px, sotto i 24×24 di WCAG 2.2: portate a 30×24 e 24×24 su richiesta dell'utente, il 2026-11-07, e `a11y-keyboard.mjs` ora lo controlla); i nomi accessibili «Conversazione» e «Risposte a questo messaggio» non contengono il conteggio visibile (WCAG 2.5.3) e «con messaggi non letti» sta solo nella descrizione; stessa etichetta del gruppo su più conversazioni aperte; apertura automatica del primo messaggio all'avvio che a volte non parte (vista 1 volta su 4 sull'eseguibile, non attribuita a questo lavoro); il titolo «N priorità» e l'intestazione del gruppo Priorità possono divergere per il messaggio aperto; tasti singoli R, E e N (WCAG 2.1.4); `accent-text` su `badge` a 4,39:1 nel tema scuro.
- **Test instabili, non causati da questo lavoro:** `workspace-startup-native` ha fallito 1 volta su 3 (la cartella locale finta riceve una sottocartella `Microsoft` dal sistema; il lavoro tocca solo la registrazione di `read_replies` e il colore di sfondo della finestra); `mail-sync-feedback-native` non è ermetico (scarica un modello di embedding); due test di temporizzazione di `sqlite_store` (`concurrent_writers_both_commit`, `killing_the_process_mid_write…`) e la famiglia `undo_retry` falliscono a volte su questa macchina anche prima.

**Verificato dopo l'implementazione** (aggiunte a quanto sopra): eseguibile costruito con `Build-Qamille.bat` (pagina `http://tauri.localhost/`, `theme-boot.js` presente); `thread-rows-native.mjs` 12 controlli verdi sull'eseguibile; migrazione 3→4 su una copia del vero `store.sqlite` (161 messaggi, nessuno perso, nessuna altra colonna cambiata, riapertura identica, conversazioni in 4–7 ms); difetto trovato sulla copia del vero `store.json` (417 messaggi ricevuti con il genitore solo in `thread_id`, mai riempito dal motore JSON né dall'importazione) e corretto in `domain::conversation::parent_header` con test fallito prima; suite `app` 942 verdi; scala 1–2 nell'anteprima; tema scuro con l'elenco destinatari aperto (testo ≥ 4,5:1); migrazione al modulo `desktop/src/ds` (ADR-0011, ADR-0008 v2, ADR-0009 v2, ADR-0010 v2 accettate) con 61 schermate identiche a ogni passo; token Doop sincronizzati, sei schermate HTML generate e confrontate con l'app (0 o 1 pixel diversi), design system in Claude aggiornato dal modulo (`npm run ds:export`).

## 9. Obblighi di sincronizzazione e test

- **Doop** (regola già in vigore): dopo aver cambiato `theme.css` si esporta la guida (`checks/doop-sync/tokens.mjs`, `set_guidelines`, `mark-synced`); dopo aver cambiato le schermate si ricatturano i frame (`extract.mjs --verify`, `shots.mjs`).
- **Design system e prototipo in Claude** (`checks/claude-design/`): vanno rigenerati e ripubblicati dopo la modifica. Se prima si fa il modulo `desktop/src/ds/`, il prototipo si rigenera da lì.
- **Test da aggiornare/aggiungere:** `desktop/tests/style-tokens.mjs` (invariato nelle regole); i test che citano conversazioni o thread (`activity`, `done-latency`, `events`, `mail-export`, `mail-html`, `mail-opening`, `refresh-coalesce`, `search-native`, `sidebar-threads`, `workspace-startup-native`) vanno riletti; nuovi test per: riga di risposta (apertura, lettura, chiusura del nodo), altezza fissa con 40 destinatari, «e altri N» nel lettore, invariante §5.2.3 (la puntina e la selezione su una conversazione non toccano Inviati; nel lettore, Archivia, Fatto ed `E` su un messaggio inviato sono disattivati), «Mostra altre N risposte» oltre il tetto, filtro Cestino/Spam, titolo «N priorità · N da leggere» con più di una pagina di posta, migrazione 3→4 con riempimento della sola posta ricevuta, eco della risposta inviata sia sostituita sia unita sul posto.
- **ADR:** la decisione «thread nella lista a righe, con membri di visualizzazione separati da quelli di azione» va registrata come ADR proposta. Oggi lo strumento si ferma su un record già accettato: `docs/adr/ADR-0007-v3-concorrenza-e-reattivita.md` ha, alla riga 47, `- ADR-0007 v2` sotto `supersedes`, che il validatore non accetta. Va corretta prima di poter registrare qualsiasi ADR.

## 10. Ordine dei lavori e stime

**Indipendenza.** Questo progetto è scritto sul layout dei file di oggi (`theme.css` e `styles.css` in `desktop/src/`) e **non dipende** dal modulo `desktop/src/ds/`. Quel modulo non esiste: l'utente ha detto «ok modulo ds», ma la sua ADR non è registrata (la bozza del testo è in `checks/claude-design/adr-body-ds.md`, cartella ignorata da git). Se il modulo viene fatto prima, i percorsi dell'Appendice B cambiano e il resto no.

**[mia scelta]** Consiglio il modulo prima e il restyle sopra: toccano entrambi `theme.css` e `styles.css`, e la verifica «nessun pixel cambia» del modulo non regge se nello stesso momento si cambiano i colori. L'ordine lo decide chi pianifica.

Stime a occhio, non misurate: modulo `ds` 2-3 giorni; stile e gerarchia 1 giorno; thread nella lista con dati, migrazione e test 2 giorni. Il restyle si può anche fare prima del modulo: costa un secondo giro di spostamento dei file.

## 11. Lasciato al costruttore

- Dove mettere il nuovo codice dentro `MailList.svelte` e `Reader.svelte` (componenti piccoli per la riga di risposta e per l'elenco destinatari).
- Il tetto esatto di risposte per conversazione (suggerito 30) e il testo del caso «zero da leggere».
- Come riconoscere gli indirizzi dell'utente per scrivere «te» (alias inclusi).
- Il modo di ordinare le risposte con la stessa data.
- Il nome esatto dei nuovi token e come raggruppare i valori letterali dell'anteprima.
- Il nome e la forma dei due campi nuovi di `MailPage` per i conteggi del titolo, e se allineare anche il badge della sidebar quando la lista è filtrata su un account.
- Come si disattivano i pulsanti del lettore su un messaggio solo mostrato (il testo di spiegazione è fissato nel §4.3).

## 12. Domande del revisore e come sono state risolte

Il revisore indipendente ha chiesto cose che il testo non diceva e ha trovato quattro errori sul codice. Sono tutti risolti qui sopra; questo è il riepilogo.

| Domanda o errore | Risolto in | Come |
|---|---|---|
| Qual è la riga principale di una conversazione? | §4.3 | il più recente, come oggi **[tua scelta]** |
| Da dove vengono i nomi dei destinatari? | §4.4 | contatto, poi `from_name` noto, poi indirizzo; nessuno schema nuovo **[mia scelta]** |
| Quali viste e quali membri? | §4.3, §5.2.4, §5.2.6 | modalità Conversazioni, non la ricerca; membri di ogni cartella tranne Cestino e Spam; conteggi solo sui membri della vista **[mia scelta]** |
| Oltre il tetto di risposte? | §4.3, §5.2.5 | riga «Mostra altre N risposte» **[mia scelta]** |
| Cosa conta «N priorità · N da leggere» e da dove? | §4.2 | stesso numero del badge; calcolato dal backend sullo stesso ambito della lista **[mia scelta]** |
| Quali righe del §6 si costruiscono? | §6 | ogni riga ha una decisione; la barra «Seleziona caricati» è **[tua scelta]** |
| Cosa fanno le azioni su una risposta aperta? | §4.3, §5.2.3 | disattivate su un messaggio solo mostrato **[mia scelta]** |
| Il divieto di «riordinarsi da soli» vieta il salto per data? | §2 | no: vieta solo lo scambio mentre si guarda; l'ordine resta quello di oggi |
| Il modulo `ds` fa parte di questo progetto? | §10 | no, è indipendente e non esiste ancora |
| *Errore:* `mail_export::thread` esclude Cestino e Spam | §5.2.4 | non lo fa; è lavoro nuovo |
| *Errore:* `message_ids` è l'elenco di tutte le azioni | §5.1 | vale per puntina, selezione, trascinamento; non per i pulsanti del lettore e `E` |
| *Errore:* il riempimento da `thread_id` vale per tutti | §5.2.1 | solo per la posta ricevuta; l'eco ha due percorsi |
| *Errore:* conteggio titolo come «N messaggi» in Conversazioni | §4.2 | oggi in Conversazioni dice «N conversazioni»; il testo resta |

## Appendice A: token (chiaro; il tema scuro nel file di riferimento)

| Token | Prima | Dopo | | Token | Prima | Dopo |
|---|---|---|---|---|---|---|
| `--ground` | — | `#f2f4f8` | | `--text` | `#222b3c` | `#131a27` |
| `--sidebar` | `#f6f7fa` | `#f2f4f8` | | `--text-secondary` | `#465168` | `#3d4656` |
| `--body-text` | `#3c4658` | `#1f2735` | | `--muted` | `#626c80` | `#5b6474` |
| `--nav-text` | `#646e80` | `#3d4656` | | `--border` | `#e9ecf2` | `#e4e7ee` |
| `--border-soft` | `#f0f2f6` | `#eef0f5` | | `--border-strong` | `#d1d7e2` | `#c8ced9` |
| `--accent` | `#4771d9` | `#2f5fd6` | | `--accent-hover` | `#3964cc` | `#274fb8` |
| `--accent-text` | `#3559bd` | `#2450b8` | | `--on-accent-soft` | `#dce6ff` | `#e6ecff` |
| `--selection` | `#edf2ff` | `#e7ecf6` | | `--badge` | `#dce7ff` | `#d9e1f2` |
| `--hover` | `#f4f6fa` | `#f0f2f7` | | `--avatar` | `#e9eef8` | `#e7ecf6` |
| `--label-bg` | `#f0f2f7` | `#eef0f5` | | `--label-text` | `#5c6577` | `#4a5363` |
| `--priority-text` | `#8a5a24` | `#8a4a06` | | `--priority-mark` | `#c98a4f` | `#c46a0a` |
| `--priority-bg` | `#faf2e8` | `#fdf1df` | | `--priority-wash` | — | `#fffaf2` |
| `--summary-bg` | `#f8faff` | `#f3f6fd` | | `--summary-border` | `#e9edf8` | `#dbe4f7` |
| `--label-1…5` | `#7393df #b595d8 #56a68b #dfad66 #d58294` | `#4a73d9 #8f67c4 #2e8a6c #b27a22 #c45c74` | | `--radius-sheet` | — | `14px` |
| `--text-2xs…3xl` | 11 12 13 14 16 18 20 24 | 12 13 14 15 16 18 22 26 | | `--shadow-sm` | ombra | nessuna |

Scuro (valori diversi dai precedenti): `--ground` e `--sidebar` `#12161e`; `--surface` `#1a202c`; `--text` `#e8ecf3`; `--text-secondary` e `--nav-text` `#c0c8d6`; `--body-text` `#d5dbe6`; `--muted` `#9aa5b8`; `--border` `#2a3240`; `--border-soft` `#242b38`; `--border-strong` `#414c60`; `--selection` `#263248`; `--hover` `#222a38`; `--badge` `#33415c`; `--priority-text` `#f0b878`; `--priority-mark` `#e8a25a`; `--priority-bg` `#3d2f1e`; `--priority-wash` `#221d18`; `--summary-bg` `#1f2738`; `--summary-border` `#313d55`. L'accento scuro resta `#86a8fb`; le etichette scure restano quelle attuali.

## Appendice B: dove si interviene

`desktop/src/theme.css`, `desktop/src/styles.css`, `desktop/src/components/MailList.svelte`, `Reader.svelte`, `Sidebar.svelte`, `desktop/src/App.svelte`, `desktop/src/lib/types.ts`, `desktop/src/lib/conversations.ts`, `desktop/src/lib/preview.ts` e `preview-export.ts` (dati di esempio con thread), `desktop/public/fonts/`, `desktop/src-tauri/tauri.conf.json`; `app/src/domain/model.rs`, `app/src/infrastructure/mail.rs`, `sqlite_store.rs`, `app/src/application/drafts.rs`, `app/src/application/mail.rs`, `app/src/domain/mail_export.rs`.
