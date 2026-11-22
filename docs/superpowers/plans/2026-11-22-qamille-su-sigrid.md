# Qamille su Sigrid (sotto-progetto 4): piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Qamille si basa solo su Sigrid (`acaprino/sigrid`, `@acaprino/sigrid`) per colori, densità, carattere, parole, primitive, icone e movimento, e tiene come parametri i colori «Compatto», la densità compatta e i raggi. A vista cambia solo nelle «Differenze accettate» della spec e in quelle che il proprietario accetta dall'elenco «Che cosa cambierà a vista». Alla fine:
- `desktop/src/ds/` non c'è più;
- `desktop/package.json` non ha più nessuna libreria che Sigrid già porta: `@ark-ui/react`, `@base-ui/react`, `motion`, le icone, `clsx`, `class-variance-authority`, `tailwind-merge`, `@internationalized/date`, `tw-animate-css`;
- restano in Qamille solo i suoi componenti di prodotto, costruiti sulle sole esportazioni di Sigrid, e i suoi token propri.

**La regola del proprietario (2026-11-22):** «Qamille non deve avere nulla di questo, deve basarsi solo su Sigrid».
- Ogni comportamento, primitiva, icona e animazione viene da Sigrid. Ciò che a Sigrid manca si chiede a Sigrid («Che cosa chiedere a Sigrid»), e non si scrive mai in Qamille.
- Le librerie che Sigrid tiene dentro il suo bundle vanno bene, purché Qamille non ne abbia di proprie. Per questo la richiesta P1 delle versioni precedenti del piano (Ark e Motion come peer dependency) è ritirata.
- La regola vince sul piano e sulla spec dove sono diversi. La spec è aggiornata in «Decisioni del proprietario».

**Architecture:**
- **Il canale.** `npm run sigrid:sync` (già scritto) mette l'ultima release del `main` di Sigrid in `desktop/vendor/sigrid/`, git-ignorata. Qamille la raggiunge con un alias di Vite e un `paths` di TypeScript, senza una voce in `package.json`: nessun pin, e il lockfile non cambia a ogni release di Sigrid. Il sync entra in `predev` e `prebuild`, come `artcraft:sync`, e nella CI.
- **Il tema.** Il tema «Compatto» è una `ThemeDef` di Qamille (`desktop/src/theme/compatto.ts`), accompagnata da una `DensityDef`. Uno script fa calcolare al motore di Sigrid i token dei due schemi e li scrive in `desktop/src/theme/compatto.css`, file generato e versionato.
  - Il primo disegno resta CSS puro: `theme-boot.js` scrive solo `data-scheme`, e i blocchi di `compatto.css` sono già nel foglio.
  - Il carattere è quello di Sigrid (`fonts.css`).
  - Un test confronta il file con il motore della copia sincronizzata. Una release di Sigrid che cambia un colore di Qamille ferma la build finché qualcuno non rigenera il file e guarda la differenza.
- **Gli alias.** Durante la migrazione i nomi di Qamille (`--ground`, `--text`, `--muted`…) sono alias dei token di Sigrid. Alla fine il codice usa i nomi di Sigrid, e in un file di Qamille restano solo i token che Sigrid non definisce.
- **Il modulo dell'app.** `desktop/src/ui/` è il «pacchetto di prodotto» di Qamille su Sigrid, e con `src/theme/` è il solo posto che importa `@acaprino/sigrid`. Contiene:
  - un adattatore sottile per ogni primitiva, che tiene le props di oggi e le traduce in quelle di Sigrid;
  - i componenti di prodotto: `RecipientsInput` su `ChipsInput`, `NavList` su `NavList`/`NavItem`, `Icon` su `Icon`;
  - le parole (`words.ts`) e la radice (`SigridRoot`).

  Il movimento dell'app passa da `useMotion()` di Sigrid. Nessun file di Qamille importa più `motion`, `@ark-ui/*` o `@base-ui/*`, e lo controlla `tests/only-sigrid.mjs`.
- **Le primitive passano una alla volta, con un'eccezione: le superfici che si aprono passano tutte insieme.**
  - Durante il passaggio `ds/primitives/<X>.tsx` diventa una riga che riesporta `ui/<X>`, quindi i punti d'uso non cambiano.
  - Sigrid tiene per scelta del proprietario due librerie: Base UI sotto dialoghi, schede laterali, select, popover, menu e suggerimenti; Ark sotto avvisi, calendario e campi con suggerimenti.
  - Una sovrapposizione di Ark di Qamille dentro una di Base UI di Sigrid, o il contrario, si chiude da sola o resta irraggiungibile. Per questo avvisi, calendario, select, popover, menu, suggerimenti, dialoghi, schede laterali e destinatari si sviluppano in parallelo, ma entrano nel ramo con un solo merge (ondata C).

**Tech Stack:** Sigrid 3.0.0 alla data del piano (ultima release, commit `2a567ad`; il piano non la fissa), React 19.3.1, Tailwind 4 con `@tailwindcss/vite` (solo come strumento di build, finché vale R12), Vite 8, TypeScript 5.9. In uscita: @ark-ui/react 5.39.3, motion 14.7.1, @internationalized/date 3.12.4, clsx 2.1.1, tailwind-merge 3.7.0, tw-animate-css 1.4.0. Test: `node --test`, Playwright con Edge (`npm run test:browser -- <nomi>`), `tests/visual-snapshot.mjs` e `tests/visual-compare.mjs`, `node tests/native-feel.mjs`, `tests/release-theme-probe.mjs` sull'exe di release.

**Spec:** `docs/superpowers/specs/2026-11-20-toolkit-qamille-design.md`, intera, con la decisione del proprietario del 2026-11-22 in «Decisioni del proprietario». Sigrid: `desktop/vendor/sigrid/README.md` («The four layers»), `dist/*.d.ts`, `src/`. Qamille: `desktop/src/ds/README.md`, `docs/adr/ADR-0011-v3-modulo-sistema-visivo.md`, `docs/aspetto-nativo.md`, `AGENTS.md`.

## Prima di cominciare: Sigrid 3.0.0

Le misure sono state prese il 2026-11-22 sulla copia in `desktop/vendor/sigrid`, su v2.0.1, v2.1.0 e v3.0.0. Qui vale la 3.0.0.

**Che cosa porta la 3.0.0 che conta per Qamille:**
- **Base UI resta.** È una decisione del proprietario del 2026-11-22 (README, «The four layers»).
  - Base UI porta i controlli di base e le superfici che si aprono: dialog, sheet, select, popover, menu, tooltip, checkbox, radio, toggle group, input, più `useRender` sotto `Button` e `Badge`.
  - Ark porta avvisi, date picker, tags input, file upload, combobox, slider, switch, tabs, avatar.
  - Le sovrapposizioni di Ark di Sigrid (la scheda del calendario, la lista del `Combobox`, i suggerimenti di `ChipsInput`) non vanno in un portale: stanno dentro la superficie di Base UI che le contiene.
- **Il disegno del proprietario applicato ai controlli:**
  - interruttore, casella, radio e slider accesi nell'accento;
  - il bordo di campi, select e pulsanti con contorno è la linea chiara, un gradino più scuro sotto il puntatore;
  - l'anello del focus è sempre staccato di 2 px;
  - la riga sotto il puntatore prende la tinta della selezione al 60 %, e i titoli dei gruppi sono maiuscoletti di 12 px;
  - `NavItem` ha una barra corta nell'accento sulla voce attiva, e il conto è una pillola;
  - l'errore di un campo è un riquadro su fondo rosso tenue;
  - il pulsante distruttivo è rosso pieno;
  - con `MotionProvider` acceso i colori dei controlli sfumano in 100 ms;
  - niente più `cursor-pointer` né `cursor-not-allowed`.
- **I campi.** `ChipsInput` ha `display`, `invalid` e `separators`. `DateField` non ha più il pulsante che toglie la data.
- **Il movimento.** `useMotion()` dà il modulo del movimento di Sigrid (`MotionImpl`) quando le animazioni sono accese e il chunk è arrivato. Contiene:
  - `m`, i componenti animati;
  - `Presence`, `ListItem`, `Leaving`, `Highlight`, `Turn`, `HoverHighlight`, `pressable`;
  - le durate e le curve di Fluent (`MOTION`, `DURATION_MS`).

  Con le animazioni spente, o con la riduzione delle animazioni, dà `null`, e i componenti si disegnano fermi.
- **Le icone.** `Icon` disegna i 33 significati di `ICON_NAMES`. Un'app aggiunge i suoi con `IconPackProvider icons`, ma deve passare un disegno di lucide (`IconDrawing.lucide`), cioè importare `lucide-react`, oppure disegnarlo da sé. Con la regola del proprietario, le icone di Qamille che non stanno in `ICON_NAMES` vanno chieste a Sigrid (P5).
- **Il motore dà gli stessi valori di «Compatto»** della 2.0.1: provato sulla 3.0.0 con i parametri del Task 2.

| # | Che cosa serve a Qamille | Stato in v3.0.0 | Che cosa aspetta |
|---|---|---|---|
| **P1** | Ritirata (regola del proprietario, 2026-11-22). Ark, Base UI e Motion dentro il bundle di Sigrid vanno bene, purché Qamille non ne abbia di propri. | — | Niente. La copia unica di Zag e di Motion la dà la rimozione delle dipendenze di Qamille (Task 20), non Sigrid. |
| **P2** | Base UI e Ark convivono dentro Sigrid senza difetti. | Da provare. Il disegno c'è (nessun portale per i popup di Ark), ma nessun test di Sigrid arriva nel pacchetto. Il Task 0 scrive la prova (`tests/sigrid-coexistence.mjs`) e la lancia. | L'ondata C. |
| **P3** | Raggi come parametro della definizione del tema. | Aperto: `shapeOf` dà 4/6/10/10, e `ThemeDef` non ha campi di forma. | Task 20. Fino ad allora i raggi 4/6/8/12 restano token di Qamille: due fonti, ammesse solo durante la migrazione. |
| **P4** | Gradini di titolo fino ad almeno 26 px. | Aperto: la scala si ferma a `base` (16 px). | Task 20. Fino ad allora i titoli da 15 a 26 px restano token di Qamille. |
| **P5** | Sigrid disegna i significati delle icone di Qamille. | Aperto: `ICON_NAMES` ne ha 33, generici, mentre Qamille ne usa più di 40 (posta, posta in arrivo, rispondi, archivia, invia, allega, grassetto, corsivo, sottolineato, elenchi, citazione, annulla, ripeti, link, copia, fissa, aggiorna, cerca, chiudi, spunta, frecce, più, meno, informazione, avviso, orologio, cronologia, modifica, conversazioni, microfono, stampa, scarica, carica, calendario, contatti, i comandi della finestra, la barra laterale…). | Task 19. Fino ad allora `Icon` resta di Qamille. |

**Come si procede:**
- **I controlli.** Il Task 0 scrive tre test:
  - `tests/sigrid-preconditions.mjs`: P3, P4 e P5, statici;
  - `tests/sigrid-coexistence.mjs`: P2, in browser;
  - `tests/only-sigrid.mjs`: la regola del proprietario.

  Ogni task che dipende da un prerequisito lancia il suo controllo allo Step 0, e non parte se fallisce.
- **Le ondate A e B** non dipendono da nessun prerequisito.
- **L'ondata C** aspetta P2. Se la prova fallisce, è il caso che il README di Sigrid nomina («a defect from the two living together»): il controller si ferma e la richiesta va a Sigrid.

## Che cosa chiedere a Sigrid

Questa sezione si può passare così com'è a chi lavora su `acaprino/sigrid`. Ogni richiesta dice tre cose: che cosa cambiare, perché serve a Qamille, e come lo verifica Qamille sulla copia sincronizzata. Basta che arrivi in una release del `main`: Qamille la prende con `sigrid:sync`.

Qamille, per decisione del proprietario, non ha librerie proprie di comportamento, icone o movimento. Ciò che manca qui non lo può aggiungere da sé.

**P2. Base UI e Ark convivono dentro Sigrid, con un contratto scritto e un test per ogni coppia.**
- **Che cosa cambiare:**
  - Il README dice che nessuna sovrapposizione di Ark va in un portale, e che ognuna funziona dentro ogni superficie di Base UI (`DialogRoot`/`Dialog`, `Sheet`, `Popover`). È quello che `date-field.tsx`, `chips.tsx` e `combobox.tsx` fanno già.
  - Un test di Sigrid per ogni coppia: `DateField`, `DateTimeField`, `Combobox` e `ChipsInput` con suggerimenti, dentro `Dialog`, `Sheet` e `Popover`. In ogni coppia:
    - scegliere un giorno o una voce lascia aperta la superficie esterna;
    - Esc chiude solo la sovrapposizione più interna, e il secondo Esc la superficie;
    - il focus resta nella superficie, e Tab non ne esce.
  - Con un `Dialog` modale aperto, un clic su un avviso di `Toasts` (l'azione o la «×») funziona e non chiude il dialogo (`ignoreToasts`).
  - Il `Dialog` modale lascia raggiungibili le sovrapposizioni di Ark che contiene: non le rende `inert`, non le nasconde ai lettori di schermo e non le tratta come «fuori».
- **Perché:**
  - Qamille porta tutte le sue sovrapposizioni su quelle di Base UI in un'ondata sola, e vi mette dentro i campi di Sigrid che stanno su Ark: un calendario in un dialogo d'evento, i destinatari in un dialogo, una data nel pannello di ricerca.
  - Se una coppia non regge, il calendario chiude il dialogo o Esc butta via un modulo. È il difetto che, per il README di Sigrid, giustifica lo spostamento di un controllo.
- **Come lo verifica Qamille:** `tests/sigrid-coexistence.mjs` monta nella galleria le sole primitive di Sigrid, nelle stesse coppie, e prova le stesse cose.

**P3. I raggi come parametro della definizione del tema.**
- **Che cosa cambiare:** `ThemeDef` prende un campo facoltativo per i raggi. Il nome proposto è `radii`, con `{ sm, md, lg, xl }`, più `button` e `chip` se restano a parte. `themeTokens(...).shape`, `themeVariables` e `applyTheme` lo scrivono in `--radius-*`. Senza il campo tutto resta com'è: 4/6/10/10.
- **Perché:** i raggi 4/6/8/12 di «Compatto» sono un parametro di Qamille per decisione del proprietario. Senza il campo, Qamille scrive `--radius-*` da sé sopra quelli di Sigrid: due fonti per lo stesso token, che la spec vieta alla fine della migrazione.
- **Come lo verifica Qamille:** con `themeTokens({ ...COMPATTO, radii: { sm: 4, md: 6, lg: 8, xl: 12 } }, 'light').shape`, `--radius-lg` vale `8px` e `--radius-xl` vale `12px`. Se Sigrid sceglie un altro nome, si cambia solo quella riga del controllo.

**P4. Gradini di titolo sopra `base`.**
- **Che cosa cambiare:** la scala di `src/styles/theme.css` ha gradini sopra `base` fino ad almeno 26 px (1.625rem), ciascuno con la sua interlinea. Sono gradini di Sigrid, con valori suoi. La regola di `DESIGN.md` che oggi ammette solo `2xs`, `xs`, `sm` e `base` li ammette per i titoli.
- **Perché:** i titoli di Qamille sono di 15, 18, 20 e 26 px, e la spec li vuole come gradini della scala di Sigrid. Senza, restano token di Qamille.
- **Come lo verifica Qamille:** `theme.css` definisce un `--text-*` di almeno 1.625rem, con la sua interlinea.

**P5. Le icone che servono a un'app di posta, calendario e rubrica.**
- **Che cosa cambiare:**
  - `ICON_NAMES` prende i significati generici che Qamille usa, disegnati in ogni pacchetto (lucide, duotone, solar, fluent, emoji, twemoji): `mail`, `inbox`, `reply`, `replyAll`, `forward`, `archive`, `send`, `attach`, `bold`, `italic`, `underline`, `list`, `listOrdered`, `quote`, `undo`, `redo`, `link`, `copy`, `pin`, `refresh`, `search`, `close`, `check`, `chevron`, `arrow`, `plus`, `minus`, `info`, `alert`, `clock`, `history`, `edit`, `threads`, `mic`, `stop`, `print`, `download`, `upload`, `calendar`, `contacts`, `spark`, `windowMinimize`, `windowMaximize`, `windowRestore`, `sidebarOpen`, `sidebarClose`.
  - L'elenco esatto lo dà il Task 19, che lo legge dal codice di Qamille. Sono significati di qualsiasi app, non nomi di Qamille.
  - Un'alternativa sarebbe che `IconDrawing` accettasse un disegno SVG anche per la linea (`lucide`), come fa già per gli altri pacchetti, così un'app passerebbe i suoi tracciati senza importare `lucide-react`. Per la regola del proprietario, però, le icone vengono da Sigrid: la richiesta è la prima strada.
- **Perché:** oggi Qamille disegna da sé più di 40 icone (`ds/primitives/Icon.tsx`). Con la regola non può tenerle, e non può importare `lucide-react` per darle a `IconPackProvider`.
- **Come lo verifica Qamille** (`sigrid-preconditions.mjs`, P5): ogni nome di `src/ui/icon-names.ts` (Task 19) sta in `ICON_NAMES` della copia sincronizzata.

**Consigliate.** Non bloccano le ondate, ma ognuna toglie una riga da «da decidere»; R12 permette anche di togliere Tailwind da Qamille.
- **R5. Il bordo a riposo di campi, select e pulsanti con contorno almeno a 3:1 (WCAG 1.4.11), almeno in scuro, o un parametro per sceglierlo.**
  - In scuro la linea della 3.0.0 (`#2a3240` sul foglio `#1a202c`) fa circa 1,2:1. Il bordo di oggi di Qamille (`#6b7488`) fa circa 3,4:1, e `tests/contrast.mjs` lo esige.
  - È il punto che la review nativa aveva segnalato, e che la spec aveva accettato di correggere.
- **R6. `DateField` e `DateTimeField` che accettano una data scritta,** nel formato della lingua e in ISO, come il campo di oggi di Qamille.
- **R7. `ChipsInput` con `disabled` e con un `ref` sul campo di testo.**
- **R8. `Toasts` con una durata per avviso e un annuncio immediato (`alert`) per gli errori.**
- **R9. La riga scelta, e quella sotto il puntatore, visibili in scuro.**
  - Con «Compatto» la riga scelta (`--color-selected` `#1e1f34`) sul foglio (`#1a202c`) fa circa 1,01:1, cioè non si distingue. La riga sotto il puntatore (`bg-selected/60`) ancora meno.
  - Oggi la riga scelta di Qamille (`#262b4a`) fa circa 1,19:1, ed è già poca.
  - Si chiede che il motore tenga una distanza minima di luminosità fra `selected` e il fondo su cui la riga sta, in tutti e due gli schemi.
- **R10. Il pulsante che chiude un avviso col nome dal dizionario.** `Toasts.tsx` ha `aria-label={`Close the notice: ${item.text}`}`, scritto in inglese nel codice: in un'interfaccia italiana il lettore di schermo dice «Close the notice». Serve una parola di `Words` che prende il testo (`common.closeNotice: (text) => …`).
- **R11. I suggerimenti di `ChipsInput` senza il nome ripetuto.** Con `display`, un suggerimento mostra il nome e poi il valore intero in grigio: se il valore è «Ann Rossi <ann@x.it>», si legge «Ann Rossi Ann Rossi <ann@x.it>». Serve una funzione per la seconda parte (`detail?: (value: string) => string`, per esempio solo l'indirizzo), oppure che Sigrid non ripeta ciò che `display` ha già mostrato.
- **R12. Primitive che non dipendono dal preflight di Tailwind.**
  - Le primitive di Sigrid danno per scontato il reset di Tailwind (bordi, pulsanti, margini). Qamille non lo usa, perché toglierebbe gli stili predefiniti all'editor della composizione, e così deve rimettere da sé, in `@layer base`, ciò che serve alle parti di Sigrid.
  - Si chiede che ogni primitiva porti con le sue classi ciò che le serve, oppure un reset limitato alle parti di Sigrid (`:where([data-slot])`).
  - Si chiede anche che `styles.css`, il CSS già compilato, esca senza preflight: Qamille potrebbe allora usarlo, e togliere anche `tailwindcss` e `@tailwindcss/vite`.
- **R13. IBM Plex Sans variabile** (pesi da 100 a 700), o almeno il 700, in `fonts.css`. Qamille usa 550, 650 e 700: con i soli 400, 500 e 600 di Sigrid passano a 500 e 600 (vedi «Carattere»).

- **R14. `ChipsInput`: scegliere un suggerimento col puntatore non deve aggiungere anche il testo digitato.** Provato in `tests/sigrid-coexistence.mjs` (Task 0): scrivendo «b» e cliccando `bob@example.com` il campo contiene `b, bob@example.com`. Riguarda il campo dei destinatari (Task 15).
- **R15. Un `dist` che si riduca in fase di build (tree-shaking).** `dist/index.js` è un file di 2,3 MB e circa 1,7 MB restano nel pacchetto anche importando solo `WordsProvider` e `MotionProvider`: il pezzo condiviso caricato all'avvio, dalla finestra principale e dal promemoria, passa da 338 kB a 1186 kB (JavaScript totale da 1836 a 2685 kB). Il Task 21 misura l'avvio sull'exe.
- **R16. Niente `cursor-not-allowed` e `cursor-progress` nelle parti** (`checkbox`, `input`, `radio-group`, `select`, `switch`, `textarea`, `file-drop`). Un'app Windows tiene la freccia (`docs/aspetto-nativo.md`, regola 3). Finché ci sono, `system.css` tiene la freccia su di esse (`[class*='cursor-not-allowed']`) e il controllo 29 di `native-feel.mjs` ammette solo i file di oggi.

- **R17. `Tabs`: gli attributi del chiamante su una scheda, un pulsante accanto alla scheda e un clic sulla scheda già scelta.** La fascia delle bozze aperte sopra il compositore (`ui/DraftTabs.tsx`) ne ha bisogno: una scheda per bozza con icona, titolo e segno di modifiche non salvate, un pulsante che la chiude, `aria-expanded`, `onFocus`. Anche gli id di scheda e pannello (`ids`) che oggi si impostano dopo il disegno (`ui/Tabs.tsx`). Con questo Qamille userebbe `Tabs` di Sigrid anche lì e butterebbe `DraftTabs`.

- **R18. `Select`: una voce disattivata (`disabled`), `aria-required` e un'etichetta visibile.** Il `Select` di Qamille li ha (`ui/Select.tsx`): una voce `disabled` e `aria-required="true"` (la rubrica obbligatoria nel modulo del contatto) oggi non arrivano, e l'etichetta visibile è un `<label>` che scrive Qamille.

## Global Constraints

> **Regola del proprietario (2026-11-22, prevale su tutto ciò che segue): «lavora direttamente in develop», «no worktrees».**
> - Niente ramo `qamille-sigrid`, niente rami `sigrid/*`, niente worktree: ogni task fa i suoi commit su `develop`, nella cartella del repository, aggiungendo solo i propri file.
> - Dove sotto si legge `qamille-sigrid` o `sigrid/wave-c`, vale `develop`. L'ondata C si fa su `develop` un task dopo l'altro, nell'ordine 10, 11, 12, 13, 14, 15; il Task 16 non unisce rami ma fa le prove dell'app intera alla fine dell'ondata.
> - In mezzo `develop` ha due copie di Zag e di Motion (quelle di Qamille e quelle dentro Sigrid), e tra un task e l'altro dell'ondata C l'app può avere superfici delle due librerie: per questo **nessuna release `develop` → `main` finché il Task 20 non toglie Ark e Motion di Qamille**, salvo decisione del proprietario.
> - In parallelo solo task con file separati; un solo agente alla volta lancia i test del browser e `visual-snapshot` (porte fisse).


- **Solo Sigrid** (regola del proprietario, 2026-11-22).
  - **Che cosa è vietato.** Nessun file di Qamille importa `@ark-ui/*`, `@base-ui/*`, `motion`/`motion/*`, `lucide-react`, `@fluentui/*`, `clsx`, `class-variance-authority`, `tailwind-merge`, `@internationalized/date` o `tw-animate-css`.
  - **Durante la migrazione.** Li importano ancora solo i file elencati in `PENDING` di `tests/only-sigrid.mjs`: `src/ds/**` e i file che usano Motion direttamente. L'elenco si accorcia e non si allunga mai: un import nuovo fuori elenco fa fallire il test, dal Task 1.
  - **Alla fine (Task 20)** l'elenco è vuoto, e i pacchetti escono da `desktop/package.json` e dal lockfile. Il test controlla anche quelli.
  - **Ciò che a Sigrid manca** va in «Che cosa chiedere a Sigrid», mai in Qamille.
  - **Che cosa resta.** In Qamille restano `react` e `react-dom` (peer di Sigrid), `@tauri-apps/api`, `@tiptap/*`, `dompurify` e gli strumenti di build e di test. `tailwindcss` e `@tailwindcss/vite` restano come strumenti di build finché vale R12.
- **Rami.** Ramo `qamille-sigrid`, da `develop` (`AGENTS.md`).
  - I rami dei worktree sono solo locali (`sigrid/task-N`, da `qamille-sigrid`; per l'ondata C `sigrid/wave-c`, vedi Task 16). Non si pubblicano e si cancellano subito dopo il merge.
  - Mai `main`, mai un cambio di versione, mai un tag.
  - **L'ondata A torna in `develop` da sola**, perché non porta copie in più.
  - **Dall'ondata B il cambio sta su `qamille-sigrid`, e torna in `develop` solo quando Qamille non ha più Ark né Motion propri,** cioè dopo il Task 20. Sul ramo, in mezzo, il bundle ha due copie di Zag e di Motion: quelle di Qamille e quelle dentro Sigrid. In `develop`, e quindi in una release, non ci sono mai.
  - Il ramo resta corto come ramo, anche se non come durata: porta solo questo lavoro, e `develop` vi entra con un merge all'inizio di ogni ondata.
- **Solo il repository ufficiale.** Sigrid arriva solo da `npm run sigrid:sync`.
  - Nessun file di `desktop/vendor/sigrid/` si modifica a mano.
  - `SIGRID_DIR` serve solo a provare in locale una modifica di Sigrid non ancora rilasciata: nessun commit di Qamille dipende da codice di Sigrid non rilasciato.
- **Nessun pin di Sigrid.**
  - Nessuna voce `@acaprino/sigrid` in `package.json` o nel lockfile.
  - Nessuna versione o commit di Sigrid scritti nel codice o nei test di Qamille: chi deve saperli legge `vendor/sigrid/provenance.json`.
  - `compatto.css` non porta la versione nell'intestazione.
- **Dipendenze npm** a versione esatta, senza `^`, e mai una nuova che tocchi l'interfaccia.
- **Confini del codice.**
  - Solo `desktop/src/ui/**` e `desktop/src/theme/**` importano `@acaprino/sigrid`.
  - Fino al Task 20 i componenti dell'app importano ancora `ds/primitives/*`, che diventano riesportazioni.
  - Il codice dell'app non cerca le parti di Ark (`[data-scope=…]`) né lo stato di Ark (`[data-state="open"]`) per riconoscere un controllo: usa ruoli e `data-slot`.
- **Parametri di Qamille.**
  - Colori «Compatto», densità compatta, raggi 4/6/8/12, e IBM Plex Sans, che dal Task 2 è il carattere di Sigrid (`fonts.css`).
  - L'utente sceglie solo chiaro, scuro o sistema: nell'interfaccia non compaiono `ThemeGallery`, `LookSettings`, `DensityChoice`, `IconPackChoice` né `ColorMatch`.
  - Il pacchetto di icone è il predefinito di Sigrid (`DEFAULT_ICON_PACK`). `ThemeDef.spot` non si usa.
- **Strada A, e quando fermarsi.**
  - Forma, testo e movimento sono quelli di Sigrid.
  - Una differenza a vista che non sta né nelle «Differenze accettate» né in «Che cosa cambierà a vista» ferma il task, e si riporta al controller.
  - Una differenza non si «corregge» mai con una regola di Qamille sopra le classi di Sigrid: si chiede a Sigrid.
- **Due lingue.** Ogni testo visibile o accessibile è in italiano e in inglese.
  - Nel codice dell'app si scrive con `tr('italiano', 'English')`.
  - Le parole che le primitive dicono da sé passano solo da `src/ui/words.ts`.
  - Una prop di Qamille vince sul dizionario.
  - Un testo inglese scritto dentro Sigrid si chiede a Sigrid (R10).
- **Valori visivi solo da token** (ADR-0009): `node tests/style-tokens.mjs` resta verde. `compatto.css` è generato ed è esente.
- **Aspetto nativo.**
  - Dopo ogni modifica a CSS globali, `theme.css`, `compatto.css`, `theme-boot.js`, `index.html`, `reminder.html` o al foglio di Tailwind: `node tests/native-feel.mjs`, tutto `ok`.
  - Restano verdi `npm run test:browser -- forced-colors`, `node tests/contrast.mjs light dark` e `npm run test:ds`.
  - Questo piano non tocca Rust. Se `--color-paper` cambiasse, si aggiornano anche `backgroundColor` in `tauri.conf.json` e `DARK_GROUND` in `native_feel.rs`, e si lancia `node tests/webview-recovery-native.mjs`.
- **Ordine dei passi.** Ogni task scrive prima i test che falliscono, poi implementa, li vede passare e fa il commit (`superpowers:test-driven-development`).
  - Prima del commit di una primitiva: `visual-snapshot` sulla base del ramo e dopo la modifica, poi `visual-compare`. Le differenze devono stare tutte nell'elenco del task.
  - `visual-snapshot.mjs` usa la porta fissa 28176: nei worktree si lancia uno alla volta.
- **Worktree in parallelo.**
  - Ognuno ha il suo `npm ci` e il suo `npm run sigrid:sync` in `desktop/`.
  - Tutti i worktree di un'ondata usano la stessa release: il controller la annota all'apertura dell'ondata, e chi ne trova una più nuova si ferma e lo dice.
- **Documenti.** Commenti nel codice in inglese.
  - Ogni primitiva sostituisce la sua sezione di `desktop/src/ds/README.md` con un rinvio di due righe.
  - `docs/aspetto-nativo.md` lo toccano solo i Task 2 e 20.
- **Build di release** (Task 21): `build-release.bat --no-install`, oppure `npm run build` in `desktop/` e poi `cargo build --manifest-path desktop/src-tauri/Cargo.toml --release --features custom-protocol --locked --jobs 1`. Prima di usare l'exe, controllare che contenga `theme-boot.js`.

## Lavoro in parallelo

> **Regola del proprietario (2026-11-22, prevale su tutto ciò che segue): «lavora direttamente in develop», «no worktrees».**
> - Niente ramo `qamille-sigrid`, niente rami `sigrid/*`, niente worktree: ogni task fa i suoi commit su `develop`, nella cartella del repository, aggiungendo solo i propri file.
> - Dove sotto si legge `qamille-sigrid` o `sigrid/wave-c`, vale `develop`. L'ondata C si fa su `develop` un task dopo l'altro, nell'ordine 10, 11, 12, 13, 14, 15; il Task 16 non unisce rami ma fa le prove dell'app intera alla fine dell'ondata.
> - In mezzo `develop` ha due copie di Zag e di Motion (quelle di Qamille e quelle dentro Sigrid), e tra un task e l'altro dell'ondata C l'app può avere superfici delle due librerie: per questo **nessuna release `develop` → `main` finché il Task 20 non toglie Ark e Motion di Qamille**, salvo decisione del proprietario.
> - In parallelo solo task con file separati; un solo agente alla volta lancia i test del browser e `visual-snapshot` (porte fisse).


### Ondate

| Ondata | Task | Dipende da |
|---|---|---|
| 0 (controller, seriale) | 0. Punto di partenza, controlli, punti d'innesto | — |
| A (poi in `develop`) | 1. Sigrid in sviluppo, test, CI e release; poi in parallelo 2. «Compatto» come parametri, carattere e primo disegno · 3. Le parole | 0; 2 e 3 dopo 1 unito |
| B (nessun prerequisito; controlli senza sovrapposizioni) | 4. Switch · 5. Tabs · 6. Count e Slider · 7. Button · 8. Checkbox · 9. Field, in parallelo | A unita |
| C (P2; le superfici che si aprono) | 10. Toaster · 11. DateField · 12. Select · 13. Popover, Menu e Tooltip · 14. Modal e SidePanel · 15. RecipientsInput su `ChipsInput`, in parallelo da una base sola | B unita, P2 |
| C, unione (controller) | 16. Unione dell'ondata C e prove della convivenza | 10-15 pronti |
| D | 17. Navigazione e aiuti · 18. Movimento solo da Sigrid · 19. Icone solo da Sigrid (P5), in parallelo; poi 20. Via le dipendenze, nomi dei token, via `src/ds`, ADR-0011 v4 (P3, P4) | 16 unito |
| Finale | 21. Verifica sull'exe e approvazione; poi `qamille-sigrid` in `develop` | tutti |

**Come si uniscono i task.**
- **Ondate A, B e D.** Si unisce un task alla volta in `qamille-sigrid`, ognuno con `npm run test:ds`, `npm run check`, `node --test tests/only-sigrid.mjs` e i test del task verdi sul ramo unito. Al merge il controller toglie da `PENDING` di `only-sigrid.mjs` i file che il task ha liberato.
- **Ondata C.** I task non si uniscono uno per uno: con metà delle superfici su Ark di Qamille e metà su Base UI di Sigrid, l'app non funziona.
  - Ognuno parte dalla stessa base e prova la sua primitiva nella galleria, con le superfici di Sigrid.
  - Il Task 16 li unisce tutti in `sigrid/wave-c`, prova l'app intera e porta il risultato in `qamille-sigrid` con un merge solo.

**Perché tutte e due le cose: superfici insieme, e il ramo fuori da `develop` fino al Task 20.**
- **Le superfici insieme (ondata C)** servono perché l'app funzioni a ogni merge nel ramo, e i test di flusso abbiano un senso. Una trappola del focus di Base UI e una di Ark di Qamille non devono mai stare una dentro l'altra.
- **Il ramo fuori da `develop` fino al Task 20** serve perché solo allora Qamille non ha più Ark e Motion propri. Prima, `develop` e una release porterebbero due copie di Zag e di Motion.
- **La strada scartata è unire in `develop` un'ondata alla volta.** È più corta come ramo, ma lascerebbe `develop` per settimane con due librerie di sovrapposizioni e due copie di Zag.

### File condivisi: proprietario e punto d'innesto

| File | Proprietario | Chi altro lo tocca, e dove |
|---|---|---|
| `desktop/tests/only-sigrid.mjs` (`PENDING`) | il controller (il Task 0 lo scrive, ogni merge ne toglie le righe del task) | Task 20: `PENDING` vuoto e il controllo di `package.json` e del lockfile. |
| `desktop/package.json`, `package-lock.json` | Task 1 | Task 2: una riga di script (`theme:css`). Task 20: le dipendenze che escono. |
| `desktop/vite.config.ts`, `desktop/tsconfig.json`, `.github/workflows/ci.yml`, `.gitignore`, `AGENTS.md`, `desktop/scripts/sync-sigrid.mjs`, `desktop/scripts/test-browser.mjs` | Task 1 | Task 2: una riga in `OTHERS` di `test-browser.mjs` (`first-paint`). Task 20: `OTHERS` (`ds-export` via) e i percorsi di `AGENTS.md`. Task 21: solo `NODE_TEST`. |
| `desktop/src/main.tsx`, `desktop/src/reminder/main.tsx` | Task 1 (`SigridRoot`) | Task 18: via `MotionConfig`. Task 20: il percorso di `forced-colors.css`. |
| `desktop/src/ui/SigridRoot.tsx` (nuovo) | Task 1 | Task 3: solo le parole. Task 19: `IconPackProvider`. |
| `desktop/src/ui/words.ts` (nuovo) | Task 3 | Task 15: `recipientWords`, in fondo al file. |
| `desktop/src/ds/tokens/theme.css`, `desktop/src/ds/styles/tailwind.css`, `public/theme-boot.js`, `index.html`, `reminder.html`, `public/fonts/` | Task 2 | Task 20 li sposta o li cancella. |
| `desktop/src/ds/styles/system.css` | Task 2 (base in `@layer base`) | Task 7 solo la sezione dei pulsanti, Task 9 solo quella del campo, separate dai commenti del Task 0. Task 20 lo cancella. |
| `desktop/src/ds/styles/ark.css`, `forced-colors.css` | Task 0 (un blocco o una sezione per primitiva, con `/* Task N */`) | Ogni task di primitiva tocca solo il suo blocco. |
| `desktop/tests/support/Gallery.tsx`, `tests/primitives-controls.mjs`, `tests/primitives-overlays.mjs`, `tests/sigrid-leak.mjs`, `tests/sigrid-coexistence.mjs` | Task 0 (un segnaposto per primitiva) | Ogni task scrive solo sotto il suo segnaposto. |
| `desktop/src/lib/shortcut-guard.ts` | nessuno (solo innesti) | Task 12: `OWN_KEYS` (`:5-8`). Task 14: `LAYERS` e `OPEN_MODAL` (`:16-17`). Unite nel Task 16. |
| `desktop/src/components/MailList.tsx`, `UndoStack.tsx`, `ActivityCenter.tsx`, `Composer.tsx`, `ComposerDock.tsx`, `Contacts.tsx`, `meetings/*`, `src/reminder/Reminder.tsx` | Task 18 (movimento) | Task 10: una riga in `ComposerDock.tsx` e una in `UndoStack.tsx` (la misura per `--bottom-bar-room`), nell'ondata C, prima del 18. |
| `desktop/src/ds/README.md` | nessuno (solo innesti) | Ogni task la sezione della sua primitiva. Task 20 cancella il file. |
| `desktop/src/App.tsx` | nessuno (solo innesti) | Task 2: la riga dell'effetto del tema (`:1506`). Task 18: le animazioni. |
| `desktop/src/styles.css`, i `.css` dei componenti | nessuno (solo innesti) | Task 2: `[data-theme='dark']` → `[data-scheme='dark']` e i pesi del carattere. Ogni task di primitiva: solo le regole della sua primitiva. Task 20: i nomi dei token. Nessuna riformattazione. |
| `desktop/tests/native-feel.mjs` | Task 2 (controlli 3, 4 e 29) | Task 20: i percorsi nei controlli 1, 2, 4, 11 e 14. |
| `docs/aspetto-nativo.md` | Task 2 (regole 5 e 6) | Task 20: i percorsi delle regole 1, 2, 3, 18, 22, 24 e della checklist. |
| `desktop/tests/ds-usage.mjs` | Task 1 (INV-009) | Task 20 lo riscrive come `ui-usage.mjs`. |
| `desktop/tests/compatto-*.mjs`, `tests/support/compatto.mjs`, `tests/support/theme-tokens.mjs`, `tests/fonts.mjs` | Task 2 | Ogni task di primitiva: solo i casi della sua primitiva. |

## Review Focus

1. **Il primo disegno non lampeggia, e la finestra non ha una striscia di un altro colore.**
   - In tema scuro, o con «Sistema» su un Windows scuro, il primo fotogramma è già scuro e lo sfondo della finestra nativa è `--color-paper`.
   - I rischi:
     - blocchi di `compatto.css` legati a un attributo che `theme-boot.js` non scrive;
     - un foglio che arriva dopo il primo disegno;
     - il blocco grigio di riserva di Sigrid (`BASE_THEME`) che vince su quello di Qamille.
   - Test:
     - `native-feel.mjs` 3 e 4;
     - `tests/first-paint.mjs` (Task 2), sulla build di produzione, con lo schema scuro emulato e il chunk JavaScript bloccato;
     - sull'exe, `release-theme-probe.mjs` nei due schemi (Task 21).
2. **Le regole di Qamille non vincono sulle classi di Sigrid.**
   - Le regole di elemento di `system.css` sono fuori dai layer, e battono le utility di Sigrid: `button { background: none }` toglie il fondo a un pulsante primario.
   - Le regole di `styles/ark/*.css` scelgono `[data-scope][data-part]` in tutta la pagina, e colpiscono anche le parti di Ark delle primitive di Sigrid.
   - Lo stesso fanno le regole di pagina che puntano a `button` o `input` dentro un contenitore.
   - In più le primitive di Sigrid contano sul preflight, che Qamille non usa (R12).
   - Test: `tests/sigrid-leak.mjs` (Task 0, un caso per primitiva). Monta ogni primitiva migrata due volte: in una pagina con il solo CSS di Sigrid più `compatto.css`, e nella pagina di Qamille. Gli stili calcolati di ogni parte devono coincidere, salvo i raggi finché manca P3.
   - Test: il Task 2 verifica che ogni regola di elemento di `system.css` stia in `@layer base`.
3. **Due librerie di sovrapposizioni e due copie di Zag e di Motion.**
   - Un popup di Ark di Qamille dentro un `Dialog` modale di Base UI di Sigrid è «fuori»: chiude il dialogo, oppure non si raggiunge.
   - Un `Select` di Base UI di Sigrid dentro un `Modal` di Ark di Qamille: la trappola del focus di Zag non lascia entrare il focus.
   - Fra due copie di Zag, Esc chiude insieme il calendario e il dialogo.
   - Infine, un import di `motion` o di Ark che rientra da una parte qualsiasi riporta la seconda copia.
   - Test:
     - nessun task dell'ondata C entra da solo nel ramo: il Task 16 li unisce tutti;
     - `tests/sigrid-coexistence.mjs` (P2) prova le coppie dentro Sigrid;
     - in `primitives-overlays.mjs`, i casi di Qamille dentro un `Modal`: una data (Task 11), una voce di `Select` (12) e una di `Menu` (13); Esc nel calendario (11); i suggerimenti dei destinatari (15); l'azione di un avviso col `Modal` aperto (10);
     - `build-check.mjs` conta le copie: una di Zag dal Task 16, una di Motion dal Task 18, ed è la sola dentro Sigrid;
     - `only-sigrid.mjs` fallisce su ogni import vietato fuori da `PENDING`, e dal Task 20 su ogni pacchetto vietato in `package.json`.
4. **Parole inglesi in un'interfaccia italiana.**
   - Esempi: il pulsante che chiude un dialogo letto «Close»; «Error:» davanti a un errore; un destinatario annunciato come «label added»; «Close the notice» sulla «×» di un avviso, scritto dentro Sigrid (R10).
   - Le parole che non si rifanno al cambio di lingua.
   - Test: `tests/sigrid-words.mjs` (Task 3):
     - ogni chiave di `DEFAULT_WORDS` ha la sua parola italiana, e una chiave nuova di una release di Sigrid fa fallire il test;
     - le parole dei destinatari (`recipientWords`, Task 15) coprono il gruppo `labels`.
   - Test in browser: il cambio di lingua con gli avvisi (Task 3) e con un `Modal` (Task 14).
   - Test (Task 10): in `toaster-layer.mjs` il nome della «×» si legge in italiano. Fallisce finché manca R10, ed è registrato come difetto noto.
5. **I comportamenti cambiano sotto i controlli, senza che si veda nel codice dell'app.**
   - **Un modulo che non invia più.** Il `Button` di Sigrid ha `type="button"` per default: un pulsante d'invio senza `type="submit"` smetterebbe di inviare.
   - **Le scorciatoie dietro un dialogo.** `shortcut-guard.ts` riconosce un modale aperto da `[data-state="open"]` e un `Select` da `[data-scope="select"]`. Base UI scrive `data-open`, e il suo `Select` non ha `data-scope`: con un dialogo di Sigrid aperto, E archivierebbe il messaggio dietro.
   - **L'IME.** L'Esc che annulla un candidato non chiude un dialogo (regola 24), e l'Invio che lo conferma non aggiunge un destinatario.
   - **La data.** `DateField` di Sigrid non si scrive e non si svuota: senza un pulsante accanto, una data facoltativa della ricerca non si toglie più.
   - **Le animazioni.** Senza Motion proprio, un'animazione di Qamille passa da `useMotion()`. Deve restare ferma con la riduzione delle animazioni, e la pagina deve funzionare anche quando il modulo del movimento non è ancora arrivato (`null`).
   - Test:
     - Task 7: i quattro moduli d'invio;
     - Task 12 e 14: `shortcut-guard.mjs` e in browser;
     - Task 14: l'Esc dell'IME;
     - Task 15: l'Invio dell'IME;
     - Task 11: «Togli la data»;
     - Task 18: le righe della lista con le animazioni accese, spente e prima che arrivi il modulo;
     - `native-feel.mjs` 29 e `build-check.mjs` per i cursori.

## Interfacce tra i task

| Nome | Firma | Da | Per |
|---|---|---|---|
| alias `@acaprino/sigrid` | Vite `resolve.alias` `^@acaprino\/sigrid$` → `vendor/sigrid/dist/index.js`; tsconfig `paths` → `vendor/sigrid/dist/index.d.ts` | 1 | tutti |
| CSS di Sigrid | `@import` relativi di `vendor/sigrid/src/styles/theme.css` e `fonts.css` in `ds/styles/tailwind.css`, con `@source` su `vendor/sigrid/src` e `@source not` su `internal/motion-impl.tsx` | 2 | 4-19 |
| `SigridRoot` | `export function SigridRoot({ children }: { children: ReactNode })`: `WordsProvider`, `MotionProvider animated` e, dal Task 19, `IconPackProvider pack={DEFAULT_ICON_PACK}`; montata in `main.tsx` e in `reminder/main.tsx` | 1, 3, 19 | — |
| `only-sigrid.mjs` | `FORBIDDEN` (i pacchetti della regola) e `PENDING` (i file che li importano ancora, solo da togliere); dal Task 20 legge anche `package.json` e `package-lock.json` | 0 | ogni merge, 20 |
| `checkPreconditions` | `export function checkPreconditions(dir: string): Record<'P3' \| 'P4' \| 'P5', { ok: boolean; why: string }>`; dalla riga di comando `node tests/sigrid-preconditions.mjs --require P2,P5`. Con `P2` nell'elenco lancia anche `sigrid-coexistence.mjs` | 0 | Step 0 dei task 10-16, 19 e 20 |
| `COMPATTO`, `COMPATTO_TONE`, `QAMILLE_DENSITY`, `ALIASES` | come nel Task 2: la `ThemeDef`; `{ intensity: 'theme', light: 'theme' }`; `densityWith('compact', { id: 'compatto', control: { sm: 32, xs: 32 } })`; nome di Qamille → nome di Sigrid | 2 | 20 |
| `compattoCss` | `export function compattoCss(sigrid): string`, dove `sigrid` è il modulo di `vendor/sigrid/dist/index.js`; `npm run theme:css` scrive `src/theme/compatto.css` | 2 | `tests/tokens-compatto.mjs` |
| attributi della radice | `<html data-theme="compatto" data-density="compatto" …>` statici in `index.html` e `reminder.html`; `data-scheme` lo scrivono `theme-boot.js`, `App.tsx` e `Reminder.tsx` | 2 | tutti |
| `qamilleWords` | `export function qamilleWords(italian: boolean): Words` (`src/ui/words.ts`), completo in italiano | 3 | 10-15 |
| `recipientWords` | `export function recipientWords(italian: boolean): Pick<WordsOverride, 'labels'>`, per un `WordsProvider` annidato attorno a `ChipsInput` | 15 | — |
| adattatori | `src/ui/<X>.tsx`, con la stessa default export e lo stesso tipo di props di `ds/primitives/<X>.tsx`; `ds/primitives/<X>.tsx` diventa `export { default } from '../../ui/<X>'` più i tipi | 4-15, 17, 19 | 20 |
| `notify` | invariato: `notify(message: string, error = false, actions: NoticeAction[] = [])` | 10 | app |
| `DateField` | le props di oggi più `clearable?: boolean`; `value` resta `'YYYY-MM-DD'` o `''`, che l'adattatore traduce in `null` | 11 | `SearchBar.tsx` |
| movimento | i componenti di Qamille prendono `const motion = useMotion()` da `src/ui/motion.ts`, che riesporta `useMotion` e il tipo `MotionImpl` di Sigrid, e usano `motion?.Presence`, `motion?.ListItem`, `motion?.m.div`, `motion?.Highlight`; con `null` si disegnano fermi | 18 | — |
| `ICON_MAP` | `src/ui/icon-names.ts`: nome di oggi di Qamille → nome di `ICON_NAMES` di Sigrid; da qui il controllo P5 | 19 | — |
| `compareWithCleanRoom` | `export async function compareWithCleanRoom(page, which: string, props?: object): Promise<string[]>` (`tests/support/sigrid-leak.mjs`) | 0 | 4-19 |

## Che cosa cambierà a vista

### Accettate dalla spec («Differenze accettate»)

| Dove | Cambio | Task |
|---|---|---|
| Avvisi | La colonna di Sigrid (fino a dieci), con il fondo invertito, al posto di un avviso alla volta. Un errore non viene più sostituito (F18). | 10 |
| Menu, popover, dialoghi, schede laterali, liste del Select | L'ombra di Park al posto del bordo. | 12, 13, 14 |
| Pulsanti secondari | Senza bordo: un fondo grigio leggero. | 7 |
| Testo | 12, 13 e 14 px per etichette, date e testo, invece di 11,5, 12,5 e 13,5. I titoli passano ai gradini di Sigrid con P4 (Task 20). | 2, 20 |
| Movimento | Le transizioni brevi di Sigrid (da 100 a 200 ms, solo dopo un gesto) al posto delle molle. Con la regola del proprietario vale per **tutte** le animazioni di Qamille (righe della lista, pila degli annullamenti, promemoria, riunioni, barra laterale), non solo per interruttore, schede e contatori. La cifra animata dei contatori sparisce. | 4, 5, 6, 18 |
| Contrasto | Il bordo di Qamille (`--border-strong`) a 3:1. Con la 3.0.0 vale solo per i campi di Qamille che non sono un `Field`: quelli di Sigrid usano la linea chiara (vedi R5). | 2 |

### Trovate in Sigrid 3.0.0, o portate dalla regola del proprietario: da decidere

Ognuna viene dalla forma di Sigrid, quindi per la strada A vince Sigrid. Il task che la porta non si unisce finché il proprietario non la accetta, oppure Sigrid non offre un parametro. I colori sono quelli che il motore dà con i parametri del Task 2.

| Dove | Oggi | Con Sigrid | Task |
|---|---|---|---|
| Icone di tutta l'app | i disegni di Qamille, più di 40 | i disegni di Sigrid, nel pacchetto predefinito (lucide), in pulsanti, barra laterale, menu, lettore, composer e barra del titolo | 19 |
| Carattere | IBM Plex Sans variabile, con i pesi 550, 650 e 700 | il Plex di Sigrid, con i pesi 400, 500 e 600: 550 diventa 500, mentre 650 e 700 diventano 600, quindi i titoli sono un po' meno neri (R13). Per i glifi fuori dal latino la riserva è Inter, al posto di Segoe UI | 2 |
| Righe della posta | casella nativa; molla all'entrata e all'uscita | la `Checkbox` di Sigrid; `ListItem` e `Presence` di Sigrid (200 ms, curve di Fluent) | 18 |
| Testo attenuato (chiaro) | `#4b5563` per `--muted` e `--text-secondary` | `ink-mute` `#5a6376` (più chiaro) e `ink-soft` `#3a4254` (più scuro) | 2 |
| Riga selezionata | `#ededfc` in chiaro, `#262b4a` in scuro | `#dbe1ff` e `#1e1f34`, e sotto il puntatore la stessa tinta al 60 %. In scuro la riga scelta quasi non si vede (circa 1,01:1, R9) | 2, 12, 13, 17 |
| Rosso e verde di stato | `--danger` `#b03f49`/`#ef9ba1`, `--success` `#3f7560`/`#a3cfb8` | fissi nel motore: `#ad2a1e`/`#ff9a8d`, `#1c6b3f`/`#7fd3a0` | 2 |
| Anello del focus | 2 px in `--accent`, staccato di 3 px | 2 px in `--color-ring` (`#4338ca` in chiaro), staccato di 2 px | 2, 4-15 |
| Barra laterale | `#16181d`; in scuro uguale al fondo | `#15171b` nei due schemi; testo `#d1d4da`, voce attiva `#282b31` | 2 |
| Piccoli scarti | — | indaco sotto il puntatore `#4535d5` (in scuro `#8d8ee9`); testo sull'indaco in scuro `#06070f`; barra della voce attiva in chiaro `#adb8ff` | 2 |
| Barre di scorrimento | quelle di Windows | la pillola di Sigrid | 2 |
| Colori che sfumano | cambiano di colpo | con `MotionProvider` acceso, i colori di pulsanti, link, campi, voci e schede di tutta l'app sfumano in 100 ms, mai con la riduzione delle animazioni | 1 |
| Interruttore | 32×18 | 40×20 | 4 |
| Schede | sottolineatura che scorre | fondo tinto che scorre; la scheda scelta in semigrassetto | 5 |
| Pulsanti | fermi | si abbassano un poco quando li si preme (`tapScale`) | 7 |
| Pulsante solo testo | testo, nessun fondo | `ghost`: testo più tenue, fondo leggero sotto il puntatore | 7 |
| Bordi dei campi | circa 1,3:1 in chiaro, circa 3,4:1 in scuro | la linea chiara a riposo (circa 1,25:1 e 1,2:1), e `--color-input` sotto il puntatore. In scuro scende sotto 3:1 (WCAG 1.4.11, R5) | 7, 9, 12 |
| Errori e aiuti dei campi | testo rosso sotto il campo | un riquadro su fondo rosso tenue; l'aiuto a 12 px | 9 |
| Campi con lo zoom alto | 13,5 px | sotto 860 px CSS (1280 px al 150 %, o 1000 px al 125 %) i campi di Sigrid passano a 16 px, e la colonna degli avvisi cambia posto | 9, 10 |
| Data | si scrive o si sceglie; il testo vuoto la toglie | si sceglie solo nel calendario; dove è facoltativa, Qamille mette accanto «Togli la data» (R6) | 11 |
| Avvisi | uno stato resta 4,5 s, uno con azioni 10 s, un errore 12 s; un errore è un `alert` | una durata sola (proposta: 10 s); un errore resta finché non lo si chiude, con «Errore:» per i lettori di schermo, ma in una regione cortese (R8); la «×» ha un nome inglese (R10) | 10 |
| Destinatari | chip di Qamille; un suggerimento mostra nome e indirizzo | i chip di Park (28 px); il suggerimento ripete il nome (R11) | 15 |
| Navigazione della barra laterale | una barra lunga che scorre; i conti come numeri | `NavItem` di Sigrid: barra corta e ferma, il conto in una pillola, la riga sotto il puntatore tinta | 17 |
| Icone dentro le primitive | i disegni di Qamille | quelle di Sigrid: la X, la freccia del Select, il calendario, la spunta, il segno di un errore | 10-15 |

`Sidebar`/`SidebarGroup` di Sigrid non si usano. Porterebbero la riduzione a icone, una ricerca tra le voci e, sotto 860 px CSS, un cassetto da sinistra al posto della colonna: niente di questo c'è oggi in Qamille, e la spec non lo chiede. `components/Sidebar.tsx` resta di Qamille, costruito su `NavList` di Sigrid.

Il marchio di Google (`GoogleMark`) non è un'icona: è il logo che le linee guida di Google chiedono sul pulsante d'accesso. Diventa un file d'immagine (`public/brand/google.svg`) mostrato con `<img>`, non un componente che disegna.

---

### Task 0: punto di partenza, controlli e punti d'innesto (controller)

**Files:**
- Create:
  - `desktop/tests/sigrid-preconditions.mjs` (`node --test`, e riga di comando con `--require`);
  - `desktop/tests/sigrid-coexistence.mjs` (browser, sulla galleria);
  - `desktop/tests/only-sigrid.mjs` (`node --test`);
  - `desktop/tests/sigrid-leak.mjs` (browser, con il solo segnaposto);
  - `desktop/tests/support/sigrid-leak.mjs` (`compareWithCleanRoom`);
  - `desktop/tests/support/clean-room.html`.
- Modify: `desktop/src/ds/styles/ark.css`, `forced-colors.css` e `system.css`, `desktop/tests/support/Gallery.tsx`, `desktop/tests/primitives-controls.mjs` e `desktop/tests/primitives-overlays.mjs`: solo commenti e righe vuote.

**Decisioni:**
- **`only-sigrid.mjs`.**
  - `FORBIDDEN`: `@ark-ui/`, `@base-ui/`, `motion`, `lucide-react`, `@fluentui/`, `clsx`, `class-variance-authority`, `tailwind-merge`, `@internationalized/date`, `tw-animate-css`.
  - `PENDING`: oggi `src/ds/**`, più i file che importano `motion`: `App.tsx`, `main.tsx`, `ActivityCenter.tsx`, `Composer.tsx`, `ComposerDock.tsx`, `Contacts.tsx`, `MailList.tsx`, `UndoStack.tsx`, i cinque `meetings/*`, `reminder/Reminder.tsx`.
  - Legge gli import di `src/**/*.{ts,tsx}` e gli `@import` dei `.css` con il parser di TypeScript, come `ds-usage.mjs`.
- **I controlli statici di `checkPreconditions`.** Le loro descrizioni corrispondono a «Che cosa chiedere a Sigrid»: chi cambia l'una cambia l'altra.
  - **P3:** con `radii: { sm: 4, md: 6, lg: 8, xl: 12 }`, `--radius-lg` vale 8px e `--radius-xl` 12px.
  - **P4:** c'è un `--text-*` di almeno 1.625rem, con la sua interlinea.
  - **P5:** ogni nome di destinazione in `src/ui/icon-names.ts` sta in `ICON_NAMES`. Finché il file non c'è, cioè finché il Task 19 non è cominciato, il controllo stampa i nomi che Qamille usa oggi e che Sigrid non ha.
- **P2** è `sigrid-coexistence.mjs`, con le sole primitive di Sigrid nella galleria. Prova le coppie e i comportamenti di «Che cosa chiedere a Sigrid», P2. Il controller ne annota l'esito su 3.0.0; se fallisce, la richiesta va a Sigrid con l'elenco delle coppie che non reggono.
- **Il punto di partenza visivo:** `node tests/visual-snapshot.mjs D:\Projects\qamille-mail\capture\sigrid-baseline`, sulla base del ramo.

- [ ] **Step 1:** `only-sigrid.mjs`, con i suoi test: un file finto fuori da `PENDING` che importa `motion` fa fallire il test, un file di `PENDING` no.
- [ ] **Step 2:** `checkPreconditions`, su copie finte; la copia vera si stampa soltanto (su 3.0.0: P3, P4 e P5 falsi). Poi `sigrid-coexistence.mjs`, e se ne annota l'esito.
- [ ] **Step 3:** i segnaposto, ognuno separato dagli altri da una riga vuota:
  - in `ark.css` e `forced-colors.css` un blocco per primitiva: switch 4, tabs 5, slider 6, checkbox 8, toast 10, date-picker 11, select 12, popover e menu 13, dialog 14, recipients 15, nav 17;
  - in `system.css` `/* Task 7: buttons */` e `/* Task 9: field */`;
  - in `Gallery.tsx`, `primitives-controls.mjs`, `primitives-overlays.mjs`, `sigrid-leak.mjs` e `sigrid-coexistence.mjs` un commento `// Task N: <primitiva>`, più `// Task 3: words`.
- [ ] **Step 4:** `npm run test:ds`, `node tests/native-feel.mjs`, `node --test tests/only-sigrid.mjs`, `npm run test:browser -- primitives-controls primitives-overlays`: tutto verde. Poi l'istantanea del punto di partenza.
- [ ] **Step 5: Commit** — `git commit -m "Check that Qamille relies only on Sigrid, check Sigrid's preconditions, and leave one insertion point per primitive"`

### Task 1: Sigrid in sviluppo, nei test, nella CI e nella release

**Decisioni:**
- **Nessuna dipendenza npm per Sigrid.** Con `"@acaprino/sigrid": "file:vendor/sigrid"`:
  - il lockfile scriverebbe la versione e l'albero delle dipendenze di Sigrid, cioè un pin;
  - `npm ci` cambierebbe dopo ogni sync, e fallirebbe senza la cartella;
  - si installerebbero Base UI, lucide e Fluent, che la regola vieta.

  Si usano invece:
  - Vite `resolve.alias` sul nome esatto, verso `vendor/sigrid/dist/index.js`;
  - `resolve.dedupe: ['react', 'react-dom']`;
  - tsconfig `paths` verso `dist/index.d.ts`.

  I tipi di `dist` che nominano pacchetti che Qamille non ha diventano `any`, quindi gli adattatori dichiarano le loro props per intero.
- **Senza la copia di Sigrid si ferma tutto, con un messaggio.** `vite.config.ts` controlla `vendor/sigrid/provenance.json` e, se manca, lancia «Sigrid manca: `npm run sigrid:sync` in `desktop/`».
- **Il sync entra in `predev` e `prebuild`:** `npm run artcraft:sync && npm run sigrid:sync`. `AGENTS.md`, nella sezione «Sigrid», non dice più che il sync è manuale ma che parte con `predev` e `prebuild`, e aggiunge la regola «Qamille si basa solo su Sigrid», con un rinvio a `only-sigrid.mjs`.
- **CI.** Nel job `desktop` di `ci.yml`: `npm run sigrid:sync` dopo `artcraft:sync`, con `GH_TOKEN: ${{ github.token }}`, e `node --test tests/only-sigrid.mjs` dopo `npm run check`.
  - `release.yml` non cambia: `build-release.bat` lancia `npm run build`, che passa da `prebuild`.
  - Su un runner nuovo, senza Sigrid, la build non parte: si ferma sempre, mai con una copia vecchia.
- **Licenza e provenienza nell'exe.** `sigrid:sync` copia `LICENSE`, `THIRD_PARTY.md` e `provenance.json` in `desktop/public/sigrid-licenses/` (git-ignorata). `THIRD_PARTY.md` di Sigrid elenca anche ciò che Sigrid porta dentro di sé: con la regola, è la sola nota di terze parti dell'interfaccia.
- **Tailwind 4.3.5**, a versione esatta per `tailwindcss` e `@tailwindcss/vite`, come strumento di build: è la versione con cui si compila Sigrid.
- **`SigridRoot`.** `WordsProvider` con `DEFAULT_WORDS` e `MotionProvider animated`, attorno ad `App` in `main.tsx` e attorno al promemoria in `reminder/main.tsx`. `MotionConfig` di Motion resta fino al Task 18.
- **INV-009** (bozza per l'ADR-0011 v4), in `ds-usage.mjs`: solo `src/ui/**` e `src/theme/**` importano `@acaprino/sigrid`.

**Files:**
- Create: `desktop/src/ui/SigridRoot.tsx`; `desktop/tests/sigrid-wiring.mjs` (`node --test`); `desktop/tests/build-check.mjs` (`vite build` in una cartella temporanea)
- Modify: `desktop/vite.config.ts`, `desktop/tsconfig.json`, `desktop/package.json`, `package-lock.json`, `desktop/scripts/sync-sigrid.mjs`, `desktop/tests/sigrid-sync.mjs`, `desktop/scripts/test-browser.mjs` (`build-check` in `OTHERS`), `desktop/src/main.tsx`, `desktop/src/reminder/main.tsx`, `desktop/tests/ds-usage.mjs`, `.github/workflows/ci.yml`, `.gitignore`, `AGENTS.md`

- [ ] **Step 1: test che falliscono.**
  - `sigrid-wiring.mjs`:
    - `test('no npm entry pins Sigrid')`;
    - `test('Vite and TypeScript reach the synced copy')`;
    - `test('predev and prebuild sync Sigrid after ArtCraft')`;
    - `test('CI syncs Sigrid before the type check and checks the only-Sigrid rule')`;
    - `test('the build stops with a message when the copy is missing')`.
  - `sigrid-sync.mjs`:
    - `test('licences and provenance land in public/sigrid-licenses')`;
    - `test('a sync with no release and no copy fails, and leaves nothing half-written')`.
  - `build-check.mjs`:
    - una copia di React;
    - le copie di Zag e di Motion, contate e stampate. Con `--strict`: una copia di Zag (dal Task 16) e una di Motion (dal Task 18);
    - nel CSS niente `cursor: pointer`, `not-allowed` né `progress`.
  - `ds-usage.mjs`: INV-009, con un file finto che la viola.
- [ ] **Step 2:** `node --test tests/sigrid-wiring.mjs tests/sigrid-sync.mjs`, `node tests/ds-usage.mjs`: FAIL.
- [ ] **Step 3:** implementare. Poi `npm ci` e `npm run sigrid:sync`.
- [ ] **Step 4:** i test dello step 2 PASS; `node tests/build-check.mjs`; `node --test tests/only-sigrid.mjs`; `npm run check`; `npm run test:ds`; `npm run test:browser -- smoke onboarding-professional`.
- [ ] **Step 5:** su un clone pulito, senza `vendor/sigrid`:
  - `npm ci` e poi `npm run build`, che scarica Sigrid da sé;
  - con la rete staccata, `npm run build` si ferma col messaggio di `sigrid:sync`.

  Il job `desktop` di `ci.yml` gira solo sulla pull request verso `develop`, che apre il proprietario.
- [ ] **Step 6: Commit** — `git commit -m "Reach the synced Sigrid through an alias, sync it before dev and build, and in CI"`

### Task 2: «Compatto» come parametri di Sigrid, carattere, primo disegno, alias dei token

**Decisioni:**
- **La definizione** (`src/theme/compatto.ts`).
  - `COMPATTO`: `sidebar: 'scura'`, `palette: 'crisp'`. La tinta e la saturazione dei grigi vengono da quelli di Compatto; `accent` dalla saturazione e dalla tinta OKLCH di `#4f46e5`. Nessuno `spot`.
  - `surfaces` curate nei due schemi: `paper` = `--ground`, `raised` = `--surface`, `line` = `--border`, `ink` = `--text`, `accent` = `--accent-text`, `fill` = `--accent`, e `sidebar` = una corsia **chiara** (per esempio `#eef0f3`).
  - In `surfaces` la colonna scura **non** va messa: la disegna il motore con `sidebar: 'scura'`. Con `#16181d` in `surfaces.light.sidebar` il motore porta inchiostro, linee e rosso a `#000000`/`#ffffff` (provato il 2026-11-22).
  - `COMPATTO_TONE` = `{ intensity: 'theme', light: 'theme' }`: il tono predefinito (Vivid) sposterebbe l'indaco a `#3f00d9`.
  - Con questi valori il motore (2.0.1, 2.1.0 e 3.0.0) tiene esatti `paper`, `raised`, `ink`, `line`, `accent` e `primary` nei due schemi, e non corregge niente.
- **La densità.** `QAMILLE_DENSITY` = `compact` (32/28/24) con `sm` e `xs` a 32, perché oggi ogni controllo di Qamille è alto 32 px. Con un puntatore grossolano valgono i 44 px di Sigrid.
- **`compatto.css`, generato e versionato.**
  - `scripts/theme-css.mjs` importa `vendor/sigrid/dist/index.js` in Node, e funziona: all'import non tocca `document`. Scrive:
    - il blocco `:root`, con `densityVariables(QAMILLE_DENSITY)`;
    - il blocco chiaro (`:root:not([data-scheme]), :root[data-scheme='light']`) e quello scuro (`:root[data-scheme='dark']`), con `themeVariables({ theme: COMPATTO, scheme, tone: COMPATTO_TONE })`, senza `--radius-*` finché manca P3;
    - il blocco degli alias.
  - `tests/tokens-compatto.mjs` controlla:
    - i parametri, con `corrected` vuoto o uguale a un elenco noto annotato;
    - che `compatto.css` sia uguale a `compattoCss(sigrid)`. Se non lo è, il messaggio dice di rilanciare `npm run theme:css` e di guardare il diff.
- **Gli alias.** `ALIASES` porta i nomi di Qamille sui token di Sigrid per tutto ciò che Sigrid possiede: `--ground`, `--surface`, `--sidebar*`, `--text`, `--text-secondary`, `--body-text`, `--muted`, `--nav-text`, `--border`, `--border-soft`, `--border-strong`, `--accent`, `--accent-hover`, `--accent-text`, `--on-accent`, `--ring`, `--accent-pro`, `--selection`, `--danger*`, `--success*`, `--highlight`, `--shadow*`, `--control-height`, `--font-text`, `--font-title`.
  - Per ogni nome si sceglie il ruolo più vicino, e il task riporta la tabella prima/dopo.
  - `--border-strong` va su `--color-input` finché il proprietario non decide su R5.
  - I token solo di Qamille restano scritti a mano in `ds/tokens/theme.css`, col blocco scuro su `[data-scheme='dark']`.
- **Il testo.**
  - Qamille non definisce più `--text-2xs`, `--text-xs` e `--text-sm`: valgono quelli di Sigrid, 12, 13 e 14 px.
  - `--text-md` diventa alias di `--text-sm`.
  - `--text-lg`, `--text-xl`, `--text-2xl` e `--text-3xl` restano di Qamille fino a P4.
- **Il carattere: quello di Sigrid** (regola del proprietario).
  - `ds/styles/tailwind.css` importa `fonts.css` di Sigrid (IBM Plex Sans 400, 500 e 600, e Inter per i glifi fuori dal latino). Vite lo impacchetta come file dell'app, quindi `font-src 'self'` basta.
  - Si tolgono `public/fonts/ibm-plex-sans/`, il `<link rel="preload">` di `index.html` e il `@font-face` di `reminder.css`.
  - Nei `.css` di Qamille i pesi passano a quelli che ci sono: 550 → 500, 650 e 700 → 600, così il browser non inventa un grassetto (R13).
  - `tests/fonts.mjs` controlla che il testo sia disegnato con il Plex di Sigrid, servito da `'self'`.
- **La cascata.** `ds/styles/tailwind.css` diventa, nell'ordine:
  1. `@layer theme, base, components, utilities`;
  2. il tema di Tailwind;
  3. `@theme { --color-*: initial }`;
  4. `theme.css` e `fonts.css` di Sigrid, con percorsi relativi;
  5. le utility;
  6. `@source` su `vendor/sigrid/src` e `@source not` su `internal/motion-impl.tsx`.

  Altre regole:
  - Niente preflight, così la composizione e l'editor tengono i default del browser.
  - Le regole di elemento di `system.css` entrano in `@layer base`, dopo il `base` di Sigrid, con ciò che serve alle parti di Sigrid senza preflight (R12).
  - La mappa dei nomi di shadcn sparisce.
  - La variante `dark` diventa `[data-scheme='dark']`.
- **Gli attributi.**
  - `theme-boot.js` scrive `dataset.scheme`.
  - `index.html` e `reminder.html` portano `data-theme="compatto"`, `data-density="compatto"` e gli attributi di `ThemeTokens.attributes`.
  - L'effetto del tema in `App.tsx` (`:1506`) e in `Reminder.tsx` (`:110`) scrive `dataset.scheme`; lo stesso fa `message-html.ts`.
  - Ogni `[data-theme='dark']` dei `.css` diventa `[data-scheme='dark']`.
  - Qamille non chiama `applyTheme` né `applyDensity` a runtime.
- **La finestra.** `--color-paper` vale `#f6f7f9` e `#12161e`, cioè il `--ground` di oggi: `tauri.conf.json` e `DARK_GROUND` non cambiano. Il controllo 4 legge `compatto.css`.

**Files:**
- Create: `desktop/src/theme/compatto.ts` e `compatto.css` (generato); `desktop/scripts/theme-css.mjs`; `desktop/tests/first-paint.mjs`; una riga in `OTHERS` di `test-browser.mjs`
- Modify: `desktop/src/ds/tokens/theme.css`, `ds/styles/tailwind.css`, `ds/styles/system.css`, `desktop/public/theme-boot.js`, `desktop/public/fonts/` (via), `desktop/index.html`, `desktop/reminder.html`, `desktop/src/App.tsx` (`:1506`), `desktop/src/reminder/Reminder.tsx` (`:110`), `desktop/src/reminder/reminder.css`, `desktop/src/lib/message-html.ts`, `desktop/package.json` (script `theme:css`), `desktop/src/styles.css` e i `.css` dei componenti (`[data-theme='dark']` e i pesi del carattere)
- Modify, test: `tests/tokens-compatto.mjs`, `tests/contrast.mjs`, `tests/style-tokens.mjs`, `tests/fonts.mjs`, `tests/support/theme-tokens.mjs`, `tests/support/compatto.mjs`, `tests/compatto-*.mjs`, `tests/support/clean-room.html`, e i test che leggono `data-theme`: `calendar-flow`, `ds-export`, `mail-html`, `recipients-dark`, `release-theme-probe`, `smoke`, `undo`, `update-notice-fixture.js`, `window-native`
- Modify: `desktop/tests/native-feel.mjs` (controlli 3 e 4, nuovo 29); `docs/aspetto-nativo.md` (regole 5 e 6); `desktop/src/ds/README.md` («Colore», «Tipografia», «Tailwind»)

- [ ] **Step 1: test che falliscono.**
  - `tokens-compatto.mjs`, riscritto.
  - `contrast.mjs`, sulle coppie di Qamille e su quelle miste.
  - `fonts.mjs`, sul Plex di Sigrid.
  - `native-feel.mjs`:
    - 3: `dataset.scheme`;
    - 4: legge `compatto.css`;
    - 29: niente `cursor-pointer`, `cursor-not-allowed` o `cursor-progress` in `vendor/sigrid/src`, nemmeno dopo una variante.
  - `first-paint.mjs`:
    - `vite build` e `vite preview`, con lo schema scuro emulato e il chunk d'ingresso bloccato;
    - il fondo di `body` vale `#12161e` e `data-scheme` vale `dark` prima di `DOMContentLoaded`;
    - lo stesso in chiaro.
  - `system.css`: nessuna regola di elemento fuori da `@layer base`.
- [ ] **Step 2:** `node tests/tokens-compatto.mjs`, `node tests/native-feel.mjs`, `node tests/first-paint.mjs`, `npm run test:browser -- fonts`: FAIL.
- [ ] **Step 3:** implementare, poi `npm run theme:css`.
- [ ] **Step 4:** test PASS; `node tests/build-check.mjs`; `npm run test:ds`; `npm run check`; `npm run test:browser -- smoke forced-colors fonts compatto-dark compatto-list compatto-reader compatto-settings compatto-sidebar compatto-notices recipients-dark mail-html`.
- [ ] **Step 5:** `visual-snapshot` prima e dopo, poi `visual-compare`. Le differenze devono stare tutte in queste righe di «Che cosa cambierà a vista»: testo, carattere, contrasto, testo attenuato, riga selezionata, rosso e verde, `--ring`, barra laterale, piccoli scarti, barre di scorrimento. Il task riporta la tabella dei token prima/dopo.
- [ ] **Step 6:** `docs/aspetto-nativo.md`, regole 5 e 6; `ds/README.md`, sezioni «Colore», «Tipografia» e «Tailwind».
- [ ] **Step 7: Commit** — `git commit -m "Compute the Compatto look with Sigrid's engine, use Sigrid's typeface, paint it before any script, and alias the old token names"`

### Task 3: le parole di Sigrid in italiano e in inglese

**Decisioni:**
- `qamilleWords(italian)` dà un `Words` intero: una chiave nuova di una release di Sigrid diventa un errore di `tsc` e del test.
  - In inglese: `DEFAULT_WORDS`, con le parole di Qamille dove Qamille dice altro.
  - In italiano: tutto, con i plurali, le combinazioni di tasti, i nomi dei colori e le parole del calendario allineate a `lib/calendar-words.ts`.
- Il dizionario copre ogni chiave, anche quelle di componenti che Qamille non usa.
- `SigridRoot` passa `qamilleWords(useLanguage())`, memorizzato per lingua.
- Le funzioni di Sigrid fuori da un componente prendono `useWords()`.
- Un componente che parla d'altro, come i destinatari del Task 15, annida un suo `WordsProvider`, che Sigrid unisce a quello della radice.

**Files:**
- Create: `desktop/src/ui/words.ts`, `desktop/tests/sigrid-words.mjs`
- Modify: `desktop/src/ui/SigridRoot.tsx`; `Gallery.tsx` e `primitives-overlays.mjs` (segnaposto «Task 3: words»)

- [ ] **Step 1: test che falliscono.**
  - `test('every Sigrid word has an Italian one')`;
  - `test('Italian words differ from English ones')`;
  - `test('counted words agree with 1 and 3')`;
  - `test('a key added by Sigrid fails here')`;
  - in browser: `Toasts` sotto `SigridRoot`, e al cambio di lingua il nome della regione cambia.
- [ ] **Step 2:** `node --test tests/sigrid-words.mjs`, `npm run test:browser -- primitives-overlays`: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** test PASS; `npm run check`; `npm run test:browser -- smoke primitives-overlays`.
- [ ] **Step 5: Commit** — `git commit -m "Give Sigrid's own words in Italian and English, following the interface language"`

### Procedura comune dei task di primitiva (4-15, 17, 19)

- [ ] **Step 0:**
  - `npm run sigrid:sync`;
  - nell'ondata C, `node tests/sigrid-preconditions.mjs --require P2`; nel Task 19, `--require P5`. Se il controllo fallisce, il task non parte;
  - poi `visual-snapshot` della base in `capture/sigrid-task-N-before`.
- [ ] **Step 1: test che falliscono.**
  - Nella galleria la primitiva passa all'adattatore.
  - In `sigrid-leak.mjs`, il suo caso.
  - In `primitives-*.mjs`, i casi che leggono le classi di `system.css` o di `ark/*.css` diventano casi su ruoli, nomi e `[data-slot]`. Su `[data-scope][data-part]` restano solo per le primitive di Sigrid su Ark: switch, tabs, slider, date picker, tags input, combobox, toast.
  - In `forced-colors.mjs`, il suo caso.
- [ ] **Step 2:** `npm run test:browser -- <test della primitiva> sigrid-leak forced-colors`: FAIL.
- [ ] **Step 3: implementare.**
  - `src/ui/<X>.tsx`, solo su esportazioni di Sigrid. `ds/primitives/<X>.tsx` diventa la riesportazione.
  - Si tolgono il blocco di `ark.css` e il file `ds/styles/ark/<x>.css`.
  - La sezione di `forced-colors.css` si riscrive sulle parti di Sigrid.
  - I selettori di `styles.css`, dei `.css` dei componenti e di `src/lib/` che puntano a classi, parti o stati di Ark passano a ruoli e `data-slot`.
  - Le copie di unlumen usate solo da questa primitiva si cancellano.
  - Ciò che l'adattatore non può fare con le sole esportazioni di Sigrid va in «Che cosa chiedere a Sigrid»: il task lo riporta, e non lo scrive in Qamille.
- [ ] **Step 4:** test PASS; `npm run test:ds`; `npm run check`; `node tests/build-check.mjs`; `node --test tests/only-sigrid.mjs`; i test di flusso del task. Nell'ondata C i test di flusso dell'app si lanciano nel Task 16.
- [ ] **Step 5:** `visual-snapshot` dopo, poi `visual-compare`. Ogni differenza va ricondotta a una riga di «Che cosa cambierà a vista»: una che non c'è ferma il task. Nell'ondata C il confronto si fa sull'unione, nel Task 16.
- [ ] **Step 6:** la sezione di `ds/README.md`; il commit col messaggio del task.

### Task 4: Switch (B)

- **Decisioni:** `src/ui/Switch.tsx` su `Switch` di Sigrid (Ark). Senza un'etichetta visibile, il nome è l'`aria-label` di Qamille. Si tolgono `unlumen/apple-switch.tsx` e `styles/ark/switch.css`.
- **Files:** `desktop/src/ui/Switch.tsx`, `desktop/src/ds/primitives/Switch.tsx`, `desktop/src/ds/unlumen/apple-switch.tsx` (via), `desktop/src/ds/styles/ark/switch.css` (via), i segnaposto «Task 4».
- **Differenze attese:** movimento (accettata); 40×20 (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-controls mcp-settings meetings-settings compute-settings`.
- **Commit:** `git commit -m "Draw Switch with Sigrid's switch"`

### Task 5: Tabs (B)

- **Decisioni:** `src/ui/Tabs.tsx` su `Tabs*` di Sigrid (Ark), con l'evidenza scorrevole di Sigrid. Si tolgono `styles/ark/tabs.css` e il `layoutId` di Motion della primitiva.
- **Files:** `desktop/src/ui/Tabs.tsx`, `desktop/src/ds/primitives/Tabs.tsx`, `desktop/src/ds/styles/ark/tabs.css` (via), i segnaposto «Task 5».
- **Differenze attese:** movimento (accettata); fondo tinto e semigrassetto (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-controls composer-tabs`.
- **Commit:** `git commit -m "Draw Tabs with Sigrid's tabs"`

### Task 6: Count e Slider (B)

- **Decisioni:**
  - `src/ui/Count.tsx` su `Count` di Sigrid; si toglie `unlumen/animate-digits.tsx`.
  - `Slider` non ha consumatori in `src/`: si tolgono `ds/primitives/Slider.tsx`, `styles/ark/slider.css` e i loro casi di test.
- **Files:** `desktop/src/ui/Count.tsx`, `desktop/src/ds/primitives/Count.tsx`, `desktop/src/ds/primitives/Slider.tsx` (via), `desktop/src/ds/unlumen/animate-digits.tsx` (via), `desktop/src/ds/styles/ark/slider.css` (via), i segnaposto «Task 6».
- **Differenze attese:** la cifra non si anima (accettata).
- **Test di flusso:** `npm run test:browser -- primitives-controls sidebar-threads undo`.
- **Commit:** `git commit -m "Show counts with Sigrid's Count and drop the unused Slider"`

### Task 7: Button (B)

- **Decisioni:**
  - `src/ui/Button.tsx` tiene `variant: 'primary' | 'secondary' | 'icon' | 'text'`, `tip` e `shortcut`. Le varianti diventano:
    - `primary` → `default`;
    - `primary` con la classe `danger` → `destructive`, rosso pieno come oggi `.primary.danger`;
    - `secondary` → `secondary`;
    - `icon` → `ghost` con `size="icon"`;
    - `text` → `ghost`;
    - `text` con `danger` → `ghost` più l'utility `text-destructive`, sul token di Sigrid.
  - `tip` usa ancora `ds/primitives/Tooltip` fino al Task 13.
  - **`type`** (Review Focus 5). Prima del cambio si cercano tutti i `<form>` di `src/` (oggi 16, con 11 `type="submit"`), e ogni pulsante che oggi invia per default prende `type="submit"` esplicito, nello stesso commit.
  - Le regole `.primary`, `.secondary`, `.icon-button`, `.text-button` e `.danger` di `system.css` (sezione «Task 7») si tolgono quando nessun HTML scritto a mano le usa più. I pulsanti `danger-solid` e `danger-text` di `Calendar.tsx` passano a `Button`.
- **Files:** `desktop/src/ui/Button.tsx`, `desktop/src/ds/primitives/Button.tsx`, `desktop/src/ds/styles/system.css` (sezione «Task 7»), `desktop/src/components/Calendar.tsx`, i `<form>` senza `type="submit"`, i segnaposto «Task 7».
- **Test nuovo:** in `onboarding-professional`, `server-connect`, `password-reset` e `signup-professional`, l'Invio nel campo e il clic sul pulsante d'invio inviano. Un pulsante che non è d'invio, dentro un `<form>`, non invia.
- **Differenze attese:** secondari senza bordo (accettata); `ghost`, `tapScale` e bordo chiaro (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-button a11y-keyboard smoke shortcut-guard onboarding-professional server-connect password-reset signup-professional calendar-flow`.
- **Commit:** `git commit -m "Draw buttons with Sigrid's Button, and make every submit button say so"`

### Task 8: Checkbox (B)

- **Decisioni:** `src/ui/Checkbox.tsx` su `Checkbox` di Sigrid (Base UI), accesa nell'accento come oggi. La casella delle righe della lista è del Task 18.
- **Files:** `desktop/src/ui/Checkbox.tsx`, `desktop/src/ds/primitives/Checkbox.tsx`, `desktop/src/ds/styles/ark/checkbox.css` (via), i segnaposto «Task 8».
- **Differenze attese:** 18 px; il bordo più scuro sotto il puntatore.
- **Test di flusso:** `npm run test:browser -- primitives-controls contacts meetings-settings`.
- **Commit:** `git commit -m "Draw Checkbox with Sigrid's checkbox"`

### Task 9: Field (B)

- **Decisioni:**
  - `src/ui/Field.tsx` su `FormField` con `Input` e `Textarea` di Sigrid.
  - La sezione «Task 9» di `system.css` (`.field`) si toglie.
  - Il task non si unisce finché il proprietario non decide su R5.
- **Files:** `desktop/src/ui/Field.tsx`, `desktop/src/ds/primitives/Field.tsx`, `desktop/src/ds/styles/system.css` (sezione «Task 9»), i segnaposto «Task 9».
- **Test nuovo:** un `Field` nella finestra minima, con lo zoom dell'app a 100, 125 e 200 %: il corpo è di 14 px sopra 860 px CSS e di 16 px sotto, finché il proprietario non decide altrimenti.
- **Differenze attese:** bordi, errori e aiuti, 16 px con lo zoom alto (da decidere).
- **Test di flusso:** `npm run test:browser -- onboarding-professional server-connect signup-professional password-reset contacts`.
- **Commit:** `git commit -m "Draw fields with Sigrid's FormField, Input and Textarea"`

### Task 10: Toaster (C)

- **Decisioni:**
  - `src/ui/Toaster.tsx` tiene `notify(message, error, actions)`, reso con `Toasts` di Sigrid.
  - `notify(message, true)` dà `kind: 'error'`: un errore resta finché non lo si chiude.
  - Gli altri avvisi hanno `durationMs` a 10 s (R8).
  - Con un `Modal` aperto la regione resta letta, e un clic su un avviso non chiude il `Modal` (`ignoreToasts`, dal Task 16).
  - Un solo scrittore di `--bottom-bar-room`: `src/lib/bottom-room.ts`, che somma il dock e `UndoStack`. Per questo c'è una riga in `ComposerDock.tsx` e una in `UndoStack.tsx`.
  - Si toglie `styles/ark/toast.css`.
- **Files:** `desktop/src/ui/Toaster.tsx`, `desktop/src/ds/primitives/Toaster.tsx`, `desktop/src/ds/styles/ark/toast.css` (via), `desktop/src/lib/bottom-room.ts`, una riga ciascuno in `ComposerDock.tsx` e `UndoStack.tsx`, i segnaposto «Task 10».
- **Test nuovi** (`toaster-layer.mjs`):
  - «un errore resta finché non lo si chiude»;
  - «"Errore:" si sente prima del testo, in italiano e in inglese»;
  - «l'azione di un avviso funziona con un Dialog di Sigrid aperto»;
  - «la × ha un nome in italiano»: fallisce finché manca R10, ed è segnato come difetto noto, con un rinvio a R10.
- **Differenze attese:** colonna e fondo invertito (accettata); durata, errore, annuncio, nome della «×» e posto della colonna con lo zoom alto (da decidere).
- **Test di flusso (nel Task 16):** `toaster-layer undo compatto-notices mail-export`; `node --test tests/undo-notices.mjs`.
- **Commit:** `git commit -m "Show notices in Sigrid's column, with errors that stay until they are closed"`

### Task 11: DateField (C)

- **Decisioni:**
  - `src/ui/DateField.tsx` su `DateField` di Sigrid. `value` resta `'YYYY-MM-DD'` o `''`, e `''` diventa `null`.
  - La lingua e il formato vengono da `regional.ts`.
  - La data non si scrive più (R6): i test che la scrivono la scelgono nella griglia.
  - `clearable` aggiunge accanto un pulsante `ghost` a sola icona, «Togli la data» / «Remove the date», usato dalle due date facoltative della ricerca.
  - Niente scelte rapide né `DateRangeField`.
  - Qamille non importa più `@internationalized/date`.
  - Si toglie `styles/ark/date-picker.css`.
- **Files:** `desktop/src/ui/DateField.tsx`, `desktop/src/ds/primitives/DateField.tsx`, `desktop/src/components/SearchBar.tsx` (`clearable`), `desktop/src/ds/styles/ark/date-picker.css` (via), i segnaposto «Task 11».
- **Test nuovi:**
  - in `primitives-overlays.mjs`: una data scelta dentro un `Modal` lascia il `Modal` aperto e il focus dentro; Esc col calendario aperto chiude solo il calendario;
  - in `primitives-controls.mjs`: con `clearable` il pulsante toglie la data; senza, il pulsante non c'è.
- **Differenze attese:** data che non si scrive, scheda di Park, icone di Sigrid, «Togli la data» (da decidere).
- **Test di flusso (nel Task 16):** `calendar-flow calendar-series meetings search`.
- **Commit:** `git commit -m "Pick dates with Sigrid's date field, and offer to remove an optional date beside it"`

### Task 12: Select (C)

- **Decisioni:**
  - `src/ui/Select.tsx` traduce `items` in `SelectOption`/`SelectGroup`.
  - `OWN_KEYS` di `shortcut-guard.ts` prende `[role="listbox"]` e `[data-slot^="select"]`.
  - Il selettore di `SearchBar.tsx:237` passa a `[data-slot="select-trigger"]`.
- **Files:** `desktop/src/ui/Select.tsx`, `desktop/src/ds/primitives/Select.tsx`, `desktop/src/ds/styles/ark/select.css` (via), `desktop/src/lib/shortcut-guard.ts` (`OWN_KEYS`), `desktop/src/components/SearchBar.tsx` (`:237`), `desktop/tests/shortcut-guard.mjs`, i segnaposto «Task 12».
- **Test nuovi:** una voce di un `Select` dentro un `Modal` lascia il `Modal` aperto; una lettera in un `Select` aperto è del `Select`.
- **Differenze attese:** ombra (accettata); bordo chiaro, riga sotto il puntatore, freccia e spunta (da decidere).
- **Test di flusso (nel Task 16):** `primitives-select regional meetings-settings compute-settings search`.
- **Commit:** `git commit -m "Draw Select with Sigrid's select, and let the shortcut guard recognize it"`

### Task 13: Popover, Menu e Tooltip (C)

- **Decisioni:**
  - `src/ui/Popover.tsx`, `src/ui/Menu.tsx` e `src/ui/Tooltip.tsx` su `Popover*`, `DropdownMenu*` e `Tooltip` di Sigrid.
  - `Menu` tiene il suo contratto (`ds/README.md`, «Menu»):
    - gruppi separati da `DropdownMenuSeparator`, senza titolo;
    - le voci `destructive` in fondo;
    - `disabled` con `title`;
    - `checked` come `DropdownMenuCheckboxItem`;
    - `shortcut` come `DropdownMenuShortcut`, con `aria-keyshortcuts`.
  - Il focus torna al pulsante prima del comando.
  - `data-keyboard` resta solo se Sigrid non distingue da sé la tastiera dal mouse; in quel caso lo si riporta.
- **Files:** `desktop/src/ui/{Popover,Menu,Tooltip}.tsx`, `desktop/src/ds/primitives/{Popover,Menu,Tooltip}.tsx`, `desktop/src/ds/styles/ark/popover.css` e `menu.css` (via), le regole del suggerimento in `ark.css`, i segnaposto «Task 13».
- **Test nuovo:** una voce di `Menu` dentro un `Modal` che apre un altro `Modal`: alla chiusura il focus torna al pulsante del menu.
- **Differenze attese:** ombra (accettata); riga sotto il puntatore e icone (da decidere).
- **Test di flusso (nel Task 16):** `primitives-overlays shortcut-guard meetings reply-all`.
- **Commit:** `git commit -m "Draw popovers, menus and hints with Sigrid's Popover, DropdownMenu and Tooltip"`

### Task 14: Modal e SidePanel (C)

- **Decisioni:**
  - `src/ui/Modal.tsx` su `DialogRoot`/`Dialog*`, e `src/ui/SidePanel.tsx` su `Sheet*`. Le misure restano token di Qamille, passate con `className`.
  - Il focus torna dove era con `restoreFocus` di Sigrid. Se non basta lo si chiede a Sigrid, e `focus-return.ts` resta finché non arriva.
  - Su `onOpenChange(false)` decidono le guardie di chiusura di Qamille.
  - L'Esc durante una composizione IME non chiude: se Base UI non lo fa da sé, l'adattatore lo ferma con `imeComposing` nel gestore di chiusura di Sigrid.
  - In `shortcut-guard.ts`, `OPEN_MODAL` riconosce anche `[role="dialog"][aria-modal="true"][data-open]`.
- **Files:** `desktop/src/ui/{Modal,SidePanel}.tsx`, `desktop/src/ds/primitives/{Modal,SidePanel}.tsx`, `desktop/src/ds/styles/ark/dialog.css` (via), `desktop/src/lib/shortcut-guard.ts` (`LAYERS`, `OPEN_MODAL`), `desktop/tests/shortcut-guard.mjs`, i segnaposto «Task 14».
- **Test nuovi:**
  - l'Esc dell'IME;
  - le modifiche non salvate, con la X e con Esc;
  - il nome del pulsante di chiusura al cambio di lingua;
  - con un `Modal` di Sigrid aperto, il tasto E è del dialogo.
- **Differenze attese:** ombra (accettata); la X di Sigrid (da decidere).
- **Test di flusso (nel Task 16):** `primitives-overlays modal-actions-edge google-sign-in attachment-panel meetings calendar-flow close-guards`; `node --test tests/close-guards.mjs tests/shortcut-guard.mjs`.
- **Commit:** `git commit -m "Draw dialogs and side panels with Sigrid's Dialog and Sheet"`

### Task 15: RecipientsInput su `ChipsInput` (C)

**Decisioni:**
- `src/ui/RecipientsInput.tsx` tiene le props di oggi e sta solo su `ChipsInput` di Sigrid. I suggerimenti restano dentro il composer o il dialogo.
- **Le props:**
  - `value`: le voci di oggi;
  - `display`: il nome, o l'indirizzo se il nome manca;
  - `invalid`: la funzione di Qamille;
  - `separators`: `[',', ';']`;
  - `dot`: `false`;
  - `suggestions`: le voci di `{ email, name }`;
  - `onQuery` e `suggestionsLabel`: passano come sono.
- **Ciò che `ChipsInput` non ha:**
  - `aria-label`: un `<label htmlFor={id}>` nascosto alla vista;
  - `disabled`: un `<fieldset disabled>` attorno al campo (R7);
  - `inputRef`: il focus si dà con `document.getElementById(id)` (R7);
  - `pending`: è il testo che `onQuery` riceve a ogni tasto;
  - lo stesso indirizzo due volte: l'adattatore lo toglie in `onChange`;
  - l'altezza massima (`--recipients-max`): un contenitore di Qamille attorno, che scorre;
  - il nome ripetuto nei suggerimenti: si chiede a Sigrid (R11), e non si corregge in Qamille.
- **Le parole:** un `WordsProvider` annidato con `recipientWords(italian)` per il gruppo `labels`.
- Si toglie `styles/ark/recipients.css`, e Qamille non importa più Ark.

**Files:** `desktop/src/ui/RecipientsInput.tsx`, `desktop/src/ds/primitives/RecipientsInput.tsx`, `desktop/src/ui/words.ts` (`recipientWords`, in fondo al file), `desktop/src/ds/styles/ark/recipients.css` (via), i segnaposto «Task 15».

- **Test nuovi:**
  - in `primitives-overlays.mjs`: i suggerimenti dentro un `Modal` si scelgono col mouse e con le frecce;
  - in `recipients.mjs`:
    - l'Invio dell'IME non aggiunge un destinatario;
    - «Ann <a@x.it>» e «a@x.it» non stanno insieme;
    - il testo non ancora diventato chip conta all'invio;
    - un indirizzo senza dominio viene segnato e annunciato.
- **Differenze attese:** chip di Park, X di Sigrid, nome ripetuto (da decidere).
- **Test di flusso (nel Task 16):** `recipients recipients-dark composer-tabs mailto reply-all contacts`.
- **Commit:** `git commit -m "Build the recipients field on Sigrid's ChipsInput"`

### Task 16: unione dell'ondata C e prove della convivenza (controller)

**Decisioni:**
- Si crea `sigrid/wave-c` da `qamille-sigrid`, dopo l'ondata B e un merge di `develop`. I Task 10-15 si uniscono lì, nell'ordine 10, 11, 12, 13, 14, 15. `shortcut-guard.ts` riceve due parti lontane fra loro.
- Su `sigrid/wave-c` l'app intera è coerente: tutte le superfici e i campi con sovrapposizioni stanno su Sigrid, e in `src/` non resta `@ark-ui/*`. In `PENDING` di `only-sigrid.mjs` restano solo i file che usano Motion.
- Solo con tutto verde si porta in `qamille-sigrid`, con un merge solo.

- [ ] **Step 0:** `node tests/sigrid-preconditions.mjs --require P2`.
- [ ] **Step 1:** unire i sei rami, poi togliere da `PENDING` i file di `src/ds/` che non importano più niente di vietato.
- [ ] **Step 2:**
  - tutti i test di flusso dei Task 10-15;
  - `npm run test:browser -- primitives-overlays primitives-select sigrid-coexistence sigrid-leak forced-colors a11y-keyboard`;
  - `node --test tests/shortcut-guard.mjs tests/close-guards.mjs tests/undo-notices.mjs tests/only-sigrid.mjs`;
  - `node tests/build-check.mjs` (React 1, Zag 1);
  - `npm run test:ds`, `npm run check`.
- [ ] **Step 3:** `visual-snapshot` e `visual-compare` con `qamille-sigrid` prima dell'unione. Le immagini nei due schemi vanno al proprietario: dialogo, scheda laterale, menu, select, calendario, avvisi, destinatari.
- [ ] **Step 4:** `git merge --no-ff sigrid/wave-c -m "Move every surface that opens to Sigrid in one step"`; poi si cancellano `sigrid/wave-c` e i rami dei task.

### Task 17: navigazione e aiuti (D)

**Decisioni:**
- **`NavList`/`NavItem`** su quelli di Sigrid, con il tono `sidebar`.
  - La voce attiva di Sigrid ha la barra nell'accento, e per la strada A vince la forma di Sigrid.
  - Ciò che oggi viene da Sidebar 001 di unlumen, cioè la barra che scorre e l'evidenza che segue il puntatore, sparisce.
  - Il conto è il `Count` di Sigrid, in pillola sulla riga.
- **`Sidebar`/`SidebarGroup`** di Sigrid non si usano: `components/Sidebar.tsx` resta di Qamille, composto con `NavList` di Sigrid.
- **Gli aiuti.**
  - `cn` è quello di Sigrid: si toglie `ds/cn.ts`, e con lui `clsx` e `tailwind-merge`.
  - `ime.ts` va in `src/lib/`, perché non è visivo.
  - `focus-return.ts` si toglie, se il Task 14 l'ha reso inutile.
- **`GoogleMark`** diventa `public/brand/google.svg`, mostrato con `<img alt="">` dove oggi c'è il componente (`ConnectMailbox.tsx`, `ServerConnect.tsx`).
- Si tolgono `ds/unlumen/` e `styles/nav.css`.

**Files:** `desktop/src/ui/NavList.tsx`, `desktop/src/ds/primitives/{NavList,GoogleMark}.tsx` (riesportazione o via), `desktop/public/brand/google.svg`, `desktop/src/components/ConnectMailbox.tsx`, `server-connect/ServerConnect.tsx`, `desktop/src/lib/ime.ts` (spostato da `ds/ime.ts`, con gli import), `desktop/src/ds/cn.ts` (via), `desktop/src/ds/unlumen/` (via), `desktop/src/ds/styles/nav.css` (via), i segnaposto «Task 17».

- [ ] **Step 0-6** della procedura comune.
- **Test di flusso:** `npm run test:browser -- sidebar-threads compatto-sidebar a11y-keyboard forced-colors google-sign-in onboarding-professional`.
- **Commit:** `git commit -m "Build the navigation on Sigrid's NavList, and leave unlumen and the helpers Sigrid already has"`

### Task 18: movimento solo da Sigrid (D)

**Decisioni:**
- **Nessun file di Qamille importa più `motion`.**
  - Passano a `useMotion()` di Sigrid (`src/ui/motion.ts`) le animazioni di `App.tsx`, `MailList.tsx`, `UndoStack.tsx`, `ActivityCenter.tsx`, `Composer.tsx`, `ComposerDock.tsx`, `Contacts.tsx`, dei cinque `meetings/*` e di `Reminder.tsx`.
  - `MotionConfig` di `main.tsx` si toglie: la riduzione delle animazioni la gestisce Sigrid.
- **La corrispondenza:**
  - `AnimatePresence` → `Presence`;
  - un elemento di una lista che entra ed esce (`motion.li`, `useIsPresent`) → `ListItem` o `Leaving`;
  - `motion.div` e `motion.span` → `m.div` e `m.span`, con `MOTION.enter`, `MOTION.leave` e `MOTION.slide` al posto delle molle;
  - `layoutId` → `Highlight`;
  - `useReducedMotion` → niente, perché con la riduzione delle animazioni `useMotion()` dà `null`.
- **Con `useMotion()` uguale a `null`** (animazioni spente, riduzione delle animazioni, modulo non ancora arrivato) ogni componente si disegna fermo, con lo stesso DOM e senza un salto quando il modulo arriva.
- **Le righe della posta.** Restano markup dell'app. La loro casella diventa la `Checkbox` di Sigrid (non più quella nativa della decisione 6 dell'ADR-0011 v3), e l'entrata e l'uscita sono `ListItem`. Si misura il costo su una cartella lunga (`done-latency`, `glance-list`).
- **Ciò che Sigrid non ha.** Un'animazione che non si può fare con `MotionImpl`, per esempio un trascinamento elastico o un `animate()` imperativo, si toglie e si chiede a Sigrid. Il task elenca quali.

**Files:** quelli elencati nelle Decisioni, più `desktop/src/ui/motion.ts`, `desktop/src/main.tsx` (via `MotionConfig`), `desktop/tests/only-sigrid.mjs` (`PENDING`, al merge, dal controller).

- [ ] **Step 1: test che falliscono.**
  - `only-sigrid.mjs` senza i file del task in `PENDING`.
  - `build-check.mjs --strict`: una sola copia di Motion.
  - In browser (`thread-rows`, `undo`, `meetings`, `mail-opening`): le righe entrano ed escono con le animazioni accese, e si disegnano ferme con la riduzione delle animazioni. Il primo disegno, prima che arrivi il modulo del movimento, è già quello finale.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare, un file alla volta.
- [ ] **Step 4:** test PASS; `npm run test:browser -- thread-rows undo meetings mail-opening conversation-rows sidebar-threads done-latency glance-list`; `npm run check`.
- [ ] **Step 5:** `visual-compare` con le animazioni spente: nessuna differenza, salvo la casella delle righe. Con le animazioni accese si guarda a vista, e si riporta.
- [ ] **Step 6: Commit** — `git commit -m "Animate Qamille only through Sigrid's motion"`

### Task 19: icone solo da Sigrid (D, P5)

**Decisioni:**
- `src/ui/icon-names.ts` (`ICON_MAP`) porta ogni nome che Qamille usa oggi sul nome di `ICON_NAMES` di Sigrid. L'elenco viene da tutti gli `<Icon name="…">` e dai nomi costruiti nel codice, come le icone delle cartelle. I nomi che Sigrid non ha sono la richiesta P5.
- `src/ui/Icon.tsx` tiene la firma di oggi (`name`, `size`) e disegna con `Icon` di Sigrid. `ds/primitives/Icon.tsx`, con i suoi tracciati, sparisce.
- `SigridRoot` prende `IconPackProvider pack={DEFAULT_ICON_PACK}`: il pacchetto non si sceglie.
- Un nome che Sigrid non disegna diventa uno spazio vuoto della stessa misura. Per questo il task non parte senza P5, e il test fallisce su ogni nome mancante.

**Files:** `desktop/src/ui/Icon.tsx`, `desktop/src/ui/icon-names.ts`, `desktop/src/ds/primitives/Icon.tsx` (riesportazione), `desktop/src/ui/SigridRoot.tsx` (`IconPackProvider`), i segnaposto «Task 19».

- [ ] **Step 0-6** della procedura comune, con `--require P5`. Il test nuovo controlla che ogni nome di `ICON_MAP` porti a un nome di `ICON_NAMES`, e che nella galleria ogni icona disegni un `<svg>` non vuoto.
- **Differenze attese:** le icone di tutta l'app (da decidere).
- **Test di flusso:** `npm run test:browser -- smoke a11y-keyboard sidebar-threads compatto-reader compatto-composer forced-colors`.
- **Commit:** `git commit -m "Draw every icon with Sigrid's Icon"`

### Task 20: via le dipendenze, nomi dei token, via `src/ds`, ADR-0011 v4 (P3, P4)

**Decisioni:**
- **Le dipendenze.** Escono da `desktop/package.json` e dal lockfile `@ark-ui/react`, `motion`, `@internationalized/date`, `clsx`, `tailwind-merge` e `tw-animate-css`.
  - `PENDING` di `only-sigrid.mjs` è vuoto, e il test legge anche `package.json` e `package-lock.json`: nessun pacchetto di `FORBIDDEN` fra le dipendenze dirette di Qamille.
  - `tailwindcss` e `@tailwindcss/vite` restano come strumenti di build finché vale R12.
- **I nomi.** Si riscrivono `var(--ground)` e gli altri nomi di `ALIASES` con quelli di Sigrid, con uno script una tantum, lanciato e non versionato. `paintWindowBackground` legge `--color-paper`.
- **Gli alias e i token di Qamille.** Gli alias e `ALIASES` spariscono, e i token solo di Qamille vanno in `desktop/src/theme/qamille-tokens.css`. Con P3 i raggi passano come parametri di `COMPATTO`; con P4 i titoli diventano gradini di Sigrid.
- **I percorsi.** Gli import `ds/primitives/<X>` diventano `ui/<X>`, poi si cancella `desktop/src/ds/`.
  - `system.css` diventa `src/theme/base.css`, solo `@layer base`.
  - `forced-colors.css` va in `src/theme/`, sempre importato per ultimo.
- **ADR-0011 v4** (`docs/adr/ADR-0011-v4-modulo-sistema-visivo.md`, che sostituisce la v3), più l'indice di `docs/adr/README.md`.
  - **La regola.** Qamille si basa solo su Sigrid: ogni comportamento, primitiva, icona, carattere e animazione viene da Sigrid, preso con `sigrid:sync` senza pin.
    - Qamille non ha dipendenze dirette su librerie che Sigrid porta (`FORBIDDEN` di `only-sigrid.mjs`).
    - Ciò che manca si chiede a Sigrid, e non si scrive in Qamille.
    - Dentro Sigrid ci sono Base UI e Ark, per scelta del proprietario scritta nel README di Sigrid.
  - `src/ui/` è il solo consumatore di Sigrid.
  - I parametri sono `COMPATTO` e `QAMILLE_DENSITY`, e i token di Qamille stanno in `qamille-tokens.css`.
  - Le invarianti:
    - INV-001: `src/ui/` non importa da `components/`;
    - INV-002 e INV-003: riscritte per `src/ui/`;
    - INV-004: token di Sigrid o di Qamille, mai letterali;
    - INV-006: una sola implementazione per componente;
    - INV-007: nessun file di Qamille importa un pacchetto di `FORBIDDEN`, e nessuno di quei pacchetti sta in `package.json` (`only-sigrid.mjs`);
    - INV-009: solo `src/ui/` e `src/theme/` importano `@acaprino/sigrid`;
    - INV-010: il codice dell'app riconosce i controlli da ruoli e `data-slot`, mai dalle parti o dagli stati di una libreria;
    - INV-008 cade.
  - Cadono anche le eccezioni della v3: le righe della lista ora usano `ListItem` e `Checkbox` di Sigrid; `UndoStack` resta un componente di Qamille fuori da Toast, ma animato da Sigrid.
- **I test.**
  - `ds-usage.mjs` diventa `ui-usage.mjs`.
  - `style-tokens.mjs` ammette i nomi di Sigrid e quelli di `qamille-tokens.css`.
  - `native-feel.mjs` prende i percorsi nuovi.
  - `test:ds` diventa `test:ui`.
- **Gli strumenti di `ds`.** Si ritirano `ds:export`, `scripts/ds-export*`, `scripts/doop-sync/` e `tests/ds-export.mjs`.
- **La guida.** `desktop/src/ui/README.md` dice che cosa è di Qamille e che cosa è di Sigrid, con la regola in testa.

**Files:** `desktop/package.json`, `package-lock.json`; tutto `desktop/src/ds/` (via); `desktop/src/theme/{qamille-tokens,base,forced-colors}.css`; `desktop/src/ui/README.md`; i `.css` e i `.tsx` dell'app; `desktop/src/main.tsx`; `desktop/src/reminder/reminder.css`; `desktop/src/lib/native-window.ts`; `desktop/tests/{only-sigrid,ui-usage,style-tokens,tokens-compatto,contrast,native-feel}.mjs`; `desktop/scripts/test-browser.mjs`; `docs/adr/ADR-0011-v4-modulo-sistema-visivo.md`, `ADR-0011-v3-modulo-sistema-visivo.md` (`superseded_by`), `docs/adr/README.md`; `docs/aspetto-nativo.md`; `AGENTS.md`.

- [ ] **Step 0:** `node tests/sigrid-preconditions.mjs --require P3,P4`.
- [ ] **Step 1: test che falliscono.**
  - `only-sigrid.mjs`, con `PENDING` vuoto e il controllo di `package.json`;
  - `ui-usage.mjs`;
  - `style-tokens.mjs`, con un nome di alias vietato;
  - `tokens-compatto.mjs`, con i raggi;
  - `native-feel.mjs`, con i percorsi nuovi;
  - `test('src/ds is gone')`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare, nell'ordine:
  1. i nomi;
  2. `qamille-tokens.css`, P3 e P4, e poi `npm run theme:css`;
  3. gli import;
  4. la cancellazione di `ds/`;
  5. le dipendenze, e `npm install`, che aggiorna il lockfile;
  6. i test;
  7. i documenti.
- [ ] **Step 4:** `npm ci` da zero; `npm run build`; `node tests/native-feel.mjs`; `node tests/build-check.mjs --strict`; `node tests/first-paint.mjs`; `node --test tests/only-sigrid.mjs`; `npm run test:browser` (tutti).
- [ ] **Step 5:** `visual-compare` con la base del ramo: nessuna differenza, salvo i titoli che passano ai gradini di Sigrid (P4), elencati con la misura di prima e di dopo.
- [ ] **Step 6: Commit** — `git commit -m "Depend only on Sigrid: drop Ark, Motion and the helpers from Qamille, use Sigrid's token names, and retire src/ds (ADR-0011 v4)"`

### Task 21: verifica sull'exe e approvazione del proprietario

- [ ] **Step 1:** `build-release.bat --no-install`. Controllare che l'exe contenga `theme-boot.js`, e che `public/sigrid-licenses/provenance.json` corrisponda alla copia sincronizzata.
- [ ] **Step 2:** con Qamille chiuso, sull'exe di `release/`, con cartelle dati di prova: `node tests/release-theme-probe.mjs` nei due schemi, `node tests/window-native.mjs`, `node tests/a11y-native.mjs`.
- [ ] **Step 3:** `visual-compare` fra `capture/sigrid-baseline` e lo stato finale, raggruppato per riga di «Che cosa cambierà a vista». Gli screenshot nei due schemi vanno al proprietario: posta, lettore, composer con i destinatari, calendario con un evento, rubrica, ricerca con le date, impostazioni, un dialogo, un menu, gli avvisi con un errore, il promemoria. È la «Confirmation» dell'ADR-0011 v4.
- [ ] **Step 4:** `npm run test:browser` (tutti), `node tests/native-feel.mjs`, `npm run test:ui`, `node tests/build-check.mjs --strict`, `node tests/first-paint.mjs`, `node --test tests/only-sigrid.mjs tests/sigrid-wiring.mjs tests/sigrid-words.mjs tests/sigrid-preconditions.mjs tests/sigrid-sync.mjs`. I file nuovi entrano in `NODE_TEST`.
- [ ] **Step 5: Commit** — `git commit -m "Record Qamille on Sigrid and its checks on the release build"`. Poi `qamille-sigrid` torna in `develop`, con il consenso del proprietario.

---

## Decisioni che il proprietario potrebbe voler cambiare

1. **Le differenze che la spec non elenca.** Sono nella tabella «da decidere», e vengono da Sigrid 3.0.0 o dalla regola «solo Sigrid».
   - **Dalla regola:**
     - le icone di tutta l'app disegnate da Sigrid;
     - il carattere senza i pesi 550, 650 e 700 (R13);
     - la casella delle righe della posta;
     - tutte le animazioni con le transizioni di Sigrid.
   - **Da Sigrid 3.0.0:**
     - il bordo chiaro dei campi, sotto 3:1 in scuro (R5);
     - la riga scelta quasi invisibile in scuro (R9);
     - la data che non si scrive e non si toglie dal calendario (R6);
     - l'interruttore di 40×20;
     - le schede tinte;
     - i colori di stato del motore;
     - le barre di scorrimento a pillola;
     - i colori che sfumano in tutta l'app;
     - gli errori dei campi in un riquadro.

   Per ognuna l'alternativa è una richiesta a Sigrid, mai una correzione in Qamille.
2. **Il cambio sta su un ramo solo, che torna in `develop` dopo il Task 20,** quando Qamille non ha più Ark né Motion propri. Dentro il ramo, le superfici che si aprono passano insieme (Task 16).
   - L'alternativa è unire in `develop` un'ondata alla volta: un ramo più corto, ma `develop` e le release avrebbero per settimane due copie di Zag e di Motion, e due librerie di sovrapposizioni.
3. **Le icone aspettano Sigrid (P5).** Finché Sigrid non mette in `ICON_NAMES` i significati che usa Qamille, il Task 19 non parte, e le icone restano di Qamille.
   - L'alternativa è che Sigrid accetti un disegno SVG anche per la linea (`IconDrawing.lucide`), e che Qamille passi i suoi tracciati. Ma così i disegni restano in Qamille, contro la regola.
4. **Il marchio di Google resta un'immagine** (`public/brand/google.svg`): è un logo richiesto dalle linee guida di Google, non un'icona. L'alternativa è chiederlo a Sigrid come componente.
5. **Tailwind resta come strumento di build** finché Sigrid non ha primitive e uno `styles.css` senza preflight (R12). Dopo, Qamille userebbe lo `styles.css` già compilato, e toglierebbe anche `tailwindcss` e `@tailwindcss/vite`.
6. **`sigrid:sync` in `predev` e `prebuild`**, quindi non più manuale (cambia `AGENTS.md`). Una release di Sigrid che cambia i colori di «Compatto» ferma la build finché qualcuno non rigenera `compatto.css` e ne guarda il diff.
7. **Avvisi:**
   - gli errori restano finché non li si chiude;
   - gli altri avvisi durano 10 s;
   - un errore si annuncia come `status` (R8);
   - la «×» ha un nome inglese finché manca R10.
8. **`UndoStack` resta un componente di Qamille**, animato da Sigrid, sotto la colonna degli avvisi. In alternativa gli annullamenti diventano avvisi di Sigrid, con «Annulla» e il tasto Z.
9. **`ds:export` e la guida Doop si ritirano** (Task 20).
