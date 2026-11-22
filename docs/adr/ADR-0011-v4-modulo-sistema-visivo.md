---
adr_id: ADR-0011
version: 4
title: "Qamille si basa solo su Sigrid: l'interfaccia sta in desktop/src/ui e desktop/src/theme, e nessuna libreria che Sigrid porta è una dipendenza di Qamille"
status: Proposed
author: Claude Code (su richiesta del proprietario del repository)
scope:
  paths:
    - desktop/src/ui/**
    - desktop/src/theme/**
    - desktop/src/styles.css
    - desktop/package.json
    - desktop/vite.config.ts
    - desktop/tests/ui-usage.mjs
    - desktop/tests/only-sigrid.mjs
    - desktop/tests/style-tokens.mjs
    - desktop/tests/tokens-compatto.mjs
    - desktop/scripts/sync-sigrid.mjs
    - desktop/scripts/theme-css.mjs
  modules:
  domains:
    - design-system
governs_symbols:
depends_on:
  - ADR-0008
  - ADR-0009
created_from_task: qamille-su-sigrid-2026-11-22
proposed_on: 2026-11-22
proposed_at_sha: 290f6fc
accepted_on: null
accepted_at_sha: null
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes: ADR-0011 v3
superseded_by: null
changes:
  - INV-001
  - INV-002
  - INV-003
  - INV-004
  - INV-006
  - INV-007
  - INV-008
  - INV-009
  - INV-010
---

# ADR-0011 v4: Qamille si basa solo su Sigrid

<!-- adr-header:start -->
Version 4 · Proposed 2026-11-22 at 290f6fc
<!-- adr-header:end -->

## Context and Problem Statement

La v3 teneva l'interfaccia in `desktop/src/ds/`: primitive su `@ark-ui/react`, componenti copiati da unlumen UI, animazioni
di `motion`, colori e raggi in `tokens/theme.css`. Nel frattempo il proprietario ha un design system condiviso, Sigrid
(`acaprino/sigrid`, pacchetto `@acaprino/sigrid`): un motore di temi, token, caratteri, primitive e movimento che Qamille
ed Ethel usano senza copiarli. Il 2026-11-22 il proprietario ha deciso che Qamille dipende solo da Sigrid: ciò che Sigrid
non ha si chiede a Sigrid e non si scrive in Qamille, e ogni differenza visibile che ne segue è accettata («tutto come
Sigrid»).

La domanda: che cosa resta di Qamille quando le primitive, i token, i caratteri, le icone e il movimento sono di Sigrid?

## Decision Drivers

- Una sola fonte per ogni comportamento e ogni valore: se Qamille e Sigrid dicono la stessa cosa in due posti, uno dei due
  cambierà da solo.
- Tastiera, focus, ARIA ed Esc restano quelli dei controlli di Sigrid, che porta Base UI e Ark al suo interno (scelta del
  proprietario scritta nel README di Sigrid).
- Qamille segue Sigrid senza pin: l'ultima release del suo `main`, scaricata da `npm run sigrid:sync`.
- L'aspetto di Qamille è un insieme di parametri, non codice: «Compatto» è una definizione di tema.

## Decision

1. **Sigrid è la sola fonte.** Primitive, token, caratteri, icone e movimento vengono da Sigrid, preso con
   `npm run sigrid:sync` (l'ultima GitHub Release del `main`, senza pin; `SIGRID_DIR` solo per un checkout locale).
   Qamille non ha dipendenze dirette su librerie che Sigrid porta: `@ark-ui/*`, `motion`, `@internationalized/date`, `clsx`,
   `tailwind-merge`, `tw-animate-css`, `lucide-react` e le loro parenti sono vietate (`FORBIDDEN` di
   `desktop/tests/only-sigrid.mjs`), né in `package.json` né nel lockfile. `tailwindcss` e `@tailwindcss/vite` restano come
   strumenti di build.
2. **`desktop/src/ui/` è il solo consumatore di Sigrid** (insieme a `desktop/src/theme/`). Ogni primitiva è un
   adattatore: dà a un componente di Sigrid il nome, le props e le parole che l'app usa da sempre. I file dell'app importano
   `ui/<Nome>`; nessuno importa `@acaprino/sigrid`.
3. **L'aspetto sono parametri.** `src/theme/compatto.ts` definisce «Compatto» (`COMPATTO`, con `radii` 4/6/8/12; `COMPATTO_TONE`;
   `QAMILLE_DENSITY`). `npm run theme:css` lo trasforma in `compatto.css` con il motore di Sigrid. I nomi dei token sono
   quelli di Sigrid (`--color-paper`, `--color-ink`, `--color-primary`…): Qamille non ha alias. I soli token di Qamille
   stanno in `src/theme/qamille-tokens.css` (priorità, etichette, avatar, scrim, livelli `--z-*`) e non ridefiniscono nulla
   che Sigrid possiede.
4. **I titoli seguono la scala di Sigrid**: 14, 16, 18, 20 e 24 px (`sm`, `base`, `lg`, `xl`, `2xl`). I titoli di 15 e 26 px
   di prima diventano 16 e 24.
5. **Le icone sono di Sigrid** (`ICON_MAP` in `src/ui/icon-names.ts` porta i nomi di Qamille sui suoi significati), nel
   pacchetto che l'utente sceglie; i loro tratti sono quelli di Sigrid.
6. **La base resta di Qamille**: `src/theme/base.css` (regole degli elementi), `tailwind.css` (l'ingresso di Tailwind con il
   tema di Sigrid) e `forced-colors.css` (i temi a contrasto di Windows, importato per ultimo).

## Rejected Alternatives

- **RA-010**: tenere `src/ds/` accanto a Sigrid, con le primitive proprie dove Sigrid ne ha di diverse. Due implementazioni
  dello stesso controllo divergono; il proprietario ha scelto una sola fonte.
- **RA-011**: fissare una versione di Sigrid in `package.json`. Un pin è una dipendenza che invecchia; la regola è l'ultima
  release del `main`.
- **RA-012**: copiare in Qamille ciò che Sigrid non ha. Si chiede a Sigrid (è così che sono arrivati `radii` e 48 icone in
  Sigrid 3.2.1).

## Enforceable Invariants

- **INV-001**: `desktop/src/ui/` non importa da `components/`, da `App`, `main`, `reminder` né da `@tauri-apps`.
- **INV-002**, **INV-003**: fuori da `src/ui/`, il markup non usa le classi `primary`, `secondary`, `icon-button`, `text-button`,
  `popover`, `side-panel`, `modal`, né `<dialog>`, `<select>`, `<input type="checkbox">` o `role="tablist"` scritti a mano.
  Eccezione: la casella di selezione delle righe della posta (vedi sotto). `node tests/ui-usage.mjs`.
- **INV-004**: ogni colore, livello, raggio e dimensione del testo è un token di Sigrid o di `qamille-tokens.css`, mai un
  letterale. `node tests/style-tokens.mjs`.
- **INV-006**: ogni controllo ha una sola implementazione, quella di Sigrid dietro il suo adattatore.
- **INV-007**: nessun file di Qamille importa un pacchetto di `FORBIDDEN`, e nessuno di quei pacchetti è in `package.json`.
  `node --test tests/only-sigrid.mjs`.
- **INV-009**: solo `src/ui/` e `src/theme/` importano `@acaprino/sigrid`. `node tests/ui-usage.mjs`.
- **INV-010**: il codice dell'app riconosce i controlli da ruoli e `data-slot`, mai dalle parti o dagli stati di una libreria.
- **INV-008** cade: non ci sono più componenti copiati.

## Consequences

- Cambia a vista: i titoli (24 al posto di 26 px, 16 al posto di 15), i tratti delle icone, i bordi a riposo dei campi, le
  forme dei controlli di Sigrid. Le differenze sono accettate dal proprietario; le immagini di riferimento di
  `visual-snapshot.mjs` e `visual-compare.mjs` vanno rifatte.
- **Aperto: il movimento.** Finora `src/ui/motion.tsx` disegna ferme le animazioni di Qamille (entrata e uscita delle righe,
  pila degli «Annulla», bozze, riunioni): sono un sostituto provvisorio, in attesa che ogni posto usi `ListItem`, `Presence` e
  `Leaving` di Sigrid (`useMotion()`). Finché non è fatto, le eccezioni della v3 restano: le righe della lista sono markup
  semplice con la casella nativa, e `UndoStack` sta fuori da Toast.
- **Aperto: le schede delle bozze.** `DraftTabs` è un tablist di Qamille perché Sigrid non ha schede con un pulsante accanto
  (richiesta a Sigrid).
- Gli strumenti `ds:export`, `scripts/ds-export*` e `scripts/doop-sync/` sono ritirati: erano costruiti su Svelte e non
  funzionavano più.
- ADR-0005, ADR-0006, ADR-0007, ADR-0008 e ADR-0010 citano file `.svelte` o `ds/`: valgono per i `.tsx` omonimi e per `src/ui/`.

## Confirmation

- La CI passa: `npm run check`, `node --test tests/only-sigrid.mjs`, `npm run test:ui` e `test:browser -- onboarding-professional`.
- L'insieme dei test del browser (`npm run test:browser`) e `visual-compare` sono rifatti prima del rilascio, con i tests
  tenuti alle strutture di Sigrid (`data-slot` e ruoli) dove cercavano gli attributi di Ark.
- `node tests/release-theme-probe.mjs` e `node tests/native-feel.mjs` passano sull'exe di release.
- Gli screenshot delle schermate principali nei due temi sono approvati dall'utente.
