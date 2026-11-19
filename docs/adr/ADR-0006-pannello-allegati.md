---
adr_id: ADR-0006
version: 1
title: Pannello laterale unico per gli allegati, mail allegate comprese
status: Proposed
author: Alfio Caprino
scope:
  paths:
    - desktop/src/components/AttachmentPanel.svelte
    - desktop/src/components/AttachedMessageView.svelte
    - desktop/src/components/Reader.svelte
    - app/src/domain/attached_message.rs
  modules:
    - desktop/src/components
    - app/src/domain
  domains:
    - mail-reader
    - attachments
governs_symbols:
depends_on:
created_from_task: brainstorm-eml-allegati
proposed_on: 2026-11-04
proposed_at_sha: null
accepted_on: null
accepted_at_sha: null
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: null
changes:
---

# ADR-0006: Pannello laterale unico per gli allegati, mail allegate comprese

<!-- adr-header:start -->
Version 1 · Proposed 2026-11-04 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

Gli allegati si aprivano in un riquadro dentro il lettore, sotto l'elenco
degli allegati (`desktop/src/components/Reader.svelte`, prima del
2026-11-04). Un `.eml` allegato, per esempio la `postacert.eml` di una PEC
inoltrata, veniva mostrato come testo grezzo: `app/src/infrastructure/attach.rs:47`
trattava `eml` come un file di testo. Serve un modo per leggere una mail
allegata come una mail vera, con i suoi allegati, e capire dove si apre.

## Decision Drivers

- L'utente legge le PEC inoltrate sulla casella normale e non vuole entrare nella casella PEC.
- Un solo posto per aprire qualsiasi allegato: documenti ArtCraft (PDF, DOCX, PPTX, XLSX) e mail allegate.
- Una mail allegata può contenere altri allegati, comprese altre mail.

## Decision

Ogni allegato del lettore si apre in un pannello laterale che scorre da destra
sopra il lettore (`desktop/src/components/AttachmentPanel.svelte`). Un allegato
è una mail quando il tipo è `message/rfc822` o il nome finisce in `.eml`. In quel caso
il pannello la mostra come mail formattata: oggetto, mittente, destinatari, data, corpo HTML con le
stesse protezioni del lettore, elenco dei suoi allegati
(`desktop/src/components/AttachedMessageView.svelte`). Un allegato della mail
allegata si apre nello stesso pannello, con un ritorno al livello precedente.
Il backend indirizza una parte annidata con il percorso degli indici di
allegato a partire dall'allegato del messaggio (`app/src/domain/attached_message.rs`).

## Verified Interview Examples

- **EX-001** (nominal case): Input: clic su `postacert.eml` in una mail inoltrata -> Expected: il pannello mostra la mail con oggetto, mittente, testo e i suoi PDF.
- **EX-002** (boundary case): Input: clic su un PDF dentro `postacert.eml` -> Expected: il PDF si apre nello stesso pannello con `← postacert.eml` per tornare indietro.
- **EX-003** (rejection or error case): Input: un `.eml` che non contiene intestazioni da mail, o un percorso oltre 8 livelli -> Expected: il pannello mostra un errore leggibile, nessun contenuto inventato.

## Rejected Alternatives

- **RA-001**: riquadro sotto gli allegati nel lettore, come prima. L'utente ha scelto il pannello laterale per avere un unico posto per tutti gli allegati.
- **RA-002**: sostituire tutto il lettore con la mail allegata. Si perde il contesto della mail di inoltro.
- **RA-003**: trattamento speciale della busta PEC (`daticert.xml`). L'utente apre direttamente `postacert.eml` e vuole solo la mail.

## Enforceable Invariants

- **INV-001**: ogni allegato del lettore, documento o mail, si apre nel pannello laterale e non dentro il corpo del lettore.
- **INV-002**: la vista di una mail allegata usa `MessageHtml`, con lo stesso blocco delle immagini remote del lettore.
- **INV-003**: una parte annidata si raggiunge solo dall'allegato del messaggio aperto, entro `MAX_DEPTH` = 8 livelli e 25 MB.

## Consequences

- Il lettore non mostra più l'anteprima in linea. Lo smoke test apre e chiude il pannello.
- Nuovo comando `read_attached_message`. `attachment_preview_open` e `read_attachment` accettano un `path` facoltativo.
- Con il pannello aperto, le scorciatoie a un tasto (R, E, frecce) non agiscono sulla mail dietro.

## Confirmation

- `app/src/domain/attached_message.rs` test unitari (busta PEC, `postacert.eml`, due livelli, rifiuti).
- `desktop/tests/attachment-panel.mjs` (mail allegata, PDF annidato, ritorno, mail annidata, Esc).
- `desktop/tests/smoke.mjs` passo "Attachment preview".
