---
adr_id: ADR-0003
version: 1
title: Sincronizzazione contatti Google con People API
status: Accepted
author: Alfio Caprino
scope:
  paths:
    - app/src/infrastructure/google_people.rs
  modules:
  domains:
    - contacts
    - google-sync
governs_symbols:
depends_on:
created_from_task: rubriche
proposed_on: 2026-11-03
proposed_at_sha: null
accepted_on: 2026-11-03
accepted_at_sha: null
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: null
changes:
---

# ADR-0003: Sincronizzazione contatti Google con People API

<!-- adr-header:start -->
Version 1 · Accepted 2026-11-03 before the first commit · proposed 2026-11-03 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

Le rubriche Google degli account Gmail vanno lette e modificate. Il consenso OAuth del calendario è già separato da quello della posta (`app/src/infrastructure/oauth.rs:21-23`) e il consenso è un flusso browser bloccante (`oauth::sign_in_with_scope`). Domanda: come si sincronizza, si scrive e si gestiscono consenso e conflitti?

## Decision Drivers

- Il sync token scade dopo 7 giorni (`EXPIRED_SYNC_TOKEN`).
- `updateContact` richiede `etag`; le scritture per lo stesso utente vanno in sequenza.
- Le scritture si propagano ai sync con ritardo di minuti.
- Il consenso non può scattare da una coda in background.

## Decision

1. Consenso separato da posta e calendario. Lettura (`contacts.readonly`, `contacts.other.readonly`) al collegamento della rubrica; scrittura (`contacts`) alla prima modifica salvata che tocca quella rubrica. Se negata, la modifica non si salva. Un solo segreto `contacts:{account_id}`.
2. Lettura incrementale con `requestSyncToken`, `pageSize` 1000. Token scaduto: risincronizzazione completa, caso normale.
3. Scrittura con `etag` e `updatePersonFields`, in sequenza per account. Dopo una scrittura si applica la risposta localmente, non si rilegge via sync.
4. Etag diverso: confronto a tre vie (base, locale, remoto). Conflitto sullo stesso campo: scheda "in conflitto", l'utente sceglie per scheda intera (Tieni la mia / Tieni quella di Google). Mai sovrascrittura cieca.
5. Coda persistente per le scritture senza rete; mai scritture "in attesa di consenso".
6. Sincronizzazione all'avvio in background, all'apertura della Rubrica e con "Aggiorna ora"; nessun job periodico.
7. "Altri contatti": sola lettura; la promozione usa `copyOtherContactToMyContactsGroup`.
8. Dati non gestiti (gruppi, foto, altri campi) conservati e riscritti invariati.

## Verified Interview Examples

- **EX-001** (nominal case): Input: modifica locale di un contatto Google con consenso già dato -> Expected: `updateContact` con etag, risposta applicata localmente.
- **EX-002** (boundary case): Input: app chiusa 8 giorni -> Expected: risincronizzazione completa senza errore.
- **EX-003** (rejection case): Input: etag obsoleto e stesso campo cambiato -> Expected: scheda in conflitto; nessuna sovrascrittura.

## Rejected Alternatives

- **RA-001**: scrittura in blocco concorrente; l'API richiede sequenza.
- **RA-002**: rileggere via sync dopo ogni scrittura; propagazione ritardata.
- **RA-003**: consenso di scrittura al collegamento; amplia i permessi a chi usa solo la lettura.

## Enforceable Invariants

- **INV-001**: una sola richiesta di scrittura alla volta per account.
- **INV-002**: nessun `updateContact` senza etag.
- **INV-003**: nessuna scrittura in coda senza consenso valido.

## Consequences

- Nuovo adapter `google_people.rs`; nuovo `CONTACTS_SCOPE` in `oauth.rs`; People API abilitata nel progetto (guida `docs/oauth-google.md`).

## Confirmation

- Gateway Google finto: token scaduto, etag obsoleto, scrittura in sequenza, coda offline, conflitto. L'accesso reale richiede la prova manuale dell'utente.
