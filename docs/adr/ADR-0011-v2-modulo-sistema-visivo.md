---
adr_id: ADR-0011
version: 2
title: "Il sistema visivo di Qamille è un modulo interno desktop/src/ds/ le cui primitive stanno su Ark UI"
status: Superseded
author: Alfio Caprino
scope:
  paths:
    - desktop/src/ds/**
    - desktop/src/lib/shortcut-guard.ts
    - desktop/src/styles.css
    - desktop/scripts/ds-export.mjs
    - desktop/tests/ds-usage.mjs
    - desktop/tests/style-tokens.mjs
    - desktop/package.json
  modules:
  domains:
    - design-system
governs_symbols:
depends_on:
  - ADR-0008
  - ADR-0009
created_from_task: ark-compatto-2026-11-08
proposed_on: 2026-11-08
proposed_at_sha: e9e42ce94fde6ea51f28835185d7ee20ad5560b5
accepted_on: 2026-11-08
accepted_at_sha: f9d1ee2218c182f4c18af8bd481025a784f3a2e2
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes: ADR-0011
superseded_by: ADR-0011 v3
changes:
  - INV-003
  - INV-004
  - INV-005
  - INV-006
  - INV-007
  - RA-005
  - RA-006
  - RA-007
---

# ADR-0011: Il sistema visivo di Qamille è un modulo interno desktop/src/ds/ le cui primitive stanno su Ark UI

<!-- adr-header:start -->
Version 2 · Accepted 2026-11-08 at f9d1ee2 · proposed 2026-11-08 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

La versione 1 (2026-11-07) ha messo il sistema visivo in un modulo interno, `desktop/src/ds/`, con token, stili di sistema, guida e primitive scritte a mano: `Button`, `Modal` e `SidePanel` su `<dialog>`, `Popover` con la sua chiusura, `Field` e `Icon`. I controlli che mancavano alle primitive (42 `<select>`, 23 checkbox, 6 campi data, 2 campi ora, 1 cursore, 3 `role="tablist"` a mano, 61 `title=` usati come suggerimento, secondo l'inventario del 2026-11-08 su `main`) restavano markup nativo nei componenti, ognuno con il suo aspetto e il suo comportamento da tastiera.

Il 2026-11-08 l'utente ha scelto la direzione visiva «B · Compatto» (spec `docs/specs/2026-11-08-ark-compatto-design.md`) e ha chiesto di ricostruire l'interfaccia su Ark UI (`@ark-ui/svelte`, motore Zag.js), restando su Svelte 5. Questo cambia che cosa contiene il modulo, da quali pacchetti può dipendere, quali valori dei token esistono e quale regola impedisce che i controlli nativi tornino nei componenti.

La domanda: come cambia la forma del modulo ADR-0011 quando le primitive passano su Ark, e che cosa resta fuori da Ark?

## Decision Drivers

- Una modifica a un controllo tocca un solo file (invariato dalla v1).
- Tastiera, focus, ARIA e chiusura con Esc sono quelli di Ark, non scritti a mano.
- Le dipendenze nuove sono poche, a versione fissa e verificabili: nessun kit con tema, nessun Tailwind.
- Le righe della posta restano veloci: `test:refresh`, `test:done-latency` e `test:mail-opening` non devono peggiorare (ADR-0007).
- Vale ADR-0008 (tre contenitori sovrapposti, livelli solo dalla scala) e ADR-0009 (ogni valore visivo da un token).
- Il testo resta nitido: i caratteri sono file locali, la CSP non cambia.

## Decision

Il modulo `desktop/src/ds/` resta interno, non un pacchetto, e cambia in questo modo:

1. **Le primitive stanno su Ark UI.** `primitives/` contiene `Button` (con `tip` e `shortcut` su `Tooltip`), `Tooltip`, `Modal` e `SidePanel` (Ark `Dialog`, il secondo non modale), `Popover`, `Select`, `Checkbox`, `Switch`, `Tabs` (varianti `underline`, `segment`, `plain`), `Slider`, `DateField` (Ark `DatePicker`), `RecipientsInput` (Ark `TagsInput` più `Combobox`), `Toaster` (Ark `Toast`: gli avvisi dell'app, `desktop/src/ds/primitives/Toaster.svelte`), `Field` e `Icon`. Ognuna avvolge il componente di Ark, ne espone solo le props che l'app usa e lo lascia stilare ai selettori `[data-scope][data-part]`. I componenti dell'app usano le primitive, mai `@ark-ui/svelte` direttamente.
2. **L'aspetto delle parti di Ark** sta in `styles/ark.css` (Tooltip e regole comuni) e in `styles/ark/*.css`, uno per primitiva, importati in testa a `ark.css`. Usa solo token; le regole di un contenitore sono in `:where()` perché una classe di pagina le possa ridefinire.
3. **Dipendenze ammesse** (spec §9): `@ark-ui/svelte` alla versione esatta `5.25.1` (senza `^`), con i suoi circa 68 pacchetti `@zag-js/*` fissati dal lockfile; `@floating-ui/dom`; `@internationalized/date`, dichiarata alla versione esatta `3.12.4`. Il carattere, IBM Plex Sans, è copiato come woff2 in `desktop/public/fonts/ibm-plex-sans/` con `OFL.txt`, senza dipendenza npm. Nessun'altra dipendenza di interfaccia: niente Tailwind, niente kit con tema (Skeleton, shadcn-svelte, Park UI). `package-lock.json` è aggiornato e `npm audit` non segnala vulnerabilità high o critical sui pacchetti nuovi. La CSP di `desktop/src-tauri/tauri.conf.json` non cambia.
4. **Token del Task 2.** `tokens/theme.css` porta i valori di «Compatto» nel tema chiaro e un tema scuro con la stessa struttura: carattere unico IBM Plex Sans (`--font-text` e `--font-title` puntano allo stesso carattere), scala del testo 11,5 / 12,5 / 13,5 / 14 / 15 / 18 / 20 / 26 px, accento indaco `--accent` `#4f46e5` con `--accent-hover` e `--accent-text`, barra laterale scura con `--sidebar-active`, `--sidebar-hover`, `--sidebar-text`, `--sidebar-text-active`, `--sidebar-muted` e `--accent-on-sidebar`, `--thread-open` per le righe della conversazione aperta, `--control-height` 32 px, e il livello `--z-modal` (30) tra `--z-panel` e `--z-toast`. Le parti portate fuori dal flusso (dialog, popover, suggerimenti, toast) usano la scala `--z-popover` < `--z-panel` < `--z-modal` < `--z-toast` senza valori nuovi; le liste di `Select`, `Combobox` e i suggerimenti stanno a `--z-toast` per restare usabili dentro un `Modal` e un `SidePanel`. Come si arriva in `<body>`: `Modal`, `Popover` e l'elenco dei suggerimenti di `RecipientsInput` usano l'attachment `toBody` di `desktop/src/ds/portal.ts`, non il `Portal` di Ark; la lista di `Select` e `Tooltip` usano il `Portal` di Ark, a `--z-toast`. Il calendario di `DateField` non è spostato in `<body>`: resta dentro il `Modal` o il pannello che lo contiene (trappola del focus, Esc) a `--z-popover`, con il motivo scritto nel commento di `DateField.svelte`.
5. **Esc a strati.** Un livello di Ark aperto (lista di un `Select`, suggerimenti, calendario di `DateField`, `Popover`, `Modal`, `SidePanel`) prende Esc prima di tutto il resto, perché Ark ascolta `keydown` su `document` in fase di cattura; la finestra di scrittura si riduce a icona con Esc solo se nessun livello è aperto e il focus è al suo interno. Le scorciatoie a tasto singolo di `App.svelte` (`N`, `R`, `E`, frecce, `Ctrl+Z`) ignorano i tasti premuti dentro un campo di testo o una parte di Ark che legge i tasti da sé, tramite `shortcutIgnored` in `desktop/src/lib/shortcut-guard.ts`.
6. **Eccezioni, scritte qui perché sono volute.**
   - Le righe della lista della posta restano markup Svelte semplice stilato con i token, senza Ark (spec §7.6): Ark serve ai controlli, non a ogni riga. La casella di selezione di ogni riga resta un `<input type="checkbox">` nativo e sta nella lista di eccezioni di INV-003 con il motivo «rows stay plain markup (spec 7.6)».
   - La finestra di scrittura resta una finestra agganciata in basso e non usa `Modal` (ADR-0008, EX-003 della v1).
   - L'editor (Tiptap) e l'anteprima allegati (ArtCraft/PdfCraft) non sono controlli di Ark: cambiano solo nell'aspetto. Anche i campi dell'ora restano il campo nativo stilizzato.
   - `UndoStack` resta fuori da Ark `Toast`, deciso: il suo conto alla rovescia batte ogni 50 ms e `desktop/tests/undo.mjs` conta il battito, mentre `Toast` ha i suoi timer; passarlo a `Toast` cambierebbe i tempi e la semantica dell'annullamento. Conserva la sua struttura e prende solo stile e primitive (`Button`). Gli avvisi semplici (`notify`) passano invece dal `Toaster`.

Restano come nella v1 il percorso del modulo, `ds-export` e la guida versionata (`desktop/src/ds/README.md`, riscritta per lo stato finale: token, tabella delle primitive con le props, regole di `ark.css`, un esempio per primitiva).

## Verified Interview Examples

- **EX-001** (nominal case): Input: cambia l'altezza del pulsante in `desktop/src/ds/tokens/theme.css` (`--control-height`) -> Expected: cambiano insieme pulsanti, `Select`, `DateField` e `RecipientsInput`, e nessun file in `desktop/src/components/`.
- **EX-002** (nominal case): Input: un componente ha bisogno di una lista di scelte -> Expected: usa `Select` da `desktop/src/ds/primitives/Select.svelte` e passa `items`; non scrive un `<select>` e non importa `@ark-ui/svelte`.
- **EX-003** (boundary case): Input: la riga della posta ha una casella di selezione -> Expected: è un `<input type="checkbox">` nativo, ed è l'unica voce della lista di eccezioni di `node tests/ds-usage.mjs`, con il motivo «rows stay plain markup (spec 7.6)».
- **EX-004** (boundary case): Input: Esc con la lista di un `Select` aperta dentro la finestra di scrittura -> Expected: si chiude la lista; la finestra si riduce a icona solo con un secondo Esc.
- **EX-005** (rejection or error case): Input: un nuovo `<select>`, `<input type="checkbox">`, `role="tablist"` scritto a mano o `<dialog>` in `desktop/src/components/` -> Expected: `node tests/ds-usage.mjs` fallisce indicando file, riga e primitiva da usare.
- **EX-006** (rejection or error case): Input: una dipendenza di interfaccia con `^` nella versione, o fuori dal gruppo Ark/Zag più `@floating-ui/dom` e `@internationalized/date` -> Expected: non si accetta nella revisione; è fuori da questa decisione.
- **EX-007** (nominal case): Input: `npm run ds:export` due volte senza modifiche al modulo -> Expected: file identici byte per byte.

## Rejected Alternatives

- **RA-001** (dalla v1): un pacchetto separato `packages/design-system`. Con un solo consumatore aggiunge versioni e build senza guadagno.
- **RA-002** (dalla v1): una libreria React per poter usare `/design-sync`. Sarebbe una seconda implementazione che l'app non usa. Vale anche per la migrazione a React chiesta all'inizio: non è né più sicura né più leggera di Svelte, e l'aspetto dipende dal CSS.
- **RA-003** (dalla v1): la sola cartella `desktop/src/components/ui/`. Lascerebbe token, stili, guida ed esportazione in posti diversi.
- **RA-004** (dalla v1): lasciare le classi come sono.
- **RA-005**: un kit con tema già fatto (Skeleton, shadcn-svelte, Park UI) o Tailwind. Imporrebbe un aspetto e una catena di dipendenze fuori dal gruppo Ark/Zag, e i token di ADR-0009 smetterebbero di essere l'unica fonte.
- **RA-006**: Ark in ogni riga della posta (Collapsible per le conversazioni, Checkbox per ogni riga). Aggiunge macchine a stati a centinaia di righe e mette a rischio i tempi misurati da `test:refresh`, `test:done-latency` e `test:mail-opening`.
- **RA-007**: usare `@ark-ui/svelte` direttamente nei componenti. Ogni schermata ripeterebbe `translations`, portal, livelli e stile; la regola «un controllo, un file» cadrebbe.

## Enforceable Invariants

- **INV-001**: i file in `desktop/src/ds/` non importano da `desktop/src/lib/`, da `desktop/src/components/` né da `@tauri-apps`. Possono importare da `@ark-ui/svelte`, `@internationalized/date` e `@floating-ui/dom`.
- **INV-002**: fuori da `desktop/src/ds/`, il markup non usa le classi `primary`, `secondary`, `icon-button`, `text-button`, `popover`, `side-panel-layer` né l'elemento `<dialog>`.
- **INV-003** (nuovo): fuori da `desktop/src/ds/`, il markup non usa `<select>`, `<input type="checkbox">`, `role="tablist"` scritto a mano né `<dialog>`. Fa eccezione la casella di selezione di ogni riga della lista della posta, registrata nella lista di eccezioni di `desktop/tests/ds-usage.mjs` con il motivo «rows stay plain markup (spec 7.6)». `node tests/ds-usage.mjs` lo controlla.
- **INV-004** (era INV-003 della v1): i valori dei token stanno solo in `desktop/src/ds/tokens/theme.css`; `desktop/tests/style-tokens.mjs` lo controlla.
- **INV-005** (era INV-004 della v1): `npm run ds:export` è deterministico: a parità di modulo e di dati di esempio produce gli stessi file.
- **INV-006** (era INV-005 della v1): il modulo contiene una sola implementazione di ogni componente, in Svelte.
- **INV-007** (nuovo): fuori da `desktop/src/ds/` nessun file importa da `@ark-ui/*`: i componenti dell'app passano da una primitiva. `node tests/ds-usage.mjs` lo controlla (nello script il controllo portava per errore l'etichetta INV-003, ora INV-007).

## Consequences

- La numerazione degli invarianti cambia rispetto alla v1: il nuovo INV-003 spinge i tre successivi di un posto; INV-007 si aggiunge in coda. I riferimenti a INV-003, INV-004 e INV-005 della v1 (la guida del modulo, `ds-export`, i commenti) si leggono con la numerazione nuova una volta approvata questa versione.
- Il modulo dipende da `@ark-ui/svelte` e dal suo albero di pacchetti `@zag-js/*`: ogni aggiornamento è una modifica del lockfile da rivedere, non un `npm update`.
- Un controllo nuovo dell'app o una variante di uno esistente nasce come primitiva (o come props di una esistente) con un'anteprima nell'esportazione, non come markup di pagina.
- ADR-0008 resta valido: i tre contenitori sono ora `Modal`, `SidePanel` e `Popover` su Ark, con `--z-modal` come livello del dialog centrato.
- Il design system in Claude e la guida Doop non sono risincronizzati in questo lavoro (spec §3): vanno riallineati dopo l'approvazione, in entrambi i versi.
- `UndoStack` resta con la sua struttura, fuori da Ark `Toast` (vedi Decision, punto 6).

## Confirmation

- `npm run build` passa (`check`, `test:ds`, `vite build`); `node tests/ds-usage.mjs` elenca zero residui oltre alla lista di eccezioni.
- `npm run test:browser` e i `test:*` di `desktop/package.json` non peggiorano rispetto alla baseline del Task 0.
- `node tests/style-tokens.mjs`, `node tests/tokens-compatto.mjs` e `node tests/contrast.mjs light dark` passano.
- `npm run test:ds-export` passa: due esportazioni identiche byte per byte.
- `npm ls @ark-ui/svelte` mostra `5.25.1`, `package.json` non ha `^` sui pacchetti del gruppo Ark, e `npm audit` non segnala vulnerabilità high o critical sui pacchetti nuovi.
- Gli screenshot di ogni schermata a 1440×900 e 1000×700, nei due temi, sono approvati dall'utente prima di rigenerare le immagini di riferimento di `visual-snapshot.mjs` e `visual-compare.mjs`.
