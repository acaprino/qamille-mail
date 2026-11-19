# Indicizzazione completa e incrementale della posta

Requisiti concordati il 2 novembre 2026. Questo documento sostituisce l'ambito
precedente di 90 giorni / 10.000 messaggi per l'indicizzazione iniziale.

## Copertura

La prima scansione attraversa tutte le cartelle IMAP accessibili di ogni account,
senza limite di anzianità. Elabora una email alla volta: oggetto, mittente,
destinatari, data, corpo, allegati e immagini incorporate. Ogni parte conserva
il collegamento alla mail e alla posizione remota. I contenuti ricevuti servono
temporaneamente a costruire l'indice; l'opzione offline resta separata.

EmbeddingGemma 2 è multimodale. Le immagini devono essere elaborate dal suo
encoder visivo e dal backbone condiviso, non trasformate soltanto in nomi di
file. Testo estraibile e OCR sono fonti complementari per ricerche letterali;
un embedding non dimostra da solo la presenza di una frase esatta.

Formato non supportato, file cifrato, contenuto troppo grande, errore di rete o
quota insufficiente devono produrre uno stato parziale visibile. Non dichiarare
«tutto indicizzato» quando una parte è stata saltata.

## Esecuzione

1. Salvare avanzamento per account/cartella e generazione UID; riprendere dopo
   chiusure, interruzioni o perdita di rete senza rifare tutto lo storico.
2. Dare precedenza alle nuove mail; proseguire lo storico in background e
   alternare gli account. Rilevare nuove cartelle e cambi di generazione.
   In ogni cartella si legge dal messaggio più recente al più vecchio: due
   cursori (`after_uid`, `before_uid`) delimitano l'intervallo già letto; le mail
   arrivate durante la scansione si leggono prima di proseguire verso il basso.
   Un checkpoint della vecchia scansione (solo `after_uid`) vale «letto da UID 1
   a `after_uid`».
3. Evitare duplicati fra etichette usando identità remote verificate e hash del
   contenuto. Ripetere un lavoro dopo un crash deve essere sicuro.
4. Offrire pausa/ripresa, avanzamento e motivi degli errori; non richiedere una
   ricostruzione manuale per ogni nuovo messaggio.

## Memoria e spazio

Il lavoro di embedding deve essere isolato dall'interfaccia in un processo con
budget di memoria. Elaborare un solo documento/allegato alla volta, con batch 1,
caricamento degli encoder quando servono, limite alle dimensioni delle immagini
decodificate e rilascio del processo quando inattivo. La scansione non accumula
in RAM corpi o allegati dell'intera casella.

Il budget riguarda il processo di embedding, non l'intera app: UI, metadati e
buffer di rete hanno limiti separati. Un errore di allocazione, un processo
terminato o memoria disponibile insufficiente sospendono il lavoro e conservano
l'ultimo checkpoint; nessuna ripartenza infinita sotto pressione di memoria.

Gli indici rispettano la quota del singolo account; i modelli sono componenti
comuni. La scansione totale non autorizza a superare il limite impostato: se non
entra, la UI deve indicare copertura parziale e permettere di aumentare il limite.

## Fonti del modello

- [Model card Google](https://ai.google.dev/gemma/docs/embeddinggemma/model_card_2).
- [Conversione ONNX](https://huggingface.co/onnx-community/embeddinggemma-2-ONNX),
  revisione fissata `daa72c51243991dfcaf9f9137d2c573d8f7790c0`.
- Preprocessing di riferimento: Hugging Face Transformers.js,
  `models/gemma4/image_processing_gemma4.js` e
  `models/embedding_gemma2/processing_embedding_gemma2.js` (Apache 2.0).

## Implementazione e limiti attuali

Il servizio `MailIndexService` usa porte separate per scansione/lettura dei
messaggi e scrittura dell'indice. Tauri avvia il lavoro in background. I
checkpoint sono salvati per account/cartella; una coda di tentativi mantiene
i messaggi falliti senza bloccare i successivi. Nuovi arrivi e storico si
alternano, così come gli account. La schermata Ricerca mostra pausa, ripresa,
contenuti incompleti e spazio insufficiente.

Il modello di ricerca (circa 288 MB) si installa da solo in background al primo
avvio dell'app Tauri, senza richiedere un pulsante; non parte nell'area di
esempio. Ogni file viene scaricato in `<nome>.part`: dopo una chiusura o una
perdita di rete il tentativo successivo chiede solo i byte mancanti (`Range`) e
verifica l'hash dell'intero file. Se il server ignora la richiesta parziale, il
file riparte da zero. I tentativi falliti ripartono dopo 30 secondi, poi con
attesa doppia fino a 15 minuti. La schermata Ricerca mostra percentuale e MB
scaricati, e «Riprova ora» dopo un'interruzione.

| Risorsa | Limite attuale |
|---|---|
| Processo modello su Windows | 1 GiB di memoria impegnata, imposto dal sistema; priorità ridotta, CPU con metà dei core logici per inferenza (minimo 2, massimo 6, calcolati all'avvio del processo), nessun caricamento GPU |
| Pressione memoria | Prima dell'avvio servono almeno 1.408 MiB disponibili; fra le richieste almeno 384 MiB. Si considera il minimo fra RAM disponibile e capacità di impegno residua di Windows. Dopo un guasto: attesa di 60 secondi. |
| Inattività | Rilascio del processo dopo 60–70 secondi senza richieste; encoder immagini caricato alla prima immagine, nessun accumulo di sessioni per account |
| Contenuti temporanei | MIME massimo 32 MiB; allegato massimo 25 MiB; documento estratto massimo 4 MiB; testo degli allegati complessivo massimo 8 MiB per email; massimo 1.000 parti MIME |
| Immagini e vettori | Decodifica immagine massimo 96 MiB / lato 16.384 px; 2.520 patch di ingresso; passaggi di testo da 256 token; record separati per email e ricerca dei vettori in streaming |

L'estrazione di documenti durante l'indicizzazione avviene nello stesso processo
limitato. Gli archivi Office hanno un limite sull'espansione. I contenuti
temporanei non vengono salvati nel repository delle email: restano riferimenti,
hash e vettori, che rientrano nella quota del relativo account. La manutenzione
del vecchio indice monolitico usa una lettura in streaming.

Sono implementati testo, documenti con testo estraibile e immagini statiche.
PDF di sole scansioni, file cifrati, audio/video e formati non estraibili sono
segnalati come copertura parziale; il supporto multimodale del modello non
significa che ogni decoder sia già collegato. Un file sopra i limiti non viene
considerato completamente indicizzato. La ricerca letterale di contenuti non
disponibili localmente richiede ancora il recupero/verifica della fonte: i
vettori consentono la ricerca semantica, non dimostrano una corrispondenza esatta.

La cancellazione remota viene gestita dalla sincronizzazione ordinaria; questa
prima scansione non è ancora una riconciliazione completa delle cancellazioni
storiche in ogni cartella. I vecchi comandi dell'interfaccia nativa conservano il
percorso di indicizzazione precedente; il flusso completo descritto qui è quello
dell'app Tauri. UI, repository JSON e buffer di rete non rientrano nel limite del
processo modello: non si dichiara un tetto di 1 GiB per l'intera applicazione.

Verifica ripetibile, senza account personali: `checks/download-indexing-model.ps1`
scarica i file pubblici con hash verificati in `checks/indexing-model`.
L'esempio `app/examples/embedding_probe.rs` prova testo, immagini, picco di memoria
del processo e rilascio. I test del servizio usano caselle simulate per controllare
ripresa, contenuti temporanei, nuovi arrivi, errori e isolamento tra account.

Prova reale del 2 novembre 2026, Windows, modello ONNX quantizzato fissato sopra:
picco di memoria impegnata 461 MiB con testo e 592 MiB includendo la foto pubblica
`cats.jpg` del dataset `Xenova/transformers.js-docs`. Rimisurata il 6 novembre 2026 con la nuova regola dei thread (6 su un PC da 22 core logici): 461 MiB con testo e 602 MiB con la stessa foto, quindi la memoria non cambia in modo apprezzabile. La ricerca testo→immagine
ha superato il confronto con un documento non pertinente; vettori finiti di
256 dimensioni e rilascio del processo verificati. È una misura di questa prova,
non una promessa di consumo identico per qualsiasi allegato. Il tetto imposto
dal sistema resta 1.024 MiB e, se non basta, l'elaborazione non viene marcata
come completata. Log: `checks/indexing-model-smoke.log`.

Verifiche completate:
- 47 test dei servizi, inclusi i 5 nuovi casi dell'indicizzazione: superati.
- 212 test unitari superati nella suite generale; il restante test HTTP del
  calendario è stato corretto per decodificare i blocchi e confrontare il JSON,
  poi ripetuto con successo. 5 test manuali risultano ignorati dalla suite.
- Controllo Rust/Tauri con `custom-protocol` e dipendenze bloccate: superato.
- Controllo Svelte senza errori e build dell'interfaccia: superati.

Le prove hanno usato messaggi simulati e il modello pubblico, senza accedere
a caselle personali. Questa verifica non equivale a una prova completa del
download di una casella IMAP reale o a una build release dell'eseguibile.

## Percentuale per account e ricerca BM25

La schermata Ricerca espone percentuale, email complete/totale e contenuti
incompleti separati. Il censimento Gmail usa `X-GM-MSGID`, univoco fra le
cartelle dello stesso account: le etichette non sommano copie della stessa
email. Si leggono identificativi in gruppi da 500, senza corpi. Il totale è
uno snapshot, aggiornabile con Riprova; gli arrivi già conosciuti possono
aumentarlo. Un provider senza identificativi globali resta senza percentuale
finché la scansione completa non permette un conteggio locale deduplicato.
La UI mostra una barra indeterminata durante il calcolo. Errori/contenuti
incompleti non contribuiscono alle email complete; una rimozione per quota
impedisce di mostrare 100%. Il censimento ha un budget di 2 milioni di ID:
oltre questo limite si prosegue senza totale anticipato.

Ogni account ha un database SQLite FTS5 `search-lexical/<account>.sqlite`,
contentless-delete, con termini, posizioni e lunghezze per BM25. Si indicizzano
oggetto, mittente, destinatari, corpo e testo estraibile degli allegati.
Non viene duplicata la riga sorgente, ma termini e posizioni restano dati
personali e possono consentire ricostruzioni: questo sostituisce il precedente
requisito che vietava anche i token dei corpi.

La ricerca normale della posta usa BM25 e applica account/filtri prima della
paginazione; gli ordinamenti espliciti per data o vicinanza restano disponibili.
Assistente e ricerca semantica combinano BM25 e Gemma mediante RRF. Le frasi
dell'indice seguono la tokenizzazione FTS5, non equivalgono a un confronto dei
byte originali. Anteprime e citazioni richiedono comunque la fonte. Se il
modello non è disponibile, l'indice lessicale già costruito resta interrogabile.

Cache SQLite: 8 MiB per connessione; ordinamenti temporanei su disco. Il file
effettivo del database è conteggiato nella quota dell'account. Prima delle
scritture si riserva margine per transazione/journal e si impone un limite alle
pagine del database; un fallimento non pubblica il nuovo file. La pulizia
elimina i record delle mail espulse e compatta l'indice. L'aggiornamento allo
stato v3 rilegge temporaneamente i contenuti per BM25 e riusa i vettori v2 già
validi nei file per messaggio, senza ricomputarli.

Fonti: [SQLite FTS5](https://www.sqlite.org/fts5.html),
[identificativi Gmail](https://developers.google.com/workspace/gmail/imap/imap-extensions).

Verifica dell'aggiornamento BM25/percentuali: 49 test dei servizi superati,
222 test unitari superati e 5 manuali ignorati; controllo Rust/Tauri con
dipendenze bloccate superato; Svelte senza errori o avvisi e build UI riuscita.
I casi nuovi verificano deduplicazione del totale, 50%/100%, contenuti incompleti,
quota, frasi/esclusioni, aggiornamento dell'indice, pulizia e paginazione della
posta con corpi disponibili solo online. Durante la regressione è stata corretta
anche la notifica IDLE rimasta in coda dopo la rimozione dell'account.
Log in `checks/bm25-*.log`. Non è stata eseguita una prova su una casella personale
né una nuova build release completa.

## Correzioni successive alla code review

- L'ammissione delle scritture usa il consumo effettivo dopo la pulizia di una
  sola email alla volta. La quota attribuita a una riga per i contatori non è
  considerata spazio SQLite fisicamente liberato. L'ordine resta il più vecchio
  dello stesso account; bozze e stato locale protetto restano preservati.
- La compattazione consuma tutte le righe restituite da `incremental_vacuum` e
  mantiene un promemoria durevole finché riesce, anche se un tentativo precedente
  ha già eliminato i record. Un errore non viene trasformato in pulizia completata.
- Il repository rileva database lessicali mancanti o danneggiati, rende durevole
  lo stato da ricostruire e rimette in coda le email interessate. Le copie
  temporanee del database danneggiato restano conteggiate e vengono rimosse solo
  dopo il salvataggio dello stato di recupero. Gli errori reali di accesso restano
  visibili. Le email già parziali per contenuti non indicizzabili restano
  riprovabili esplicitamente, senza ritentare automaticamente file troppo grandi.
- Il cambiamento delle cartelle invalida il vecchio totale: la percentuale torna
  indeterminata fino al nuovo censimento. Un errore di lettura dello stato rimuove
  dall'interfaccia i vecchi contatori, evitando un 100% non più verificabile.
- Le ricerche indicizzate valutano sempre i metadati noti, comprese le etichette
  modificate senza cambio del MIME. Le espressioni sui contenuti incompleti
  distinguono vero, falso e sconosciuto: un ramo OR noto può soddisfare la query,
  mentre l'assenza in una parte mancante non soddisfa un'esclusione.

Un tentativo esplicito riammette anche i metadati delle attività espulse per
quota; il contenuto non viene ripristinato senza verifica e la quota viene
nuovamente applicata prima di dichiarare completa l'indicizzazione.

Prima della pulizia fisica viene salvato un intento durevole con gli ID
interessati. Se l'app si interrompe fra la rimozione di un vettore e il salvataggio
del repository, il riavvio rimette quelle voci in ricostruzione. La conferma del
nuovo limite resta una stima; l'ammissione definitiva usa spazio misurato e
compattazione effettiva, anche quando una quota SQLite attribuita a un messaggio
protetto avrebbe fatto rifiutare erroneamente una riduzione valida.

Verifiche finali delle correzioni: **240 test unitari e 52 test dei servizi
superati**, 5 test manuali ignorati. Controllo Rust/Tauri con dipendenze bloccate
superato; Svelte senza errori o avvisi e build dell'interfaccia riuscita. Log:
`checks/review-fixes-unit-final.log`, `checks/review-fixes-services-final.log`,
`checks/review-fixes-tauri.log` e `checks/review-fixes-ui-build.log`.
