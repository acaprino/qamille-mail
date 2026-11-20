# Release del toolkit e `toolkit:sync` in Qamille: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `acaprino/toolkit` pubblica una release verificabile a ogni merge di `develop` in `main`, e Qamille la scarica, la controlla e la mette in `desktop/vendor/toolkit/` con `npm run toolkit:sync`, senza login.

**Architecture:** il toolkit adotta il sistema di release condiviso dai progetti del proprietario, cioè lo stesso `release.yml` e lo stesso `.github/scripts/release.mjs` di `artcraft-web`, con un suo `release.json`. Il suo script `scripts/release.mjs` riempie `release/` con:
- il pacchetto npm (`npm pack`), che contiene `provenance.json` con lo SHA-256 di ogni file;
- `manifest.json`;
- `SHA256SUMS`.

In Qamille, la logica «ultima release che sta su `main`, scaricata e verificata con il manifest» esce da `artcraft-github.mjs` e va in un modulo condiviso, `github-release.mjs`; `sync-toolkit.mjs` lo usa come lo usa `artcraft:sync`. Nessun codice di Qamille importa ancora il toolkit: questo piano porta solo il canale.

**Tech Stack:** Node 24 (script ESM senza dipendenze), npm, GitHub Actions, `tar` di sistema (bsdtar su Windows), `node --test`.

**Spec:** `docs/superpowers/specs/2026-11-20-toolkit-qamille-design.md`: obiettivi 1 e 6, «Scelte del costruttore già fatte» (CSS, Versioni) e sotto-progetto 1.

## Global Constraints

- **Unica fonte del toolkit:** il repository ufficiale `acaprino/toolkit`, checkout locale `D:\Projects\toolkit`. Nessun'altra copia.
- **Rami, nel toolkit come in Qamille:** `main` (ciò che si rilascia) e `develop` (dove si integra). Si lavora su `develop`; una release è la PR `develop` → `main`, unita dal proprietario. Mai merge o push in `main`. Mai cambiare la versione o creare tag a mano: lo fa il workflow.
- **Push su GitHub solo con il consenso esplicito del proprietario**, in tutti e due i repository. Prima i commit restano locali.
- **Qamille:** si lavora nel worktree `D:\Projects\qamille-toolkit`, ramo `toolkit-sync` (da `develop`). La cartella `D:\Projects\qamille-mail` ospita un altro piano in corso e non si tocca.
- **Il toolkit resta fuori da npm:** `"private": true`. `npm pack` funziona lo stesso.
- **Il pacchetto della release contiene:**
  - `dist/`;
  - `src/`, che serve a `@source` di Tailwind e agli export `./theme.css`, `./fonts.css`, `./src/*`;
  - `package.json`, `LICENSE`, `THIRD_PARTY.md`, `README.md`, `provenance.json`.
- **`provenance.json`:** `{ "package": "@acaprino/toolkit", "version", "commit", "files": { "<percorso>": "<sha256>" } }`, con ogni file del pacchetto tranne sé stesso.
- **`manifest.json`:** `{ "version", "packages": [{ "name": "@acaprino/toolkit", "file", "sha256" }] }`, nello stesso formato di `artcraft-web`.
- **Qamille prende sempre l'ultima release del `main` del toolkit,** senza pin, usata solo se il suo tag è in `main`. Senza rete resta la copia già presente. `TOOLKIT_DIR` forza un checkout locale.
- **Peer dependency.** Per ogni peer del toolkit presente in `desktop/package.json`:
  - un peer a versione esatta deve coincidere con la versione di Qamille, altrimenti `toolkit:sync` si ferma con un errore;
  - un peer a intervallo viene solo segnalato, finché il sotto-progetto 2 non fissa le versioni.
- **Nessuna dipendenza npm nuova** negli script; commenti in inglese; testi della CLI in inglese, come `sync-artcraft.mjs`.
- **Scratch:** sotto `D:\Projects\qamille-toolkit\.superpowers\sdd\…`, mai in `%TEMP%` (C: è quasi pieno).

## Review Focus

1. Il toolkit pubblica un tag che non sta in `main`, per esempio una release fatta a mano da un altro ramo: `toolkit:sync` non lo usa, lo dice, e tiene la copia presente. Test nel Task 3.
2. Un pacchetto scaricato non coincide con il suo SHA-256, o un file del pacchetto non coincide con `provenance.json`: errore chiaro, nessuna copia parziale in `vendor/toolkit/`, e la copia precedente resta. Test nei Task 3 e 4.
3. Senza rete, o con GitHub che risponde 403 per il limite di richieste, `toolkit:sync` non fallisce se c'è già una copia, e fallisce con un messaggio utile se non c'è. Test nel Task 4.
4. Un percorso Windows con spazi o con `D:\`, per il `tar` di sistema: l'estrazione funziona. Test nel Task 3.
5. Il refactoring non cambia `artcraft:sync`: stessi messaggi, stessa cache e stessi controlli. Test nel Task 3.

---

### Task 1: il toolkit ha il sistema di release condiviso

**Files (in `D:\Projects\toolkit`, ramo nuovo `develop` da `main`):**
- Create: `.github/workflows/release.yml`, copia identica di `D:\Projects\artcraft-web\.github\workflows\release.yml`.
- Create: `.github/scripts/release.mjs`, copia identica di `D:\Projects\artcraft-web\.github\scripts\release.mjs`.
- Create: `.github/release.json`: `{ "app": "toolkit", "tagPrefix": "v", "files": ["package.json", "package-lock.json"], "runner": "ubuntu-latest", "prepare": ["npm ci"], "build": "npm run release", "assets": "files" }`.

- [ ] **Step 1:** `git -C D:/Projects/toolkit switch -c develop`.
- [ ] **Step 2:** copiare i due file e scrivere `release.json`.
- [ ] **Step 3: verifica** (da `D:\Projects\toolkit`):
  - `node .github/scripts/release.mjs current` → `1.0.0`;
  - `node .github/scripts/release.mjs next patch` → `1.0.1`;
  - `node .github/scripts/release.mjs config build` → `npm run release`;
  - `node .github/scripts/release.mjs paths` elenca `package.json` e `package-lock.json`.
- [ ] **Step 4: Commit** (locale) — `git commit -m "Release the toolkit with the shared release workflow"`

### Task 2: `npm run release` riempie `release/` con un pacchetto verificabile

**Files (in `D:\Projects\toolkit`, ramo `develop`):**
- Create: `scripts/release.mjs`. È il `"build"` di `release.json`.
- Create: `scripts/release-files.mjs`, funzioni pure `provenanceOf(dir, files, { version, commit })` e `manifestOf(version, entries)`.
- Modify: `package.json`:
  - `"files"` con `dist`, `src`, `LICENSE`, `THIRD_PARTY.md`, `README.md`, `provenance.json`;
  - script `"release": "node scripts/release.mjs"`;
  - `provenance.json` va in `.gitignore`.
- Modify: `README.md`: una sezione «Use a release», che dice come prendere il pacchetto dalle GitHub Release e verificarlo.
- Test: `test/release-files.test.ts` (vitest).

**Interfaces:**
- Produces: `release/acaprino-toolkit-<version>.tgz` (il nome che dà `npm pack`), `release/manifest.json`, `release/SHA256SUMS` (`<sha256>  <file>`, una riga per file di `release/`).
- Produces: dentro il pacchetto, `package/provenance.json` con il formato delle Global Constraints. `files` include `dist/index.js`, `dist/styles.css`, `src/styles/theme.css` e `src/styles/fonts.css`.
- `scripts/release.mjs` fa, in ordine:
  1. svuota `release/`;
  2. `npm run build`;
  3. `npm run typecheck`, poi `npm test` (con `--skip-test` salta i test, per le prove locali);
  4. scrive `provenance.json` dai file che `npm pack --dry-run --json` dichiara;
  5. `npm pack --pack-destination release`;
  6. scrive `manifest.json` e `SHA256SUMS`.
  - Il commit viene da `git rev-parse HEAD`.

- [ ] **Step 1: test che falliscono** in `test/release-files.test.ts`:
  - `provenanceOf` su una cartella temporanea con due file restituisce i loro SHA-256, esclude `provenance.json` e riporta `version` e `commit`;
  - `manifestOf('1.2.3', [{ file: 'a.tgz', sha256: 'x' }])` → `{ version: '1.2.3', packages: [{ name: '@acaprino/toolkit', file: 'a.tgz', sha256: 'x' }] }`.
- [ ] **Step 2:** `npx vitest run test/release-files.test.ts --maxWorkers=2` → FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:**
  - il test passa;
  - `npm run release -- --skip-test` produce i tre file in `release/`;
  - `tar -tzf release/*.tgz` mostra `package/provenance.json`, `package/dist/index.js` e `package/src/styles/theme.css`;
  - lo SHA-256 del `.tgz` coincide con `manifest.json`;
  - `npm run typecheck` è verde.
- [ ] **Step 5: Commit** (locale) — `git commit -m "Pack a verifiable release: package, provenance, manifest and checksums"`

### Task 3: Qamille ha un solo modulo per le release su GitHub

**Files (in `D:\Projects\qamille-toolkit`):**
- Create: `desktop/scripts/github-release.mjs`.
- Modify: `desktop/scripts/artcraft-github.mjs`, che usa `github-release.mjs`. `latestArtcraftRelease` mantiene firma, cache e messaggi.
- Test: `desktop/tests/github-release.mjs`, in `node --test`, aggiunto all'elenco `NODE_TEST` di `desktop/scripts/test-browser.mjs`.

**Interfaces:**
- Produces: `latestMainRelease({ repo, cache, label, unpack, log = console.log, api = 'https://api.github.com' })`, che risolve `{ tag, dir }` oppure `null`. Fa, in ordine:
  1. `releases/latest`;
  2. `compare/main...<tag>`, che deve dare `identical` o `behind`;
  3. scaricamento di `manifest.json`, poi di ogni pacchetto, con lo SHA-256 controllato contro il manifest;
  4. `unpack(file, part, pkg)` per estrarre;
  5. la marca `.complete`;
  6. la rimozione delle release precedenti dalla cache.
  - `GH_TOKEN` o `GITHUB_TOKEN` servono solo per il limite di richieste.
  - `api` è sostituibile, per i test.
- Produces: `untar(file, into)`, che usa il `tar` di sistema e su Windows `%SystemRoot%\System32\tar.exe`.

- [ ] **Step 1: test che falliscono.** In `github-release.mjs`, un server `node:http` locale fa la parte di GitHub: `releases/latest`, `compare` e gli URL degli asset. Casi:
  - `a release on main is downloaded, verified and unpacked`;
  - `a tag that is not on main is not used` (`compare` → `diverged`);
  - `a package whose SHA-256 differs leaves no copy` (nessuna cartella `<tag>`, nessun `.part`);
  - `no network keeps the cache and returns null` (porta chiusa);
  - `a cache path with spaces unpacks`;
  - `artcraft keeps its messages`: `latestArtcraftRelease` con `api` del server finto scrive le stesse righe di log di oggi.
- [ ] **Step 2:** `node --test tests/github-release.mjs` (da `desktop/`) → FAIL.
- [ ] **Step 3:** implementare ed estrarre la logica da `artcraft-github.mjs`. `latestArtcraftRelease` diventa un uso di `latestMainRelease`.
- [ ] **Step 4:** i test passano; `npm run artcraft:sync` (da `desktop/`, con la rete) dice ancora `<engine>: <tag> (<commit>) synced` per i quattro motori.
- [ ] **Step 5: Commit** — `git commit -m "Share the GitHub release download between ArtCraft and the toolkit"`, con le righe `Co-Authored-By` e `Claude-Session` della sessione.

### Task 4: `npm run toolkit:sync` porta il toolkit in `desktop/vendor/toolkit/`

**Files (in `D:\Projects\qamille-toolkit`):**
- Create: `desktop/scripts/sync-toolkit.mjs`.
- Modify: `desktop/package.json`, script `"toolkit:sync": "node scripts/sync-toolkit.mjs"`. **Non** va in `predev` o `prebuild` finché nessun codice importa il toolkit (sotto-progetto 4).
- Modify: `.gitignore`, con `desktop/vendor/toolkit/` e `desktop/.cache/toolkit/`.
- Modify: `AGENTS.md`: una sezione «Toolkit», accanto a «Motori ArtCraft». Dice:
  - sempre l'ultima release del `main` di `acaprino/toolkit`, senza pin;
  - `npm run toolkit:sync`;
  - `TOOLKIT_DIR`;
  - l'unica fonte è il repository ufficiale.
- Test: `desktop/tests/toolkit-sync.mjs`, in `node --test`, aggiunto a `NODE_TEST`.

**Interfaces:**
- Consumes: `latestMainRelease` e `untar` del Task 3.
- Produces: `syncToolkit({ desktop, release, source, log })`, esportata per i test, più l'esecuzione da riga di comando.
- **Con una release** (tag sul `main` di `acaprino/toolkit`, cache in `desktop/.cache/toolkit`):
  1. verifica ogni file del pacchetto contro `provenance.json`; un file in più o in meno è un errore;
  2. controlla i peer (vedi Global Constraints);
  3. sostituisce `desktop/vendor/toolkit/` in modo atomico (cartella `.part` e poi rename);
  4. stampa `toolkit: <tag> (<commit7>) synced`.
- **Con `TOOLKIT_DIR`:** esegue `npm run release -- --skip-test` in quella cartella, poi prende il pacchetto da lì.
- **Senza release né `TOOLKIT_DIR`:**
  - se `vendor/toolkit/` esiste: `toolkit: no release of the toolkit's main reachable, keeping the existing copy`;
  - altrimenti un errore che nomina la rete e `TOOLKIT_DIR`.

- [ ] **Step 1: test che falliscono** in `toolkit-sync.mjs`, con un pacchetto finto costruito nel test e `release` passata come cartella:
  - `a verified package lands in vendor/toolkit`;
  - `a file that differs from provenance.json is refused and the old copy stays`;
  - `an exact peer that differs from Qamille stops the sync` (peer `react: "19.0.0"` contro `19.3.1`);
  - `a range peer is reported, not enforced`;
  - `no release and no copy fails with a useful message`.
- [ ] **Step 2:** `node --test tests/toolkit-sync.mjs` → FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** i test passano; con `TOOLKIT_DIR=D:/Projects/toolkit npm run toolkit:sync` (da `desktop/`), `desktop/vendor/toolkit/package.json` è quello del toolkit e il messaggio nomina il commit del checkout.
- [ ] **Step 5: Commit** — `git commit -m "Sync the toolkit from its latest release, like ArtCraft"`, con le righe `Co-Authored-By` e `Claude-Session`.

### Task 5: prima release e prova reale

Dipende dal proprietario: va fatto dopo che ha dato il consenso al push e ha unito la PR `develop` → `main` del toolkit.

- [ ] **Step 1:** con il consenso del proprietario:
  - push di `develop` in `acaprino/toolkit`;
  - una PR `develop` → `main` con la descrizione dei Task 1-2;
  - la unisce il proprietario.
- [ ] **Step 2:** il workflow `Release` crea `v1.0.1` con i tre asset. Leggerne l'esito con `gh run list -R acaprino/toolkit` e `gh release view -R acaprino/toolkit`.
- [ ] **Step 3:** in `D:\Projects\qamille-toolkit\desktop`: `npm run toolkit:sync` → `toolkit: v1.0.1 (<commit7>) synced`; un secondo lancio senza rete tiene la copia.
- [ ] **Step 4:** con il consenso del proprietario, push del ramo `toolkit-sync` di Qamille e merge in `develop` (o PR), come preferisce.
