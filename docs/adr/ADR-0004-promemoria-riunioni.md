---
adr_id: ADR-0004
version: 1
title: Promemoria riunioni in una finestra Tauri guidata dal motore Rust
status: Superseded
author: Alfio Caprino
scope:
  paths:
    - desktop/src-tauri/src/**
    - app/src/application/meeting_reminders.rs
    - desktop/src/reminder/**
  modules:
  domains:
    - meetings
    - calendar
governs_symbols:
depends_on:
created_from_task: change-promemoria-riunioni
proposed_on: 2026-11-03
proposed_at_sha: null
accepted_on: 2026-11-03
accepted_at_sha: null
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: ADR-0004 v2
changes:
---

# ADR-0004: Promemoria riunioni in una finestra Tauri guidata dal motore Rust

<!-- adr-header:start -->
Version 1 · Accepted 2026-11-03 before the first commit · proposed 2026-11-03 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

L'utente vuole un chip in sovrimpressione in alto al centro dello schermo poco prima di una riunione online, con i comandi "Entra", "Entra e registra" e "×" (specifica: `docs/specs/2026-11-03-promemoria-riunioni-design.md`). Domanda: quale meccanismo mostra il chip e da dove arrivano gli eventi, perché l'avviso compaia anche con Qamille in secondo piano e per le riunioni ricorrenti?

## Decision Drivers

- Il chip deve avere due pulsanti d'azione e comparire sopra le altre finestre.
- L'app è portatile e non è registrata in Windows come mittente di notifiche.
- WebView2 rallenta i timer della finestra principale ridotta a icona.
- Le copie locali degli eventi Google contengono le serie e solo le occorrenze degli intervalli già aperti nella vista Calendario; non si aggiornano da sole.
- La ricerca per intervallo della vista Calendario (`list_events`) salva copie locali e riscrive l'intero workspace a ogni lettura.

## Decision

Il chip è una finestra Tauri dell'app (etichetta `meeting-reminder`): senza bordi, trasparente attorno alla pillola, sempre in primo piano, fuori dalla barra delle applicazioni, senza focus, in alto al centro dell'area di lavoro del monitor principale. Un ciclo nel processo Rust sceglie i chip ogni 15 secondi. Gli eventi Google del calendario principale (`primary`) di ogni account con accesso al calendario si leggono ogni 60 secondi con una richiesta propria in sola lettura all'API, con ricorrenze espanse, che non scrive né il workspace né `calendar-state.json`. Gli eventi del calendario locale si leggono dal workspace. La pagina del chip invia solo identificativo del chip e comando; il motore apre il link memorizzato. Il chip si chiude con la finestra principale. La scelta dei chip è una funzione pura del motore `app/`, testata senza interfaccia.

## Consequences

- Funziona con Qamille ridotto a icona, ma non con l'app chiusa: non c'è un'icona nell'area di notifica.
- Una chiamata Google al minuto per account, ben sotto le quote dell'API.
- Il chip non emette suoni e non segue "Non disturbare" di Windows.

## Verified Interview Examples
- **EX-001** (nominal): Input: evento Google "Standup" alle 10:00 con `hangoutLink`, anticipo 1 minuto, ore 09:59 -> Expected: chip in alto al centro "Standup · 10:00–10:15 · tra 1 min" con Entra, Entra e registra e ×.
- **EX-002** (nominal): Input: clic su "Entra e registra" -> Expected: link aperto nel browser, registrazione "Standup" avviata con microfono e audio di sistema predefiniti, chip rimosso.
- **EX-003** (boundary): Input: registrazione già in corso -> Expected: "Entra e registra" disattivato con il suggerimento "Registrazione già in corso"; "Entra" apre il link.
- **EX-004** (rejection): Input: evento rifiutato, di un giorno intero o senza link `https://` -> Expected: nessun chip.
- **EX-005** (boundary): Input: Qamille aperto alle 10:04 per una riunione delle 10:00 -> Expected: chip "iniziata da 4 min", rimosso da solo alle 10:10.

## Rejected Alternatives
- **RA-001**: notifica di Windows con pulsanti; scartata perché il plugin di notifiche Tauri non ha pulsanti sul desktop, l'app portatile dovrebbe registrarsi in Windows e "Non disturbare" la silenzia.
- **RA-002**: controllo con un timer nella finestra principale; scartato perché WebView2 rallenta i timer delle finestre ridotte a icona.
- **RA-003**: usare le copie locali degli eventi Google; scartata perché contengono solo le occorrenze degli intervalli già aperti nella vista Calendario e non si aggiornano da sole.
- **RA-004**: leggere Google con `list_events` della vista Calendario; scartata perché a ogni lettura salva copie locali e riscrive l'intero workspace.

## Enforceable Invariants
- **INV-001**: il chip non compare per eventi rifiutati, di un giorno intero o senza link `https://`.
- **INV-002**: si aprono solo link `https://`.
- **INV-003**: "Entra e registra" non interrompe né sostituisce una registrazione in corso.
- **INV-004**: la finestra del chip non prende il focus quando compare e non compare nella barra delle applicazioni.
- **INV-005**: il controllo periodico non scrive su disco.
- **INV-006**: la pagina del chip non può far aprire indirizzi diversi dal link scelto dal motore.

## Confirmation

Test Rust della funzione che sceglie i chip e della scelta dei dispositivi; prova nell'app reale con un evento locale di prova.
