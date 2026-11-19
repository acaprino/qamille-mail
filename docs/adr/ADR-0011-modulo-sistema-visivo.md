---
adr_id: ADR-0011
version: 1
title: Il sistema visivo di Qamille è un modulo interno desktop/src/ds/, non un pacchetto
status: Superseded
author: Alfio Caprino
scope:
  paths:
    - desktop/src/theme.css
    - desktop/src/styles.css
    - desktop/src/components/Icon.svelte
    - desktop/src/ds/**
    - desktop/scripts/ds-export.mjs
    - desktop/tests/ds-usage.mjs
    - desktop/tests/style-tokens.mjs
  modules:
  domains:
    - design-system
governs_symbols:
depends_on:
  - ADR-0008
  - ADR-0009
created_from_task: modulo-ds-2026-11-07
proposed_on: 2026-11-07
proposed_at_sha: 72e9e16aec4ef5679d40d434113ac4514e395b6d
accepted_on: 2026-11-07
accepted_at_sha: bdd91531fc66
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: ADR-0011 v2
changes:
---

# ADR-0011: Il sistema visivo di Qamille è un modulo interno desktop/src/ds/, non un pacchetto

<!-- adr-header:start -->
Version 1 · Superseded by ADR-0011 v2 · proposed 2026-11-07 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

Qamille non ha un modulo per il proprio sistema visivo. I token stanno in `desktop/src/theme.css` (80 variabili, 117 dichiarazioni nei due temi), le classi in `desktop/src/styles.css` (2880 righe, di cui 398 di base e di sistema e 2481 di pagina, secondo una classificazione per selettore fatta il 2026-11-07), il markup dei controlli è riscritto a mano: le classi `primary`, `secondary`, `icon-button` e `text-button` compaiono 177 volte in 30 dei 39 file Svelte, e `<dialog>` 8 volte in 8 file, ognuno con la propria gestione di velo e chiusura.

La guida Doop e il design system in Claude si ricavano da fonti diverse: `checks/doop-sync/tokens.mjs` legge `theme.css`, gli script che hanno prodotto il design system catturano il DOM dall'app in esecuzione, e la cartella `checks/` è ignorata da git. `/design-sync` non è utilizzabile: accetta solo librerie React con un `dist/`.

La domanda: dove vive il sistema visivo di Qamille, e in quale forma?

## Decision Drivers

- Una modifica a un controllo tocca un solo file.
- Il sistema visivo ha una sola fonte versionata, da cui si generano le copie esterne con un comando ripetibile.
- L'app resta il solo consumatore: nessuna versione o build in più senza un secondo utilizzatore.
- Nessuna seconda implementazione dei componenti.
- Vale ADR-0008 (tre contenitori sovrapposti) e ADR-0009 (ogni valore visivo da un token).

## Decision

Il sistema visivo diventa un modulo interno, `desktop/src/ds/`, non un pacchetto:

- `tokens/theme.css`: i token, spostati da `desktop/src/theme.css`.
- `primitives/`: componenti Svelte `Button` (varianti primary, secondary, icon, text), `Modal`, `SidePanel`, `Popover`, `Field` e `Icon`, spostato da `desktop/src/components/Icon.svelte`.
- `styles/system.css`: le regole di base e di sistema oggi in `styles.css`. Le regole di pagina restano in `styles.css` o negli stili dei componenti.
- `README.md`: la guida del sistema visivo, versionata.

`desktop/scripts/ds-export.mjs`, lanciato con `npm run ds:export`, genera i file del design system in Claude dal modulo. Le primitive hanno anteprime prodotte dal modulo; i componenti di pagina (per esempio `Reader`, `Calendar`, `Settings`) continuano ad avere anteprime catturate dall'app con i dati di esempio. Il flusso Doop resta quello a tre vie di `tokens.mjs`, che legge il nuovo percorso, e gli script necessari passano sotto git.

## Verified Interview Examples

- **EX-001** (nominal case): Input: cambia il padding della variante primary in `desktop/src/ds/primitives/Button.svelte` -> Expected: cambiano tutti i pulsanti primari dell'app e nessun file in `desktop/src/components/`.
- **EX-002** (boundary case): Input: una classe usata da una sola pagina, come `.calendar-event` -> Expected: resta nello stile della pagina e non entra in `desktop/src/ds/`.
- **EX-003** (boundary case): Input: la finestra di composizione -> Expected: resta una finestra agganciata e non usa `Modal`, come stabilisce ADR-0008.
- **EX-004** (rejection or error case): Input: un nuovo `<button class="primary">` o `<dialog class="modal">` in `desktop/src/components/` -> Expected: `node tests/ds-usage.mjs` fallisce indicando file, riga e primitiva da usare.
- **EX-005** (nominal case): Input: `npm run ds:export` due volte senza modifiche al modulo -> Expected: file identici byte per byte.

## Rejected Alternatives

- **RA-001**: un pacchetto separato `packages/design-system`. Con un solo consumatore aggiunge versioni e build senza guadagno; si rivaluta solo con un secondo consumatore reale.
- **RA-002**: una libreria React per poter usare `/design-sync`. Sarebbe una seconda implementazione che l'app non usa, e quindi destinata a divergere.
- **RA-003**: la sola cartella `desktop/src/components/ui/` con le primitive. Lascerebbe token, stili di sistema, guida ed esportazione in posti diversi.
- **RA-004**: lasciare le classi come sono. Le 177 occorrenze e gli 8 dialog resterebbero da modificare file per file.

## Enforceable Invariants

- **INV-001**: i file in `desktop/src/ds/` non importano da `desktop/src/lib/`, da `desktop/src/components/` né da `@tauri-apps`.
- **INV-002**: fuori da `desktop/src/ds/`, il markup non usa le classi `primary`, `secondary`, `icon-button`, `text-button`, `popover`, `side-panel-layer` né l'elemento `<dialog>`.
- **INV-003**: i valori dei token stanno solo in `desktop/src/ds/tokens/theme.css`; `desktop/tests/style-tokens.mjs` lo controlla al nuovo percorso.
- **INV-004**: `npm run ds:export` è deterministico: a parità di modulo e di dati di esempio produce gli stessi file.
- **INV-005**: il modulo contiene una sola implementazione di ogni componente, in Svelte.

## Consequences

- La migrazione tocca 30 dei 39 file Svelte; `Icon` è importato da 26 file.
- `theme.css` è citato in 8 file e `styles.css` in 15, tra cui `desktop/src/reminder/reminder.css`, i test e gli ADR. ADR-0008 e ADR-0009 citano questi percorsi: ricevono una nuova versione nello stesso lavoro dello spostamento.
- Il design system in Claude diventa un risultato generato: una modifica fatta sulla sua pagina va riportata nel modulo, o la prossima esportazione la sovrascrive.
- `/design-sync` resta inutilizzabile: il modulo è in Svelte.
- La migrazione non deve cambiare l'aspetto: frame Doop e prototipo non vanno ricatturati se non cambia nulla.

## Confirmation

- `node desktop/tests/style-tokens.mjs` e il nuovo `node desktop/tests/ds-usage.mjs` passano.
- `npm run check` e i test browser del frontend passano.
- `checks/doop-sync/extract.mjs --verify` prima e dopo la migrazione: stesse schermate, 0 pixel diversi tra le due catture.
- Due esecuzioni di `npm run ds:export` producono file identici.
