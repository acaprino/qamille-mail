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

## Sigrid

- Qamille usa sempre l'ultima release del `main` di `acaprino/sigrid`, senza pin: `npm run sigrid:sync`
  (in `desktop/`) scarica l'ultima GitHub Release (repo pubblica, senza login;
  `desktop/scripts/github-release.mjs`, come per ArtCraft), controlla gli SHA-256 del manifest e ogni file
  contro `provenance.json`, e sostituisce `desktop/vendor/sigrid/` (generata, ignorata da git). Usata solo se il
  suo tag è in `main`; senza rete resta la copia già presente. Non fissare versioni o commit di Sigrid in Qamille.
- Una release il cui tag non è `v<versione di provenance.json>`, o con file che non corrispondono a `provenance.json`,
  è rifiutata; se veniva dalla cache (`desktop/.cache/sigrid`), la sua cartella viene rimossa e il lancio
  successivo la scarica di nuovo.
- `sigrid:sync` è manuale: non fa parte di `predev` né di `prebuild` (a differenza di `artcraft:sync`), quindi va
  lanciato a mano quando serve una versione nuova di Sigrid.
- `SIGRID_DIR` forza un checkout locale di Sigrid (modifiche non ancora rilasciate): lo script esegue lì
  `npm run release -- --skip-test` e prende il pacchetto da `release/`. Quel comando ha bisogno delle dipendenze del
  checkout: `npm ci` una volta lì dentro, prima del primo `sigrid:sync` con `SIGRID_DIR`.
- L'unica fonte è il repository ufficiale `acaprino/sigrid`: niente fork, copie a mano o pacchetti di altra
  origine in `desktop/vendor/sigrid/`.

## Build Release dell'app desktop

- Per produrre `release/Qamille.exe` e installarla in `D:\Software\Qamille` usa `build-release.bat`
  (`--no-install` si ferma a `release\`; con `QAMILLE_SERVER` impostata costruisce l'edizione
  Professional).
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
- Dopo modifiche al ripristino di WebView2 (`recovery.rs`, `native_feel.rs`) esegui anche `node tests/webview-recovery-native.mjs` in `desktop/`, con Qamille chiuso. Nuovo `--ground`: aggiorna `tauri.conf.json` e `DARK_GROUND` in `native_feel.rs`.
