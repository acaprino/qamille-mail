---
adr_id: ADR-0004
version: 2
title: Promemoria riunioni in una finestra Tauri guidata dal motore Rust; le notifiche di Windows per gli altri fatti
status: Accepted
author: Claude Code (su richiesta del proprietario del repository)
scope:
  paths:
    - desktop/src-tauri/src/**
    - app/src/application/meeting_reminders.rs
    - desktop/src/reminder/**
  modules:
  domains:
    - meetings
    - calendar
    - notifications
governs_symbols:
depends_on:
created_from_task: nativo-fase-3-task-3
proposed_on: 2026-11-21
proposed_at_sha: 82de4ec
accepted_on: 2026-11-21
accepted_at_sha: null
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes: ADR-0004
superseded_by: null
changes:
  - RA-001
---

# ADR-0004 v2: Promemoria riunioni in una finestra Tauri guidata dal motore Rust; le notifiche di Windows per gli altri fatti

<!-- adr-header:start -->
Version 2 · Accepted 2026-11-21 before its commit · proposed 2026-11-21 at 82de4ec
<!-- adr-header:end -->

## Context and Problem Statement

L'utente vuole un chip in sovrimpressione in alto al centro dello schermo poco prima di una riunione online, con i comandi "Entra", "Entra e registra" e "×" (specifica: `docs/specs/2026-11-03-promemoria-riunioni-design.md`). Domanda: quale meccanismo mostra il chip e da dove arrivano gli eventi, perché l'avviso compaia anche con Qamille in secondo piano e per le riunioni ricorrenti?

Con la fase 3 nativa (`docs/superpowers/plans/2026-11-21-nativo-fase-3.md`) Qamille resta aperto nell'area di notifica quando si chiude la finestra, e il proprietario ha approvato le notifiche di Windows per tre fatti che oggi si vedono solo nell'app: la posta nuova, un invio programmato non riuscito, un accesso da rinnovare. Domanda in più: il promemoria delle riunioni diventa anch'esso una notifica di Windows?

## Decision Drivers

- Il chip deve avere due pulsanti d'azione e comparire sopra le altre finestre.
- Un promemoria che non si vede fa perdere la riunione; gli altri tre fatti restano anche nell'app (`background-notice`, le Bozze, lo stato della sincronizzazione).
- Un'app non installata da un pacchetto deve registrare in Windows la sua identità perché le sue notifiche siano affidabili; la Professional si distribuisce anche come zip.
- WebView2 rallenta i timer della finestra principale ridotta a icona o nascosta.
- Le copie locali degli eventi Google contengono le serie e solo le occorrenze degli intervalli già aperti nella vista Calendario; non si aggiornano da sole.
- La ricerca per intervallo della vista Calendario (`list_events`) salva copie locali e riscrive l'intero workspace a ogni lettura.

## Decision

Il chip è una finestra Tauri dell'app (etichetta `meeting-reminder`): senza bordi, trasparente attorno alla pillola, sempre in primo piano, fuori dalla barra delle applicazioni, senza focus, in alto al centro dell'area di lavoro del monitor principale. Un ciclo nel processo Rust sceglie i chip ogni 15 secondi. Gli eventi Google del calendario principale (`primary`) di ogni account con accesso al calendario si leggono ogni 60 secondi con una richiesta propria in sola lettura all'API, con ricorrenze espanse, che non scrive né il workspace né `calendar-state.json`. Gli eventi del calendario locale si leggono dal workspace. La pagina del chip invia solo identificativo del chip e comando; il motore apre il link memorizzato. Il chip si chiude con la finestra principale. La scelta dei chip è una funzione pura del motore `app/`, testata senza interfaccia.

Il promemoria resta il chip. Le notifiche di Windows (`docs/aspetto-nativo.md`, regola 28) coprono gli altri tre fatti: la posta nuova in Posta in arrivo, un invio programmato non riuscito, un accesso da rinnovare. Le manda Rust come toast di Windows con `tauri-winrt-notification` (il motore del plugin ufficiale `tauri-plugin-notification`, usato direttamente perché il plugin tiene un thread in attesa di ogni notifica), da eventi tipizzati del motore che non arrivano alla pagina; l'app registra per l'utente l'identità con cui le manda (`HKCU\Software\Classes\AppUserModelId\<identificatore per cartella dati>`) finché sono accese.

## Consequences

- Funziona con Qamille ridotto a icona o nell'area di notifica, ma non con l'app chiusa.
- Una chiamata Google al minuto per account, ben sotto le quote dell'API.
- Il chip non emette suoni e non segue "Non disturbare" di Windows; le notifiche degli altri tre fatti lo seguono, e restano nel centro notifiche.
- Le notifiche di Windows hanno un interruttore in Impostazioni → Generali → Windows: accese per default nella Professional, spente nell'open source portatile, dove accenderle scrive una voce nel registro dell'utente.

## Verified Interview Examples
- **EX-001** (nominal): Input: evento Google "Standup" alle 10:00 con `hangoutLink`, anticipo 1 minuto, ore 09:59 -> Expected: chip in alto al centro "Standup · 10:00–10:15 · tra 1 min" con Entra, Entra e registra e ×.
- **EX-002** (nominal): Input: clic su "Entra e registra" -> Expected: link aperto nel browser, registrazione "Standup" avviata con microfono e audio di sistema predefiniti, chip rimosso.
- **EX-003** (boundary): Input: registrazione già in corso -> Expected: "Entra e registra" disattivato con il suggerimento "Registrazione già in corso"; "Entra" apre il link.
- **EX-004** (rejection): Input: evento rifiutato, di un giorno intero o senza link `https://` -> Expected: nessun chip.
- **EX-005** (boundary): Input: Qamille aperto alle 10:04 per una riunione delle 10:00 -> Expected: chip "iniziata da 4 min", rimosso da solo alle 10:10.

## Rejected Alternatives
- **RA-001** (v2, superata in parte): notifica di Windows con pulsanti per il promemoria; scartata per il promemoria perché una notifica di Windows riporta un clic solo con Qamille aperto, i pulsanti non sono quelli del chip e "Non disturbare" la silenzia: un promemoria che non si vede fa perdere la riunione. Per gli altri tre fatti la notifica di Windows è invece la scelta: l'app registra la sua identità in Windows, e un fatto silenziato da "Non disturbare" resta nel centro notifiche e nell'app.
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

Test Rust della funzione che sceglie i chip e della scelta dei dispositivi; prova nell'app reale con un evento locale di prova. Per le notifiche di Windows: il controllo 24 di `desktop/tests/native-feel.mjs`, i test `notify_rules_*` e `app/tests/system_notices.rs`, e gli scenari F e G del Task 9 sull'exe.
