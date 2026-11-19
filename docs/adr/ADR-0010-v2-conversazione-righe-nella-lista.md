---
adr_id: ADR-0010
version: 2
title: "Conversazione come righe nella lista: membri mostrati separati da membri su cui agire"
status: Accepted
author: Alfio Caprino
scope:
  paths:
    - desktop/src/components/MailList.svelte
    - desktop/src/components/Reader.svelte
    - desktop/src/App.svelte
    - desktop/src/lib/types.ts
    - desktop/src/lib/conversations.ts
    - app/src/domain/model.rs
    - app/src/domain/mail_export.rs
    - app/src/application/mail.rs
    - app/src/application/drafts.rs
    - app/src/infrastructure/mail.rs
    - app/src/infrastructure/sqlite_store.rs
  modules:
  domains:
    - mail-list
    - threads
governs_symbols:
depends_on:
  - ADR-0009
created_from_task: final-review-2026-11-07
proposed_on: 2026-11-07
proposed_at_sha: bdd91531fc66
accepted_on: 2026-11-07
accepted_at_sha: bdd91531fc66
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: null
changes:
  - INV-004
---

# ADR-0010: Conversazione come righe nella lista: membri mostrati separati da membri su cui agire

<!-- adr-header:start -->
Version 2 · Accepted 2026-11-07 at bdd9153 · proposed 2026-11-07 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

Nella modalità Conversazioni la lista raggruppa i messaggi della cartella aperta in una riga con un contatore (`app/src/application/mail.rs:478-509`, `desktop/src/components/MailList.svelte:296-300`), e i messaggi della conversazione si vedono solo nel pannello di lettura (`desktop/src/components/Reader.svelte:231-266`). Chi legge posta tutto il giorno e si perde nelle informazioni (richiesta dell'utente, 2026-11-07) non vede dalla lista chi ha risposto a chi, cosa è nuovo dentro una conversazione né quanti messaggi ne fanno parte.

Tre fatti del codice condizionano la risposta:

- La conversazione è un insieme, non un albero: `conversation_ids` unisce per uguaglianza di `message_id` e `thread_id` (`app/src/domain/mail_export.rs:15`).
- Il genitore diretto di una risposta ricevuta è già in `thread_id` (`mail.rs:505-513`), ma per una risposta inviata da Qamille `thread_id` copia quello del messaggio a cui si risponde (`app/src/application/drafts.rs:261`): un livello troppo in alto se si risponde a una risposta.
- `message_ids` di una riga è l'elenco su cui agiscono la puntina, la barra di selezione e il trascinamento (`MailList.svelte:75-79`), ma i pulsanti del pannello di lettura e il tasto `E` agiscono su un solo messaggio (`desktop/src/App.svelte:837`, `:1060`). Le risposte inviate stanno in Inviati, fuori dalla vista della Posta in arrivo.

La specifica completa è in `docs/specs/2026-11-07-aspetto-professionale-e-thread-design.md`.

## Decision Drivers

- Capire una conversazione con un colpo d'occhio dalla lista, senza aprire il lettore.
- Mai nascondere qualcosa senza lasciare una traccia visibile (numero e segno di non letto).
- Nessuna azione deve toccare per errore messaggi che la vista non mostra.
- Una riga non cambia altezza né si sposta mentre l'utente la guarda.

## Decision

**Forma.** Nella modalità Conversazioni la riga principale è il messaggio più recente della conversazione tra quelli della vista, come oggi. Aperta con la freccia, mostra sotto tutti gli altri messaggi, uno per riga, **tutti sulla stessa linea, senza rientri né linee di collegamento** (variante «D» scelta dall'utente), in ordine d'albero a partire dal primo messaggio. Ogni riga dice a chi risponde: «a *Persona*» e «+N» per gli altri destinatari di «A». Ogni risposta che ne ha altre sotto ha una freccia che le chiude; da chiusa mostra quante ne nasconde, in blu se tra loro c'è un messaggio non letto.

**Membri mostrati e membri su cui agire.** Le righe sotto la principale vengono da tutte le cartelle dello stesso account tranne Cestino e Spam. Solo i messaggi che passano il filtro della vista aperta sono «membri d'azione» (`message_ids`). Una riga di risposta non ha casella, puntina né trascinamento. Nel pannello di lettura, su un messaggio solo mostrato, Archivia, Fatto, Posticipa, Fissa, Segna come non letto, Elimina e il tasto `E` sono disattivati.

**Dati.** `Email` riceve un campo `in_reply_to` (nuova colonna, `SCHEMA_VERSION` 4). Si riempie alla ricezione dall'intestazione In-Reply-To e all'invio da `draft.reply_to`. Per i messaggi già salvati si ricava da `thread_id` solo per la posta ricevuta; per quelli in Inviati resta ignoto e la riga si appende sotto il messaggio più vecchio. `thread_id` non cambia significato. I nomi dei destinatari non vengono salvati: si usano il contatto della Rubrica, il `from_name` noto per quell'indirizzo o l'indirizzo.

## Verified Interview Examples

- **EX-001** (nominal case): Input: Posta in arrivo in modalità Conversazioni, una conversazione di 9 messaggi di cui 8 risposte -> Expected: una riga principale con il messaggio più recente e 8 righe sotto, tutte con lo stesso margine sinistro, senza linee; la freccia mostra 9.
- **EX-002** (boundary case): Input: una risposta con altre 5 sotto, chiusa con la sua freccia, con un messaggio non letto tra le 5 -> Expected: le 5 spariscono, la freccia mostra «5» in blu; riaperta, ricompaiono tutte e 5.
- **EX-003** (boundary case): Input: una risposta con 40 destinatari in «A» e 6 in Cc -> Expected: nella lista la riga ha la stessa altezza delle altre e mostra «a *Persona* +39»; nel pannello di lettura si vedono tre nomi e «e altri 37», e «Cc» ha la sua riga col suo conteggio.
- **EX-004** (rejection or error case): Input: una conversazione con una risposta inviata (in Inviati); si archivia dalla riga principale, poi si apre quella risposta e si preme `E` -> Expected: l'archiviazione tocca solo i messaggi della vista aperta; sulla risposta inviata Archivia, Fatto, Elimina e il tasto `E` sono disattivati e non fanno nulla.
- **EX-005** (rejection or error case): Input: un messaggio della stessa conversazione è in Cestino -> Expected: non compare tra le righe e non entra in nessun conteggio.

## Rejected Alternatives

- **RA-001**: albero con linee di collegamento e fino a 5 livelli di rientro. L'utente lo ha giudicato confusionario; con 3 livelli restava il problema del legame, e le righe piatte con «a *Persona*» lo risolvono senza consumare larghezza.
- **RA-002**: lasciare la conversazione solo nel pannello di lettura. Non dà il colpo d'occhio dalla lista, che è lo scopo.
- **RA-003**: correggere `thread_id` perché diventi sempre il genitore diretto. Lo usano `app/src/adapters/mcp.rs:185` e `conversation_ids`: cambiarne il significato rompe l'uno e l'altro.
- **RA-004**: ricavare il genitore delle risposte inviate già salvate da `thread_id`. Punta un livello troppo in alto e darebbe un legame falso con aria di certezza.
- **RA-005**: salvare i nomi dei destinatari in colonne nuove. Non serve alla decisione, non riempie i dati già salvati e allarga lo schema.
- **RA-006**: includere le risposte inviate in `message_ids`. Archivia o Fissa dalla Posta in arrivo toccherebbe messaggi in Inviati.

## Enforceable Invariants

- **INV-001**: `message_ids` di una riga di conversazione contiene solo messaggi che passano il filtro della vista aperta; un messaggio solo mostrato non entra mai nella selezione, nella puntina né nel trascinamento.
- **INV-002**: nel pannello di lettura, le azioni che cambiano lo stato di un messaggio e il tasto `E` sono disattivati su un messaggio che non appartiene alla vista.
- **INV-003**: l'altezza di una riga di risposta non dipende dal numero di destinatari né dalla lunghezza di nomi e indirizzi; il «+N» e la data restano sempre visibili.
- **INV-004**: i messaggi in Cestino o Spam non compaiono tra le righe di una conversazione, tranne nella vista di quella cartella.
- **INV-005**: una risposta chiusa mostra sempre il numero dei messaggi che nasconde, in blu se tra loro ce n'è uno non letto; lo stato aperto o chiuso non cambia da solo all'arrivo di posta.
- **INV-006**: `in_reply_to` è riempito per ogni messaggio ricevuto o inviato dopo la migrazione; la migrazione non lo deduce mai per i messaggi già salvati in Inviati.
- **INV-007**: il `read` della riga e i conteggi del titolo e dei gruppi dipendono solo dai messaggi della vista, non da quelli solo mostrati.

## Consequences

- Cambiano il modello e l'archivio (`app/src/domain/model.rs`, `app/src/infrastructure/sqlite_store.rs`, migrazione dello schema da 3 a 4), l'invio (`app/src/application/drafts.rs`), la lettura (`app/src/infrastructure/mail.rs`) e `list_mail` (`app/src/application/mail.rs`), che deve restituire le righe di risposta e un tetto con «Mostra altre N risposte».
- Nel frontend cambiano `MailList.svelte`, `Reader.svelte`, `App.svelte` e `desktop/src/lib/types.ts`. Nella modalità Conversazioni la sezione «Messaggi della conversazione» del lettore si nasconde e i messaggi in Cestino o Spam, che oggi mostra, non compaiono più.
- Il modo di leggere i messaggi come non letti cambia: una riga di conversazione non può dire quale messaggio sia non letto (`App.svelte:586`), mentre le righe separate sì.
- I test che citano conversazioni e thread vanno riletti; la ricerca resta una lista piatta.
- Frame Doop, design system e prototipo in Claude vanno rigenerati dopo la modifica.

## Confirmation

- Test del frontend per EX-001…EX-005 e per INV-002, INV-003 e INV-005 (apertura e chiusura dei nodi, 40 destinatari, azioni disattivate sul messaggio inviato).
- Test del backend per INV-001, INV-004, INV-006 e INV-007, compresa la migrazione 3→4 e l'eco della risposta inviata sia sostituita sia unita sul posto (`mail_cache_download.rs:566-578`, `app/src/application/mail.rs:1696`).
- `node desktop/tests/style-tokens.mjs` continua a passare (ADR-0009).
