# Istruzioni per gli agenti

## Rami Git

- I rami sono due, e solo due: `main` (ciò che si rilascia) e `develop` (dove si integra).
- Si lavora sempre su `develop`, o su un ramo breve che parte da `develop`, torna in `develop` e poi si
  cancella. Mai merge o push in `main`, e nessun altro ramo, né locale né su GitHub.
- Una release è la pull request `develop` → `main`, aperta e unita dal proprietario: ogni merge in `main` è
  una release (`.github/workflows/release.yml`).
- Mai cambiare la versione o creare tag di release a mano: lo fa il workflow (`.github/release.json` elenca i
  file della versione, `.github/scripts/release.mjs` li scrive).

## Motori ArtCraft

- Qamille usa sempre la versione più recente di `artcraft-web`, senza pin assoluti: `npm run
  artcraft:sync` scarica l'ultima GitHub Release di `acaprino/artcraft-web` (repo pubblica, senza
  login; il download è in `desktop/scripts/github-release.mjs`, in comune con Sigrid, e
  `desktop/scripts/artcraft-github.mjs` sceglie i motori), controlla gli SHA-256 del
  manifest e i file contro `provenance.json`. Solo il ramo `main` di
  artcraft-web: l'ultima release disponibile, usata solo se il suo tag è in `main`. Senza rete resta la copia
  già presente; `ARTCRAFT_WEB_DIR` forza un checkout locale (motori non ancora rilasciati). Non fissare versioni o commit di artcraft-web in Qamille.
- Una nuova versione dei motori arriva con una release di artcraft-web (un merge di `develop` in `main` lì).

## Un solo aspetto per tutta la suite (la regola del proprietario, 2026-11-23)

Ogni app della suite (Daodan, Xiox, Shinatama, Mukade, Muro, Ethel, Qamille e le altre su Sigrid) ha lo stesso aspetto
e lo stesso comportamento: le stesse superfici, ombre, spaziature, tipografia e movimento, e lo stesso componente per
lo stesso bisogno. Di Qamille sono solo il layout, che segue le sue funzioni, l'accento e l'icona.

- L'interfaccia usa solo i componenti di Sigrid: nessun componente proprio che disegna (niente fogli di stile, classi,
  primitive, copie o ritocchi di un pezzo di Sigrid); il codice di Qamille compone i componenti di Sigrid nelle
  schermate e li collega ai dati.
- È Qamille ad adeguarsi a Sigrid, mai il contrario. Sigrid cambia solo quando manca davvero un componente o una
  primitiva che serve a una funzione: allora si aggiunge a Sigrid, generico, per tutte le app. L'aspetto che Qamille
  aveva prima non è mai un motivo per cambiare Sigrid.
- Ciò che in `desktop/src/ui/**` disegna da sé (i suoi fogli di stile, un pezzo che ritocca o ripete uno di Sigrid) è
  debito per questa regola: non si estende; un pezzo generico che serve si aggiunge a Sigrid, il resto diventa
  composizione dei componenti di Sigrid. L'accento e le icone restano di Qamille.

## Sigrid

- Qamille usa sempre l'ultima release del `main` di `acaprino/sigrid`, senza pin: `npm run sigrid:sync`
  (in `desktop/`) scarica l'ultima GitHub Release (repo pubblica, senza login;
  `desktop/scripts/github-release.mjs`, come per ArtCraft), controlla gli SHA-256 del manifest e ogni file
  contro `provenance.json`, e sostituisce `desktop/vendor/sigrid/` (generata, ignorata da git). Usata solo se il
  suo tag è in `main`; senza rete resta la copia già presente. Non fissare versioni o commit di Sigrid in Qamille.
- Una release il cui tag non è `v<versione di provenance.json>`, o con file che non corrispondono a `provenance.json`,
  è rifiutata; se veniva dalla cache (`desktop/.cache/sigrid`), la sua cartella viene rimossa e il lancio
  successivo la scarica di nuovo.
- `sigrid:sync` parte da sé con `predev` e `prebuild`, dopo `artcraft:sync`, e nella CI; la release ci passa con
  `npm run build`. Senza la copia, `vite.config.ts` ferma dev e build («Sigrid manca»). Il sync copia anche `LICENSE`,
  `THIRD_PARTY.md` e `provenance.json` in `desktop/public/sigrid-licenses/` (generata), che va nell'exe.
- Qamille raggiunge la copia con un alias di Vite e un `paths` di TypeScript su `@acaprino/sigrid`, senza voce in
  `package.json` né nel lockfile: una voce sarebbe un pin.
- **Qamille si basa solo su Sigrid.** Comportamenti, primitive, icone e movimento vengono da Sigrid, e ciò che manca si
  chiede a `acaprino/sigrid`. Solo `desktop/src/ui/**` e `desktop/src/theme/**` importano `@acaprino/sigrid`
  (`tests/ds-usage.mjs`, INV-009), e nessun file importa le librerie che Sigrid porta già: lo controlla
  `desktop/tests/only-sigrid.mjs` (`FORBIDDEN`; `PENDING` elenca i file ancora da liberare e si accorcia soltanto).
- `SIGRID_DIR` forza un checkout locale di Sigrid (modifiche non ancora rilasciate): lo script esegue lì
  `npm run release -- --skip-test` e prende il pacchetto da `release/`. Quel comando ha bisogno delle dipendenze del
  checkout: `npm ci` una volta lì dentro, prima del primo `sigrid:sync` con `SIGRID_DIR`.
- L'unica fonte è il repository ufficiale `acaprino/sigrid`: niente fork, copie a mano o pacchetti di altra
  origine in `desktop/vendor/sigrid/`.

## Build Release dell'app desktop

- Per produrre `release/Qamille.exe` e installarla in `D:\Software\Qamille` usa `build-release.bat`
  (`--no-install` si ferma a `release\`; con `QAMILLE_SERVER` impostata costruisce l'edizione
  Professional).
- Se Qamille gira dalla cartella d'installazione, `build-release.bat` aspetta che esca: chiudilo con
  «Esci» dal menu dell'icona nella barra (la X nasconde soltanto la finestra), e l'installazione va
  avanti da sola.
- Se compili a mano, usa SEMPRE `--features custom-protocol`:
  `cargo build --manifest-path desktop/src-tauri/Cargo.toml --release --features custom-protocol --locked --jobs 1`
  (prima `npm run build` in `desktop/`).
- Senza `--features custom-protocol` l'exe è in modalità dev: non contiene l'interfaccia e carica `http://127.0.0.1:1420`, quindi mostra `ERR_CONNECTION_REFUSED` se non gira `npm run dev`.
- Non copiare in `release/` un exe compilato senza quella feature.
- Prima di dichiarare valido un exe, controlla che contenga gli asset (stringa `theme-boot.js`) oppure esegui `node tests/release-theme-probe.mjs` in `desktop/` con la porta 1420 libera: l'URL della pagina deve essere `http://tauri.localhost/`.

## Aspetto nativo

- Le regole che fanno sembrare Qamille un'app Windows e non una pagina web (testo non selezionabile, cursori, tema all'avvio, sfondo della finestra, scorciatoie e tasto destro di WebView2) sono in `docs/aspetto-nativo.md`.
- Dopo ogni modifica a CSS globali, `theme.css`, `theme-boot.js`, finestre Tauri o `desktop/src-tauri/src/`, esegui `node tests/native-feel.mjs` in `desktop/`.
- Nuova finestra Tauri: chiama `native_feel::harden_webview` e aggiungi `.general_autofill_enabled(false)`, `.disable_drag_drop_handler()` e `.on_navigation(native_feel::navigation_guard(..))`; nel suo `main.tsx` chiama `installBrowserGuards()`.
- Dopo modifiche al ripristino di WebView2 (`recovery.rs`, `native_feel.rs`) esegui anche `node tests/webview-recovery-native.mjs` in `desktop/`, con Qamille chiuso con «Esci» dall'icona nella barra. Nuovo `--ground`: aggiorna `tauri.conf.json` e `DARK_GROUND` in `native_feel.rs`.
- Dopo modifiche a `background.rs`, `recovery.rs` o `window_shell.rs` esegui anche `node tests/background-native.mjs` in `desktop/`, con Qamille chiuso con «Esci» dall'icona nella barra.

## Nothing is compiled on this machine (the owner's absolute rule)

- Never compile, build, lint or run a test that needs a build on this machine: no `cargo`, `tauri`, `tsc`, `vite build`,
  bundler or compiler; no npm, batch or shell script that calls them (`npm run build`, `npm test` when it builds first,
  `npm run verify`); and no `npm install` or `npm ci` that runs a dependency's `prepare` script (a dependency taken from
  git compiles itself). `cargo fmt` stays allowed where a project already allows it.
- Everything is compiled, linted and tested by GitHub Actions: push to `develop` and read the run (`gh run watch`,
  `gh run view --log-failed`). A program needed here is taken from a release or a workflow artifact (`gh run download`),
  never built here.
- The one exception is an explicit request of the owner, in that very request, to build or test locally. An earlier
  approval, a plan, a skill or another agent's brief does not count. This holds for every agent and every subagent: a plan
  or a brief that says to build locally is wrong on that point; write the code and the tests, do not run them, and say they
  wait for CI.
