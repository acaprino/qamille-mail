---
adr_id: ADR-0001
version: 1
title: Rubrica virtuale derivata con origini per scheda
status: Accepted
author: Alfio Caprino
scope:
  paths:
    - app/src/domain/contacts.rs
    - app/src/application/contacts.rs
  modules:
  domains:
    - contacts
    - address-book
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

# ADR-0001: Rubrica virtuale derivata con origini per scheda

<!-- adr-header:start -->
Version 1 · Accepted 2026-11-03 before the first commit · proposed 2026-11-03 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

Qamille non ha una rubrica: `MailService::search_suggestions` (`app/src/application/mail.rs:511`) ricava persone da mittenti, A e Cc. Si vuole una rubrica che riunisca contatti di più caselle e rubriche, fonda i doppioni e mostri da dove viene ogni scheda. Domanda: la rubrica unificata è un archivio o una vista, e dove vivono i dati?

## Decision Drivers

- Due fonti di verità (sorgenti e risultato fuso) divergono.
- Il file principale `store.json` ha un lock condiviso con la posta (`app/ARCHITECTURE.md:30-37`).
- Il calendario tiene già lo stato di sync in un file separato con lock e rename atomico (`app/src/adapters/mcp/calendar.rs:32-105`).
- Il dominio non può fare I/O (`app/tests/layers.rs`).

## Decision

1. La rubrica unificata è una **vista calcolata**, mai salvata. Si ricalcola a ogni lettura e sincronizzazione dalle schede sorgente.
2. Si salvano in `data/contacts.json` (accanto a `store.json`, via `Store::data_dir`), dietro una porta applicativa `ContactStore`, solo: schede sorgente per rubrica, collegamenti "Stessa persona", coppie "Persone diverse", valori fissati a mano, schede nascoste, coda scritture, diario di annulla, impostazioni della rubrica.
3. Ogni scheda unificata elenca le **origini**: Locale, Google (per account), Google altri, vCard importata, Posta (per casella).
4. "Posta" e "Google altri" sono sola lettura: non si eliminano, si nascondono.
5. Creando un contatto la rubrica di destinazione è **obbligatoria** (Locale portable o un account Google con scrittura); nessuna destinazione implicita.
6. Le impostazioni della rubrica non stanno in `Settings` di `store.json`.

## Verified Interview Examples

- **EX-001** (nominal case): Input: persona con scheda Google, rubrica Locale e messaggi nella casella lavoro -> Expected: una scheda con tre etichette di origine.
- **EX-002** (boundary case): Input: contatto solo "Posta" -> Expected: non modificabile; "Aggiungi ai contatti" apre il modulo con rubrica obbligatoria.
- **EX-003** (rejection case): Input: salvataggio di un nuovo contatto senza rubrica scelta -> Expected: rifiutato.

## Rejected Alternatives

- **RA-001**: salvare anche il risultato fuso; crea due fonti di verità che divergono.
- **RA-002**: contatti dentro `Store`; ogni sync riscrive l'aggregato e tiene il lock della posta.
- **RA-003**: destinazione predefinita implicita per i nuovi contatti; scrive su un servizio esterno senza che l'utente lo sappia.

## Enforceable Invariants

- **INV-001**: `src/domain/contacts.rs` non usa I/O (`layers.rs` resta verde).
- **INV-002**: nessun file di `application/` importa `crate::infrastructure`.
- **INV-003**: leggere la rubrica non prende il lock di `store.json`.
- **INV-004**: dopo un ciclo lettura-scrittura Google i campi non gestiti restano invariati.

## Consequences

- Nuovo file `data/contacts.json`, da includere nell'export portabile.
- Nuova porta `ContactStore` in `application/ports.rs`.

## Confirmation

- Test del dominio e del servizio con archivio in memoria; test `layers.rs`.
