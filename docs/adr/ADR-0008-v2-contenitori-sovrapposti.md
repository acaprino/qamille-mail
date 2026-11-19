---
adr_id: ADR-0008
version: 2
title: "Tre contenitori sovrapposti: pannello laterale, dialog centrato, popover"
status: Accepted
author: Alfio Caprino
scope:
  paths:
    - desktop/src/ds/tokens/theme.css
    - desktop/src/ds/styles/system.css
    - desktop/src/ds/primitives/Modal.svelte
    - desktop/src/ds/primitives/SidePanel.svelte
    - desktop/src/ds/primitives/Popover.svelte
    - desktop/src/components/AttachmentPanel.svelte
    - desktop/src/components/Calendar.svelte
    - desktop/src/components/UnsavedDialog.svelte
    - desktop/src/components/GoogleSignInDialog.svelte
    - desktop/src/components/GoogleSetupGuide.svelte
    - desktop/src/components/PrintPreview.svelte
    - desktop/src/components/Contacts.svelte
    - desktop/src/components/contacts/ContactForm.svelte
    - desktop/src/components/contacts/MergeDialog.svelte
  modules:
    - desktop/src/components
  domains:
    - ui-overlays
    - design-system
governs_symbols:
depends_on:
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
  - RA-003
---

# ADR-0008: Tre contenitori sovrapposti: pannello laterale, dialog centrato, popover

<!-- adr-header:start -->
Version 2 · Accepted 2026-11-07 at f780db0 · proposed 2026-11-07 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

L'app apre contenuti sopra la vista corrente in sei modi diversi
(`desktop/src`, prima del 2026-11-04): un pannello laterale con velo per gli
allegati (`AttachmentPanel.svelte`), un foglio flottante senza velo per il
dettaglio evento del calendario (`Calendar.svelte` `.event-sheet`), sette dialog
modali centrati ciascuno con velo e larghezza propri (`#0f183055`, `#1d253680`,
`#16223877`), la finestra del composer con raggio fuori scala, due popover e i
riquadri in linea. Gli z-index sono sparsi (8, 10, 12, 20, 35, 50, 100), le
animazioni diverse (160 ms, 220 ms, nessuna). L'utente percepisce l'incoerenza:
"a volte appaiono popup laterali, a volte pannelli drawer".

## Decision Drivers

- Una sola risposta visiva per lo stesso tipo di compito, in tutte le funzioni.
- I valori condivisi (velo, larghezze, z-index, durata) vivono in `ds/tokens/theme.css`,
  così il Doop sync li esporta nella guida di stile.
- Nessun cambio di comportamento: stesse azioni, stessi tasti, stesso contesto.

## Decision

Tre contenitori, uno stile ciascuno, con i token in `desktop/src/ds/tokens/theme.css`:

1. **Pannello laterale** (`.side-panel`): "guardo o modifico una cosa senza
   perdere il contesto". Scorre da destra sopra la vista, velo leggero
   (`--scrim-soft`), non modale, Esc e bottone chiudono. Due larghezze:
   `--panel-narrow` (420 px) per il dettaglio/modifica evento del calendario,
   `--panel-wide` (760 px) per l'anteprima allegati.
2. **Dialog centrato** (`.modal`): "devo decidere prima di continuare".
   `<dialog>` modale con un solo velo (`--scrim`), bordo `--border`, raggio
   `--radius-xl`, ombra `--shadow`. Tre larghezze: `--modal-sm` (420 px:
   modifiche non salvate, Google sign-in, elimina contatto), `--modal-md`
   (560 px: scheda contatto, unisci, guida Google) e `--modal-lg` (960 px:
   anteprima stampa).
3. **Popover** (`.popover`): menu o filtri ancorati a un bottone, stesso
   bordo, raggio `--radius-lg` e ombra.

Il composer resta una finestra ancorata in basso a destra ma prende raggio,
bordo e ombra dai token. I riquadri in linea (dettaglio riunione, dettaglio
contatto, duplicati) non cambiano. Gli z-index usano la scala
`--z-popover` < `--z-panel` < `--z-modal` < `--z-toast`; le entrate durano
`--motion-panel` (180 ms) e rispettano `prefers-reduced-motion`.

## Verified Interview Examples

- **EX-001** (nominal case): Input: clic su un evento del calendario -> Expected: il dettaglio scorre da destra come un pannello laterale stretto, con lo stesso velo e la stessa animazione dell'anteprima allegati.
- **EX-002** (boundary case): Input: apertura della scheda contatto, poi del dialog "modifiche non salvate" -> Expected: entrambi sono dialog centrati con lo stesso velo, lo stesso bordo e lo stesso raggio; cambia solo la larghezza.
- **EX-003** (rejection or error case): Input: finestra più stretta della larghezza del pannello -> Expected: il pannello occupa tutta la larghezza, mai un overflow orizzontale.

## Rejected Alternatives

- **RA-001**: allegati come foglio flottante, come il calendario. L'utente ha confermato il pannello laterale, già scelto in ADR-0006.
- **RA-002**: un solo contenitore per tutto. I dialog di decisione devono bloccare, il pannello laterale no.
- **RA-003**: lasciare i valori nei componenti. Il Doop sync esporta solo `ds/tokens/theme.css`.

## Enforceable Invariants

- **INV-001**: ogni overlay usa una delle tre classi (`.side-panel`, `.modal`, `.popover`) e nessun colore di velo, larghezza o z-index letterale fuori da `ds/tokens/theme.css`.
- **INV-002**: il pannello laterale non è modale (`aria-modal="false"` o `dialog.show()`), il dialog centrato lo è (`showModal()`).
- **INV-003**: un contenuto che si guarda o si modifica in contesto apre un pannello laterale; una scelta che blocca il flusso apre un dialog centrato.

## Consequences

- Il dettaglio evento del calendario cambia posizione: da foglio ancorato in alto a destra a pannello a tutta altezza.
- Lo stile `.event-editor` residuo viene rimosso.
- I frame Doop del calendario, del lettore con allegato e dei dialog vanno ricatturati; la guida di stile riceve i nuovi token.

## Confirmation

- `desktop/tests/smoke.mjs` e `desktop/tests/attachment-panel.mjs` continuano a passare.
- Ricerca testuale: nessun `::backdrop` o `position: fixed/absolute` con valori letterali di velo, larghezza o z-index nei componenti overlay.

## Note 2026-11-08

Con ADR-0011 v2 i tre contenitori stanno su Ark UI (`Dialog`, `Popover`): INV-001 e INV-002 non si leggono più come le classi `.side-panel`, `.modal`, `.popover` né come `showModal()`, ma come i marcatori `data-scope` e `data-variant` sulle parti di Ark `Dialog` (`data-variant="modal"` per il dialog centrato, `data-variant="side"` per il pannello non modale, `aria-modal` di conseguenza) e sul `Popover`. Il testo sopra resta come registrato.
