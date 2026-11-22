# Qamille su Sigrid (sotto-progetto 4): piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Qamille prende colori, densità, parole, primitive e movimento da Sigrid (`acaprino/sigrid`, `@acaprino/sigrid`), tiene come parametri i colori «Compatto», IBM Plex Sans, la densità compatta e i raggi, e cambia a vista solo nelle «Differenze accettate» della spec e in quelle che il proprietario accetta dall'elenco «Che cosa cambierà a vista». Alla fine `desktop/src/ds/` non c'è più: restano in Qamille solo i suoi componenti di prodotto e i suoi token propri.

**Architecture:**
- **Il canale.** `npm run sigrid:sync` (già scritto) mette l'ultima release del `main` di Sigrid in `desktop/vendor/sigrid/`, git-ignorata. Qamille la raggiunge con un alias di Vite e un `paths` di TypeScript, senza una voce in `package.json`: nessun pin, e il lockfile non cambia a ogni release di Sigrid. Il sync entra in `predev` e `prebuild`, come `artcraft:sync`, e nella CI.
- **Il tema.** Il tema «Compatto» è una `ThemeDef` di Qamille (`desktop/src/theme/compatto.ts`) con la densità (`DensityDef`). Uno script fa calcolare al motore di Sigrid i token dei due schemi e li scrive in `desktop/src/theme/compatto.css`, file generato e versionato. Il primo disegno resta CSS puro: `theme-boot.js` scrive solo `data-scheme`, e i blocchi di `compatto.css` sono già nel foglio. Un test confronta il file con il motore della copia sincronizzata: una release di Sigrid che cambia un colore di Qamille ferma la build finché qualcuno non rigenera il file e guarda la differenza.
- **Gli alias.** Durante la migrazione i nomi di Qamille (`--ground`, `--text`, `--muted`…) sono alias dei token di Sigrid. Alla fine il codice usa i nomi di Sigrid, e in un file di Qamille restano solo i token che Sigrid non definisce.
- **Il modulo dell'app.** `desktop/src/ui/` è il «pacchetto di prodotto» di Qamille su Sigrid. Contiene:
  - un adattatore sottile per ogni primitiva, che tiene le props di oggi di Qamille e le traduce in quelle di Sigrid;
  - i componenti di prodotto (`RecipientsInput`, `NavList`, `Icon`, `GoogleMark`);
  - le parole (`words.ts`) e la radice (`SigridRoot`).

  Solo `src/ui/` e `src/theme/` importano `@acaprino/sigrid`. Così una release di Sigrid che rinomina una prop rompe un solo file, e lo dice `tsc`.
- **Le primitive.** Passano una alla volta. Durante il passaggio `ds/primitives/<X>.tsx` diventa una riga che riesporta `ui/<X>`, quindi i punti d'uso non cambiano. Insieme se ne vanno il foglio `ds/styles/ark/<x>.css`, la sezione del README e le copie di unlumen che servivano solo a quella primitiva.

**Tech Stack:** Sigrid 2.1.0 alla data del piano (ultima release, commit `e7b74a6`; il piano non la fissa), React 19.3.1, @ark-ui/react 5.39.3, motion 14.7.1, Tailwind 4 con `@tailwindcss/vite`, Vite 8, TypeScript 5.9. Test: `node --test`, Playwright con Edge (`npm run test:browser -- <nomi>`), `tests/visual-snapshot.mjs` e `tests/visual-compare.mjs`, `node tests/native-feel.mjs`, `tests/release-theme-probe.mjs` sull'exe di release.

**Spec:** `docs/superpowers/specs/2026-11-20-toolkit-qamille-design.md`, intera: decisioni del proprietario, strada A, «Differenze accettate», «Scelte del costruttore già fatte», sotto-progetto 4. Sigrid: `desktop/vendor/sigrid/README.md`, `dist/*.d.ts`, `src/`. Qamille: `desktop/src/ds/README.md`, `docs/adr/ADR-0011-v3-modulo-sistema-visivo.md`, `docs/aspetto-nativo.md`, `AGENTS.md`.

## Prima di cominciare: che cosa manca in Sigrid 2.1.0

La spec dava i sotto-progetti 1-3 come base di questo. La copia sincronizzata (v2.1.0) li ha solo in parte. Le misure qui sotto sono state prese sulla copia in `desktop/vendor/sigrid`, il 2026-11-22, prima su v2.0.1 e poi di nuovo su v2.1.0.

Rispetto alla 2.0.1, la 2.1.0 porta tre cose nuove:
- un avviso può essere un errore (`ToastItem.kind: 'error'`, vedi Task 7);
- il campo per un intervallo di date (`DateRangeField`) e le scelte rapide (`dayPicks`, `rangePicks`);
- `Sidebar`/`SidebarGroup`.

Le parole nuove sono `common.error`, `date.noRange`, `date.clearRange`, `date.today`, `date.tomorrow`, `date.weekend`, `date.nextDays` e `sidebar.*`. Il motore dei temi, la densità e `theme.css` non cambiano: i valori di «Compatto» sono gli stessi. Nessuno dei quattro prerequisiti qui sotto è cambiato.

| # | Che cosa chiede la spec | Che cosa ha v2.1.0 | Che cosa aspetta in Qamille |
|---|---|---|---|
| **P1** | React, Ark e Motion fuori dal bundle, come peer dependency a versione esatta; una sola versione per dipendenza nelle due app («Vincoli», «Versioni»). | `dist/index.js` pesa 2,2 MB e importa da fuori solo `react`, `react/jsx-runtime` e `react-dom`. Dentro ci sono Ark, Zag, Base UI, lucide e le icone Fluent; Motion sta in un chunk pigro (`motion-impl`, 388 KB). `peerDependencies` ha solo `react` e `react-dom` a intervallo, mentre `dependencies` ha `motion` 14.6.0 (Qamille ha 14.7.1) e diversi `^`. | Ondate B, C e D: ogni primitiva porterebbe una seconda copia di Zag accanto a quella di Qamille (anti-obiettivo), con due pile di livelli che non si vedono (vedi Review Focus 3). |
| **P2** | Tutto su Ark: Base UI esce dalle dipendenze (obiettivo 4). | Base UI sta ancora sotto `Dialog`, `Sheet`, `Select`, `Popover`, `DropdownMenu`, `Checkbox`, `Tooltip`, `RadioGroup`, `ToggleGroup`, `Input`, `Textarea`, `Collapsible`, `Field`, `PopoverPrimitive`, e sotto `Button` e `Badge` con `useRender`. Il loro DOM ha `data-slot`, non `[data-scope][data-part]`. | Ondata C. Migrarle adesso vorrebbe dire riscrivere i test due volte e mettere nel bundle una trappola del focus di Base UI accanto a quella di Ark. |
| **P3** | Raggi, ombre e carattere come parametri della definizione del tema (obiettivo 2). | La forma è fissa (`shapeOf`: raggi 4/6/10/10, pulsante 6, chip 6, elevazione di Park), e `ThemeDef` non ha campi di forma. Il carattere va già bene: il predefinito è IBM Plex Sans. | Solo il Task 16: finché manca, i raggi 4/6/8/12 restano token di Qamille, che vincono su quelli di Sigrid. Sono due fonti per lo stesso token, ammesse finché la migrazione non è finita e mai dopo. |
| **P4** | I titoli di Qamille fino a 26 px come gradini in più della scala di Sigrid («Differenze accettate», scala del testo). | La scala si ferma a `base` (16 px): `2xs` 12, `xs` 13, `sm` 14, `base` 16. | Solo il Task 16: fino ad allora `--text-lg`, `--text-xl`, `--text-2xl` e `--text-3xl` (15, 18, 20, 26 px) restano token di Qamille. |

Già pronto in v2.1.0: il motore dei temi (`themeTokens`, `themeVariables`, `ThemeDef` con `surfaces`, `sidebar: 'scura'`), la densità (`densityWith`; `compact` è 32/28/24 px, non 36/32/28 come dice ancora il README di Sigrid), le parole (`WordsProvider`, `DEFAULT_WORDS`), e su Ark `Switch`, `Tabs`, `Slider`, `Toasts`, `DateField`/`Calendar`, `Combobox`, `ChipsInput`, `Avatar`.

**Come si procede:**
- **Il controllo.** Il Task 0 scrive `tests/sigrid-preconditions.mjs`, che dice quali di P1-P4 valgono sulla copia sincronizzata. Ogni task che ne dipende lo lancia al suo Step 0 e non parte se fallisce.
- **Ondata A.** Canale, tema e parole non dipendono da P1-P4: si può unire in `develop` da sola.
- **Lo stop.** Se alla fine dell'ondata A manca P1, il controller si ferma e lo dice al proprietario. Le modifiche vanno fatte in `acaprino/sigrid`, mai in Qamille.

## Global Constraints

- **Rami.** Ramo `qamille-sigrid`, da `develop`, che torna in `develop` e poi si cancella (`AGENTS.md`).
  - I rami dei worktree sono solo locali (`sigrid/task-N`, da `qamille-sigrid`), non si pubblicano e si cancellano subito dopo il merge.
  - Mai `main`, mai un cambio di versione, mai un tag.
- **Solo il repository ufficiale.** Sigrid arriva solo da `npm run sigrid:sync`.
  - Nessun file di `desktop/vendor/sigrid/` si modifica a mano.
  - `SIGRID_DIR` serve solo a provare in locale una modifica di Sigrid non ancora rilasciata: nessun commit di Qamille dipende da codice di Sigrid non rilasciato.
- **Nessun pin di Sigrid.**
  - Nessuna voce `@acaprino/sigrid` in `package.json` o nel lockfile.
  - Nessuna versione o commit di Sigrid scritti nel codice o nei test di Qamille: chi deve saperli legge `vendor/sigrid/provenance.json`.
  - `compatto.css` non porta la versione nell'intestazione, altrimenti ogni release cambierebbe il file.
- **Dipendenze npm** a versione esatta, senza `^`.
  - Una dipendenza nuova è ammessa solo se è una peer dependency esatta di Sigrid, che `sigrid:sync` (`checkPeers`) vuole uguale a quella di Qamille.
  - Niente `@base-ui/*`, `lucide-react`, `@fluentui/*` in Qamille.
- **Confini del codice.**
  - Solo `desktop/src/ui/**` e `desktop/src/theme/**` importano `@acaprino/sigrid`.
  - Fino al Task 16 i componenti dell'app importano ancora `ds/primitives/*`, che diventano riesportazioni.
  - `@ark-ui/*` resta solo in `desktop/src/ds/` fino al Task 15, poi solo in `desktop/src/ui/`.
- **Parametri di Qamille.**
  - Colori «Compatto», IBM Plex Sans (la copia variabile di `public/fonts`, perché Qamille usa i pesi 550, 650 e 700), densità compatta e raggi 4/6/8/12.
  - L'utente sceglie solo chiaro, scuro o sistema: nessun `ThemeGallery`, `LookSettings`, `DensityChoice` o `IconPackChoice` di Sigrid nell'interfaccia.
- **Strada A, e quando fermarsi.**
  - Forma, testo e movimento sono quelli di Sigrid.
  - Una differenza a vista che non sta né nelle «Differenze accettate» della spec né in «Che cosa cambierà a vista» ferma il task: si riporta al controller.
  - Una differenza non si «corregge» mai con una regola di Qamille sopra le classi di Sigrid: sarebbe una seconda fonte. Si chiede a Sigrid un parametro.
- **Due lingue.** Ogni testo visibile o accessibile è in italiano e in inglese.
  - Nel codice dell'app con `tr('italiano', 'English')`.
  - Le parole che le primitive dicono da sé passano solo da `src/ui/words.ts`.
  - Una prop di Qamille (`placeholder`, `clearLabel`, `aria-label`) vince sul dizionario.
- **Valori visivi solo da token** (ADR-0009): `node tests/style-tokens.mjs` resta verde. `compatto.css` è generato ed è esente, perché è la fonte.
- **Aspetto nativo.**
  - Dopo ogni modifica a CSS globali, `theme.css`, `compatto.css`, `theme-boot.js`, `index.html`, `reminder.html` o al foglio di Tailwind: `node tests/native-feel.mjs`, tutto `ok`.
  - Restano verdi `npm run test:browser -- forced-colors`, `node tests/contrast.mjs light dark` e `npm run test:ds`.
  - Questo piano non tocca Rust. Se `--color-paper` cambiasse, si aggiornano anche `backgroundColor` in `tauri.conf.json` e `DARK_GROUND` in `native_feel.rs`, e si lancia `node tests/webview-recovery-native.mjs` (`AGENTS.md`).
- **Ordine dei passi.** Ogni task scrive prima i test che falliscono, poi implementa, li vede passare e fa il commit (`superpowers:test-driven-development`).
  - Prima del commit di una primitiva: `visual-snapshot` sulla base del ramo e dopo la modifica, poi `visual-compare`. Le differenze devono stare tutte nell'elenco del task.
  - `visual-snapshot.mjs` usa la porta fissa 28176: nei worktree si lancia uno alla volta.
- **Worktree in parallelo.** Ognuno ha il suo `npm ci` e il suo `npm run sigrid:sync` in `desktop/`, perché la copia di Sigrid è git-ignorata. Tutti i worktree di un'ondata usano la stessa release: il controller la annota all'apertura dell'ondata, e chi ne trova una più nuova si ferma e lo dice.
- **Documenti.** Commenti nel codice in inglese.
  - Ogni primitiva sostituisce la sua sezione di `desktop/src/ds/README.md` con un rinvio di due righe a `src/ui/<X>.tsx` e alla primitiva di Sigrid.
  - `docs/aspetto-nativo.md` lo toccano solo i Task 2 e 16.
- **Build di release** (Task 17): `build-release.bat --no-install`, oppure `npm run build` in `desktop/` e poi `cargo build --manifest-path desktop/src-tauri/Cargo.toml --release --features custom-protocol --locked --jobs 1`. Prima di usarlo, l'exe deve contenere `theme-boot.js`.

## Lavoro in parallelo

### Ondate

| Ondata | Task | Dipende da |
|---|---|---|
| 0 (controller, seriale) | 0. Punto di partenza, controllo dei prerequisiti, punti d'innesto | — |
| A | 1. Sigrid in sviluppo, test, CI e release; poi in parallelo 2. «Compatto» come parametri e primo disegno · 3. Le parole | 0; 2 e 3 dopo 1 unito |
| B (P1) | 4. Switch · 5. Tabs · 6. Count e Slider · 7. Toaster · 8. DateField, in parallelo | A unita, P1 |
| C1 (P1, P2) | 9. Button e Tooltip · 10. Checkbox · 11. Field · 12. Select, in parallelo | B unita, P2 |
| C2 (P1, P2) | 13. Popover e Menu, poi 14. Modal e SidePanel (in serie: ogni sovrapposizione sta dentro un dialogo, e il dialogo va per ultimo) | C1 unita |
| D | 15. Componenti di prodotto su Sigrid; poi 16. Nomi dei token, fine degli alias, via `src/ds`, ADR-0011 v4 (P3, P4) | C2 unita |
| Finale | 17. Verifica sull'exe e approvazione del proprietario | tutti |

Si unisce un task alla volta, ognuno con `npm run test:ds`, `npm run check` e i test del task verdi sul ramo unito.

### File condivisi: proprietario e punto d'innesto

| File | Proprietario | Chi altro lo tocca, e dove |
|---|---|---|
| `desktop/package.json`, `package-lock.json` | Task 1 | Task 2: una riga di script (`theme:css`), dopo il Task 1. Task 16: le dipendenze che nessuno importa più. |
| `desktop/vite.config.ts`, `desktop/tsconfig.json`, `.github/workflows/ci.yml`, `.gitignore`, `AGENTS.md`, `desktop/scripts/sync-sigrid.mjs`, `desktop/scripts/test-browser.mjs` | Task 1 | Task 2: una riga in `OTHERS` di `test-browser.mjs` (`first-paint`). Task 16: `OTHERS` (`ds-export` via). Task 17: solo la lista `NODE_TEST`. |
| `desktop/src/main.tsx` | Task 1 | Task 16: il percorso di `forced-colors.css`. |
| `desktop/src/ui/SigridRoot.tsx` (nuovo) | Task 1 | Task 3: solo le parole. |
| `desktop/src/ds/tokens/theme.css`, `desktop/src/ds/styles/tailwind.css`, `public/theme-boot.js`, `index.html`, `reminder.html` | Task 2 | Task 16 li sposta o li cancella. Nessun task di primitiva li tocca: i token rimasti senza uso si tolgono nel Task 16. |
| `desktop/src/ds/styles/system.css` | Task 2 (base in `@layer base`) | Task 9 solo la sezione dei pulsanti, Task 11 solo quella del campo, separate dai commenti del Task 0. Task 16 lo cancella. |
| `desktop/src/ds/styles/ark.css` | Task 0 (un `@import` per blocco, separati da una riga vuota e da `/* Task N */`) | Ogni task di primitiva toglie solo il suo blocco. |
| `desktop/src/ds/styles/forced-colors.css` | Task 0 (una sezione per primitiva, con `/* Task N: … */`) | Ogni task riscrive solo la sua sezione sulle parti di Sigrid. Task 16 sposta il file. |
| `desktop/tests/support/Gallery.tsx`, `tests/primitives-controls.mjs`, `tests/primitives-overlays.mjs`, `tests/sigrid-leak.mjs` | Task 0 (un segnaposto per primitiva) | Ogni task scrive solo sotto il suo segnaposto. |
| `desktop/src/ds/README.md` | nessuno (solo innesti) | Ogni task la sezione della sua primitiva, che è lontana dalle altre. Task 16 cancella il file. |
| `desktop/src/App.tsx` | nessuno (solo innesti) | Task 2: la riga dell'effetto del tema (`:1506`). Task 7: solo se il montaggio del Toaster cambia (`<Toaster`). |
| `desktop/src/styles.css`, i `.css` dei componenti | nessuno (solo innesti) | Task 2: `[data-theme='dark']` → `[data-scheme='dark']`, in tutti. Ogni task di primitiva tocca solo le regole che nominano le classi o le parti della sua primitiva. Task 16: i nomi dei token. Nessuna riformattazione. |
| `desktop/tests/native-feel.mjs` | Task 2 (controlli 3, 4 e 29) | Task 16: i percorsi nei controlli 1, 2, 4, 11 e 14. |
| `docs/aspetto-nativo.md` | Task 2 (regole 3, 5, 6) | Task 16: i percorsi delle regole 1, 2, 3, 18, 22, 24 e della checklist. |
| `desktop/tests/ds-usage.mjs` | Task 1 (INV-009) | Task 15: `@ark-ui/*` ammesso solo in `src/ui/`. Task 16 lo riscrive come `ui-usage.mjs`. |
| `desktop/tests/compatto-*.mjs`, `tests/support/compatto.mjs`, `tests/support/theme-tokens.mjs` | Task 2 (scala del testo, colori, `data-scheme`) | Ogni task di primitiva: solo i casi della sua primitiva. |

## Review Focus

1. **Il primo disegno non lampeggia, e la finestra non ha una striscia di un altro colore.**
   - In tema scuro, o con «Sistema» su un Windows scuro, il primo fotogramma è già scuro e lo sfondo della finestra nativa è `--color-paper`.
   - Il rischio:
     - blocchi di `compatto.css` legati a un attributo che `theme-boot.js` non scrive (`data-theme` invece di `data-scheme`);
     - un foglio che arriva dopo il primo disegno;
     - il blocco chiaro di riserva di Sigrid (`BASE_THEME`, grigio) che vince su quello di Qamille.
   - Test: `native-feel.mjs` 3 (`theme-boot.js` scrive `data-scheme` con la stessa regola di `resolvedAppearance`) e 4 (`backgroundColor` e `DARK_GROUND` uguali a `--color-paper` dei due blocchi generati).
   - Test: `tests/first-paint.mjs` (Task 2), sulla build di produzione (`vite build` e `vite preview`: in sviluppo il CSS lo inietta il JavaScript, quindi non conta). La pagina ha lo schema scuro emulato e il chunk JavaScript d'ingresso bloccato, quindi solo HTML, il `<link>` del CSS e `theme-boot.js`. Il fondo calcolato di `body` è `--color-paper` scuro.
   - Sull'exe: `release-theme-probe.mjs` nei due schemi (Task 17).
2. **Le regole di Qamille non vincono sulle classi di Sigrid.**
   - Le regole di elemento di `system.css` sono fuori dai layer: `button { background: none }` batte `bg-primary` di `@layer utilities`, e un pulsante primario di Sigrid resterebbe senza fondo. Lo stesso vale per `input { padding; border }`.
   - Le regole di `styles/ark/*.css` scelgono `[data-scope][data-part]` in tutta la pagina, e colpiscono anche le parti di Ark delle primitive di Sigrid.
   - Le regole di pagina di `styles.css` che puntano a `button` o `input` dentro un contenitore fanno lo stesso.
   - Test: `tests/sigrid-leak.mjs` (Task 0, un caso per primitiva). Monta ogni primitiva migrata due volte: in una pagina con il solo CSS di Sigrid più `compatto.css`, e nella pagina di Qamille. Gli stili calcolati di ogni parte (`[data-slot]`, `[data-part]`) devono coincidere, salvo i token di Qamille che sono parametri (font, raggi finché manca P3).
   - Test: il Task 2 verifica che ogni regola di elemento di `system.css` stia in `@layer base`.
3. **Due copie di Ark o di Zag: un popup dentro un dialogo lo chiude.**
   - Con il bundle di Sigrid 2.1.0, una primitiva di Sigrid con un livello (il calendario di `DateField`, un `Select`) aperta dentro un `Modal` di Qamille è un livello che la copia di Zag di Qamille non vede. Un clic su un giorno o su una voce è «fuori» per il dialogo, che si chiude. Le due trappole del focus si contendono il focus.
   - Test: `tests/build-check.mjs` (Task 1). Su `vite build` conta le copie di Zag, React e Motion; Zag e React devono essere 1 dal Task 1, Motion da quando vale P1 (`--strict`).
   - Test: in `primitives-overlays.mjs`, «una data, una voce di Select e una voce di Menu scelte dentro un Modal lasciano il Modal aperto, e il focus resta nel Modal» (Task 8, 12 e 13, ciascuno con il suo caso).
4. **Parole inglesi in un'interfaccia italiana.**
   - Il pulsante che chiude un dialogo letto «Close», la regione degli avvisi «Notifications», il calendario con «Clear» e «Previous month», un'etichetta tolta annunciata in inglese.
   - Peggio: le parole restano italiane dopo il passaggio all'inglese, perché il dizionario non si rifà al cambio di lingua.
   - Test: `tests/sigrid-words.mjs` (Task 3, `node --test`). Ogni chiave di `DEFAULT_WORDS` ha la sua parola italiana; ogni funzione dà un testo non vuoto e diverso dall'inglese, con i plurali giusti per 1 e per 3; una chiave nuova di una release di Sigrid fa fallire il test, invece di restare inglese.
   - Test: in browser (Task 3, `primitives-overlays.mjs`), si cambia lingua e il nome della regione degli avvisi di Sigrid cambia senza ricaricare. Il Task 14 aggiunge lo stesso caso per il pulsante che chiude un `Dialog`.
5. **I comportamenti cambiano sotto i controlli, senza che si veda nel codice dell'app.**
   - **Un modulo che non invia più.** Il `Button` di Sigrid ha `type="button"` per default, quello di Qamille è un `<button>` semplice, che dentro un `<form>` invia. Un pulsante d'invio senza `type="submit"` (accesso, server, password, registrazione) smetterebbe di inviare, col clic e con Invio.
   - **L'Esc dell'IME che chiude un dialogo.** L'Esc che annulla un candidato dell'IME non deve chiudere un `Dialog` o uno `Sheet` di Sigrid (regola 24): lì l'Esc lo gestisce Zag, non un gestore di Qamille.
   - **La mano e il divieto sui controlli.** Sigrid usa `cursor-pointer` (`switch.tsx`) e `cursor-not-allowed` (`checkbox`, `input`, `radio-group`, `select`, `switch`, `textarea`). La regola 3 vuole la freccia, e il controllo 2 di `native-feel.mjs` legge solo `src/`, quindi non le vede.
   - Test: Task 9. Prima del cambio, ogni `<form>` di `src/` e il suo pulsante d'invio: ognuno prende `type="submit"` esplicito. Poi, in `onboarding-professional`, `server-connect`, `password-reset` e `signup-professional`, Invio nel campo e clic sul pulsante inviano.
   - Test: in `primitives-overlays.mjs` (Task 14), una composizione IME aperta in un campo di un `Modal`: Esc annulla la composizione e il `Modal` resta aperto. Il secondo Esc lo chiude.
   - Test: `build-check.mjs` (Task 1) legge il CSS prodotto da `vite build`: niente `cursor: pointer`, `cursor: not-allowed` o `cursor: progress`. Il controllo 29 di `native-feel.mjs` (Task 2) verifica che `tailwind.css` escluda quelle utility.

## Interfacce tra i task

| Nome | Firma | Da | Per |
|---|---|---|---|
| alias `@acaprino/sigrid` | Vite `resolve.alias` `^@acaprino\/sigrid$` → `vendor/sigrid/dist/index.js`; tsconfig `paths` → `vendor/sigrid/dist/index.d.ts` | 1 | tutti |
| CSS di Sigrid | `@import` relativo di `vendor/sigrid/src/styles/theme.css` in `ds/styles/tailwind.css`, con `@source` su `vendor/sigrid/src` e `@source not` su `internal/motion-impl.tsx` | 2 | 4-15 |
| `SigridRoot` | `export function SigridRoot({ children }: { children: ReactNode })`: `WordsProvider` e `MotionProvider animated`, montata in `main.tsx` attorno ad `App` | 1 (vuota), 3 (parole) | — |
| `checkPreconditions` | `export function checkPreconditions(dir: string): Record<'P1' \| 'P2' \| 'P3' \| 'P4', { ok: boolean; why: string }>`; dalla riga di comando `node tests/sigrid-preconditions.mjs --require P1,P2` | 0 | Step 0 dei task 4-16 |
| `COMPATTO` | `export const COMPATTO: ThemeDef` (`desktop/src/theme/compatto.ts`) | 2 | 16, 17 |
| `COMPATTO_TONE` | `export const COMPATTO_TONE: ThemeTone = { intensity: 'theme', light: 'theme' }` | 2 | 16 |
| `QAMILLE_DENSITY` | `export const QAMILLE_DENSITY: DensityDef = densityWith('compact', { id: 'compatto', control: { sm: 32, xs: 32 } })` | 2 | 16 |
| `ALIASES` | `export const ALIASES: Readonly<Record<string, string>>`, nome di Qamille → nome di Sigrid (`'--ground': '--color-paper'`) | 2 | 16 (lo usa per riscrivere i nomi e poi lo cancella) |
| `compattoCss` | `export function compattoCss(sigrid): string`, con `sigrid` il modulo di `vendor/sigrid/dist/index.js` (`scripts/theme-css.mjs`); `npm run theme:css` scrive `src/theme/compatto.css` | 2 | `tests/tokens-compatto.mjs` |
| attributi della radice | `<html data-theme="compatto" data-density="compatto" …>` statici in `index.html` e `reminder.html` (anche quelli di `ThemeTokens.attributes`); `data-scheme` da `theme-boot.js`, dall'effetto del tema in `App.tsx` e da `Reminder.tsx` | 2 | tutti |
| `qamilleWords` | `export function qamilleWords(italian: boolean): Words` (`src/ui/words.ts`), completo in italiano | 3 | 7, 8, 13, 14, 15 |
| adattatori | `src/ui/<X>.tsx` con la stessa default export e lo stesso tipo di props di `ds/primitives/<X>.tsx`; `ds/primitives/<X>.tsx` = `export { default } from '../../ui/<X>'` più i tipi | 4-14 | 16 |
| `notify` | invariato: `notify(message: string, error = false, actions: NoticeAction[] = [])` | 7 | app |
| `compareWithCleanRoom` | `export async function compareWithCleanRoom(page, which: string, props?: object): Promise<string[]>`: le differenze di stile, vuote se non ce ne sono (`tests/support/sigrid-leak.mjs`) | 0 | 4-15 |

## Che cosa cambierà a vista

### Accettate dalla spec («Differenze accettate»)

| Dove | Cambio | Task |
|---|---|---|
| Avvisi | La colonna di Sigrid (fino a dieci, `COLUMN_MAX`), con il fondo invertito, al posto di un avviso alla volta. Un errore non viene più sostituito (F18). | 7 |
| Menu, popover, dialoghi, schede laterali, liste del Select | L'ombra di Park al posto del bordo. | 12, 13, 14 |
| Pulsanti secondari | Senza bordo: un fondo grigio leggero (`bg-ink/6`). | 9 |
| Testo | 12, 13 e 14 px per etichette, date e testo, invece di 11,5, 12,5 e 13,5. I titoli da 15 a 26 px restano finché manca P4. | 2 |
| Movimento | Le transizioni brevi di Sigrid (Fluent, da 100 a 200 ms, solo dopo un gesto) al posto delle molle in interruttore e schede. La cifra animata dei contatori sparisce (il `Count` di Sigrid non si anima). | 4, 5, 6 |
| Contrasto | Bordi di campi, select e pulsanti con contorno a 3:1: `--border-strong` da `#dde1e7` a `#87898d` in chiaro, da `#6b7488` a `#626b7b` in scuro. | 2, 11 |

### Trovate in Sigrid 2.1.0 e non elencate dalla spec: da decidere

Ognuna viene dalla forma di Sigrid, quindi per la strada A vince Sigrid. Nessuna sta nell'elenco della spec, e il task che la porta non si unisce finché il proprietario non la accetta oppure Sigrid non offre un parametro. I valori sono quelli che il motore dà con i parametri del Task 2.

| Dove | Oggi | Con Sigrid | Task |
|---|---|---|---|
| Testo attenuato (chiaro) | `--muted` e `--text-secondary` sono lo stesso `#4b5563` | due gradini: `ink-mute` `#5a6376` (più chiaro, 5,9:1 sul foglio) e `ink-soft` `#3a4254` (più scuro) | 2 |
| Riga selezionata | `#ededfc` in chiaro, `#262b4a` in scuro | `#dbe1ff` (tinta più forte) e `#1e1f34` (più debole) | 2 |
| Rosso e verde di stato | `--danger` `#b03f49`/`#ef9ba1`, `--success` `#3f7560`/`#a3cfb8` | fissi per ogni tema nel motore: `#ad2a1e`/`#ff9a8d`, `#1c6b3f`/`#7fd3a0`, e i fondi `-soft` | 2 |
| Anello del focus | 2 px in `--accent` (`#4f46e5`), staccato di 3 px | 2 px in `--color-ring` (`#4338ca` in chiaro), staccato di 2 px; nei campi e nei pulsanti con contorno sta sul bordo | 2 (`--ring`), 4-14 |
| Barra laterale | `#16181d`; in scuro uguale al fondo (`#12161e`) | `#15171b` nei due schemi (in scuro un gradino più chiara del fondo); testo `#d1d4da`, voce attiva `#282b31` | 2 |
| Piccoli scarti | — | indaco sotto il puntatore `#4338ca` → `#4535d5` (scuro `#8a8cf3` → `#8d8ee9`); testo sull'indaco in scuro `#0f1020` → `#06070f`; barra della voce attiva in chiaro `#a7a3f2` → `#adb8ff` | 2 |
| Barre di scorrimento | quelle di Windows | la pillola di Sigrid (inchiostro al 18 %, senza frecce), dal layer `base` di Sigrid | 2 |
| Interruttore | 32×18, acceso in indaco | 40×20, acceso in inchiostro (quasi nero in chiaro) | 4 |
| Casella, radio, cursore dello Slider | spuntata in indaco | spuntata in inchiostro | 6, 10 |
| Schede | sottolineatura che scorre | fondo tinto (`bg-selected`) che scorre | 5 |
| Avvisi | uno stato resta 4,5 s, un avviso con azioni 10 s, un errore 12 s; un errore è un `alert` | gli avvisi che non sono errori hanno una durata sola (`durationMs`, proposta 10 s); un errore (`kind: 'error'`) non scade mai e resta finché non lo si chiude, con un segno davanti e «Errore:» per i lettori di schermo; la regione resta `status` (cortese), quindi un errore non interrompe più il lettore di schermo | 7 |
| Pulsanti | fermi | con le animazioni accese si abbassano un poco premuti (`tapScale` di Sigrid) | 9 |
| Pulsante solo testo | testo, nessun fondo | `ghost`: testo un gradino più tenue, fondo leggero sotto il puntatore | 9 |
| Campi con lo zoom alto | 13,5 px | sotto 860 px CSS (una finestra di 1280 px con lo zoom dell'app al 150 %, o quella minima di 1000 px al 125 %) i campi di Sigrid passano a 16 px (`text-base md:text-sm`) e la colonna degli avvisi cambia posto (`md:right-4`) | 7, 11 |
| Icone dentro le primitive | i disegni di Qamille | quelle di Sigrid (lucide): la X di chiusura, la freccia del Select, le frecce del calendario, la spunta, il segno di un errore negli avvisi | 7, 8, 10, 12, 13, 14 |

`Icon`, `NavList` (con la sua barra della voce attiva), `RecipientsInput`, `GoogleMark` e la barra laterale di Qamille (`components/Sidebar.tsx`) restano di Qamille, quindi non cambiano.
- Il `NavList` di Sigrid non ha la barra (R1 di Sigrid): usarlo applicherebbe a Qamille una regola di Ethel, che è un anti-obiettivo.
- Lo stesso vale per `Sidebar`/`SidebarGroup` di Sigrid (nuovi nella 2.1.0). Porterebbero anche la riduzione a icone, una ricerca tra le voci e, sotto 860 px CSS, un cassetto da sinistra: niente di questo c'è oggi in Qamille, e la spec non lo chiede.

---

### Task 0: punto di partenza, prerequisiti e punti d'innesto (controller)

**Files:**
- Create: `desktop/tests/sigrid-preconditions.mjs` (`node --test`, e riga di comando con `--require`); `desktop/tests/sigrid-leak.mjs` (browser, con il solo segnaposto); `desktop/tests/support/sigrid-leak.mjs` (`compareWithCleanRoom`); `desktop/tests/support/clean-room.html` (la pagina con il solo CSS di Sigrid e `compatto.css`, che il Task 2 completa)
- Modify: `desktop/src/ds/styles/ark.css`, `desktop/src/ds/styles/forced-colors.css`, `desktop/src/ds/styles/system.css`, `desktop/tests/support/Gallery.tsx`, `desktop/tests/primitives-controls.mjs`, `desktop/tests/primitives-overlays.mjs`: solo commenti e righe vuote

**Decisioni:**
- I quattro controlli di `checkPreconditions` leggono solo la copia sincronizzata.
  - **P1:** `package.json` di Sigrid ha `@ark-ui/react` e `motion` in `peerDependencies` a versione esatta, e `dist/index.js` li importa da fuori (`from "@ark-ui/react/…"`, `from "motion…"`). Non c'è codice di Zag dentro: si cerca un testo che Zag ha una volta per copia, scelto leggendo `node_modules/@zag-js/core`.
  - **P2:** niente `@base-ui` in `package.json`, in `src/` e in `dist/`.
  - **P3:** `--radius-lg` di `themeTokens(...).shape` vale 8px quando la definizione porta i raggi di Compatto. Il nome del campo lo sceglie Sigrid: finché il campo non esiste, il controllo dice «manca».
  - **P4:** `src/styles/theme.css` ha un gradino `--text-*` di almeno 1.625rem.
- Il punto di partenza visivo: `node tests/visual-snapshot.mjs D:\Projects\qamille-mail\capture\sigrid-baseline` sulla base del ramo. `capture/` è git-ignorata. Il Task 17 confronta con questa.

- [ ] **Step 1:** `checkPreconditions` con i suoi test, su una copia finta per ogni caso, in una cartella temporanea. La copia vera si stampa soltanto: oggi dà P1, P2, P3 e P4 falsi, ognuno col suo motivo.
- [ ] **Step 2:** i segnaposto, ognuno separato dagli altri da una riga vuota.
  - In `ark.css` un blocco per `@import`, con `/* Task N */`: switch 4, tabs 5, slider 6, toast 7, date-picker 8, checkbox 10, select 12, popover e menu 13, dialog 14, recipients e nav 15.
  - In `forced-colors.css` una sezione per primitiva, con gli stessi numeri.
  - In `system.css` i commenti `/* Task 9: buttons */` e `/* Task 11: field */` attorno alle due sezioni.
  - In `Gallery.tsx`, `primitives-controls.mjs`, `primitives-overlays.mjs` e `sigrid-leak.mjs` un commento `// Task N: <primitiva>` per ogni primitiva, più `// Task 3: words` in `Gallery.tsx` e in `primitives-overlays.mjs`.
- [ ] **Step 3:** `npm run test:ds`, `node tests/native-feel.mjs`, `npm run test:browser -- primitives-controls primitives-overlays`: tutto verde, perché i commenti non cambiano niente. Poi l'istantanea del punto di partenza.
- [ ] **Step 4: Commit** — `git commit -m "Check Sigrid's preconditions and leave one insertion point per primitive"`

### Task 1: Sigrid in sviluppo, nei test, nella CI e nella release

**Decisioni:**
- **Nessuna dipendenza npm per Sigrid.** Con `"@acaprino/sigrid": "file:vendor/sigrid"`:
  - il lockfile scriverebbe la versione e l'albero delle dipendenze di Sigrid, cioè un pin;
  - `npm ci` cambierebbe dopo ogni sync e fallirebbe senza la cartella;
  - si installerebbero Base UI, lucide e Fluent.

  Al loro posto:
  - Vite `resolve.alias` sul nome esatto, verso `vendor/sigrid/dist/index.js`;
  - `resolve.dedupe: ['react', 'react-dom']`;
  - tsconfig `paths` verso `dist/index.d.ts` (`skipLibCheck` c'è già).

  I tipi di `dist` che nominano pacchetti che Qamille non ha (`@base-ui/react/*`, `lucide-react`, `class-variance-authority`) diventano `any` finché manca P2. Per questo gli adattatori di `src/ui/` dichiarano le loro props per intero, e non le derivano da quelle di Sigrid.
- **Senza la copia di Sigrid si ferma tutto, con un messaggio.** `vite.config.ts` controlla `vendor/sigrid/provenance.json` e, se manca, lancia «Sigrid manca: `npm run sigrid:sync` in `desktop/`». Lo stesso fanno `tsc` (modulo non trovato) e i test del browser, che avviano Vite.
- **Il sync entra in `predev` e `prebuild`:** `npm run artcraft:sync && npm run sigrid:sync`. `AGENTS.md`, sezione «Sigrid», cambia da «è manuale» a «parte con `predev` e `prebuild`, come `artcraft:sync`». Il commento in testa a `sync-sigrid.mjs` dice lo stesso.
- **CI.** In `ci.yml`, job `desktop`: `npm run sigrid:sync` subito dopo `artcraft:sync`, con `GH_TOKEN: ${{ github.token }}` solo per il limite di richieste.
  - `release.yml` non cambia: `build-release.bat` → `npm run build` → `prebuild`, e il job ha già `GH_TOKEN`.
  - Su un runner nuovo non c'è una copia di riserva: senza rete, con la release fuori da `main` o con file che non corrispondono, `sigrid:sync` esce con 1 e la build non parte. Si ferma sempre, mai con una copia vecchia.
- **Licenza e provenienza nell'exe.** `sigrid:sync` copia `LICENSE`, `THIRD_PARTY.md` e `provenance.json` in `desktop/public/sigrid-licenses/` (git-ignorata, come `artcraft-licenses`). La MIT chiede l'avviso, e così si sa quale Sigrid c'è in una build senza un pin.
- **Tailwind 4.3.5**, a versione esatta, per `tailwindcss` e `@tailwindcss/vite`: la versione con cui si compila Sigrid, una sola nelle due app. `visual-compare` prima e dopo: nessuna differenza, oppure la si riporta.
- **`SigridRoot`.** `WordsProvider` con `DEFAULT_WORDS` (le parole arrivano col Task 3) e `MotionProvider animated`, attorno ad `App` in `main.tsx`. `MotionConfig reducedMotion="user"` di Motion resta per i componenti di Qamille che usano Motion direttamente.
- **INV-009** (bozza per l'ADR-0011 v4), in `ds-usage.mjs`: solo `src/ui/**` e `src/theme/**` importano `@acaprino/sigrid`; nessun file importa `@base-ui/*`, `lucide-react` o `@fluentui/*`.

**Files:**
- Create: `desktop/src/ui/SigridRoot.tsx`; `desktop/tests/sigrid-wiring.mjs` (`node --test`); `desktop/tests/build-check.mjs` (lancia `vite build` in una cartella temporanea e controlla il risultato)
- Modify: `desktop/vite.config.ts`, `desktop/tsconfig.json`, `desktop/package.json`, `desktop/package-lock.json`, `desktop/scripts/sync-sigrid.mjs`, `desktop/tests/sigrid-sync.mjs`, `desktop/scripts/test-browser.mjs` (`build-check` in `OTHERS`), `desktop/src/main.tsx`, `desktop/tests/ds-usage.mjs`, `.github/workflows/ci.yml`, `.gitignore`, `AGENTS.md`

- [ ] **Step 1: test che falliscono.**
  - `sigrid-wiring.mjs`:
    - `test('no npm entry pins Sigrid')`: nessun `@acaprino/sigrid` in `package.json` né nel lockfile;
    - `test('Vite and TypeScript reach the synced copy')`: alias esatto, `dedupe`, `paths`;
    - `test('predev and prebuild sync Sigrid after ArtCraft')`;
    - `test('CI syncs Sigrid before the type check')`: in `ci.yml` lo step sta prima di `npm run check`;
    - `test('the build stops with a message when the copy is missing')`: `vite.config.ts` importato con una cartella di Qamille finta senza `vendor/sigrid`.
  - `sigrid-sync.mjs`: `test('licences and provenance land in public/sigrid-licenses')`; `test('a sync with no release and no copy fails, and leaves nothing half-written')`, se il caso non c'è già.
  - `build-check.mjs`:
    - una copia di React e una di Zag; Motion contato e stampato, e con `--strict` anche Motion deve essere 1 (si usa da quando vale P1);
    - nel CSS prodotto niente `cursor: pointer`, `cursor: not-allowed` né `cursor: progress`. Oggi passa; dal Task 2 deve continuare a passare con le classi di Sigrid.
  - `ds-usage.mjs`: INV-009 con un file finto che la viola (`DS_USAGE_ROOT`).
- [ ] **Step 2:** `node --test tests/sigrid-wiring.mjs tests/sigrid-sync.mjs`, `node tests/ds-usage.mjs`: FAIL.
- [ ] **Step 3:** implementare come nelle Decisioni. Poi `npm ci` e `npm run sigrid:sync`.
- [ ] **Step 4:** i test dello step 2 PASS; `node tests/build-check.mjs` PASS (React 1, Zag 1, nessun cursore vietato); `npm run check`; `npm run test:ds`; `npm run test:browser -- smoke onboarding-professional`.
- [ ] **Step 5:** su un clone pulito, senza `vendor/sigrid`:
  - `npm ci`, poi `npm run build`, che scarica Sigrid da sé;
  - con la rete staccata e senza copia, `npm run build` si ferma col messaggio di `sigrid:sync`.

  Il job `desktop` di `ci.yml` si vede girare solo sulla pull request verso `develop`. Pubblicare il ramo lo decide il proprietario; fino ad allora valgono le due prove in locale.
- [ ] **Step 6: Commit** — `git commit -m "Reach the synced Sigrid through an alias, sync it before dev and build, and in CI"`

### Task 2: «Compatto» come parametri di Sigrid, primo disegno, alias dei token

**Decisioni:**
- **La definizione** (`src/theme/compatto.ts`).
  - `COMPATTO`: `sidebar: 'scura'`, `palette: 'crisp'`; tinta e saturazione dei grigi da quelli di Compatto; `accent` dalla saturazione e dalla tinta OKLCH di `#4f46e5`.
  - `surfaces` curate nei due schemi: `paper` = `--ground`, `raised` = `--surface`, `line` = `--border`, `ink` = `--text`, `accent` = `--accent-text`, `fill` = `--accent`, e `sidebar` = una corsia **chiara** (per esempio `#eef0f3`).
  - In `surfaces` la colonna scura **non** va messa: la disegna il motore con `sidebar: 'scura'`. Mettendo `#16181d` in `surfaces.light.sidebar`, il motore porta inchiostro, linee e rosso a `#000000`/`#ffffff` per tenere il contrasto (provato il 2026-11-22).
  - `COMPATTO_TONE` = `{ intensity: 'theme', light: 'theme' }`: il tono predefinito del motore (Vivid) sposta l'indaco a `#3f00d9`.
  - Con questi valori il motore (2.0.1 e 2.1.0, provato su tutte e due) tiene esatti `paper`, `raised`, `ink`, `line`, `accent` e `primary` nei due schemi e non corregge niente (`corrected` vuoto). Le differenze vengono dai ruoli che il motore deriva: sono quelle di «Che cosa cambierà a vista».
- **La densità.** `QAMILLE_DENSITY` = `compact` (32/28/24) con `sm` e `xs` a 32: oggi ogni controllo di Qamille è alto 32 px, e un `size="sm"` di Sigrid non deve scendere a 28. Le righe restano 32/4 e le spaziature 10/8/6. Con un puntatore grossolano valgono i 44 px di Sigrid.
- **`compatto.css`, generato e versionato.**
  - `scripts/theme-css.mjs` importa `vendor/sigrid/dist/index.js` in Node (funziona: nessun accesso a `document` all'import). Scrive:
    - il blocco `:root` con `densityVariables(QAMILLE_DENSITY)`;
    - un blocco chiaro, `:root:not([data-scheme]), :root[data-scheme='light']`, e uno scuro, `:root[data-scheme='dark']`, con `themeVariables({ theme: COMPATTO, scheme, tone: COMPATTO_TONE })`;
    - il blocco degli alias.
  - `--radius-*` restano fuori finché manca P3. Elevazione, icone e `--border-width` entrano: le ombre di Park sono accettate.
  - In testa al file: «generato da `npm run theme:css`, non modificare».
  - `tests/tokens-compatto.mjs` diventa due controlli:
    - i parametri, cioè la tabella dei valori di partenza di Compatto in `COMPATTO.surfaces`, le misure di `QAMILLE_DENSITY` e `corrected` vuoto (oppure uguale a un elenco noto, annotato nel test con il motivo);
    - `compatto.css` uguale a `compattoCss(sigrid)`. Il messaggio d'errore dice di rilanciare `npm run theme:css`, guardare il diff e riportarlo.
- **Gli alias.** `ALIASES` in `compatto.ts` porta i nomi di Qamille sui token di Sigrid per tutto ciò che Sigrid possiede: `--ground`, `--surface`, `--sidebar*`, `--text`, `--text-secondary`, `--body-text`, `--muted`, `--nav-text`, `--border`, `--border-soft`, `--border-strong`, `--accent`, `--accent-hover`, `--accent-text`, `--on-accent`, `--ring`, `--accent-pro`, `--selection`, `--danger*`, `--success*`, `--highlight`, `--shadow*`, `--control-height`, `--font-text`, `--font-title`.
  - La scelta, per ogni nome, è il ruolo più vicino. Per esempio `--text-secondary` → `--color-ink-soft` e `--muted` → `--color-ink-mute`. Il task riporta la tabella prima/dopo dei due schemi.
  - I token solo di Qamille restano scritti a mano in `ds/tokens/theme.css`, con il blocco scuro su `[data-scheme='dark']`: priorità, mittenti, etichette, larghezze dei pannelli e dei dialoghi, livelli, carta di stampa, `--window-close`, `--scrim*`, `--summary-*`, `--avatar*`, `--badge`, `--thread-open`, `--label-*`.
- **Il testo.**
  - Qamille non definisce più `--text-2xs`, `--text-xs` e `--text-sm`: valgono quelli di Sigrid, 12, 13 e 14 px, come vuole la spec.
  - `--text-md` diventa alias di `--text-sm`.
  - `--text-lg`, `--text-xl`, `--text-2xl` e `--text-3xl` restano di Qamille fino a P4.
- **Il carattere.** Qamille tiene il suo `@font-face` variabile di IBM Plex Sans e non importa `fonts.css` di Sigrid, che ha solo 400, 500 e 600. La pila predefinita di Sigrid (`--font-sans`) comincia con «IBM Plex Sans», quindi usa lo stesso carattere. `--font-text` e `--font-title` diventano alias di `--font-sans`.
- **La cascata.** `ds/styles/tailwind.css` diventa, nell'ordine:
  1. `@layer theme, base, components, utilities`;
  2. il tema di Tailwind;
  3. `@theme { --color-*: initial }`, per spegnere la tavolozza di Tailwind;
  4. `theme.css` di Sigrid (percorso relativo);
  5. le utility;
  6. `@source` su `vendor/sigrid/src`, `@source not` su `internal/motion-impl.tsx`;
  7. `@source not inline(…)` per `cursor-pointer`, `cursor-not-allowed` e `cursor-progress`, anche con le varianti che Sigrid usa (`disabled:`, `data-[disabled]:`): le utility che la regola 3 vieta non si generano mai, nemmeno dalle classi di Sigrid. Tailwind esclude solo le forme scritte, quindi la guardia vera è `build-check.mjs`: una forma nuova in una release di Sigrid lo fa fallire, e si aggiunge qui. A Sigrid si segnala il contrasto con la sua stessa regola («the pointer stays an arrow»).

  Altre regole della cascata:
  - Niente preflight, come oggi: la composizione e l'editor tengono i default del browser.
  - Le regole di elemento di `system.css` entrano in `@layer base`, dopo il `base` di Sigrid. Così vincono sul `base` di Sigrid (corpo, selezione, cursori), ma non sulle utility delle primitive.
  - Le classi di Qamille (`.primary`, `.field`…) restano fuori dai layer finché la loro primitiva non passa.
  - La mappa dei nomi di shadcn di `tailwind.css` sparisce: quei nomi ora li definisce Sigrid.
  - La variante `dark` diventa `[data-scheme='dark']`.
- **Gli attributi.**
  - `theme-boot.js` scrive `document.documentElement.dataset.scheme`, con la stessa regola di oggi.
  - `index.html` e `reminder.html` portano `data-theme="compatto"`, `data-density="compatto"` e gli attributi di `ThemeTokens.attributes`, scritti fissi. Il test di `tokens-compatto.mjs` li confronta con quelli del motore.
  - L'effetto del tema in `App.tsx` (`:1506`) e `Reminder.tsx` (`:110`) scrivono `dataset.scheme`.
  - Nell'iframe del messaggio (`message-html.ts`) si passa a `data-scheme`, con i selettori di `:19-20`.
  - Ogni `[data-theme='dark']` dei `.css` diventa `[data-scheme='dark']`.
  - Qamille non chiama `applyTheme` né `applyDensity` a runtime: il CSS generato basta, e il primo disegno non aspetta JavaScript.
- **La finestra.** `--color-paper` è `#f6f7f9` in chiaro e `#12161e` in scuro, cioè il `--ground` di oggi: `tauri.conf.json` e `DARK_GROUND` non cambiano.
  - Il controllo 4 legge i due valori da `compatto.css`, non più da `theme.css`.
  - `paintWindowBackground` continua a leggere `--ground` (alias, valore calcolato) fino al Task 16.
- **La finestra del promemoria** importa `compatto.css` e i token di Qamille al posto di `theme.css`, e tiene il suo `@font-face`.

**Files:**
- Create: `desktop/src/theme/compatto.ts`; `desktop/src/theme/compatto.css` (generato); `desktop/scripts/theme-css.mjs`; `desktop/tests/first-paint.mjs` (browser, sulla build); una riga in `OTHERS` di `desktop/scripts/test-browser.mjs`
- Modify: `desktop/src/ds/tokens/theme.css`, `desktop/src/ds/styles/tailwind.css`, `desktop/src/ds/styles/system.css`, `desktop/public/theme-boot.js`, `desktop/index.html`, `desktop/reminder.html`, `desktop/src/App.tsx` (`:1506`), `desktop/src/reminder/Reminder.tsx` (`:110`), `desktop/src/reminder/reminder.css`, `desktop/src/lib/message-html.ts`, `desktop/package.json` (script `theme:css`), `desktop/src/styles.css` e i `.css` dei componenti (solo `[data-theme='dark']`)
- Modify, test: `tests/tokens-compatto.mjs`, `tests/contrast.mjs`, `tests/style-tokens.mjs` (esente `compatto.css`, ammessi i nomi di Sigrid), `tests/support/theme-tokens.mjs`, `tests/support/compatto.mjs`, `tests/compatto-*.mjs`, `tests/support/clean-room.html`, e i test che leggono `data-theme`: `calendar-flow`, `ds-export`, `mail-html`, `recipients-dark`, `release-theme-probe`, `smoke`, `undo`, `update-notice-fixture.js`, `window-native`
- Modify: `desktop/tests/native-feel.mjs` (controlli 3 e 4, nuovo controllo 29); `docs/aspetto-nativo.md` (regole 3, 5 e 6); `desktop/src/ds/README.md` («Colore», «Tipografia», «Tailwind»)

- [ ] **Step 1: test che falliscono.**
  - `tokens-compatto.mjs` riscritto (vedi Decisioni).
  - `contrast.mjs`: legge i token del motore (`compatto.css`) e quelli di Qamille, e controlla le coppie di Qamille che restano, cioè testo su priorità, etichette, mittenti, sommario. Il contrasto dei ruoli di Sigrid lo garantisce Sigrid; qui si controllano solo le coppie che mescolano un token di Qamille con uno di Sigrid.
  - `native-feel.mjs`, che resta un controllo statico di meno di un secondo:
    - 3: `dataset.scheme`, con gli stessi casi;
    - 4: legge `compatto.css`;
    - 29: «Sigrid's classes never bring a hand or a forbidden cursor (rule 3)». `tailwind.css` esclude le utility di cursore vietate. Il CSS prodotto lo legge `build-check.mjs`.
  - `first-paint.mjs` (in `OTHERS` di `test-browser.mjs`, perché fa la sua build):
    - `vite build` in una cartella temporanea, poi `vite preview`;
    - lo schema scuro emulato e il chunk d'ingresso bloccato con `page.route`;
    - il fondo di `body` è `#12161e`, e `data-scheme` vale `dark` prima di `DOMContentLoaded`;
    - lo stesso in chiaro.
  - `system.css`: nessuna regola di elemento fuori da `@layer base`, controllata in `native-feel.mjs` 1 e 2, che leggono le regole dentro il layer.
- [ ] **Step 2:** `node tests/tokens-compatto.mjs`, `node tests/native-feel.mjs`, `node tests/first-paint.mjs`: FAIL.
- [ ] **Step 3:** implementare come nelle Decisioni, poi `npm run theme:css`.
- [ ] **Step 4:** test PASS; `node tests/build-check.mjs`; `npm run test:ds`; `npm run check`; `npm run test:browser -- smoke forced-colors compatto-dark compatto-list compatto-reader compatto-settings compatto-sidebar compatto-notices recipients-dark mail-html`.
- [ ] **Step 5:** `visual-snapshot` prima e dopo, poi `visual-compare`. Le differenze devono stare tutte in queste righe di «Che cosa cambierà a vista»: testo, contrasto, testo attenuato, riga selezionata, rosso e verde, `--ring`, barra laterale, piccoli scarti, barre di scorrimento. L'anello disegnato da `system.css` resta `--accent` staccato di 3 px: cambia solo con le primitive di Sigrid. Il task riporta al controller la tabella dei token prima/dopo nei due schemi.
- [ ] **Step 6:** `docs/aspetto-nativo.md`:
  - regola 5: `data-scheme`, i blocchi generati;
  - regola 6: `--color-paper` di `compatto.css` e la checklist «Cambi `--ground`», che diventa «Cambi i parametri di `COMPATTO`»;
  - regola 3: le utility di cursore escluse in `tailwind.css`.

  `ds/README.md`: le sezioni «Colore», «Tipografia» e «Tailwind».
- [ ] **Step 7: Commit** — `git commit -m "Compute the Compatto look with Sigrid's engine from Qamille's parameters, paint it before any script, and alias the old token names"`

### Task 3: le parole di Sigrid in italiano e in inglese

**Decisioni:**
- `qamilleWords(italian)` dà un `Words` intero, non un `WordsOverride`. Così una chiave nuova di una release di Sigrid è un errore di `tsc` e del test, e non un'eccezione inglese silenziosa.
  - In inglese: `DEFAULT_WORDS`, con le parole di Qamille dove Qamille dice già altro.
  - In italiano: tutto, con i plurali («1 allegato», «3 allegati»), le combinazioni di tasti lette come le legge Qamille (`keysTogether`), i nomi dei colori (`colors.names`) e le parole del calendario, allineate a `lib/calendar-words.ts`.
- `SigridRoot` passa `qamilleWords(useLanguage())`, memorizzato per lingua. La radice si rifà al cambio di lingua, come oggi l'albero sotto `App`.
- Le funzioni di Sigrid fuori da un componente (`kindLabel`, `stateText`, `overLimitNotice`, `withinLimit`) prendono le parole come ultimo argomento. Chi le usa in Qamille passa `useWords()`.
- Nessun testo di Qamille passa da questo dizionario: è solo per le parole che le primitive dicono da sé.
- Il dizionario copre ogni chiave, anche quelle di componenti che Qamille non usa, come `sidebar.*`, `date.noRange` e le scelte rapide, perché `Words` è un tipo intero. La 2.1.0 ne ha aggiunte nove (vedi «Prima di cominciare»): una release che ne aggiunge altre ferma `tsc` e `sigrid-words.mjs` finché non si traducono.

**Files:**
- Create: `desktop/src/ui/words.ts`; `desktop/tests/sigrid-words.mjs` (`node --test`, carica `dist/index.js` per `DEFAULT_WORDS`)
- Modify: `desktop/src/ui/SigridRoot.tsx`; `desktop/tests/support/Gallery.tsx` e `desktop/tests/primitives-overlays.mjs` (sotto il segnaposto «Task 3: words»)

- [ ] **Step 1: test che falliscono.**
  - `test('every Sigrid word has an Italian one')`: si percorrono le chiavi di `DEFAULT_WORDS`.
  - `test('Italian words differ from English ones')`, con l'elenco delle eguaglianze ammesse, come «OK» o un nome proprio.
  - `test('counted words agree with 1 and 3')`.
  - `test('a key added by Sigrid fails here')`: `DEFAULT_WORDS` con una chiave in più, data a mano.
  - In `primitives-overlays.mjs`: `Toasts` montati nella galleria sotto `SigridRoot`. Al cambio di lingua il nome della regione cambia senza ricaricare. Il montaggio è della sola galleria, quindi non porta Zag nel bundle dell'app e non chiede P1.
- [ ] **Step 2:** `node --test tests/sigrid-words.mjs`, `npm run test:browser -- primitives-overlays`: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** test PASS; `npm run check`; `npm run test:browser -- smoke primitives-overlays`.
- [ ] **Step 5: Commit** — `git commit -m "Give Sigrid's own words in Italian and English, following the interface language"`

### Procedura comune dei task di primitiva (4-14)

Ogni task di primitiva segue questi passi; sotto, per ciascuno, solo ciò che è suo.

- [ ] **Step 0:** `npm run sigrid:sync`, poi `node tests/sigrid-preconditions.mjs --require <prerequisiti del task>`. Se fallisce, il task non parte e lo si dice al controller. Poi `visual-snapshot` della base del ramo in `capture/sigrid-task-N-before`.
- [ ] **Step 1: test che falliscono.**
  - Nella galleria la primitiva di Qamille passa all'adattatore, nella stessa entrata.
  - In `sigrid-leak.mjs` il suo caso (`compareWithCleanRoom`).
  - In `primitives-*.mjs`: i casi che leggono le classi di `system.css` o i fogli di `ark/*.css` diventano casi su ruoli, nomi, `[data-scope][data-part]` (le primitive di Sigrid su Ark li hanno) e `[data-slot]`. Il contratto di valore e la tastiera restano gli stessi. L'anello del focus si legge su `--color-ring`.
  - In `forced-colors.mjs` il suo caso, sulle parti di Sigrid.
- [ ] **Step 2:** `npm run test:browser -- <test della primitiva> sigrid-leak forced-colors`: FAIL.
- [ ] **Step 3: implementare.**
  - Scrivere `src/ui/<X>.tsx`. `ds/primitives/<X>.tsx` diventa la riesportazione.
  - Togliere il blocco di `ark.css` e il file `ds/styles/ark/<x>.css`.
  - Riscrivere la sezione di `forced-colors.css` sulle parti di Sigrid: niente regole di forma, solo colori di sistema.
  - Cercare in `styles.css` e nei `.css` dei componenti le regole che puntano alle sue classi o parti, e toglierle o legarle a una classe di Qamille.
  - Le copie di unlumen usate solo da questa primitiva si cancellano, insieme alla loro riga in `ds/unlumen/README.md`.
- [ ] **Step 4:** test PASS; `npm run test:ds`; `npm run check`; `node tests/build-check.mjs --strict`; i test di flusso elencati nel task.
- [ ] **Step 5:** `visual-snapshot` in `capture/sigrid-task-N-after`, poi `visual-compare` con la base.
  - Ogni differenza va ricondotta a una riga di «Che cosa cambierà a vista».
  - Una differenza che non c'è, o che compare fuori dalla primitiva, ferma il task.
  - Il task riporta al controller l'elenco, con le immagini delle schermate toccate nei due schemi.
- [ ] **Step 6:** la sezione di `ds/README.md`; il commit col messaggio del task.

### Task 4: Switch (P1)

- **Decisioni:**
  - `src/ui/Switch.tsx` su `Switch` di Sigrid, che sta su Ark. Le props di oggi (valore, `onChange`, etichetta, `disabled`) passano a `SwitchProps`.
  - Se ne vanno `unlumen/apple-switch.tsx` e `styles/ark/switch.css`, e `--accent-pro` non ha più lettori: si toglie nel Task 16.
- **Files:** `desktop/src/ui/Switch.tsx`, `desktop/src/ds/primitives/Switch.tsx`, `desktop/src/ds/unlumen/apple-switch.tsx` (via), `desktop/src/ds/styles/ark/switch.css` (via), i segnaposto «Task 4».
- **Differenze attese:** movimento (accettata); 40×20, acceso in inchiostro (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-controls mcp-settings meetings-settings compute-settings`.
- **Commit:** `git commit -m "Draw Switch with Sigrid's switch"`

### Task 5: Tabs (P1)

- **Decisioni:**
  - `src/ui/Tabs.tsx` su `Tabs`, `TabsList`, `TabsTrigger` e `TabsContent` di Sigrid. L'evidenza scorrevole è quella di Sigrid (`motion.Highlight`), e la tastiera è quella di Ark, come oggi.
  - Via `styles/ark/tabs.css` e il `layoutId` di Motion della primitiva.
- **Files:** `desktop/src/ui/Tabs.tsx`, `desktop/src/ds/primitives/Tabs.tsx`, `desktop/src/ds/styles/ark/tabs.css` (via), i segnaposto «Task 5».
- **Differenze attese:** movimento (accettata); fondo tinto al posto della sottolineatura (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-controls composer-tabs`.
- **Commit:** `git commit -m "Draw Tabs with Sigrid's tabs"`

### Task 6: Count e Slider (P1)

- **Decisioni:**
  - `src/ui/Count.tsx` su `Count` di Sigrid: un numero semplice, `reserve` per le colonne allineate, `label` per i lettori di schermo. Le classi `.count-digits` di `nav.css` restano finché `NavList` non passa (Task 15).
  - Via `unlumen/animate-digits.tsx`.
  - `Slider` non ha consumatori in `src/`: `ds/primitives/Slider.tsx`, `styles/ark/slider.css` e i loro casi di test si tolgono. Se un giorno servirà, si userà lo `Slider` di Sigrid.
- **Files:** `desktop/src/ui/Count.tsx`, `desktop/src/ds/primitives/Count.tsx`, `desktop/src/ds/primitives/Slider.tsx` (via), `desktop/src/ds/unlumen/animate-digits.tsx` (via), `desktop/src/ds/styles/ark/slider.css` (via), i segnaposto «Task 6».
- **Differenze attese:** la cifra non si anima (accettata).
- **Test di flusso:** `npm run test:browser -- primitives-controls sidebar-threads undo`.
- **Commit:** `git commit -m "Show counts with Sigrid's Count and drop the unused Slider"`

### Task 7: Toaster (P1)

- **Decisioni:**
  - `src/ui/Toaster.tsx` tiene `notify(message, error, actions)` e la sua firma.
  - Dentro, una lista di `ToastItem` resa da `Toasts` di Sigrid: al massimo dieci, il più nuovo in basso, un avviso nuovo non sostituisce il precedente. Le azioni, al massimo due, diventano `ToastAction`.
  - `notify(message, true)` dà `kind: 'error'` (Sigrid 2.1.0). Un errore non scade: resta finché non lo si chiude con la «×» (`onDismiss`). Una colonna piena spinge fuori prima gli altri avvisi, e un errore solo quando non resta altro. Oggi invece un errore si chiude da solo dopo 12 s.
  - Gli altri avvisi hanno una durata sola per la colonna (`durationMs`), sospesa sotto il puntatore e col focus dentro. Oggi è 4,5 s per uno stato e 10 s con azioni. Proposta: 10 s, così un «Apri» o un «Mostra nella cartella» resta il tempo di sceglierlo. Una durata diversa per avviso tenuta con timer di Qamille rifarebbe a metà il lavoro di `Toasts`, quindi no: se serve, la si chiede a Sigrid.
  - La regione di Sigrid è `role="status"` (cortese) anche per un errore, che oggi è un `alert`. Sigrid 2.1.0 fa sentire «Errore:» prima del testo (`words.common.error`), ma un errore non interrompe più il lettore di schermo. Si riporta al proprietario («Che cosa cambierà a vista», Avvisi) e si chiede a Sigrid un annuncio immediato per gli errori.
  - La regione resta letta anche con un `Modal` aperto (audit T14): il test di `toaster-layer.mjs` lo prova sulla regione di Sigrid.
  - La colonna sta sopra il composer agganciato e sopra `UndoStack`. Un solo scrittore di `--bottom-bar-room` (`BOTTOM_BAR_ROOM_VAR`): una funzione di `src/lib/` che somma l'altezza del dock e quella di `UndoStack`.
  - `UndoStack` resta un componente di Qamille (ADR-0011 v3, decisione 6), come oggi.
  - Via `styles/ark/toast.css`.
- **Files:** `desktop/src/ui/Toaster.tsx`, `desktop/src/ds/primitives/Toaster.tsx`, `desktop/src/ds/styles/ark/toast.css` (via), `desktop/src/lib/bottom-room.ts` (nuovo), `desktop/src/components/ComposerDock.tsx` e `UndoStack.tsx` (una riga ciascuno, la misura), i segnaposto «Task 7».
- **Test nuovi** (`toaster-layer.mjs`):
  - «un errore resta finché non lo si chiude, e una colonna piena di stati non lo spinge fuori»;
  - «un lettore di schermo sente "Errore:" prima del testo di un errore, in italiano e in inglese».
- **Differenze attese:** colonna e fondo invertito (accettata); durata unica, errore che resta e annunciato in modo cortese, posto della colonna con lo zoom alto (da decidere).
- **Test di flusso:** `npm run test:browser -- toaster-layer undo compatto-notices mail-export`; `node --test tests/undo-notices.mjs`.
- **Commit:** `git commit -m "Show notices in Sigrid's column, so an error is never replaced by the next notice"`

### Task 8: DateField (P1)

- **Decisioni:**
  - `src/ui/DateField.tsx` su `DateField` e `DateTimeField` di Sigrid (date picker di Ark). La lingua e il formato sono quelli di Qamille (`regional.ts`), le parole quelle del Task 3 e i `placeholder` quelli di Qamille.
  - `@internationalized/date` resta, alla stessa versione di Sigrid.
  - Niente scelte rapide (`picks`, `dayPicks()`, nuove nella 2.1.0): oggi Qamille non le ha, e la spec non le chiede. Né `DateRangeField`, che nessuna schermata usa.
  - Via `styles/ark/date-picker.css`.
- **Files:** `desktop/src/ui/DateField.tsx`, `desktop/src/ds/primitives/DateField.tsx`, `desktop/src/ds/styles/ark/date-picker.css` (via), i segnaposto «Task 8».
- **Test nuovo** (`primitives-overlays.mjs`, Review Focus 3): «una data scelta nel calendario di un DateField dentro un Modal lascia il Modal aperto e il focus nel Modal».
- **Differenze attese:** la scheda del calendario di Park e le frecce di Sigrid (da decidere, icone).
- **Test di flusso:** `npm run test:browser -- primitives-controls calendar-flow calendar-series meetings`.
- **Commit:** `git commit -m "Pick dates with Sigrid's date fields"`

### Task 9: Button e Tooltip (P1, P2)

- **Decisioni:**
  - `src/ui/Button.tsx` tiene `variant: 'primary' | 'secondary' | 'icon' | 'text'`, `tip` e `shortcut`. Le varianti diventano:
    - `primary` → `default`;
    - `secondary` → `secondary`;
    - `icon` → `ghost` con `size="icon"`;
    - `text` → `ghost`.
  - `tip` usa il `Tooltip` di Sigrid (`label`, `shortcut`). Il suggerimento non è mai il nome: un pulsante a sola icona tiene il suo `aria-label`.
  - **`type`** (Review Focus 5). Il `Button` di Sigrid ha `type="button"` per default. L'adattatore non lo cambia: un pulsante che non invia è il default giusto. Prima del cambio però si cercano tutti i `<form>` di `src/` (oggi 16, con 11 `type="submit"`), e ogni pulsante che oggi invia per default prende `type="submit"` esplicito, nello stesso commit.
  - Le regole `.primary`, `.secondary`, `.icon-button` e `.text-button` di `system.css` (sezione «Task 9») si tolgono solo se nessun HTML scritto a mano le usa ancora. Si controlla con `grep` su `src/`, `tests/*.html` e `*-fixture.js`; quelle che restano si riscrivono sui pulsanti di Sigrid.
  - `tests/ds-usage.mjs` tiene il divieto di scriverle a mano.
- **Files:** `desktop/src/ui/Button.tsx`, `desktop/src/ui/Tooltip.tsx`, `desktop/src/ds/primitives/Button.tsx`, `desktop/src/ds/primitives/Tooltip.tsx`, `desktop/src/ds/styles/system.css` (sezione «Task 9»), i segnaposto «Task 9».
- **Test nuovo:** in `onboarding-professional`, `server-connect`, `password-reset` e `signup-professional`, Invio nel campo e clic sul pulsante d'invio inviano. Un pulsante che non è d'invio, dentro un `<form>`, non invia.
- **Differenze attese:** secondari senza bordo (accettata); `ghost` per `text`, l'anello sul bordo dei pulsanti con contorno e `tapScale` (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-button a11y-keyboard smoke shortcut-guard onboarding-professional server-connect password-reset signup-professional`.
- **Commit:** `git commit -m "Draw buttons and their hints with Sigrid's Button and Tooltip"`

### Task 10: Checkbox (P1, P2)

- **Decisioni:** `src/ui/Checkbox.tsx` su `Checkbox` di Sigrid. L'area di clic di Sigrid (`after:-inset-1.5`) resta. La casella nativa delle righe della lista resta nativa (ADR-0011 v3, decisione 6).
- **Files:** `desktop/src/ui/Checkbox.tsx`, `desktop/src/ds/primitives/Checkbox.tsx`, `desktop/src/ds/styles/ark/checkbox.css` (via), i segnaposto «Task 10».
- **Differenze attese:** spuntata in inchiostro (da decidere); 18 px.
- **Test di flusso:** `npm run test:browser -- primitives-controls contacts meetings-settings`.
- **Commit:** `git commit -m "Draw Checkbox with Sigrid's checkbox"`

### Task 11: Field (P1, P2)

- **Decisioni:**
  - `src/ui/Field.tsx` su `FormField` con `Input` e `Textarea` di Sigrid. Etichetta, aiuto ed errore li lega `FormField`.
  - La sezione «Task 11» di `system.css` (`.field`) si toglie.
  - Le regole di elemento `input` e `textarea` del `base` di Qamille restano per i campi che non sono un `Field`, come la ricerca e l'editor. Il Task 16 decide se ne resta qualcuno.
- **Files:** `desktop/src/ui/Field.tsx`, `desktop/src/ds/primitives/Field.tsx`, `desktop/src/ds/styles/system.css` (sezione «Task 11»), i segnaposto «Task 11».
- **Test nuovo:** un `Field` con la finestra minima (1000 px) e lo zoom dell'app al 100 %, 125 % e 200 %. Il corpo del campo è quello atteso per la larghezza CSS che ne viene: 14 px sopra 860 px, 16 px sotto, finché il proprietario non decide altrimenti (vedi «Campi con lo zoom alto»).
- **Differenze attese:** bordo a 3:1 (accettata); anello sul bordo e 16 px con lo zoom alto (da decidere).
- **Test di flusso:** `npm run test:browser -- onboarding-professional server-connect signup-professional password-reset contacts`.
- **Commit:** `git commit -m "Draw fields with Sigrid's FormField, Input and Textarea"`

### Task 12: Select (P1, P2)

- **Decisioni:** `src/ui/Select.tsx` tiene `items` e `value`/`onChange` e li traduce in `SelectOption`/`SelectGroup`. Il segno della voce scelta è quello di Sigrid (`renderMark` solo se Qamille ne ha uno suo).
- **Files:** `desktop/src/ui/Select.tsx`, `desktop/src/ds/primitives/Select.tsx`, `desktop/src/ds/styles/ark/select.css` (via), i segnaposto «Task 12».
- **Test nuovo** (`primitives-overlays.mjs`): «una voce scelta in un Select dentro un Modal lascia il Modal aperto».
- **Differenze attese:** ombra al posto del bordo nella lista (accettata); freccia e spunta di Sigrid (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-select regional meetings-settings compute-settings`.
- **Commit:** `git commit -m "Draw Select with Sigrid's select"`

### Task 13: Popover e Menu (P1, P2, dopo C1)

- **Decisioni:**
  - `src/ui/Popover.tsx` su `Popover*` e `src/ui/Menu.tsx` su `DropdownMenu*` di Sigrid. `Menu` tiene il suo contratto (`ds/README.md`, «Menu»):
    - `items` con `group`, `destructive` in fondo dopo un separatore, `disabled` con `title`, `checked` come `DropdownMenuCheckboxItem`, `shortcut` come `DropdownMenuShortcut` con `aria-keyshortcuts`;
    - `label`, `tip`, `open`, `onOpenChange`.
  - Il focus torna al pulsante **prima** del comando, così un `Modal` aperto da una voce ricorda il pulsante.
  - `shortcutCoveredByLayer` copre ancora `role="menu"`.
  - `data-keyboard` resta solo se Sigrid non distingue da sé la tastiera dal mouse. In quel caso è una differenza da riportare, non una regola di Qamille sopra Sigrid.
- **Files:** `desktop/src/ui/Popover.tsx`, `desktop/src/ui/Menu.tsx`, `desktop/src/ds/primitives/Popover.tsx`, `desktop/src/ds/primitives/Menu.tsx`, `desktop/src/ds/styles/ark/popover.css` e `menu.css` (via), `desktop/src/lib/shortcut-guard.ts` (solo se il selettore cambia), i segnaposto «Task 13».
- **Test nuovo** (`primitives-overlays.mjs`): «una voce di Menu dentro un Modal che apre un altro Modal rende il focus al pulsante del menu quando il secondo si chiude».
- **Differenze attese:** ombra al posto del bordo (accettata); icone di Sigrid (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-overlays shortcut-guard meetings reply-all`; `node --test tests/shortcut-guard.mjs`.
- **Commit:** `git commit -m "Draw popovers and menus with Sigrid's Popover and DropdownMenu"`

### Task 14: Modal e SidePanel (P1, P2, dopo il 13)

- **Decisioni:**
  - `src/ui/Modal.tsx` su `DialogRoot` e `Dialog*`, e `src/ui/SidePanel.tsx` su `Sheet*` di Sigrid. Le misure dei dialoghi (`--modal-sm`, `--modal-md`, `--modal-lg`) e dei pannelli (`--panel-narrow`, `--panel-wide`) restano token di Qamille, passati con `className`.
  - Il focus torna a chi lo aveva: `restoreFocus` di Sigrid, se fa quello che fa oggi `useReturnFocus`; altrimenti `focus-return.ts` resta in `src/ui/`.
  - La chiusura con dati da salvare passa ancora dalle guardie di Qamille (`use-dirty-close-guard.ts`): l'Esc e la X di Sigrid chiamano `onOpenChange(false)`, e la guardia decide.
  - L'Esc durante una composizione IME non chiude (regola 24, Review Focus 5). Se Zag non lo fa da sé, l'adattatore lo ferma in `onEscapeKeyDown` con `imeComposing`. È un comportamento, non una forma, quindi sta nell'adattatore.
- **Files:** `desktop/src/ui/Modal.tsx`, `desktop/src/ui/SidePanel.tsx`, `desktop/src/ds/primitives/Modal.tsx`, `desktop/src/ds/primitives/SidePanel.tsx`, `desktop/src/ds/styles/ark/dialog.css` (via), i segnaposto «Task 14».
- **Test nuovi** (`primitives-overlays.mjs`):
  - «Esc durante una composizione IME in un campo del Modal non chiude il Modal; il secondo Esc lo chiude»;
  - «un Modal con modifiche non salvate chiede prima di chiudersi, con la X e con Esc»;
  - «cambiando lingua con un Modal aperto, il nome del pulsante di chiusura cambia» (Review Focus 4).
- **Differenze attese:** ombra al posto del bordo (accettata); X di Sigrid (da decidere).
- **Test di flusso:** `npm run test:browser -- primitives-overlays modal-actions-edge google-sign-in attachment-panel meetings calendar-flow close-guards`; `node --test tests/close-guards.mjs`.
- **Commit:** `git commit -m "Draw dialogs and side panels with Sigrid's Dialog and Sheet"`

### Task 15: componenti di prodotto su Sigrid (dopo C2)

**Decisioni:**
- **Dove vanno.** `RecipientsInput`, `NavList`/`NavItem`, `Icon`, `GoogleMark`, `ime.ts` e `focus-return.ts` (se resta) vanno in `desktop/src/ui/`, con i loro `.css` e le regole di `nav.css` e di `ark/recipients.css` legate alle loro classi radice. `cn` diventa quello di Sigrid.
- **`RecipientsInput`** si costruisce su `ChipsInput` e `Combobox` di Sigrid, se coprono incolla, suggerimenti, convalida e la riga che non cresce oltre `--recipients-max`. Altrimenti si costruisce su Ark direttamente, dentro `src/ui/`, l'unica cartella che può importare `@ark-ui/*`: con P1 è la stessa copia di Sigrid.
- **`NavList`** tiene la barra della voce attiva (`--accent-on-sidebar`). La barra si muove con una transizione CSS, spenta con la riduzione delle animazioni: la copia di Sidebar 001 non c'è più. `ds/unlumen/` sparisce.
- **`Icon`** tiene i disegni di Qamille.
- **Le regole rimaste.** `tests/ds-usage.mjs` vieta `@ark-ui/*` fuori da `src/ui/`.

**Files:** `desktop/src/ui/{RecipientsInput,NavList,Icon,GoogleMark}.tsx` e i loro `.css`, `desktop/src/ui/ime.ts`, `desktop/src/ds/primitives/*` (riesportazioni), `desktop/src/ds/unlumen/` (via), `desktop/src/ds/styles/nav.css` e `ark/recipients.css` (via), i segnaposto «Task 15», `desktop/tests/ds-usage.mjs`.

- [ ] **Step 0-6** della procedura comune, con `--require P1,P2`.
- **Test di flusso:** `npm run test:browser -- recipients recipients-dark sidebar-threads composer-tabs mailto a11y-keyboard`.
- **Commit:** `git commit -m "Build recipients, navigation and icons on Sigrid, as Qamille's own components"`

### Task 16: nomi dei token, fine degli alias, via `src/ds`, ADR-0011 v4 (P1-P4)

**Decisioni:**
- **I nomi.** Si riscrivono `var(--ground)`, `var(--text)` e gli altri nomi di `ALIASES` nei `.css` e nei `.tsx` con i nomi di Sigrid. Lo fa uno script una tantum, lanciato e non versionato; il diff si rilegge. `paintWindowBackground` legge `--color-paper`.
- **Gli alias e i token di Qamille.** Gli alias e `ALIASES` spariscono. I token solo di Qamille vanno in `desktop/src/theme/qamille-tokens.css`.
  - Con P3 i raggi passano come parametri di `COMPATTO` e Qamille non li scrive più.
  - Con P4 i titoli diventano gradini di Sigrid.
- **I percorsi.** Gli import `ds/primitives/<X>` diventano `ui/<X>` con lo stesso script. Poi `ds/primitives/` si cancella, e con lei tutta `desktop/src/ds/`.
  - `ds/styles/system.css` diventa `src/theme/base.css` (solo `@layer base`, le regole native).
  - `forced-colors.css` va in `src/theme/`, sempre importato per ultimo da `main.tsx`.
- **ADR-0011 v4** (`docs/adr/ADR-0011-v4-modulo-sistema-visivo.md`, che sostituisce la v3, più l'indice di `docs/adr/README.md`).
  - Il sistema visivo è Sigrid, preso con `sigrid:sync` senza pin.
  - `src/ui/` è il solo consumatore di Sigrid e di Ark.
  - I parametri sono `COMPATTO` e `QAMILLE_DENSITY`, e i token di Qamille stanno in `qamille-tokens.css`.
  - Invarianti:
    - INV-001: `src/ui/` non importa da `lib/` né da `components/`, salvo `lib/i18n` per le parole;
    - INV-002, INV-003, INV-007 riscritti per `src/ui/`;
    - INV-004: token di Sigrid o di Qamille, mai letterali;
    - INV-006: una sola implementazione per componente;
    - INV-008 sostituita da INV-009 (vedi Task 1).
  - Dipendenze ammesse: le peer di Sigrid alla sua versione esatta, più quelle di Qamille che restano.
- **I test.**
  - `ds-usage.mjs` diventa `ui-usage.mjs`.
  - `style-tokens.mjs` prende come ammessi i nomi di `THEME_VARIABLES` di Sigrid, quelli di `theme.css` di Sigrid e quelli di `qamille-tokens.css`.
  - `native-feel.mjs`: i percorsi nei controlli 1, 2, 4, 11 (`ime.ts`) e 14.
  - `test:ds` diventa `test:ui` in `package.json`, e `build` lo chiama.
- **Gli strumenti di `ds`.** `ds:export` (`scripts/ds-export.mjs`, `scripts/ds-export/`, `scripts/doop-sync/`, `tests/ds-export.mjs`) esportava il sistema visivo di Qamille, che ora è Sigrid: si ritirano.
- **Le dipendenze.** Si tolgono da `package.json` quelle che nessuno importa più (`tw-animate-css`, e `clsx` e `tailwind-merge` se restano solo dentro Sigrid). `@ark-ui/react`, `motion` e `@internationalized/date` restano, perché sono peer di Sigrid e Qamille li usa ancora.
- **La guida.** `desktop/src/ui/README.md` dice che cosa è di Qamille (adattatori, componenti di prodotto, token propri, parole) e che cosa è di Sigrid, e rimanda al README di Sigrid. Sostituisce `ds/README.md`.

**Files:** tutto `desktop/src/ds/` (via); `desktop/src/theme/qamille-tokens.css`, `base.css`, `forced-colors.css`; `desktop/src/ui/README.md`; i `.css` e `.tsx` dell'app (nomi e import); `desktop/src/main.tsx`; `desktop/src/reminder/reminder.css`; `desktop/src/lib/native-window.ts`; `desktop/package.json`, `package-lock.json`; `desktop/tests/{ui-usage,style-tokens,tokens-compatto,contrast,native-feel}.mjs`; `desktop/scripts/test-browser.mjs` (`OTHERS`); `docs/adr/ADR-0011-v4-modulo-sistema-visivo.md`, `docs/adr/ADR-0011-v3-modulo-sistema-visivo.md` (`superseded_by`), `docs/adr/README.md`; `docs/aspetto-nativo.md` (percorsi); `AGENTS.md` (i percorsi di «Aspetto nativo»).

- [ ] **Step 0:** `node tests/sigrid-preconditions.mjs --require P1,P2,P3,P4`.
- [ ] **Step 1: test che falliscono.**
  - `ui-usage.mjs` con le invarianti nuove.
  - `style-tokens.mjs` con un nome di alias, che ora è vietato.
  - `tokens-compatto.mjs` con i raggi tra i parametri.
  - `native-feel.mjs` con i percorsi nuovi.
  - `test('src/ds is gone')`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare, nell'ordine:
  1. i nomi;
  2. `qamille-tokens.css` e i parametri di P3 e P4, poi `npm run theme:css`;
  3. gli import;
  4. la cancellazione di `ds/`;
  5. i test;
  6. le dipendenze;
  7. i documenti.
- [ ] **Step 4:** `npm run build` (con `test:ui`), `node tests/native-feel.mjs`, `node tests/build-check.mjs --strict`, `node tests/first-paint.mjs`, `npm run test:browser` (tutti).
- [ ] **Step 5:** `visual-compare` con la base del ramo: nessuna differenza. Il task cambia nomi e percorsi, non valori.
- [ ] **Step 6: Commit** — `git commit -m "Use Sigrid's token names, keep only Qamille's own tokens, and retire src/ds (ADR-0011 v4)"`

### Task 17: verifica sull'exe e approvazione del proprietario

- [ ] **Step 1:** `build-release.bat --no-install`. Controllare che l'exe contenga `theme-boot.js` e che `public/sigrid-licenses/provenance.json` corrisponda alla copia sincronizzata.
- [ ] **Step 2:** con Qamille chiuso, sull'exe di `release/`, con cartelle dati di prova:
  - `node tests/release-theme-probe.mjs` (l'URL è `http://tauri.localhost/`), una volta con Windows chiaro e una con Windows scuro, oppure con l'aspetto salvato «Scuro»;
  - `node tests/window-native.mjs`;
  - `node tests/a11y-native.mjs`.

  `webview-recovery-native.mjs` solo se il piano ha cambiato `tauri.conf.json` o `native_feel.rs`.
- [ ] **Step 3:** `visual-compare` tra `capture/sigrid-baseline` e lo stato finale. L'elenco di ogni differenza va raggruppato per riga di «Che cosa cambierà a vista»: deve essere vuoto fuori da quell'elenco.
  - Screenshot delle schermate principali nei due schemi per il proprietario: posta, lettore, composer, calendario, rubrica, impostazioni, un dialogo, un menu, gli avvisi. È la «Confirmation» dell'ADR-0011 v4.
- [ ] **Step 4:** `npm run test:browser` (tutti), `node tests/native-feel.mjs`, `npm run test:ui`, `node tests/build-check.mjs --strict`, `node tests/first-paint.mjs`, `node --test tests/sigrid-wiring.mjs tests/sigrid-words.mjs tests/sigrid-preconditions.mjs tests/sigrid-sync.mjs`. I tre file nuovi (`sigrid-wiring`, `sigrid-words`, `sigrid-preconditions`) entrano in `NODE_TEST` di `test-browser.mjs`.
- [ ] **Step 5: Commit** — `git commit -m "Record Qamille on Sigrid and its checks on the release build"`

---

## Decisioni che il proprietario potrebbe voler cambiare

1. **Le differenze trovate in Sigrid 2.1.0 e non elencate dalla spec.** Sono nella tabella «da decidere» di «Che cosa cambierà a vista». Le più visibili:
   - interruttore più grande e acceso in inchiostro;
   - caselle spuntate in inchiostro;
   - schede con fondo tinto;
   - rosso e verde di stato del motore;
   - selezione più tinta;
   - barre di scorrimento a pillola;
   - icone di lucide dentro le primitive.

   Per ognuna, l'alternativa alla strada A è un parametro chiesto a Sigrid.
2. **Fermarsi dopo l'ondata A finché Sigrid non ha P1 e P2.** In alternativa si migrano adesso le primitive già su Ark (ondata B), accettando per un po' due copie di Zag nel bundle. Chi lo sceglie lascia che `build-check.mjs` conti due copie di Zag fino a P1, e accetta il rischio del Review Focus 3, che i test dei Task 8, 12 e 13 misurano.
3. **`sigrid:sync` in `predev` e `prebuild`**, quindi non più manuale (cambia `AGENTS.md`).
   - Ogni build prende l'ultima release.
   - Una release di Sigrid che cambia i colori di «Compatto» ferma `npm run build` finché qualcuno non rigenera `compatto.css` e ne guarda il diff, anche nel workflow di release.
   - L'alternativa è generare `compatto.css` a ogni build senza versionarlo: le build non si fermano, ma un cambio di colore passa senza che nessuno lo veda.
4. **Il carattere resta la copia variabile di Qamille** (`public/fonts`), non `fonts.css` di Sigrid, che ha solo 400, 500 e 600, mentre Qamille usa 550, 650 e 700.
5. **Avvisi:**
   - gli errori restano finché non li si chiude (`kind: 'error'` di Sigrid 2.1.0), mentre oggi spariscono dopo 12 s;
   - gli altri avvisi hanno una durata sola, proposta 10 s;
   - un errore si annuncia come `status`, preceduto da «Errore:», invece che come `alert`.

   L'alternativa è chiedere a Sigrid una durata per avviso e un annuncio immediato per gli errori, e aspettare quella release prima del Task 7.
6. **`UndoStack` resta di Qamille**, sotto la colonna degli avvisi di Sigrid. In alternativa gli annullamenti diventano avvisi di Sigrid con l'azione «Annulla» e il tasto Z: una sola colonna, ma un cambio di comportamento che la spec non elenca.
7. **`ds:export` e la guida Doop si ritirano** (Task 16), perché il sistema visivo da esportare ora è Sigrid.
