# Ricerca email: implementazione e verifiche

Riferimento: [specifica funzionale aggiornata](ricerca-email-specifica.md). Stato al 2 novembre 2026.

## Funzioni realizzate

1. **Ricerca precisa:** grammatica condivisa con persone, oggetto, corpo, date, allegati, account, cartelle e stati. Supporta frasi tra virgolette, AND, OR, esclusioni e gruppi. Gli indirizzi completi e i domini hanno confronti precisi; `me` rispetta gli account selezionati. I filtri invalidi vengono segnalati.
2. **Date approssimative:** `around:` usa ±6 giorni di calendario, modificabili tra 0 e 365; `on:` seleziona un giorno e `between:` include il giorno finale. I confini rispettano il fuso IANA, compresi cambio dell’ora e mezzanotte assente o ripetuta. Le date mancanti sono esplicite nei nuovi messaggi scaricati.
3. **Interfaccia guidata:** filtri Da/A/Quando/Allegati/Altri filtri, suggerimenti dalle persone presenti nella posta, criteri visibili, date leggibili e testi italiano/inglese. Gli errori conservano gli ultimi risultati validi. “Non la trovo” propone modifiche annullabili, compreso ampliare un intervallo di sette giorni per lato.
4. **Risultati:** filtro account prima dell’ordinamento, paginazione oltre 20 risultati, estratti pertinenti ed evidenziazioni. Le conversazioni sono raggruppate dopo aver verificato i singoli messaggi; durante la ricerca vengono usate le date originali. Le email scaricate solo come intestazioni (corpo online) restano trovabili per mittente, destinatari, oggetto, etichette e nomi degli allegati; il corpo mancante resta sconosciuto, quindi non soddisfa né esclude una parola.
5. **Ricerca sul server:** lettura IMAP senza segnare i messaggi come letti, cancellazione, scaricamento per blocchi e rifiltro locale con la stessa grammatica. Cartelle non disponibili, errori e contenuti mancanti impediscono false dichiarazioni di completezza. I risultati attivi restano consultabili anche quando i limiti della posta offline escludono i relativi contenuti dalla cache persistente.

## Struttura

Il motore in `app/src/domain/search.rs` e le regole dei fusi in `app/src/domain/regional.rs` non accedono a file, rete o interfacce. Sistema operativo e allegati sono gestiti nei moduli infrastrutturali e attraverso le porte dei servizi. Desktop, ricerca semantica, interfaccia nativa e strumenti MCP condividono la grammatica.

L’interfaccia desktop è in `desktop/src/components/SearchBar.svelte`; la rappresentazione e il recupero dei criteri sono in `desktop/src/lib/search.ts`. L’anteprima browser usa dati di esempio e un adattatore dedicato; il prodotto desktop usa il motore Rust.

## Verifiche automatiche

1. **Motore e servizi:** grammatica, date ai confini, account, `me`, cartelle escluse, paginazione, identità delle caselle remote, contenuti non disponibili e risultati dopo l’applicazione dei limiti offline.
2. **Architettura e trasporto:** entrambi i controlli dei livelli architetturali; tre verifiche su filtro IMAP conservativo, cartelle nascoste e date mancanti; cinque verifiche dedicate ai fusi.
3. **Interfaccia:** sette scenari browser/utility su ricerca visuale ±6 giorni, recupero e annullamento, data invalida, ricerca letterale, mittente, gruppi OR e lingua italiana. Disposizione verificata a 1440 e 900 pixel; immagini in `checks/search-ui/`.
4. **Formato regionale e anteprima:** quattro verifiche regionali e quattro della ricerca sui dati di esempio; controllo Svelte senza errori o avvisi.

Esito finale: 36 test dei servizi, 11 del motore/MCP, 2 di architettura, 5 dei fusi, 3 del trasporto e 3 sulle date del calendario/MCP superati. Anche la compilazione dell’app desktop e il controllo di tutti i target Rust sono riusciti. Il controllo Rust conserva avvisi già presenti relativi a letterali numerici dell’interfaccia nativa e alla dipendenza IMAP.

È riuscita anche una prova dell’eseguibile desktop, collegando l’automazione a un’istanza con soli account Sample e dati separati in `checks/native-search-smoke`: mittente esatto, intervallo di un giorno, fuso configurato, paginazione e validazione tramite i comandi Rust; errore di data invalida con risultati precedenti conservati nell’interfaccia. Lo script è `desktop/tests/search-native.mjs`. L’istanza di prova è stata chiusa al termine.

Comandi principali: `cargo test --offline --no-default-features --test search --test services --test layers` dalla cartella `app`; `node tests/regional.mjs`, `node tests/preview-search.mjs` e `npm run build` dalla cartella `desktop`. I test browser sono in `desktop/tests/search.mjs` e richiedono l’anteprima già avviata.

In questo ambiente i test dei servizi usano `TEMP` e `TMP` dentro `checks/search-temp`, dopo un errore di accesso al percorso temporaneo esterno. I test con dati su disco richiedono `QAMILLE_DATA` separato dai dati dell’utente, per esempio `checks/search-test-data`.

## Verifiche che richiedono un ambiente reale

Il collegamento IMAP a una casella Gmail reale richiede ancora una prova nell’app con l’account configurato. Il percorso automatico è verificato con gateway simulati e controlli del trasporto. Non sono state eseguite le sessioni moderate con otto partecipanti della specifica.

La misura su 50.000 messaggi è disponibile nel test ignorato `local_fifty_thousand_messages_timing`. Il limite di 500 ms al 95° percentile resta da valutare anche sull’intero servizio, includendo preparazione dei dati e allegati; una misura del solo motore non dimostra il tempo complessivo dell’interfaccia.

Nell’ultima prova del motore in build debug su questo computer, dieci ripetizioni per ricerca hanno prodotto p95 di 75 ms per mittente, 1.870 ms per parola nell’oggetto, 1.940 ms per parola nel corpo e 1.161 ms per data intorno più parola. Le misure sono variate sensibilmente fra esecuzioni: la soglia di 500 ms **non è raggiunta in modo generale** in questo ambiente. Rimane aperta la verifica e l’ottimizzazione delle prestazioni su una build destinata alla distribuzione e sull’intero percorso del servizio.

Le date storiche già salvate come sostituzioni dell’ora corrente non sono ricostruibili automaticamente: i nuovi download le distinguono con `date_missing`. La ricerca remota recupera la data originale quando presente nella sorgente.

Le date esplicite e il calendario rispettano il fuso configurato anche nei percorsi nativo/MCP. I vecchi preset nominati di pianificazione, come “Tomorrow” e “Weekend”, conservano il calcolo nel fuso del sistema: la loro conversione è un lavoro successivo distinto dai filtri di ricerca.

## Prova rapida

1. Avvia `Start-Qamille.cmd` dopo aver chiuso eventuali istanze precedenti.
2. Premi **Quando**, scegli una data e lascia **6 giorni prima e dopo**.
3. Applica il filtro; per ampliare premi **Non la trovo** e verifica **Annulla modifica**.

`around:2026-10-15` deve mostrare dal 9 al 21 ottobre 2026. In presenza di risultati parziali, **Cerca sul server** cerca i messaggi disponibili tramite IMAP mantenendo visibili gli eventuali limiti.
