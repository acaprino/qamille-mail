---
adr_id: ADR-0011
version: 3
title: "L'interfaccia di Qamille passa a React con i componenti di unlumen UI e l'identità visiva «Compatto»; il modulo desktop/src/ds/ resta la sola casa del sistema visivo"
status: Accepted
author: Claude Code (su richiesta del proprietario del repository)
scope:
  paths:
    - desktop/src/ds/**
    - desktop/src/lib/store.ts
    - desktop/src/lib/shortcut-guard.ts
    - desktop/src/styles.css
    - desktop/vite.config.ts
    - desktop/tests/ds-usage.mjs
    - desktop/tests/style-tokens.mjs
    - desktop/tests/tokens-compatto.mjs
    - desktop/package.json
  modules:
  domains:
    - design-system
governs_symbols:
depends_on:
  - ADR-0008
  - ADR-0009
created_from_task: react-unlumen-2026-11-12
proposed_on: 2026-11-12
proposed_at_sha: 9ff8920
accepted_on: 2026-11-12
accepted_at_sha: ae0ba6d
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes: ADR-0011 v2
superseded_by: null
changes:
  - INV-001
  - INV-006
  - INV-008
  - RA-002
  - RA-005
---

# ADR-0011 v3: L'interfaccia di Qamille passa a React con i componenti di unlumen UI e l'identità «Compatto»

<!-- adr-header:start -->
Version 3 · Accepted 2026-11-12 at ae0ba6d · proposed 2026-11-12 at 9ff8920
<!-- adr-header:end -->

## Context and Problem Statement

La v2 teneva l'interfaccia su Svelte 5 con le primitive su `@ark-ui/svelte` e la direzione visiva «Compatto»,
e scartava espressamente sia la migrazione a React (RA-002) sia Tailwind e i kit con tema (RA-005).

Il 2026-11-12 l'utente ha chiesto di passare a React e a unlumen UI (ui.unlumen.com), scegliendo: la riscrittura
completa in un solo lavoro e, da unlumen, Sidebar 001/002, Motion Tabs Menu, Apple Switch con Animated Input,
Animated List e Animate Digits. Sull'aspetto, dopo una prima prova con la tavolozza di unlumen, l'utente ha chiesto di
mantenere il più possibile l'identità visiva del tema «Compatto»: unlumen porta i componenti e il movimento, non i colori. unlumen è un registro di componenti nello
stile di shadcn/ui: React, Tailwind CSS e Motion, codice copiato nel progetto, non un pacchetto.

La domanda: che cosa resta della v2 quando cambiano il framework, l'aspetto e due dei rifiuti?

## Decision Drivers

- Una modifica a un controllo tocca ancora un solo file, e tastiera, focus, ARIA ed Esc restano quelli di Ark.
- I test dell'interfaccia (ruoli, nomi accessibili, classi, testi) restano validi: il DOM cambia il meno possibile.
- Ogni valore visivo viene ancora da un token (ADR-0009); Tailwind non introduce una seconda tavolozza.
- I componenti di unlumen arrivano come codice copiato e leggibile, non come dipendenza che cambia da sola.
- Il testo resta nitido e offline: carattere locale, CSP invariata.

## Decision

1. **React 19 al posto di Svelte 5.** Ogni componente `.svelte` diventa `.tsx` con lo stesso markup (elementi,
   classi, `aria-*`, `data-*`, testi). Le regole `<style>` di un componente stanno in un `.css` accanto, annidate
   sotto la classe radice del componente. Lo stato di modulo che era `$state` (`activity-center`, `groups`,
   `i18n`) è un `reactive()` di `desktop/src/lib/store.ts`, letto con `useStore()`; la lingua si segue con
   `useLanguage()` alla radice. Niente StrictMode: gli effetti parlano col backend una volta sola.
2. **Le primitive restano su Ark UI**, ora `@ark-ui/react` (versione esatta), con lo stesso DOM `[data-scope][data-part]`,
   quindi gli stili di `styles/ark/*.css` restano validi. Le props diventano React (`value` + `onChange` al posto di
   `bind:value`, `trigger` come elemento per `Popover`, `ref` al posto di `element`). Il `Portal` di Ark sostituisce
   `toBody`: in React il contenuto del portale vive e muore con l'albero che lo contiene.
3. **unlumen UI dentro il modulo.** I componenti gratuiti scelti sono copiati in `desktop/src/ds/unlumen/` con le
   modifiche annotate nel suo README: Apple Switch è il controllo di `Switch`; Animate Digits è dentro `Count`
   (contatori che cambiano); Sidebar 001 dà a `NavList`/`NavItem` l'evidenza che segue il puntatore e la barra
   della voce corrente che scorre (il file di Sidebar 001 non è copiato); le liste usano direttamente `AnimatePresence` di Motion al posto di Animated List, per tenere il loro markup. I componenti Pro
   (Motion Tabs Menu, Sidebar 002, Animated Input) richiedono una licenza e non sono inclusi: `Tabs` riprende
   l'evidenza scorrevole delle primitive libere di unlumen (`tabs`, `highlight`) con un `layoutId` di Motion.
4. **L'identità resta «Compatto».** `tokens/theme.css` conserva i valori di «Compatto» della v2 (accento indaco
   `#4f46e5`, barra laterale scura, IBM Plex Sans, raggi 4/6/8/12 px, nessuna ombra a riposo) e aggiunge solo
   `--accent-pro` e `--ring` (l'alone dello Switch e i nomi `accent-pro` e `ring` di Tailwind), con l'indaco di Compatto. I componenti di
   unlumen sono adattati a quei token: Apple Switch ha la misura `xs` con la geometria dello switch di Compatto
   (32×18) e i suoi colori; la barra della voce corrente di `NavList` è `--accent-on-sidebar`. `styles/tailwind.css`
   mappa i colori di Tailwind (`bg-background`, `text-muted-foreground`, `bg-accent`, `bg-accent-pro`…) sugli stessi
   token, senza preflight e senza la tavolozza propria di Tailwind: `system.css` resta la base.
5. **Dipendenze ammesse**, a versione esatta: `react`, `react-dom`, `@ark-ui/react`, `motion`, `tailwindcss`,
   `@tailwindcss/vite`, `tw-animate-css`, `clsx`, `tailwind-merge`,
   `@vitejs/plugin-react`, `@internationalized/date`. Il carattere resta IBM Plex Sans in `desktop/public/fonts/ibm-plex-sans/`.
6. **Eccezioni che restano**: le righe della lista della posta sono markup semplice (ora con un'animazione di Motion
   all'entrata e all'uscita) e la loro casella resta nativa; la finestra di scrittura resta agganciata in basso;
   `UndoStack` resta fuori da Ark `Toast`.

## Rejected Alternatives

- **RA-002** (riformulato): restare su Svelte e adattare unlumen. unlumen è codice React: ogni componente andrebbe
  riscritto, e l'utente ha scelto la migrazione.
- **RA-005** (riformulato): usare unlumen e Tailwind al posto dei token. Due fonti per gli stessi valori romperebbero
  ADR-0009; Tailwind legge i token, non li sostituisce.
- **RA-008**: passare da Ark a Radix (la base di shadcn). Cambierebbe DOM, tastiera e ARIA di ogni controllo e
  invaliderebbe i test; unlumen non fornisce dialog, select o date picker propri.
- **RA-009**: una migrazione graduale con Svelte e React insieme. Scartata dall'utente.

## Enforceable Invariants

- **INV-001** (cambiato): i file in `desktop/src/ds/` non importano da `desktop/src/lib/`, da `desktop/src/components/`
  né da `@tauri-apps`; i componenti di unlumen copiati stanno in `desktop/src/ds/unlumen/` e importano solo dal modulo.
- **INV-002**, **INV-003**, **INV-007**: invariati; `node tests/ds-usage.mjs` li controlla sui `.tsx` con il parser di TypeScript.
- **INV-004**: invariato; `node tests/style-tokens.mjs` legge `.css`, `.ts` e `.tsx`, compresi i componenti copiati in `ds/unlumen/`.
- **INV-006** (cambiato): il modulo contiene una sola implementazione di ogni componente, in React.
- **INV-008** (nuovo): i componenti di unlumen copiati non portano colori propri nell'app: leggono i token di `theme.css`
  (direttamente o tramite i nomi di Tailwind). `node tests/tokens-compatto.mjs` e `node tests/contrast.mjs light dark`
  continuano a valere.

## Consequences

- I test «compatto» restano validi sui valori di «Compatto»; cambiano solo dove la struttura del DOM è diversa (il
  `Tabs` disegna la sottolineatura come evidenza scorrevole, lo `Switch` è un `<button role="switch">`). Le immagini di
  riferimento di `visual-snapshot.mjs` e `visual-compare.mjs` vanno riconfrontate per le piccole differenze di movimento.
- `npm run ds:export` e la guida Doop (`scripts/doop-sync`) sono costruiti su Svelte (classi `svelte-*`, harness Svelte):
  vanno riscritti per React prima di una nuova esportazione.
- ADR-0005, ADR-0006, ADR-0007, ADR-0008 e ADR-0010 citano file `.svelte`: valgono per i `.tsx` omonimi.

## Confirmation

- `npm run build` passa (`check` con `tsc`, `test:ds`, `vite build`).
- `node tests/release-theme-probe.mjs` e `node tests/native-feel.mjs` passano sull'exe di release.
- Gli screenshot delle schermate principali nei due temi sono approvati dall'utente.
