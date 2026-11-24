# L'interfaccia di Qamille: `src/ui` e `src/theme`

**Regola.** Qamille si basa solo su Sigrid (`acaprino/sigrid`). Ogni comportamento, primitiva, icona, carattere e
animazione viene da Sigrid, preso con `npm run sigrid:sync`, senza pin. Ciò che manca si chiede a Sigrid e non si scrive
qui. Dentro Sigrid ci sono Base UI e Ark, per scelta del proprietario scritta nel suo README: Qamille non ha dipendenze
dirette su nessuna libreria che Sigrid porta (`desktop/tests/only-sigrid.mjs`, ADR-0011 v4).

## Che cosa sta dove

- `src/ui/*.tsx` — gli adattatori: ognuno prende un componente di Sigrid e gli dà il nome, le props e le parole che
  l'app usa da sempre (`Button`, `Field`, `Select`, `Modal`, `SidePanel`, `Menu`, `Popover`, `Tooltip`, `Tabs`, `Switch`,
  `Checkbox`, `DateField`, `RecipientsInput`, `Toaster`, `NavList`, `Count`, `Icon`). Solo qui e in `src/theme` si importa
  `@acaprino/sigrid` (INV-009). I file dell'app importano `ui/<Nome>`.
- `src/ui/words.ts` — le parole che i componenti di Sigrid dicono da soli, nelle due lingue dell'interfaccia. Ogni lingua è
  un letterale completo di `Words`: una chiave che Sigrid aggiunge fa fallire `tsc` finché non è scritta in tutte e due.
- `src/ui/icon-names.ts` — i nomi delle icone di Qamille sui significati di Sigrid (`ICON_MAP`). Il disegno è di Sigrid.
- `src/ui/DraftTabs.tsx` — le schede delle bozze del compositore: un tablist suo, perché Sigrid non ha schede con un pulsante
  accanto (richiesta R17 a Sigrid).
- `src/ui/motion.tsx` — **provvisorio**: finché ogni posto non usa `ListItem`, `Presence` e `Leaving` di Sigrid, le
  animazioni di Qamille sono disegnate ferme.
- `src/ui/parts.css` — gli aiuti per ciò che sta dentro un dialogo o un pannello.
- `src/theme/compatto.ts` — il look di Qamille come parametri di Sigrid (`COMPATTO_PICK`, `COMPATTO_TONE`, `QAMILLE_DENSITY`).
  `compatto.css` è generato da `npm run theme:css` e `tests/tokens-compatto.mjs` lo tiene uguale all'uscita del motore.
- `src/theme/qamille-tokens.css` — i soli token di Qamille (priorità, etichette, avatar, scrim, livelli `--z-*`).
  Non ridefinisce nulla di ciò che Sigrid possiede: colori, raggi, scala del testo, densità.
- `src/theme/base.css`, `tailwind.css`, `forced-colors.css` — la base degli elementi, l'ingresso di Tailwind con il tema di
  Sigrid, e le regole dei temi a contrasto di Windows (importate per ultime).

## Regole che i test fanno rispettare

- Fuori da `src/ui`, il markup passa dalle primitive: niente `<select>`, `<input type="checkbox">`, `role="tablist"` o
  `<dialog>` scritti a mano, né le classi `primary`, `secondary`, `icon-button`, `text-button`, `popover`, `side-panel`,
  `modal` (INV-002, INV-003: `tests/ui-usage.mjs`).
- `src/ui` non importa da `components/`, né da `App`, `main`, `reminder`, né da `@tauri-apps` (INV-001).
- I colori, i livelli, i raggi e le dimensioni del testo vengono dai token di Sigrid o da `qamille-tokens.css`, mai da
  letterali (`tests/style-tokens.mjs`).
- Il codice dell'app riconosce un controllo dal ruolo o da `data-slot`, mai dalle parti o dagli stati di una libreria.
