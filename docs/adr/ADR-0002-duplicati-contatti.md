---
adr_id: ADR-0002
version: 1
title: Duplicati segnalati, mai risolti in automatico
status: Accepted
author: Alfio Caprino
scope:
  paths:
    - app/src/domain/contacts.rs
  modules:
  domains:
    - contacts
    - duplicates
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

# ADR-0002: Duplicati segnalati, mai risolti in automatico

<!-- adr-header:start -->
Version 1 · Accepted 2026-11-03 before the first commit · proposed 2026-11-03 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

La rubrica virtuale riunisce contatti da più rubriche. Lo stesso contatto può essere in 2, 3 o più rubriche. Domanda: quando e come si rilevano i doppioni, e chi decide cosa viene scritto sulle rubriche?

## Decision Drivers

- Un falso accorpamento costa più di un duplicato mancato.
- Gli indirizzi di ruolo (`info@`) sono condivisi da persone diverse.
- A=B per email e B=C per telefono non implica A=C.
- Soglie di similarità del nome senza supporto empirico locale.

## Decision

1. Il rilevamento produce **gruppi** di schede, non coppie. Un gruppo si forma solo con corrispondenza certa (stessa email normalizzata, stesso telefono E.164, stesso `UID`); i casi probabili sono coppie.
2. Livelli: certo (una sola scheda nella vista, segnalato), probabile (separate, segnalato), possibile (solo nel pannello). I livelli non compaiono nell'interfaccia.
3. Tre risposte soltanto: **Stessa persona** (nessuna rubrica scritta), **Unisci…** (scheda risultante modificabile + caselle delle rubriche, anteprima per rubrica, conferma), **Persone diverse** (non si ripropone).
4. **Unisci…** è l'unica operazione che elimina dati sui server. Le rubriche non spuntate perdono la copia.
5. "Posta" e "Google altri" non provocano segnalazioni.
6. Nessun tool MCP esegue Unisci…; `link_contacts` (Stessa persona) sì.
7. Prima di Unisci… si salva un diario per l'annulla; l'annulla passa dal controllo etag.

## Verified Interview Examples

- **EX-001** (nominal case): Input: stessa email in Google e Locale -> Expected: una scheda nella vista e una segnalazione; nessuna scrittura finché l'utente non risponde.
- **EX-002** (boundary case): Input: `info@azienda.it` in due schede -> Expected: nessun gruppo certo.
- **EX-003** (rejection case): Input: Unisci… con "Solo in una" su 3 rubriche -> Expected: anteprima che elenca le due eliminazioni e richiede conferma.

## Rejected Alternatives

- **RA-001**: unione automatica sui livelli alti; non reversibile sui server.
- **RA-002**: pulsante "Aggiorna" separato; sovrapposto a Unisci… con "In tutte".
- **RA-003**: tool MCP che applica l'unione; l'AI non deve eliminare dati.

## Enforceable Invariants

- **INV-001**: nessuna scrittura su una rubrica avviene senza una risposta esplicita dell'utente.
- **INV-002**: una scheda Posta o Google altri non è mai eliminata.
- **INV-003**: una coppia "Persone diverse" non torna in segnalazione.

## Consequences

- Dominio con normalizzazione, raggruppamento a chiavi (email, telefono, token di nome) e fusione campo per campo.

## Confirmation

- Corpus sintetico italiano; test dei tre pulsanti, annulla, esito parziale.
