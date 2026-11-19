---
adr_id: ADR-0009
version: 2
title: Ogni valore visivo viene dai token di theme.css
status: Accepted
author: Alfio Caprino
scope:
  paths:
    - desktop/src/ds/tokens/theme.css
    - desktop/src/ds/styles/system.css
    - desktop/src/styles.css
    - desktop/src/print.css
    - desktop/src/components
    - desktop/tests/style-tokens.mjs
    - desktop/scripts/doop-sync/tokens.notes.md
  modules:
    - desktop/src
  domains:
    - design-system
governs_symbols:
depends_on:
  - ADR-0008
created_from_task: modulo-ds-2026-11-07
proposed_on: 2026-11-07
proposed_at_sha: f780db0
accepted_on: 2026-11-07
accepted_at_sha: f780db0
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: null
changes:
  - INV-001
---

# ADR-0009: Ogni valore visivo viene dai token di theme.css

<!-- adr-header:start -->
Version 2 · Accepted 2026-11-07 at f780db0 · proposed 2026-11-07 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

Dopo l'unificazione degli overlay (ADR-0008) il resto dell'app conteneva ancora
valori visivi scritti a mano, fuori da `desktop/src/ds/tokens/theme.css` (rilevati il
2026-11-04): 21 colori esadecimali, 8 ombre con quattro intensità diverse, 6 raggi
fuori scala (3px, 6px, 8px, 11px, 999px), 8 z-index globali sparsi (6, 13, 16, 20,
35, 100) e 8 dimensioni del testo in px. Tre gruppi di campi (filtri ricerca,
filtri rubrica, scheda contatto) avevano bordo e padding diversi dal campo base.
L'utente ha chiesto di rendere coerente tutta l'app, non solo gli overlay.

## Decision Drivers

- Un valore visivo ha un solo posto: `ds/tokens/theme.css`, esportato nella guida Doop.
- Lo stesso ruolo ha lo stesso aspetto in ogni schermata.
- La regola deve reggere nel tempo senza revisioni manuali.

## Decision

Ogni colore, ombra, raggio, livello z globale e dimensione del testo dell'app
viene da un token di `desktop/src/ds/tokens/theme.css`. Nuovi token:

- `--shadow-sm` per bottoni in rilievo, barra dell'assistente e bozza evento; `--shadow` resta per le superfici flottanti.
- Scala dei livelli: `--z-popover` 10, `--z-composer` 12, `--z-dock` 13, `--z-floating` 16, `--z-panel` 20, `--z-toast` 50, `--z-window` 100. Gli z-index da 1 a 5 restano liberi per l'ordine interno di un componente.
- `--highlight` per le corrispondenze di ricerca, `--paper*` per pagine di documenti e stampa, `--sender-1..4` per gli avatar, `--label-1..5` per i pallini delle etichette, `--window-close` per il pulsante chiudi della finestra.

I campi di input, select e textarea seguono la regola base di `ds/styles/system.css`
(bordo `--border`, raggio `--radius-md`, padding 7px 9px); i componenti
cambiano solo la larghezza. `desktop/tests/style-tokens.mjs` fallisce su un
valore letterale fuori da `ds/tokens/theme.css`, con un'eccezione dichiarata per il corpo
delle email (documento isolato che non legge i token) e per il gradiente della
maschera di scorrimento.

## Verified Interview Examples

- **EX-001** (nominal case): Input: un bottone primario nel lettore, nel calendario e in un dialog -> Expected: stessa ombra `--shadow-sm`, stesso raggio `--radius-md`.
- **EX-002** (boundary case): Input: un campo nella scheda contatto e uno nel pannello evento -> Expected: stesso bordo, raggio e padding.
- **EX-003** (rejection or error case): Input: un componente nuovo con `color: #c42b1c` o `z-index: 12` -> Expected: `node tests/style-tokens.mjs` fallisce e indica file, riga e token da usare.

## Rejected Alternatives

- **RA-001**: correggere solo i valori visibili oggi senza un controllo. Le derive tornerebbero alla prima modifica.
- **RA-002**: un token per ogni valore esistente. Manterrebbe le quattro ombre e i raggi fuori scala invece di unificarli.
- **RA-003**: portare i colori del corpo email sui token. Il documento è isolato e in tema scuro viene invertito: i token non lo raggiungono.

## Enforceable Invariants

- **INV-001**: nessun colore letterale, ombra con colore letterale, raggio in px, z-index globale (6 o più) o font-size in px fuori da `desktop/src/ds/tokens/theme.css`, salvo le eccezioni elencate in `desktop/tests/style-tokens.mjs`.
- **INV-002**: input, select e textarea non ridefiniscono bordo, raggio, padding, sfondo o colore del campo base.
- **INV-003**: ogni nuovo token entra in entrambi i blocchi di tema quando il valore scuro differisce, e nella guida Doop tramite `checks/doop-sync/tokens.mjs`.

## Consequences

- Leggero cambio visivo: le ombre dei bottoni sono uniformi, alcuni campi hanno bordo più chiaro, il menu destinatari ha l'aspetto dei popover.
- Il composer torna sotto la barra delle bozze (`--z-composer` < `--z-dock`), come prima di ADR-0008.
- Il controllo va eseguito con gli altri test del frontend.

## Confirmation

- `node desktop/tests/style-tokens.mjs` passa.
- I test browser di calendario, allegati, rubrica, stampa, Google, ricerca, undo, smoke, meeting, sidebar e sync passano.
- `checks/doop-sync/extract.mjs --verify`: 0 pixel diversi sui sei frame HTML.
