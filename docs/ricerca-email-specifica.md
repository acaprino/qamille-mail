# Ricerca email: specifica funzionale e guida d’uso

**Prodotto:** Qamille · **Versione:** proposta 1.0 · **Data:** 2 novembre 2026.

**Obiettivo:** trovare un’email partendo anche da un solo ricordo, senza dover conoscere operatori, ricordare l’oggetto esatto o ripetere la ricerca da zero.

Il documento descrive il comportamento da realizzare. Gli esempi con `between:`, `around:` e gli altri nuovi operatori sono requisiti proposti, non funzionalità già disponibili. La sezione 5 distingue lo stato del codice dalle modifiche richieste.

**Conservazione dei contenuti:** la [specifica posta online e indice locale](posta-online-specifica.md) governa dove risiedono corpi, allegati ed estratti. Mantiene i filtri di questo documento, ma sostituisce le ipotesi di archivio locale completo con ricerca sul provider e recupero temporaneo; copertura, disponibilità offline e obiettivo dei 500 ms seguono le precisazioni della sua sezione 9. Le descrizioni del codice nella sezione 5 restano una fotografia della redazione, non una verifica aggiornata delle funzionalità.

## 1. Esperienza principale: partire da ciò che si ricorda

### 1.1 Una ricerca, tre modi di usarla

La stessa barra accetta parole, filtri visuali e operatori. Le tre modalità producono gli stessi criteri e gli stessi risultati. La ricerca per significato è un aiuto aggiuntivo; filtri, date e ricerca testuale devono funzionare anche senza un modello AI.

| Modalità | Esempio d’uso | Comportamento richiesto |
| --- | --- | --- |
| Parole | `preventivo cucina` | Cerca le parole nei campi disponibili; rende visibili i punti corrispondenti. |
| Filtri visuali | Da → Maria; Quando → Intorno al 15 ottobre | Costruisce la ricerca senza richiedere sintassi. |
| Operatori | `from:maria@example.com around:2026-10-15 subject:preventivo` | Applica vincoli espliciti e li rappresenta anche come filtri leggibili. |

La barra mostra il suggerimento **“Cerca una persona, una parola o una data”**. Sotto presenta al massimo cinque accessi: **Da**, **A**, **Quando**, **Allegati**, **Altri filtri**. “Oggetto” è la prima voce di “Altri filtri”.

Digitando `from:` si apre lo stesso selettore di “Da”. Qamille non ha una rubrica: i suggerimenti derivano dai mittenti e dai destinatari già presenti nella posta. Mostrano nome e indirizzo, distinguendo gli omonimi; nessun contatto viene scelto automaticamente. Gli indirizzi digitati per intero restano utilizzabili anche se non compaiono nella posta.

Ogni filtro applicato rimane visibile, modificabile e rimovibile. Esempio:

> **Da:** Maria Rossi · **Intorno al:** 15 ott 2026, ±6 giorni · **Oggetto:** preventivo
>
> **Ambito:** tutti gli account · tutta la posta, esclusi Spam e Cestino

“Mostra operatori” permette di copiare la query. Non è necessario aprirlo per usare o modificare i filtri.

### 1.2 Guida rapida per l’utente

1. **Inserisci ciò che ricordi:** una persona, una parola o un nome di file. Basta un indizio.
2. **Aggiungi un filtro solo se serve:** per una data incerta, scegli “Quando → Intorno a una data”.
3. **Apri il risultato riconoscibile:** oggetto, mittente, data e passaggio corrispondente sono visibili insieme.
4. **Se manca, premi “Non la trovo”:** compare una proposta concreta per ampliare o correggere la ricerca.

Questa guida deve essere disponibile accanto alla barra, senza apparire come tutorial obbligatorio a ogni apertura.

### 1.3 Progettazione per attenzione limitata e ADHD

L’interfaccia deve richiedere pochi passaggi e rendere riconoscibili criteri e risultati, senza presumere che tutte le persone con ADHD abbiano gli stessi bisogni. Queste scelte sono ipotesi di prodotto da verificare con utenti, non affermazioni cliniche.

1. **Riconoscere anziché ricordare:** nomi completi, date estese, anteprime, suggerimenti e filtri sempre visibili; nessun operatore da imparare.
2. **Una decisione alla volta:** controlli avanzati progressivi; nel recupero una proposta principale e al massimo due alternative.
3. **Conservare il lavoro:** tornando da un messaggio restano ricerca, filtri, posizione e risultato selezionato; ogni ampliamento è annullabile.
4. **Rendere chiaro l’effetto:** mostrare intervallo effettivo, account, cartelle e ordinamento; nessun filtro applicato in modo invisibile.
5. **Ridurre le interruzioni:** niente animazioni decorative, riordinamenti continui o notifiche che rubano il focus; uso completo da tastiera, etichette accessibili e stato non comunicato solo dal colore.

La scelta di controlli familiari, etichette esplicite e relazioni chiare tra azioni e contenuti si appoggia alle [indicazioni W3C sull’accessibilità cognitiva](https://www.w3.org/WAI/WCAG2/supplemental/objectives/o1-understandable/). La loro applicazione specifica alla ricerca email è una proposta progettuale.

## 2. Filtri e regole di ricerca

### 2.1 Persone e contenuto

| Operatore proposto | Controllo visuale | Significato ed esempio |
| --- | --- | --- |
| `from:` | Da | Mittente: `from:maria@example.com`. |
| `to:` | A | Destinatario nel campo A: `to:luca@example.com`; non include automaticamente Cc. |
| `cc:` | In copia | Destinatario nel campo Cc. |
| `subject:` | Oggetto contiene | `subject:"contratto firmato"` richiede quella sequenza nell’oggetto. |
| `body:` | Testo contiene | `body:"codice ordine"` cerca nel corpo disponibile, senza includere gli allegati. |

Un indirizzo completo seleziona quella casella esatta, senza corrispondenze per semplice sottostringa. `from:@azienda.it` seleziona il dominio esatto; eventuali sottodomini richiedono una scelta esplicita. Un nome incompleto, per esempio `from:maria`, cerca nel nome visualizzato e nella parte dell’indirizzo prima di `@`, senza inventare un’identità univoca. Un valore senza `@` è sempre un nome incompleto. I filtri A e Cc seguono le stesse regole; per la posta ricevuta il modello attuale conserva solo l’indirizzo dei destinatari, quindi finché non si salva anche il nome il confronto riguarda la sola parte prima di `@`.

Queste regole sostituiscono la corrispondenza per sottostringa attuale e cambiano alcuni risultati: `from:co.com` non trova più il dominio, che si cerca con `from:@co.com`, e `to:example` non trova più `luca@example.com`.

`from:me` e `to:me` si riferiscono agli indirizzi degli account inclusi nell’ambito; oggi `me` è una normale sottostringa. Gli alias non esistono ancora e richiedono una nuova impostazione, quindi sono un’estensione successiva. Il riepilogo permette di vedere quali indirizzi sono compresi.

Le parole libere cercano in oggetto, mittente, destinatari, corpo, etichette, nomi degli allegati e testo degli allegati già estratto. Tutte le parole devono essere presenti, anche in campi diversi. Le virgolette richiedono una sequenza contigua nello stesso campo; non sono una richiesta di somiglianza semantica. La ricerca testuale ignora maiuscole e minuscole e normalizza Unicode; eventuali correzioni ortografiche o ricerche senza accenti vengono proposte, non applicate in silenzio. Indirizzi e identificativi mantengono confronti deterministici.

### 2.2 Date esatte e intervalli

| Operatore proposto | Controllo visuale | Regola |
| --- | --- | --- |
| `on:2026-10-15` | Il giorno | Include l’intero 15 ottobre. |
| `between:2026-10-01..2026-10-31` | Dal… al… | Include entrambi i giorni estremi. |
| `after:2026-10-01` | Dal giorno | Include il 1° ottobre dalle 00:00 in avanti. |
| `before:2026-11-01` | Prima del giorno | Esclude il 1° novembre: limite alle 00:00. |
| `newer_than:7d` / `older_than:7d` | Più recente / più vecchia di | Soglia relativa rispetto all’istante di esecuzione; `d` vale 24 ore. |

“Quando” offre **Giorno preciso**, **Intervallo**, **Intorno a una data**, **Periodo recente**. Il selettore recente contiene “Oggi”, “Ieri”, “Ultimi 7 giorni” e “Mese scorso”. “Ultimi 7 giorni” significa oggi e i sei giorni di calendario precedenti; il riepilogo mostra sempre le date risolte. Non viene confuso con la durata mobile di `newer_than:7d`.

La sintassi canonica usa `AAAA-MM-GG`. Resta valida anche `AAAA/MM/GG`, già accettata dal parser. Il formato regionale (sezione 2.8) decide l’ordine della forma breve e la visualizzazione: con `it-IT` si accetta `GG/MM/AAAA` e si visualizza “15 ottobre 2026”. La forma breve diventa canonica prima di salvare o copiare la query, così una ricerca salvata non cambia significato se cambia il formato. Una data senza anno o ambigua non genera un filtro nascosto: il sistema presenta la data completa proposta da selezionare. Un intervallo invertito o impossibile resta evidenziato, con un’azione per correggerlo.

Per compatibilità con il parser esistente, le durate di `newer_than:` e `older_than:` accettano anche `w`, `m`, `y`, rispettivamente 7, 30 e 365 giorni. Il menu distingue “ultimi 30 giorni” da “mese scorso”, che è un mese di calendario.

### 2.3 Intorno a una data: `around:`

**Decisione proposta: la finestra predefinita è ±6 giorni di calendario, quindi 13 giorni inclusa la data centrale.**

| Query | Intervallo incluso |
| --- | --- |
| `around:2026-10-15` | Dal 9 al 21 ottobre 2026. |
| `around:2026-10-15/3d` | Dal 12 al 18 ottobre 2026. |
| `around:2026-10-15/6d` | Dal 9 al 21 ottobre 2026; equivale al valore predefinito. |
| `around:2026-10-15/14d` | Dal 1° al 29 ottobre 2026. |
| `around:2026-10-15/0d` | Solo il 15 ottobre 2026. |

Il controllo visuale contiene data centrale, scelta **±3**, **±6**, **±14**, **±30 giorni** e valore personalizzato intero da 0 a 365. Mostra immediatamente **“Dal 9 al 21 ottobre · 6 giorni prima e 6 dopo”**. Per finestre asimmetriche l’utente passa a “Intervallo”, con le date già compilate.

Nell’operatore testuale il raggio segue le stesse regole del controllo visuale: intero da 0 a 365, sempre in giorni. La data accetta le forme della sezione 2.2.

| Query | Esito |
| --- | --- |
| `around:15/10/2026/3d` | Dal 12 al 18 ottobre 2026, con formato `it-IT`. |
| `around:2026/10/15` | Dal 9 al 21 ottobre 2026: data senza raggio. |
| `around:2026-10-15/400d` | Errore: raggio oltre 365. |
| `around:2026-10-15/2w` | Errore: il raggio è in giorni; propone `/14d`. |
| `around:2026-10-15/6` | Errore: manca l’unità; propone `/6d`. |
| `between:2026-10-15..2026-10-15` | Solo il 15 ottobre 2026. |
| `between:2026-10-31..2026-10-01` | Errore: intervallo invertito, con azione per correggerlo. |

`around:` filtra veramente il periodo: non è soltanto un suggerimento per l’ordinamento. Per una ricerca senza testo libero, l’ordinamento predefinito è “Più vicine al 15 ottobre”, per distanza in giorni di calendario; a parità, data più recente e identificativo stabile. Con testo libero, l’ordinamento predefinito è “Più pertinenti”, con vicinanza alla data usata a parità di rilevanza. L’ordinamento attivo è sempre visibile e modificabile.

**Compatibilità:** `around:` temporale e `between:` sono estensioni di Qamille. In Gmail `AROUND` indica invece vicinanza tra parole. Non presentare questi esempi come query Gmail direttamente utilizzabili. [Documentazione ufficiale degli operatori Gmail](https://support.google.com/mail/answer/7190?hl=it).

### 2.4 Quale data viene cercata

La prima versione usa `Email.date`, lo stesso campo del filtro esistente. Contiene la data del messaggio; se questa manca, oggi la sincronizzazione salva l’ora corrente, senza alcun segno che la distingua. La data di riapparizione dopo un posticipo non deve alterare l’appartenenza al periodo. Il risultato deve mostrare la stessa data usata dal filtro, eventualmente affiancando altre date con etichette distinte.

Il fuso della ricerca è quello delle impostazioni (sezione 2.8), che all’inizio coincide con quello del sistema. È mostrato nei dettagli del filtro e fissato per quella ricerca. I periodi sono calcolati su giorni locali, gestendo cambi di mese, anno e ora legale. Internamente un intervallo inclusivo diventa `[inizio del primo giorno, inizio del giorno dopo l’ultimo)`, convertito in UTC. Non sommare semplicemente blocchi di 86.400 secondi per ottenere giorni di calendario.

Se manca una data attendibile, mostrare “Data non disponibile”. Il messaggio non soddisfa un filtro temporale; l’eventuale data sostitutiva deve essere esplicita. Per applicare questa regola il modello deve poter rappresentare la data mancante, al posto dell’ora corrente. “Ricevuta il” distinta da “Inviata il” è un’estensione successiva, subordinata alla disponibilità dei dati.

### 2.5 Allegati, stato e posizione

| Gruppo | Sintassi proposta | Comportamento |
| --- | --- | --- |
| Allegati | `has:attachment`, `filename:preventivo`, `type:pdf` | Presenza, parte del nome, tipo normalizzato da MIME ed estensione. |
| Stato | `is:unread`, `is:read`, `is:starred`, `is:pinned`, `is:snoozed` | Filtri espliciti sui metadati; includere anche gli stati già supportati dall’app. |
| Cartella | `in:inbox`, `in:sent`, `in:archive`, `in:trash`, `in:spam` | Cartelle logiche mappate sull’account; non affidarsi soltanto al nome mostrato dal provider. |
| Organizzazione | `label:fatture`, `category:newsletter` | Etichette e categorie locali con valori suggeriti dal selettore. |
| Account e ambito | `account:lavoro@example.com`, `in:anywhere` | Seleziona l’account; `in:anywhere` include anche Spam e Cestino nell’ambito degli account selezionati. |

La ricerca globale comprende posta ricevuta, inviata, archiviata e posticipata disponibile negli account selezionati. Spam e Cestino sono esclusi inizialmente. Cercare da una cartella non deve restringere la ricerca globale in modo implicito: “Solo questa cartella” è un filtro visibile. Le bozze restano fuori dai messaggi ricercabili nella prima versione.

L’inclusione esplicita di Spam o Cestino deve prevalere sull’esclusione predefinita. Un messaggio trovato lì mostra la posizione, così l’utente capisce perché non era visibile nella posta in arrivo.

### 2.6 Combinazioni, errori e ricerca per significato

1. **AND predefinito:** filtri adiacenti sono tutti obbligatori. Due mittenti alternativi diventano `(from:maria@example.com OR from:luca@example.com)`. Il selettore multiplo genera esplicitamente questo gruppo.
2. **Esclusioni e gruppi:** supportare `-subject:newsletter`, `-has:attachment`, `OR`, `AND` e parentesi. Precedenza: parentesi, esclusione, AND, OR. Il riepilogo visuale deve conservare il significato dei gruppi.
3. **Errori visibili:** `around:31/02/2026`, `from:`, virgolette aperte, raggi negativi o operatori sconosciuti non vengono ignorati. Durante la digitazione restano gli ultimi risultati validi, etichettati come tali; Invio segnala il punto da correggere senza cancellare il testo. Stringhe letterali con due punti possono essere racchiuse tra virgolette; l’errore offre anche l’azione “Cerca come testo”.
4. **Intersezione delle date:** filtri temporali multipli si intersecano. Se l’intersezione è vuota, mostrare “Questi periodi non si sovrappongono” e permettere di rimuovere un filtro. Nessun vincolo viene scartato automaticamente.
5. **Significato sotto controllo:** i filtri espliciti si applicano prima della ricerca per significato. La modalità “Parole esatte” è predefinita; “Anche per significato” aggiunge risultati compatibili con tutti i vincoli, identificati come tali. Un modello non può cambiare mittente, account, date, esclusioni o frasi esatte per produrre più risultati.

### 2.7 Compatibilità con la sintassi attuale

La nuova grammatica conserva ciò che oggi funziona, salvo i cambiamenti elencati qui. La tabella vale anche per gli strumenti MCP, che pubblicano questi operatori ai loro client.

| Sintassi attuale | Oggi | Nuova regola |
| --- | --- | --- |
| `tag:`, `folder:` | Equivalenti di `label:` e `in:`. | Restano alias. |
| `in:` con un altro nome di cartella | Confronta il nome della cartella locale. | Resta valido per le cartelle esistenti; un nome inesistente è un errore visibile. |
| `has:document`, `has:spreadsheet`, `has:presentation`, `has:reminder` | Filtri attivi. | Restano validi. |
| `is:unreplied`, `is:done`, `is:task` | Filtri attivi. | Restano validi. |
| `before:` e `after:` con `AAAA/MM/GG` | Accettati. | Restano validi. |
| `from:`, `to:`, `cc:` | Sottostringa. | Cambiano: regole della sezione 2.1. |
| `is:foo`, `has:foo`, `before:2026-13-01`, `newer_than:7x` | Nessun filtro applicato. | Errore visibile. |
| `category:foo` | Nessun risultato. | Errore visibile. |
| `newer_than:7è` | Panico nel parser. | Errore visibile. |
| Prefisso sconosciuto, come `10:30` o un indirizzo web | Parola libera. | Errore visibile con “Cerca come testo”; tra virgolette resta testo. |
| `OR`, `AND`, `-parola`, parentesi | Testo letterale. | Diventano sintassi (sezione 2.6); tra virgolette restano testo. |

### 2.8 Fuso orario, località e formato regionale

La sezione “General” delle impostazioni contiene due scelte, entrambe inizialmente su **Automatico**: chi non le tocca non deve configurare nulla.

| Impostazione | Automatico | Scelta manuale | Effetto |
| --- | --- | --- | --- |
| Fuso orario | Fuso del sistema, riletto all’avvio e al ritorno dell’app in primo piano. | Si cerca una località e si sceglie, per esempio, “Roma · Europe/Rome · UTC+1”. | Confini dei giorni nei filtri e tutte le date e ore mostrate dall’app. |
| Formato regionale | Locale del sistema. | Elenco di locale, per esempio `it-IT` o `en-US`. | Ordine delle date brevi, nomi di mesi e giorni, primo giorno della settimana, numeri e lingua dei testi della ricerca. |

1. **Località:** serve solo a trovare il fuso. È una ricerca in un elenco locale di città, senza geolocalizzazione, permessi o rete. Si salva l’identificativo IANA, come `Europe/Rome`, mai uno scarto fisso, così l’ora legale resta corretta.
2. **Una sola fonte:** filtri, risultati, messaggio, calendario, anteprima di stampa e strumenti MCP usano lo stesso fuso. Una data mostrata e il filtro che l’ha selezionata non possono usare fusi diversi. Se il fuso manuale differisce da quello del sistema, il riepilogo lo indica.
3. **Cambio di impostazione:** i dati restano istanti UTC e non richiedono migrazione. La ricerca aperta viene rieseguita e mostra le nuove date risolte. Una ricerca salvata usa il fuso corrente e lo mostra prima dei risultati.
4. **Fuso non riconosciuto:** se il sistema non lo fornisce o l’identificativo salvato non esiste, l’app usa UTC e lo dichiara con “Fuso non riconosciuto: uso UTC”, con un’azione per sceglierlo.
5. **Mezzanotte assente o doppia:** dove l’ora legale salta la mezzanotte, il giorno inizia al primo istante valido; dove la ripete, alla prima occorrenza. Il filtro resta sempre applicato.
6. **Lingua:** i testi della ricerca sono disponibili in italiano e inglese. Un locale `it-*` usa l’italiano, gli altri l’inglese con il proprio formato regionale. Gli operatori restano uguali in ogni lingua. La traduzione del resto dell’app è un lavoro separato.

## 3. Risultati e recupero quando l’email manca

### 3.1 Rendere riconoscibile il risultato

Ogni riga mostra mittente, oggetto, data completa, account o cartella quando utili e un passaggio pertinente. Il passaggio evidenzia la corrispondenza; se proviene da un allegato, mostra anche il nome del file. Per ricerche solo per filtri, il motivo può essere “Da Maria · 2 giorni prima della data indicata”. Non inventare estratti per contenuti non disponibili.

I vincoli devono essere soddisfatti dallo stesso messaggio. Se si raggruppano conversazioni, si indicano i messaggi corrispondenti e si apre quello pertinente; un mittente in un messaggio e una data in un altro non costituiscono una corrispondenza.

Mostrare una prima pagina di 20 risultati con “Mostra altri”, senza troncare la ricerca a 20 in totale. “20 di 84 risultati” è ammesso solo se 84 è un totale verificato; altrimenti “20 risultati caricati · altri disponibili”. La selezione di account e cartelle precede ordinamento e paginazione.

Durante l’apertura di un messaggio l’elenco resta stabile. Aggiornamenti e nuovi risultati non spostano l’elemento sotto il puntatore; una richiesta vecchia non deve sovrascrivere una ricerca più recente.

### 3.2 “Non la trovo”

Il pulsante è disponibile anche quando ci sono risultati: trovare molte email non significa aver trovato quella giusta. Il sistema propone una modifica basata sui criteri attivi, mantenendo la ricerca originale recuperabile con “Annulla”. Non avviare una sequenza di ampliamenti automatici.

| Situazione | Proposta principale | Esempio di testo |
| --- | --- | --- |
| Periodo incerto o stretto | Ampliare solo il periodo | “Prova 14 giorni prima e dopo il 15 ottobre.” |
| Possibile confusione tra mittente e destinatario | Cercare la stessa persona anche in A e Cc | “Cerca Maria anche tra i destinatari.” |
| Parola ricordata come oggetto | Cercarla anche nel corpo | “Cerca «preventivo» anche nel testo.” |
| Account/cartella limitati | Ampliare l’ambito, dichiarandolo | “Cerca anche negli altri account” oppure “Includi Spam e Cestino”. |
| Corpo o periodo non disponibili | Completare la ricerca dei contenuti | “La posta di quel periodo non è ancora disponibile. Cerca sul server.” |

La proposta eseguibile più pertinente occupa il posto principale; al massimo due alternative sono visibili. Copertura incompleta e problemi di connessione hanno precedenza sui suggerimenti di modifica. Se il server non è disponibile, mostrare “Riprova la connessione” senza attribuire l’assenza ai filtri.

Un’anteprima come “Con ±14 giorni ci sono altri 8 risultati” è ammessa soltanto dopo aver eseguito quel controllo nello stesso ambito. L’utente sceglie se applicarla; i risultati fuori dai criteri originali non vengono mescolati in silenzio.

Se l’utente chiede alternative non esatte, eventuali errori ortografici o corrispondenze semantiche possono essere proposti dopo i controlli di copertura. Per la persona o l’indirizzo selezionati non si sostituisce automaticamente un’identità simile.

### 3.3 Stati che devono restare distinti

| Stato | Messaggio richiesto | Azione |
| --- | --- | --- |
| Ricerca in corso | “Sto cercando…” | Conservare i criteri e consentire modifica o annullamento. |
| Zero risultati, copertura completa | “Nessuna email corrisponde a questi filtri.” | Mostrare la proposta di recupero pertinente. |
| Copertura incompleta | “Risultati parziali: manca parte della posta o del contenuto.” | Indicare cosa manca e offrire ricerca remota o completamento locale. |
| Errore | “Ricerca non completata: connessione non disponibile.” | Riprova; non mostrare un falso totale zero. |
| Risultati trovati | Conteggio verificato o numero caricato | Apri, restringi oppure “Non la trovo”. |

“Copertura completa” richiede evidenza che tutti i messaggi e tutti i campi necessari a quella query siano stati cercati nell’ambito richiesto. Se questa informazione non esiste, il sistema dichiara la copertura non verificata. Per i filtri sui soli metadati non serve avere già scaricato ogni corpo.

La ricerca locale deve mostrare quando il periodo richiesto precede la posta sincronizzata. La ricerca remota traduce i filtri supportati e completa localmente quelli non supportati, continuando la paginazione necessaria. Se non può garantire la stessa semantica, segnala il limite; non dichiara una ricerca completa. PDF scansionati, allegati cifrati o testo non estratto non sono considerati ricercati nel contenuto. Il filtro sul nome del file può comunque funzionare.

### 3.4 Riprendere una ricerca senza ricominciare

Le ultime cinque ricerche confermate sono disponibili localmente; non si registra ogni tasto digitato. L’utente può cancellare la cronologia o disattivarla. Le ricerche salvate includono account, filtri e ordinamento; quelle relative, come “Ieri”, ricalcolano le date al riutilizzo e le mostrano prima dei risultati.

Il percorso guidato “Aiutami a trovarla” parte da “Che cosa ricordi?” e presenta **Una persona**, **Un periodo**, **Una parola**, **Un allegato**. Ogni scelta apre solo il controllo pertinente; nessun percorso impone di compilare tutti i campi.

## 4. Esempi e criteri di accettazione

### 4.1 Cinque ricerche realistiche

| Ricordo dell’utente | Query equivalente proposta |
| --- | --- |
| “Maria mi ha mandato il preventivo intorno al 15 ottobre.” | `from:maria@example.com around:2026-10-15 subject:preventivo` |
| “L’ho inviata a Luca durante settembre.” | `in:sent to:luca@example.com between:2026-09-01..2026-09-30` |
| “Era un PDF chiamato contratto, forse l’ho archiviato.” | `type:pdf filename:contratto` — l’ambito predefinito include l’archivio. |
| “Da Maria o Luca, ma non una newsletter.” | `(from:maria@example.com OR from:luca@example.com) -subject:newsletter` |
| “Non so dove sia finita, ricordo solo il numero d’ordine.” | `"ORD-4821" in:anywhere` |

### 4.2 Verifiche di correttezza

1. **Limiti temporali:** `around:2026-10-15/6d` include dal 9 ottobre alle 00:00 fino a prima del 22 alle 00:00 nel fuso della ricerca; esclude 8 e 22 ottobre. Provare anche raggio zero e questi casi: `around:2026-10-31/3d` va dal 28 ottobre al 3 novembre; `around:2026-12-30/3d` va dal 27 dicembre al 2 gennaio 2027; `around:2028-02-29/1d` va dal 28 febbraio al 1° marzo; `on:2026-10-25` in `Europe/Rome` dura 25 ore, dalle 22:00 UTC del 24 alle 23:00 UTC del 25. `between:` include tutto il giorno finale.
2. **Vincoli esatti:** nessun risultato viola account, mittente, destinatario, frase esatta, esclusione o periodo, anche con ricerca semantica. Verificare gli indirizzi con sottostringhe simili, `me`, omonimi e conversazioni con corrispondenze distribuite su messaggi diversi.
3. **Ambito e pagine:** con oltre 20 corrispondenze tutte sono raggiungibili. Un risultato nel solo account selezionato non scompare perché altri account occupano i primi 20 posti. `in:trash`, `in:spam` e `in:anywhere` superano correttamente l’esclusione predefinita. Per la posta mai scaricata conta invece lo stato di copertura della sezione 3.3.
4. **Errori e incompletezza:** una data invalida non diventa un filtro ignorato; rete assente, sincronizzazione parziale e allegati non indicizzati non diventano “nessun risultato”. Le risposte a richieste superate vengono scartate. Ogni esito di errore delle sezioni 2.3 e 2.7 ha un test dedicato.
5. **Equivalenza e ripristino:** filtri visuali e query testuale producono risultati identici; conversione avanti e indietro preserva parentesi ed esclusioni. Aprire e chiudere una mail mantiene lo stato; annullare un ampliamento ripristina anche ordine e posizione.
6. **Fuso e formato:** con una mail delle 22:30 UTC del 15 ottobre 2026, `on:2026-10-15` la trova con fuso `UTC` e non con `Europe/Rome`, dove è già il 16. La data mostrata coincide sempre con quella usata dal filtro. `on:2026-03-08` in `America/Havana` inizia alle 05:00 UTC. `10/03/2026` vale 10 marzo con `it-IT` e 3 ottobre con `en-US`, e la query salvata non cambia se cambia il formato. Cambiare il fuso non modifica i dati salvati.

### 4.3 Verifiche d’uso e prestazioni

Gli obiettivi seguenti sono soglie proposte da misurare, non risultati già ottenuti.

1. **Avvio semplice:** da una barra vuota, impostare “intorno a una data” senza conoscere operatori, attraverso tre scelte: Quando, modalità, data; il raggio predefinito non richiede un passaggio aggiuntivo.
2. **Ritrovamento:** prova moderata con 8 partecipanti, includendo persone con ADHD e persone che dichiarano poca pazienza nella ricerca. Obiettivo iniziale: almeno 6 su 8 trovano una mail nota entro 60 secondi, senza suggerimenti del moderatore. Il campione serve a migliorare il prodotto, non a dimostrare efficacia clinica.
3. **Recupero:** nello stesso test, almeno 6 su 8 superano un primo tentativo senza risultati senza riscrivere l’intera query; registrare ostacoli e numero di modifiche.
4. **Risposta:** su un computer di riferimento documentato con 50.000 messaggi già indicizzati, primi risultati locali al 95° percentile entro 500 ms dalla query confermata. Digitazione con attesa di 250 ms; Invio immediato. Per operazioni oltre 1 secondo, indicazione di avanzamento e annullamento. Misurare separatamente locale, semantica e rete.
5. **Accessibilità e dati:** completare ricerca e recupero da tastiera, verificare focus e annunci con lettore di schermo. Le eventuali metriche di prodotto misurano tempi e azioni senza registrare contenuti, indirizzi o query in chiaro.

## 5. Stato del progetto e ordine di realizzazione

### 5.1 Riscontri nel codice al 2 novembre 2026

Questi sono riscontri statici nei file, non una verifica del comportamento dell’app in esecuzione.

| Area | Presente nel codice | Differenza da colmare |
| --- | --- | --- |
| Parser | `from:`, `to:`, `cc:`, `subject:`, date prima/dopo, durate, allegati, stati, categorie, etichette e cartelle. | Mancano i nuovi operatori temporali e una grammatica completa. `from:`, `to:` e `cc:` usano sottostringhe. Alcuni valori non validi vengono accettati senza filtrare e `newer_than:7è` provoca un panico in `parse_span`. La sezione 2.7 fissa la compatibilità. |
| Ricerca testuale | Le parole sono tutte richieste; i risultati vengono ordinati per punteggio. | Con parole libere il motore tronca a 20 prima del filtro account: sostituire il limite totale con paginazione dopo tutti i vincoli. Le parole libere non cercano nei destinatari né nel testo reale degli allegati: `attachment_text` legge solo il testo della posta di esempio e l’estrazione avviene solo per l’indicizzazione semantica. Va resa disponibile senza modello AI. |
| Ambito | Ricerca su dati locali, con esclusione preliminare di Spam e Cestino. | L’esclusione preliminare impedisce di recuperarli tramite un semplice `in:trash`; servono ambito esplicito e copertura dichiarata. Togliere l’esclusione non basta: la sincronizzazione scarica solo gli ultimi messaggi di `INBOX`, al massimo 500 per account. `Sent` contiene gli invii dall’app e `Archive`, `Trash` e `Spam` nascono da azioni locali, quindi la posta che sul server sta altrove non è nei dati locali. |
| Interfaccia desktop | Barra testuale e stato vuoto con suggerimento generico. | Servono filtri visuali, intervallo leggibile, motivi di corrispondenza e recupero guidato. |
| Data e significato | Esiste ricerca ibrida separata; il filtro usa `Email.date`, mentre il riepilogo desktop usa `inbox_date()`. | Unificare i vincoli tra percorsi e mostrare nei risultati la data effettivamente cercata. `Email.date` è obbligatorio e ricade sull’ora corrente: serve un modo per rappresentare la data mancante, con un trattamento per i messaggi già salvati. |
| Fuso e formato | Il filtro usa il fuso di sistema; le date mostrate usano fuso e formato della finestra. `Settings` non ha campi per fuso o locale, le dipendenze non includono un database dei fusi e i testi dell’interfaccia sono in inglese. | Aggiungere le impostazioni della sezione 2.8 e il database dei fusi, convertendo in IANA i nomi di fuso di Windows. Passare fuso e locale al parser e a tutte le date mostrate. Tradurre i testi della ricerca. Oggi, se la mezzanotte è assente o doppia, `parse_date` non produce una data e il filtro non si applica. |
| Persone | Non esistono rubrica né alias. I destinatari ricevuti sono salvati senza nome, quelli inviati come testo grezzo della bozza. | Normalizzare i destinatari prima di applicare la sezione 2.1. Alias e nomi dei destinatari restano estensioni. |
| Altri percorsi | Interfaccia nativa, che `MIGRATION.md` mantiene disponibile durante la migrazione, e strumenti MCP `list_emails`, `search_emails` e `semantic_search`. | Usano lo stesso parser con limiti propri. `split_query` ha un elenco fisso di operatori: uno nuovo non aggiunto finirebbe tra le parole del modello semantico. Serve una sola grammatica per tutti i percorsi. |

Riferimenti locali: [parser e filtri](../app/src/domain/rules.rs), [ricerca e ordinamento](../app/src/domain/actions.rs), [servizio elenco email](../app/src/application/mail.rs), [barra desktop](../desktop/src/App.svelte), [risultati desktop](../desktop/src/components/MailList.svelte). Il percorso ibrido è in [rag.rs](../app/src/infrastructure/rag.rs). Sincronizzazione: [mail.rs](../app/src/infrastructure/mail.rs). Interfaccia nativa: [ui.rs](../app/src/adapters/native/ui.rs). Strumenti MCP: [mcp.rs](../app/src/adapters/mcp.rs).

### 5.2 Consegne proposte

1. **Fondamenta corrette:** query strutturata condivisa da tutti i percorsi, validazione, ambito esplicito, filtri prima dei limiti, paginazione e stato di copertura. Data mancante rappresentabile, destinatari normalizzati, testo degli allegati disponibile alla ricerca testuale, fuso e locale passati dalle impostazioni a tutti i percorsi. Applicare la compatibilità della sezione 2.7.
2. **Prima esperienza completa:** filtri Da/A/Oggetto, `on:`, `between:`, `around:` con ±6 giorni, allegati, riepilogo leggibile, risultati stabili e ripristino della ricerca. Includere OR, esclusioni e controlli da tastiera, le impostazioni di fuso, località e formato regionale, e i testi della ricerca in italiano e inglese.
3. **Ritrovamento assistito:** “Non la trovo”, suggerimenti reversibili, completamento della ricerca oltre i dati locali e confronto con la disponibilità sul server. Senza questa copertura la prima versione resta esplicitamente una ricerca della sola posta disponibile.
4. **Evoluzioni:** interpretazione di frasi naturali con anteprima dei filtri, ricerca per significato integrata, cronologia/salvataggi, alias e ulteriori campi come dimensione, data di ricezione, nomi dei destinatari e Ccn quando disponibili. Nessun valore viene inventato per compensare dati assenti.

La prima release della ricerca descritta comprende le consegne 1–3 e le verifiche della sezione 4. L’interpretazione libera “la mail di Maria di metà mese” può arrivare dopo: date approssimative e filtri precisi devono già funzionare attraverso i controlli visuali.
