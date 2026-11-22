# Fase 3 nativa (funzioni native): piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** dare a Qamille le funzioni native approvate dal proprietario in Auto («Fai tu», poi «VAI»): presenza in background (tray, chiusura nella tray, avvio all'accesso), posizione della finestra, notifiche di sistema e stato sulla barra delle applicazioni, link `mailto:`, Apri e Mostra nella cartella, menu contestuale nativo delle righe, chiave della cassaforte protetta nella Professional, e la preparazione di updater e firma.

**Architecture:** ogni funzione ha un modulo Rust suo in `desktop/src-tauri/src/` (`background.rs`, `placement.rs`, `notifications.rs`, `taskbar.rs`, `launch_args.rs`, `mail_handler.rs`, `saved_files.rs`, `row_menu.rs`, `updates.rs`) e, accanto, un modulo di regole pure senza dipendenze che si prova con `rustc --test` come `shell_rules.rs`. I plugin si usano **solo da Rust**, dietro comandi dell'app: `capabilities/main.json` non cambia e la pagina non ottiene permessi di plugin. I segnali che fanno partire una notifica sono eventi tipizzati del crate `app` (`AppEvent`), intercettati dal sink del desktop (`runtime.rs`) e non inoltrati alla pagina. La pagina resta l'unica a decidere la chiusura vera (guardie di `window-close.ts`); Rust decide solo se il pulsante Chiudi nasconde o chiude.

**Tech Stack:** Tauri 2.12.5 (feature nuova `tray-icon`), tauri-plugin-single-instance 2.5.5, plugin nuovi `tauri-plugin-autostart`, `tauri-plugin-notification`, `tauri-plugin-updater` (2.x compatibili con tauri 2.12.5 del lockfile), wry 0.57.1, webview2-com 0.39.1, windows 0.62.2, windows-sys 0.61 (crate `app`), rfd 0.15.4, `open` 5; React 19.3.1, @ark-ui/react 5.39.3. Test: `node --test`, Playwright (anteprima, `npm run test:browser -- <nomi>`), `node tests/native-feel.mjs`, `cargo test`, `rustc --test` sui moduli puri, test `*-native.mjs` sull'exe di release.

**Spec:**
- `docs/audit-tauri-nativo.md`, sezione «Quarta applicazione della skill (2026-11-20)», tabella «Funzioni native (fase 3): stato attuale», colonna «Proposta in Auto»; difetti Q8, Q10, Q22.
- `docs/superpowers/plans/2026-11-20-nativo-fase-2.md`, elenco finale «Dopo questo piano: fase 3».
- Skill `tauri-native-desktop` 2.1.0 (`C:/Users/alfio/.claude/plugins/cache/tauri-app-builder/tauri-app-builder/2.1.0/skills/tauri-native-desktop/`): `references/background-presence.md`, `notifications.md`, `os-integration.md`, `context-menus.md`, `credentials.md`, `packaging-updates.md`, `native-plugins.md`, `acceptance.md`. Ogni task legge solo le parti che gli servono.

## Global Constraints

- **Ramo:** `nativo-fase-3` (da `develop` a `2a7d942`). Mai `main`, mai cambiare la versione, mai creare tag (`AGENTS.md`). I rami dei worktree sono solo locali (`fase3/task-N`, da `nativo-fase-3`), non si pubblicano e si cancellano subito dopo il merge in `nativo-fase-3`.
- **Dipendenze Rust:** ammessi solo i plugin ufficiali elencati nel Tech Stack, la feature `tray-icon` di `tauri`, feature nuove di `windows` 0.62.2 e di `windows-sys` 0.61 già presenti. Ogni plugin entra con l'ultima 2.x che non sposta `tauri` da 2.12.5 nel lockfile (`cargo update -p <plugin> --precise <versione>` se serve); `Cargo.lock` cambia nello stesso commit. Lo cambiano solo i Task 1, 3 e 8, in ondate diverse.
- **Permessi:** i plugin si chiamano da Rust. `capabilities/main.json` e `capabilities/reminder.json` non cambiano: nessun `core:menu`, `core:tray`, `notification:*`, `autostart:*`, `updater:*`. I comandi nuovi dell'app li autorizza da sé `build.rs`, che legge `generate_handler!`. Il test 8 di `native-feel.mjs` (regola 20, privilegi minimi) resta verde.
- **Nessuna dipendenza npm nuova** (ADR-0011 v3, decisione 5). Niente `@tauri-apps/plugin-*`.
- **Modulo `ds/` (ADR-0011 v3):** i componenti dell'app non importano `@ark-ui/*`; un comportamento nuovo di un controllo va in una primitiva di `desktop/src/ds/primitives/`. Colori, raggi, ombre e testi solo da token (`tests/style-tokens.mjs`). `npm run test:ds` verde.
- **Due lingue:** ogni testo visibile in italiano e in inglese. Nel frontend `tr('italiano', 'English')`. In Rust (tray, menu, dialoghi, notifiche) i due testi stanno nel codice e si sceglie con `window_shell::italian()`. Nessuna voce nuova in `desktop/src/lib/backend-text.ts`.
- **Invio ed Esc:** ogni gestore nuovo controlla prima `imeComposing(event)` (`desktop/src/ds/ime.ts`); lo verifica il test 11.
- **Controlli nativi:** dopo ogni modifica a CSS globali, `theme.css`, `theme-boot.js`, finestre Tauri o `desktop/src-tauri/src/`: `node tests/native-feel.mjs` in `desktop/`, tutto `ok`. I controlli nuovi hanno numeri assegnati (tabella «Lavoro in parallelo») e vanno sotto il segnaposto del proprio task.
- **Ripristino di WebView2:** dopo una modifica a `recovery.rs` o `native_feel.rs` (Task 1 e 4): `node tests/webview-recovery-native.mjs` in `desktop/`, con Qamille chiuso, sull'exe di release.
- **Build di release:** prima `npm run build` in `desktop/`, poi `cargo build --manifest-path desktop/src-tauri/Cargo.toml --release --features custom-protocol --locked --jobs 1` con `CARGO_TARGET_DIR=D:\Projects\qamille-mail\app\target-release`. Prima di usare l'exe: contiene `theme-boot.js`. Mai un exe senza `custom-protocol` in `release/`.
- **Worktree in parallelo:** ogni worktree ha il suo `CARGO_TARGET_DIR` per `cargo check` e `cargo test` (`<worktree>\target-dev`, mai `app\target-release`, che serve solo alla build di release del Task 9) e il suo `npm ci` in `desktop/`. Le regole pure si provano con `rustc --edition 2024 --test <file.rs> -o %TEMP%\<nome>.exe`, che non compila Tauri.
- **Pagina nascosta:** con la finestra nella tray la pagina gira ma non disegna, e Chromium rallenta i suoi timer. Tutto ciò che deve accadere a finestra nascosta (invii, notifiche, barra delle applicazioni, promemoria) sta in Rust, mai in un timer della pagina.
- **Registro di Windows:** ogni voce che l'app scrive in `HKCU` ha un nome per cartella dati (il Professional installato usa il nome fisso «Qamille»), si toglie con l'interruttore che l'ha creata e con il disinstallatore. I test che scrivono nel registro lo ripuliscono in `finally` e non toccano le voci delle copie del proprietario (`D:\Software\Qamille`, `D:\Software\Qamille-Professional`).
- **Finestre nuove:** questo piano non crea finestre Tauri. Se un task ne avesse bisogno, segue la checklist «Nuova finestra Tauri» di `docs/aspetto-nativo.md`.
- **Documenti:** regola nuova o cambiata del comportamento nativo → la riga sua in `docs/aspetto-nativo.md`, nello stesso commit (segnaposto del Task 0). `docs/audit-tauri-nativo.md` lo aggiorna solo il Task 9, per non far collidere i task sulle righe vicine della stessa tabella. Commenti nel codice in inglese.
- **Prima di implementare** ogni task dice al controller, in poche righe, che cosa cambia per chi usa l'app e nel progetto (`background-presence.md`, «What changes and where»): il controller lo riporta al proprietario nel resoconto della fase.

## Lavoro in parallelo

### Ondate

| Ondata | Task | Dipende da |
|---|---|---|
| 0 (seriale, controller) | 0. Punti di innesto | — |
| A (in parallelo) | 1. Presenza in background · 2. Posizione della finestra · 5. Apri e Mostra nella cartella · 6. Menu contestuale delle righe | 0 |
| B (in parallelo) | 3. Notifiche e barra delle applicazioni · 4. mailto · 7. Chiave della cassaforte (Professional) | 1 unito (3 e 4); 7 non dipende da niente ed è in B solo per non avere cinque build di Tauri insieme |
| C | 8. Updater e firma (Professional), fino ai segreti | 1 e 3 uniti |
| Finale | 9. Verifica sull'exe e documenti | tutti |

Ordine dei merge in ogni ondata: prima il task che altri aspettano (in A il Task 1), poi gli altri in qualsiasi ordine. Un conflitto in un file generato (`desktop/src-tauri/permissions/main-window.toml`, `gen/schemas/*`) non si risolve a mano: si rigenera con `cargo check` sul ramo unito.

### File condivisi: proprietario e punto d'innesto

| File | Proprietario | Chi altro lo tocca, e dove esattamente |
|---|---|---|
| `desktop/src-tauri/src/main.rs` | Task 1 (A) | Ogni task dichiara i suoi `mod` sotto il proprio segnaposto del Task 0 (`// phase 3, Task N: modules`). **A:** Task 5 aggiunge una riga di comandi in `generate_handler!` subito dopo `attachment_save,`; Task 6 subito dopo `queue_mail_action,`. **B:** Task 3 aggiunge `.plugin(...)` dopo il plugin autostart e una riga di comandi dopo `indexing_status,`; Task 4 una riga dopo `open_link,` e la lettura degli argomenti d'avvio nel segnaposto `// phase 3, Task 4: launch arguments` che il Task 1 lascia in `setup_app`. **C:** Task 8 la registrazione condizionale del plugin, una riga dopo `app_edition,` e il passaggio da `.run(context)` a `.build(context)` + `.run(…)` (vedi Task 8). |
| `desktop/src-tauri/Cargo.toml`, `Cargo.lock` | Task 1 (A) | **A:** Task 5 solo una feature nella lista di `windows` (righe 37-49), se `SHOpenFolderAndSelectItems` la chiede; nessun cambio al lockfile. **B:** Task 3 il plugin delle notifiche. **C:** Task 8 il plugin updater. |
| `desktop/src-tauri/src/window_shell.rs` | Task 2 (A) | **A:** Task 1 tocca solo le funzioni del menu di sistema (`add_reload_entry`, `label_reload_entry`, il ramo `WM_SYSCOMMAND` di `main_proc`) e aggiunge una riga in `window_appearance` dopo `save_preferences(next);`: `crate::background::relabel(window.app_handle(), italian.unwrap_or(...))`. |
| `desktop/src-tauri/src/native_feel.rs` | Task 1 (A) | **B:** Task 4 cambia solo il ramo `mailto:` di `navigation_guard` e `new_window_guard`. |
| `desktop/src-tauri/src/background.rs` (nuovo) | Task 1 (A) | **B:** Task 4 aggiunge una riga in `second_instance`; Task 3 legge `installed()` e chiama `set_tray_tooltip`. **C:** Task 8 chiama `quit` e legge `installed()`. |
| `desktop/src-tauri/src/runtime.rs` | Task 3 (B) | nessuno |
| `desktop/src-tauri/windows/installer-hooks.nsh` | Task 1 (A) | **B:** Task 3 aggiunge la rimozione della chiave AUMID dentro il blocco `${If} $UpdateMode <> 1` già esistente, dopo il `Delete` del VBS e prima del blocco dei dati; Task 4 aggiunge `NSIS_HOOK_POSTINSTALL` in fondo al file e un blocco nuovo in fondo a `NSIS_HOOK_POSTUNINSTALL`, dopo quello dei dati. |
| `desktop/src/App.tsx` | nessuno (solo innesti) | **A:** Task 5 cambia `notify` (`:349`) ed `exportEmails` (`:371-395`); Task 6 aggiunge la prop `rowMenu` al `<MailList` (`:1777`) e la funzione `openRowMenu` subito dopo `action` (`:1095-…`). **B:** Task 3 una riga `installNotificationOpen(...)` dopo `unsubscribers.push(installWindowShell());` (`:1523`); Task 4 la funzione `composeMailto` subito dopo `compose` (`:1239-1257`) e una riga `installMailtoRouting(...)` dopo la registrazione della guardia `'settings'` (`:1528-1545`). **C:** Task 8 una riga accanto a `<Toaster`. |
| `desktop/src/components/Settings.tsx` | Task 1 (A) | nessuno: i Task 3 e 4 entrano dai segnaposto di `DesktopSettings.tsx` |
| `desktop/src/components/DesktopSettings.tsx` (nuovo) | Task 1 (A) | **B:** Task 3 sostituisce la riga `{/* phase 3, Task 3: notifications */}`; Task 4 la riga `{/* phase 3, Task 4: default mail app */}`. Le due righe sono separate da altro markup. |
| `desktop/src/lib/preview.ts` | nessuno (solo innesti) | ogni task aggiunge i suoi `case` accanto al comando affine: Task 1 dopo `case 'save_settings'` (`:755`), Task 5 dopo `case 'attachment_save'` (`:815`), Task 6 dopo `case 'queue_mail_action'` (`:654`), Task 3 dopo `case 'indexing_status'` (`:858`), Task 4 prima di `case 'open_link'` (`:951`). |
| `desktop/tests/native-feel.mjs` | Task 0 (segnaposto) | ogni task inserisce i suoi controlli subito sotto il proprio segnaposto: 19-20 Task 1, 21 Task 2, 22 Task 5, 23 Task 6, 24-25 Task 3, 26 Task 4, 27 Task 8. Task 1 riscrive anche il controllo 17. |
| `docs/aspetto-nativo.md` | Task 0 (segnaposto) | ogni task sostituisce la sua riga segnaposto: 26-27 Task 1, 28 e 30 Task 3, 29 Task 2, 31 Task 5, 32 Task 4, 33 Task 6: in quest'ordine due righe che cambiano nella stessa ondata non sono mai vicine, e git le unisce da sé. La regola 25 la cambia il Task 1 (A) e poi il Task 4 (B). |
| `desktop/scripts/test-browser.mjs` | Task 9 | i file `node --test` nuovi entrano in `NODE_TEST` solo nel Task 9 (prima si lanciano con `node --test` diretto) |

## Review Focus

1. **Esci con un invio ancora annullabile.** Esci (tray o menu di sistema) con una mail nella finestra di Annulla invio: la mail parte prima dell'uscita, oppure compare una domanda che dice che non è partita e quando partirà. Mai un'uscita che la rimanda in silenzio al lancio successivo (Q8). Anche le uscite senza pagina (ripristino, chiusura forzata) la inviano. Test: Task 1, step 1 (`undo_send_now_*` nel crate `app`, `tests/undo-close.mjs`), e scenario B del Task 9.
2. **La finestra che torna è disegnata, davanti e sullo schermo.** Tray, seconda istanza, clic su una notifica, `mailto:` da un altro programma: tutti passano da `background::restore`, che rende visibile la WebView **prima** di `show`, poi `unminimize` e `set_focus`. Una finestra nascosta (tray o avvio all'accesso) non apre mai da sola il dialogo «Qamille non si è aperto». Test: Task 1, controllo 19 e `background-native.mjs`; Task 9, scenari C e D.
3. **Mai su un monitor scollegato.** La posizione salvata si applica solo se cade su un monitor collegato, con la barra del titolo nell'area di lavoro, anche dopo un cambio di scala; altrimenti la finestra si apre al centro come oggi. Test: Task 2, `placement_*` in `placement.rs`; Task 9, scenario E (coordinate fuori schermo scritte a mano).
4. **Nessuna valanga di notifiche.** La prima sincronizzazione di un account nuovo, lo storico (`history_fetch_folder`), la posta arrivata a Qamille chiuso e i messaggi già letti non notificano; tre o più messaggi insieme danno una sola notifica; con la finestra in primo piano la posta nuova non notifica. Test: Task 3, `notify_rules_*` e `mail_arrived_*`; Task 9, scenario G.
5. **La chiave della cassaforte non si perde.** Nella Professional la migrazione di `credentials.key` verso DPAPI interrotta in qualsiasi punto lascia sempre una chiave leggibile; una cartella dati copiata su un altro utente o computer lo dice e non cancella niente. Test: Task 7, `vault_key_*` in `secret.rs`.

## Interfacce tra i task

| Nome | Firma | Da | Per |
|---|---|---|---|
| `background::restore` | `pub fn restore(app: &AppHandle)` | 1 | 3, 4 |
| comando `restore_main_window` | `fn restore_main_window(app: AppHandle)` | 1 | `window-close.ts` (veto durante Esci) |
| `background::second_instance` | `pub fn second_instance(app: &AppHandle, args: Vec<String>, cwd: String)` | 1 | 4 (una riga) |
| `background::quit` | `pub fn quit(app: &AppHandle)`: alza `QUITTING` e chiama `close()` sulla principale, così decidono le guardie | 1 | 8 |
| `background::hides_on_close` / `is_quitting` | `pub fn hides_on_close() -> bool`; `pub fn is_quitting() -> bool` | 1 | `recovery.rs` |
| `background::installed` | `pub fn installed() -> bool` (c'è `uninstall.exe` accanto all'exe: installatore NSIS) | 1 | 3, 4, 8 |
| `background::set_tray_tooltip` | `pub fn set_tray_tooltip(app: &AppHandle, text: &str)` | 1 | 3 |
| `background::relabel` | `pub fn relabel(app: &AppHandle, italian: bool)` | 1 | `window_shell::window_appearance` |
| `background::AUTOSTART_ARG` | `pub const AUTOSTART_ARG: &str = "--autostart"` | 1 | 4 (un avvio con `mailto:` mostra sempre la finestra) |
| `native_feel::prepare_main_window` | `pub fn prepare_main_window(window: &WebviewWindow)`: tema, zoom, posizione e sfondo, senza `show`; chiama `window_shell::apply_startup_preferences`, dove il Task 2 applica la posizione | 1 | 1, 2 |
| `native_feel::set_webview_visible` | `pub fn set_webview_visible(window: &WebviewWindow, visible: bool)` (`ICoreWebView2Controller::SetIsVisible`) | 1 | 1 |
| comando `close_seen` | ora `fn close_seen() -> bool`: `true` se la pagina deve chiudere davvero, `false` se Rust ha nascosto la finestra | 1 | `window-close.ts` |
| `UndoService::send_now` / `unsettled` | `pub fn send_now(&self) -> AppResult<usize>`; `pub fn unsettled(&self) -> AppResult<usize>` | 1 | comando `undo_send_now` |
| comando `undo_send_now` | `async fn undo_send_now(state, wait_ms: u32) -> AppResult<UndoSendResult>`; `UndoSendResult { remaining: u32 }` | 1 | `lib/undo-close.ts` |
| `DesktopSettings` | `export default function DesktopSettings()` con due segnaposto | 1 | 3, 4 |
| `WindowPreferences.placement` | `Option<placement::SavedPlacement>` (serde default) | 2 | — |
| `AppEvent::MailArrived` | `{ account_id: u32, ids: Vec<u64> }` | 3 | sink del desktop |
| `AppEvent::ScheduledSendFailed` | `{ draft_id: u64, subject: String }` | 3 | sink del desktop |
| `AppEvent::SignInExpired` | `{ account_id: Option<u32> }` (`None` = il server Qamille) | 3 | sink del desktop |
| `notifications::observe` | `pub fn observe(app: &AppHandle, event: &AppEvent) -> bool` (`true`: evento solo di sistema, non va alla pagina) | 3 | `runtime.rs` |
| `taskbar::observe` | `pub fn observe(app: &AppHandle, event: &AppEvent)` | 3 | `runtime.rs` |
| `WorkspaceService::inbox_unread` | `pub fn inbox_unread(&self) -> AppResult<usize>`: lo stesso conto di `counts["Inbox"]` di `snapshot` (`app/src/application/workspace.rs:43-52`), con una funzione condivisa, così icona e sidebar non divergono | 3 | `taskbar.rs` |
| evento `notification-open` | payload `{ kind: 'mail' \| 'scheduled' \| 'sign-in', email_id?: number, account_id?: number }` | 3 | `App.tsx` (`installNotificationOpen`) |
| `parseMailto` | `export function parseMailto(href: string): MailtoFields \| null`; `MailtoFields = { to: string[]; cc: string[]; bcc: string[]; subject: string; body: string }` | 4 | `MessageHtml.tsx`, `App.tsx` |
| `requestCompose` / `onComposeRequest` | `export function requestCompose(fields: MailtoFields): void`; `export function onComposeRequest(handler: (f: MailtoFields) => void): () => void` | 4 | `MessageHtml.tsx`, `App.tsx` |
| evento `compose-mailto`, comando `take_launch_mailto` | payload `string` (l'indirizzo `mailto:`); `fn take_launch_mailto() -> Option<String>` | 4 | `App.tsx` |
| `saved_files::remember` | `pub fn remember(path: PathBuf) -> u64` | 5 | `attachment_save`, `export_mail` |
| comandi `saved_file_open` / `saved_file_reveal` | `fn saved_file_open(token: u64) -> AppResult<()>`; `fn saved_file_reveal(token: u64) -> AppResult<()>` | 5 | `AttachmentPreview.tsx`, `App.tsx` |
| `SavedFile` | `attachment_save -> Option<SavedFile { path: String, token: u64 }>`; `ExportSaved` guadagna `token: u64` | 5 | — |
| `notify` del Toaster | `export function notify(message: string, error = false, actions: NoticeAction[] = [])`; `NoticeAction = { label: string; run: () => void }`, al massimo 2 | 5 | `AttachmentPreview.tsx`, `App.tsx` |
| comando `show_row_menu` | `async fn show_row_menu(window: WebviewWindow, items: Vec<RowMenuEntry>, x: f64, y: f64) -> Option<String>`, con `x`, `y` in pixel fisici dell'area client (`clientX * devicePixelRatio`, come `show_system_menu`); `RowMenuEntry { value: Option<String>, label: String, enabled: bool, checked: Option<bool> }` (`value: None` = separatore) | 6 | `lib/row-menu.ts` |
| `rowMenuEntries` | `export function rowMenuEntries(rows: EmailSummary[], view: Folder, locked: boolean): RowMenuEntry[]` | 6 | `App.tsx` |
| prop `rowMenu` di `MailList` | `rowMenu?: (id: number, at: { x: number; y: number }) => void` | 6 | `App.tsx` |
| `secret::KeyStore` | `enum KeyStore { Portable, Dpapi }`; `Vault::with_key_store(root, store)` | 7 | — |
| `updates::plugin` / `run_event` | `pub fn plugin() -> Option<TauriPlugin<Wry>>`; `pub fn run_event(app: &AppHandle, event: &RunEvent)` | 8 | `main.rs` |

---

### Task 0: punti di innesto (controller, prima dell'ondata A)

**Files:**
- Modify: `desktop/tests/native-feel.mjs` (prima di `if (failures.length)`, `:522`)
- Modify: `docs/aspetto-nativo.md` (tabella «Le regole», dopo la riga 25)
- Modify: `desktop/src-tauri/src/main.rs` (dopo `mod zone;`, `:9`)

- [ ] **Step 0:** in `main.rs`, dopo `mod zone;`, sette righe di commento separate da una riga vuota: `// phase 3, Task 1: modules`, poi Task 2, 3, 4, 5, 6, 8. Ogni task scrive i suoi `mod` sotto la propria riga: inserimenti in punti diversi, che git unisce senza conflitti.
- [ ] **Step 1:** in `native-feel.mjs`, otto righe di commento separate da una riga vuota, una per task, nell'ordine dei numeri: `// phase 3, Task 1: checks 19-20`, `// phase 3, Task 2: check 21`, `// phase 3, Task 5: check 22`, `// phase 3, Task 6: check 23`, `// phase 3, Task 3: checks 24-25`, `// phase 3, Task 4: check 26`, `// phase 3, Task 8: check 27`.
- [ ] **Step 2:** in `aspetto-nativo.md`, righe segnaposto 26-33, una per regola: `| 26 | (fase 3, Task 1: chiusura nella tray e ripristino) | — | — |`, `| 27 | (fase 3, Task 1: avvio all'accesso) | — | — |`, `| 28 | (fase 3, Task 3: notifiche di sistema) | — | — |`, `| 29 | (fase 3, Task 2: posizione della finestra) | — | — |`, `| 30 | (fase 3, Task 3: barra delle applicazioni) | — | — |`, `| 31 | (fase 3, Task 5: file salvati) | — | — |`, `| 32 | (fase 3, Task 4: link mailto) | — | — |`, `| 33 | (fase 3, Task 6: menu delle righe) | — | — |`.
- [ ] **Step 3:** `node tests/native-feel.mjs`: tutto `ok` (i commenti non cambiano niente).
- [ ] **Step 4: Commit** — `git commit -m "Leave one insertion point per phase-3 task in the native checks and rules"`

---

### Task 1: presenza in background (tray, chiusura nella tray, avvio all'accesso, Q8, Q22)

**Decisioni:**
- **Chiudi nasconde nella tray**, per default anche per chi usa già Qamille. La X, Alt+F4, «Chiudi» del menu di sistema e «Chiudi finestra» della barra delle applicazioni nascondono la finestra; niente guardie, perché niente si chiude.
- **Avviso alla prima chiusura:** un dialogo di Windows (rfd, senza owner, la finestra è già nascosta), una sola volta: «Qamille resta aperto nell'area di notifica: invia i messaggi programmati, mostra i promemoria e riceve la posta. Per uscire usa Esci dal menu dell'icona.», con «OK» e «Esci, e d'ora in poi chiudi Qamille». Il secondo pulsante spegne la chiusura nella tray ed esce. Chi aggiorna lo vede alla prima X dopo l'aggiornamento: è la sua occasione per tornare al comportamento di prima.
- **Esci** sta nel menu della tray e nel menu di sistema (Alt+Spazio, voce nuova sotto «Chiudi»). Passa dalle guardie della pagina come oggi la X: `quit` alza `QUITTING` e chiama `close()` sulla finestra principale. Se una guardia pone il veto, la pagina chiama `restore_main_window`, così la domanda si vede. Prima di uscire la guardia `undo-queue` invia la coda di Annulla invio (Q8).
- **Tray:** icona dell'app (`default_window_icon`), tooltip «Qamille», menu «Mostra Qamille» ed «Esci». Clic sinistro = `restore`; il menu si apre col destro.
- **Una sola via di ripristino:** `background::restore` = `set_webview_visible(true)`, `show`, `unminimize`, `set_focus`. La usano tray, seconda istanza, notifiche (Task 3), `mailto:` (Task 4) e il veto durante Esci.
- **WebView in pausa:** a finestra nascosta o ridotta a icona `set_webview_visible(false)`; al ripristino `true`, prima di `show`. Il cambio di stato si legge da `WindowEvent::Resized` con `is_minimized()`.
- **Nessun dialogo di ripristino da una finestra nascosta:** il controllo dei 15 s (`recovery::watch_startup`) chiede solo se la finestra principale è visibile. Se scade a finestra nascosta, si annota, e il primo `restore` offre di ricaricare (`recovery::offer_reload_if_not_ready`) quando la pagina non ha ancora chiamato `frontend_ready`. Un ricaricamento della pagina non mostra mai una finestra nascosta.
- **Avvio all'accesso:** spento per default, `tauri-plugin-autostart` con l'argomento `--autostart`. Un avvio con `--autostart` resta nella tray: `prepare_main_window` sì, `show` no; il primo `restore` mostra la finestra già pronta. Lo stato si legge sempre dall'OS (`is_enabled()`, che vede anche l'interruttore di Gestione attività). Il valore `Run` si chiama «Qamille» nel Professional installato e `Qamille (<nome della cartella>, <tag>)` nelle copie portatili, con `<tag>` le prime 8 cifre dell'hash dell'identificatore per cartella dati (`main.rs:59-63`): così ogni copia ha la sua voce (Q22). Se il plugin risolto non accetta un nome (`Builder::app_name`), il valore si scrive con `windows::Win32::System::Registry`, con lo stesso nome e lo stesso argomento.
- **Il file VBS del worker sparisce:** l'avvio all'accesso è Qamille nella tray, che fa già quello che faceva il worker (scheduler e coda MCP in `scheduler.rs`). Migrazione una volta, all'avvio e mai nella demo: se `Qamille-worker.vbs` nomina questo exe, si accende l'avvio all'accesso di questa copia e si cancella il file; se nomina un exe che non esiste più, si cancella e basta; se nomina un'altra copia esistente, si lascia (lo migrerà lei). `worker_autostart` resta nel tipo `Config` (che ha `deny_unknown_fields`) ma non si mostra più e la migrazione lo rimette a `false`.
- **Preferenze:** `data/background.json` (`{ "close_to_tray": true, "tray_notice_shown": false }`, file proprio, così il Task 2 resta l'unico in `window-preferences.json`). In Impostazioni → Generali, sezione nuova «Windows»: «Chiudendo la finestra, Qamille resta nell'area di notifica» e «Avvia Qamille all'accesso a Windows, nell'area di notifica» (nascosto nelle build di sviluppo e nella demo).

**Files:**
- Create: `desktop/src-tauri/src/background.rs`; `desktop/src-tauri/src/background_rules.rs` (puro); `desktop/src/lib/undo-close.ts` (senza Tauri); `desktop/src/components/DesktopSettings.tsx`; `desktop/tests/undo-close.mjs` (node --test); `desktop/tests/background-native.mjs`
- Modify: `desktop/src-tauri/src/main.rs` (single-instance `:78-83`, `on_window_event` `:98-108`, `generate_handler!` dopo `window_shell::window_appearance, window_shell::window_zoom,` `:113`, `setup_app` `:306-333`); `desktop/src-tauri/Cargo.toml`, `Cargo.lock`
- Modify: `desktop/src-tauri/src/recovery.rs` (`watch_startup` `:353`, `close_requested` `:432`, `close_seen` `:466`, `finish_meetings_before_exit` `:502`); `desktop/src-tauri/src/native_feel.rs` (`show_main_window` `:24-29`)
- Modify: `desktop/src-tauri/src/window_shell.rs`, solo le parti del menu di sistema e una riga in `window_appearance` (vedi tabella)
- Modify: `app/src/application/undo.rs` (accanto a `commit_due`, `:565`); `desktop/src-tauri/src/commands/undo.rs`
- Modify: `app/src/infrastructure/mcp_config.rs` (`save_with_autostart` `:106`, `refresh_autostart` `:265`, `set_autostart` `:274`, test)
- Modify: `desktop/src-tauri/windows/installer-hooks.nsh`
- Modify: `desktop/src/lib/window-close.ts`; `desktop/src/components/Settings.tsx` (sezione `General`, `:353-466`); `desktop/src/components/McpSettings.tsx` (interruttore `worker_autostart`, `:242`); `desktop/src/lib/preview.ts`
- Modify: `desktop/tests/native-feel.mjs` (controllo 17 e segnaposto Task 1); i test nativi che chiudono con la X o Alt+F4: `close-supervision-native.mjs`, `mail-opening-native.mjs`, `mail-sync-feedback-native.mjs`, `meetings-native.mjs`, `search-native.mjs`, `webview-recovery-native.mjs`, `window-native.mjs`, `workspace-startup-native.mjs`
- Modify: `docs/aspetto-nativo.md` (regole 12, 21, 25; segnaposto 26-27); `docs/edizioni.md` («Disinstallare la Professional»: il worker e il VBS); `docs/portable.md` (avvio automatico)

**Interfaces:** produce tutte le righe «Da 1» della tabella «Interfacce tra i task». In più:
- `background_rules.rs` (puro): `enum CloseAction { Hide, Quit }`, `fn close_action(close_to_tray: bool, quitting: bool) -> CloseAction`; `fn starts_hidden(args: &[String]) -> bool`; `fn autostart_entry_name(installed: bool, folder: &str, tag: &str) -> String`; `enum LegacyLauncher { Adopt, RemoveStale, LeaveToOther, Nothing }`, `fn legacy_launcher(content: Option<&str>, this_exe: &str, named_exe_exists: bool) -> LegacyLauncher`.
- Comandi: `autostart_status() -> AutostartStatus { enabled: bool, supported: bool }`, `autostart_set(enabled: bool) -> AppResult<AutostartStatus>`, `close_to_tray_get() -> bool`, `close_to_tray_set(on: bool)`, `restore_main_window()`, `undo_send_now(wait_ms: u32)`, `undo_quit_anyway(window, remaining: u32) -> bool`.
- `mcp_config.rs`: `pub fn legacy_launcher() -> AppResult<Option<(PathBuf, String)>>`, `pub fn remove_legacy_launcher() -> AppResult<()>`; `save_with_autostart` non scrive più il VBS (resta la convalida); `refresh_autostart` e `set_autostart` spariscono.
- `lib/undo-close.ts`: `createUndoQueueGuard(call: Call): CloseGuard` e il suo passo di fine sessione.

- [ ] **Step 1: test che falliscono.**
  - `background_rules.rs`: `close_hides_unless_quitting_or_switched_off()`; `only_the_autostart_argument_starts_hidden()` (un `mailto:` insieme a `--autostart` non nasconde); `entry_name_is_fixed_when_installed_and_per_folder_otherwise()`; `legacy_launcher_is_adopted_only_when_it_names_this_copy()` (questo exe → `Adopt`, exe sparito → `RemoveStale`, altra copia esistente → `LeaveToOther`, nessun file → `Nothing`, nomi di percorso diversi solo per maiuscole → stesso exe).
  - crate `app`, `undo.rs`: `undo_send_now_makes_every_pending_action_due()` (con `MemoryRepository` e orologio finto: dopo `send_now`, `next_deadline` è adesso e `commit_due` le prende tutte); `undo_send_now_leaves_failed_and_committing_alone()`; `unsettled_counts_pending_and_committing()`.
  - `mcp_config.rs`: i test del VBS diventano `saving_settings_never_writes_a_startup_file()` e `legacy_launcher_reads_the_old_file_and_removes_it()`.
  - `tests/undo-close.mjs`: `test('nothing queued closes at once', …)`; `test('queued sends are sent before closing', …)` (`undo_send_now` risponde `{ remaining: 0 }` → `true`); `test('a send that did not leave asks, and keeping Qamille open vetoes', …)` (`remaining: 1` → chiama `restore_main_window` e la domanda; «Resta aperto» → `false`); `test('session end sends only when the session really ends', …)` (`sessionEnd(false)` non chiama `undo_send_now`, `sessionEnd(true)` sì, con `wait_ms` ≤ 5000).
  - `native-feel.mjs`, controllo 19 «the close button hides, and one restore path brings the window back (rule 26)»: `main.rs` passa il callback di single-instance a `background::second_instance` e ferma la chiusura con `api.prevent_close()` solo quando `hides_on_close()`; in `background.rs` `restore` chiama `set_webview_visible(…, true)` prima di `show`, poi `unminimize` e `set_focus`, e nessun altro file chiama `.show()` sulla finestra principale tranne `native_feel::show_main_window` e `restore`; il menu della tray ha `show` e `quit`, e `quit` passa da `close()`, non da `app.exit`; `watch_startup` chiede solo dopo aver letto `is_visible()`.
  - controllo 20 «start at login is Qamille in the tray, written by the OS entry, never a VBS (rule 27, audit Q22)»: `Cargo.toml` ha `tauri-plugin-autostart`; il plugin è registrato con `AUTOSTART_ARG`; `mcp_config.rs` non scrive più file d'avvio (`launcher_content` non esiste); `setup_app` chiama la migrazione solo `if !demo`.
  - controllo 17 riscritto: l'hook di disinstallazione toglie il valore `Run` col nome che `autostart_entry_name(true, …)` dà al Professional installato (letto dal sorgente), e anche il suo gemello in `StartupApproved\Run`, solo fuori da `/UPDATE`; toglie ancora il vecchio `Qamille-worker.vbs` (installazioni precedenti); `PREUNINSTALL` resta com'è.
- [ ] **Step 2:** `rustc --edition 2024 --test desktop/src-tauri/src/background_rules.rs -o %TEMP%\background_rules.exe`, `cargo test --manifest-path app/Cargo.toml --locked undo_send_now unsettled legacy_launcher saving_settings`, `node --test tests/undo-close.mjs`, `node tests/native-feel.mjs`: FAIL (moduli e funzioni non esistono; 17, 19 e 20 falliscono).
- [ ] **Step 3: implementare.**
  - `Cargo.toml`: `tauri = { version = "2", features = ["tray-icon"] }`, `tauri-plugin-autostart = "2"`; aggiornare il lockfile senza spostare `tauri`.
  - `main.rs`: single-instance → `background::second_instance`; il plugin autostart subito dopo; in `on_window_event`, `CloseRequested { api, .. }` → `if background::hides_on_close() { api.prevent_close(); background::hide(window.app_handle()) } else { recovery::close_requested(…) }`; `Resized` → `background::window_resized(window)`. In `setup_app` il thread `qamille-autostart` (`:306-319`) chiama la migrazione del VBS al posto di `refresh_autostart`; dopo `.build()?` e `harden_webview`: `background::install(app)?` (tray), poi `if background_rules::starts_hidden(&args) { native_feel::prepare_main_window(&w); native_feel::set_webview_visible(&w, false) } else { native_feel::show_main_window(&w) }`; subito dopo, la riga di commento `// phase 3, Task 4: launch arguments`.
  - `native_feel.rs`: `show_main_window` = `prepare_main_window` + `show`; `set_webview_visible` con `with_webview` e `ICoreWebView2Controller::SetIsVisible`, errori a `report`.
  - `recovery.rs`: `watch_startup` chiede solo a finestra visibile, e `offer_reload_if_not_ready` (pubblica) lo fa al primo `restore` (vedi Decisioni); `close_seen` restituisce `!background::hides_on_close()` (la pagina non esegue le guardie quando Rust ha solo nascosto); `finish_meetings_before_exit` invia anche la coda (`UndoService::send_now`, poi attesa di `unsettled() == 0` entro lo stesso `EXIT_SAVE_WAIT`), così le uscite senza pagina (`exit_app`, `restart_app`, chiusura forzata, `startup_failed`) non perdono un invio; il commento del modulo lo dice.
  - `background.rs`: stato `QUITTING` (`AtomicBool`, che torna falso quando un'uscita finisce con un veto: `close_settled`), lettura e scrittura di `background.json` (scrittura tollerante come `save_preferences`), tray, `hide` (la prima volta il dialogo d'avviso in un thread suo), `restore`, `quit`, `second_instance`, `installed` (`uninstall.exe` accanto all'exe: il nome va verificato nel template NSIS di Tauri), `relabel`, `set_tray_tooltip`, comandi. `window_resized`: ridotta a icona → `set_webview_visible(false)`; tornata visibile → `true`.
  - `window_shell.rs`: voce «Esci da Qamille» / «Quit Qamille» nel menu di sistema accanto a «Ricarica la finestra» (stesso meccanismo di `add_reload_entry` e `label_reload_entry`); il suo `WM_SYSCOMMAND` chiama `background::quit`. Una riga in `window_appearance`: `relabel`.
  - `undo.rs` (crate `app`): `send_now` mette `expires_at_ms = now` a ogni azione `Pending` (non alle `Failed` né alle `Committing`), segna `schedule_dirty`/`has_work` e sveglia la coda come `queue_*`; `unsettled` conta `Pending` + `Committing`. Il comando `undo_send_now` chiama `send_now`, poi aspetta (passi da 200 ms) che `unsettled` arrivi a 0 o che passi `wait_ms`.
  - `undo-close.ts`: guardia `'undo-queue'` con `last: true`, registrata da `installWindowCloseHandler`. Con `remaining > 0` chiama `restore_main_window`, poi il comando `undo_quit_anyway(remaining: u32) -> bool`: un dialogo di Windows con owner la finestra principale, come `meeting_confirm_close` (`commands/meetings.rs:106`), «1 messaggio non è ancora partito (nessuna connessione). Se esci, partirà al prossimo avvio di Qamille.» con «Resta aperto» / «Esci comunque»; il plurale per n > 1. Il passo di fine sessione invia solo con `last` vero (con `last` falso Windows può ancora annullare: niente di irreversibile).
  - `window-close.ts`: `onCloseRequested` aspetta `close_seen()`; con `false` non fa niente. Un veto durante un'uscita chiama `restore_main_window`.
  - `mcp_config.rs` e `McpSettings.tsx`: niente più VBS; al posto dell'interruttore una riga «Il worker non serve più all'accesso: con «Avvia Qamille all'accesso» (Impostazioni → Generali) Qamille parte nell'area di notifica e invia da sé.» / inglese.
  - `DesktopSettings.tsx` (con `Switch` di `ds/`), resa da `Settings.tsx` nella sezione `General` solo con `desktop` vero: i due interruttori, lo stato dell'avvio riletto da `autostart_status` a ogni apertura della sezione, poi i due segnaposto dei Task 3 e 4 separati dal markup della sezione. In `preview.ts` risposte finte per i comandi nuovi.
  - `installer-hooks.nsh`: in `NSIS_HOOK_POSTUNINSTALL`, dentro `${If} $UpdateMode <> 1`, `DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Qamille"` e lo stesso in `...\Explorer\StartupApproved\Run`; il `Delete` del VBS resta per le installazioni vecchie. Il commento in testa al file non cita più `refresh_autostart`.
  - Test nativi elencati in **Files**: prima di avviare l'exe scrivono `background.json` con `{"close_to_tray": false, "tray_notice_shown": true}` nella loro cartella dati, così la X continua a chiudere come si aspettano.
  - `background-native.mjs` (demo, cartella dati di prova, `background.json` con `tray_notice_shown: true` per non fermarsi sull'avviso): X → processo vivo, finestra nascosta, `document.visibilityState === 'hidden'` via CDP; seconda istanza → finestra visibile e in primo piano; ridotta a icona → `hidden`; Esci dal menu di sistema (`PostMessage` di `WM_SYSCOMMAND` con l'id della voce, da PowerShell, come farebbe Alt+Spazio) → processo finito entro 10 s; avvio con `--autostart` → nessuna finestra visibile dopo 20 s, nessun dialogo «Qamille non si è aperto», e `frontend_ready` arrivato (riga di `native.log` o stato via CDP).
- [ ] **Step 4:** i test dello step 2 PASS; `npm run check`, `npm run test:ds`, `node --test tests/session-end.mjs tests/close-guards.mjs`, `npm run test:browser -- mcp-settings mcp-ui`: verdi.
- [ ] **Step 5:** sull'exe di release (vedi Global Constraints), con Qamille chiuso: `node tests/background-native.mjs`, `node tests/webview-recovery-native.mjs`, `node tests/close-supervision-native.mjs`. Le prove del registro (avvio all'accesso acceso e poi spento da Gestione attività) sono nello scenario D del Task 9.
- [ ] **Step 6:** `docs/aspetto-nativo.md`: regola 12 (Chiudi nasconde; Esci nel menu di sistema), regola 21 (le uscite senza pagina inviano la coda), regola 25 (valore `Run` al posto del VBS), righe 26 e 27. `docs/edizioni.md` e `docs/portable.md`: avvio automatico e disinstallazione.
- [ ] **Step 7: Commit** — `git commit -m "Keep Qamille in the tray on close, start it there at sign-in, and send the undo queue before quitting"`

### Task 2: dimensione e posizione della finestra

**Decisioni:**
- Si salvano il rettangolo «normale» (non ingrandito) in pixel fisici dello schermo virtuale, la scala del suo monitor e `maximized`, in `window-preferences.json` (`placement`). Mai da ridotta a icona o nascosta.
- Si applicano in `window_shell::apply_startup_preferences` (prima del primo `show`, e prima del `restore` di un avvio nella tray), solo se il centro del rettangolo cade nell'area di lavoro di un monitor collegato (`available_monitors()`, `Monitor::work_area()`). Se la scala di quel monitor è cambiata, la misura si converte. Poi si limita all'area di lavoro, con la barra del titolo (32 px logici in alto) sempre dentro e la misura mai sotto `minWidth`/`minHeight` di `tauri.conf.json`. Senza un monitor adatto non si applica niente: restano `center` e `preventOverflow` (controllo 13, invariato).
- Ingrandita: si applica il rettangolo normale e poi `maximize()` prima di `show`. Se su Tauri 2.12.5 `maximize()` mostra una finestra nascosta, si ingrandisce subito dopo `show` nello stesso giro del thread principale, e lo scenario E del Task 9 controlla che non si veda un fotogramma di misura normale.
- Si salva con un ritardo di 500 ms dopo `Moved`, `Resized`, `ScaleFactorChanged`, e subito a `CloseRequested` e `Destroyed`: un ascoltatore suo, registrato con `window.on_window_event` dentro `window_shell::attach_main`, quindi nessuna riga in `main.rs`.

**Files:**
- Create: `desktop/src-tauri/src/placement.rs` (puro)
- Modify: `desktop/src-tauri/src/window_shell.rs` (`WindowPreferences` `:30-40`, `apply_startup_preferences` `:93`, `attach_main` `:217`), più `mod placement;` in `main.rs`, sotto il segnaposto del Task 2 (unica modifica a `main.rs`)
- Modify: `desktop/tests/native-feel.mjs` (segnaposto Task 2); `docs/aspetto-nativo.md` (riga 29)

**Interfaces:**
- Produces: in `placement.rs` `struct SavedPlacement { x: i32, y: i32, width: u32, height: u32, scale: f64, maximized: bool }` (serde), `struct WorkArea { x: i32, y: i32, width: u32, height: u32, scale: f64 }`, `struct Placement { x: i32, y: i32, width: u32, height: u32, maximized: bool }`, `fn restore(saved: &SavedPlacement, monitors: &[WorkArea], min_logical: (u32, u32)) -> Option<Placement>`.
- Consumes: `native_feel::prepare_main_window` del Task 1 chiama `apply_startup_preferences`, come oggi `show_main_window`.

- [ ] **Step 1: test che falliscono** in `placement.rs`: `placement_on_a_connected_monitor_is_kept()`; `placement_on_a_disconnected_monitor_is_dropped()` (→ `None`); `placement_partly_off_screen_keeps_the_title_bar_inside()`; `placement_follows_a_scale_change()` (salvato a 1.0, monitor a 1.5 → misura × 1.5, poi limitata); `placement_never_goes_below_the_minimum_size()`; `placement_keeps_maximized()`; `placement_bigger_than_the_work_area_shrinks_to_it()`. In `native-feel.mjs`, controllo 21 «the saved placement is applied before the first show and only on a connected monitor (rule 29)»: `apply_startup_preferences` chiama `placement::restore` con le aree di lavoro di `available_monitors()`; `attach_main` registra il salvataggio; `tauri.conf.json` tiene `center` e `preventOverflow`.
- [ ] **Step 2:** `rustc --edition 2024 --test desktop/src-tauri/src/placement.rs -o %TEMP%\placement.exe`; `node tests/native-feel.mjs`: FAIL.
- [ ] **Step 3:** implementare `placement.rs`, il campo `placement` (con `#[serde(default)]`: un file vecchio si legge ancora) e l'applicazione in `apply_startup_preferences`, il salvataggio in `attach_main`. Errori di lettura o scrittura: si ignora, come per tema e zoom.
- [ ] **Step 4:** test PASS; `node tests/native-feel.mjs` tutto `ok`; `cargo check` sul worktree.
- [ ] **Step 5:** `docs/aspetto-nativo.md`, riga 29.
- [ ] **Step 6: Commit** — `git commit -m "Reopen the window where it was, only on a connected monitor"`

### Task 3: notifiche di sistema e stato sulla barra delle applicazioni

**Decisioni:**
- **Plugin ufficiale** `tauri-plugin-notification`, chiamato solo da Rust. Prima di scrivere codice si legge nel crate risolto (`~/.cargo/registry/src/*/tauri-plugin-notification-*/src/desktop.rs`) da dove prende l'AUMID su Windows e come si registra il gestore del clic (desktop ≥ 2.6). Ci si aspetta che usi `app.config().identifier`, che in Qamille è quello per cartella dati (`main.rs:59-63`).
- **Identità:** l'app registra quell'AUMID in `HKCU\Software\Classes\AppUserModelId\<identificatore>` (`DisplayName` «Qamille», `IconUri` = `data\notification-icon.png`, scritto dall'icona dell'app) quando le notifiche sono accese, e lo toglie quando si spengono. Con un AUMID che Windows non conosce le notifiche non sono affidabili; la registrazione fatta dall'app non dipende dall'installatore, e serve, perché la Professional si distribuisce come zip (`acaprino/qamille-enterprise`, `release.yml`, «the owner wants the zip only»). **Non** si imposta l'AUMID del processo (`SetCurrentProcessExplicitAppUserModelID`): staccherebbe la finestra dalle icone già bloccate sulla barra.
- **Per edizione:** Professional accese per default (scrive la chiave al primo avvio dopo l'aggiornamento); open source portatile spente per default, con l'interruttore che dice che scrive una voce nel registro dell'utente. Il disinstallatore Professional toglie la chiave: l'app scrive l'AUMID registrato in `data\notification-aumid.txt`, l'hook lo legge prima di cancellare `data` e cancella la chiave solo se comincia con `com.qamille.desktop`.
- **Che cosa notifica**, ciascuno spegnibile (`data/notifications.json`):
  - posta nuova in Posta in arrivo, non letta, arrivata con la sincronizzazione dal vivo (mai dallo storico né dalla prima sincronizzazione di un account), solo se la finestra non è visibile o non ha il focus; con tre o più messaggi una sola notifica «N messaggi nuovi in <account>»; mittente e oggetto, mai il corpo;
  - un invio programmato non riuscito (anche a finestra in primo piano, se non ha il focus);
  - un accesso scaduto: account di posta (`sign_in_required`, o il rifiuto `invalid_grant` di Google) e server Qamille (`unauthorized` di `sync.tick`), una volta per account finché non torna a funzionare.
  Il chip dei promemoria resta com'è (ADR-0004). L'avviso nell'app (`background-notice`) resta anche quando parte la notifica.
- **Clic:** `background::restore`, poi l'evento `notification-open` alla pagina, che apre il messaggio, il cassetto Bozze o Impostazioni → Account. Un clic dal centro notifiche a processo chiuso non è richiesto (limite del plugin, detto nella regola 28).
- **Segnali tipizzati:** tre varianti nuove di `AppEvent`, emesse dal crate `app` nel punto che sa il fatto e intercettate dal sink del desktop, che **non** le inoltra alla pagina (i processi MCP e il worker hanno `NullSink`):
  - `MailArrived` in `app/src/application/mail.rs`, nel ramo della sincronizzazione che aggiunge messaggi (`current.emails.push(email)`, `:1994-1995`), solo per gli id in Inbox non letti e solo se l'account aveva già un `last_sync` prima di questo passaggio;
  - `ScheduledSendFailed` in `app/src/application/scheduler.rs`, `send_due`, accanto alla `notices.push` dell'errore;
  - `SignInExpired` dove l'errore ha ancora il suo codice: la sincronizzazione dell'account in `mail.rs` (`account_result` con `sign_in_required`, o con il codice nuovo `sign_in_expired` che `oauth::access_token` dà a `invalid_grant`, `app/src/infrastructure/oauth.rs:182`) e `SyncService::tick` per `unauthorized` (`app/src/application/sync.rs`).
- **Barra delle applicazioni:** icona sovrapposta con i non letti di Posta in arrivo, lo stesso numero del contatore della sidebar. Lo calcola Rust (`WorkspaceService::inbox_unread`, la stessa funzione del conto di `snapshot`), 300 ms dopo l'ultimo `CountsChanged`: a finestra nascosta i timer della pagina sono rallentati fino a un minuto, quindi non lo manda la pagina. L'icona è un pallino con la cifra (1-9, poi «9+»), disegnato in Rust alla misura della scala della finestra; 0 la toglie. Il tooltip della tray dice «Qamille, 3 da leggere». Barra di avanzamento per l'indicizzazione completa: da `IndexProgress` sotto il 90 % per un account che in questo processo non ha ancora raggiunto il 100 % (o dopo `search_reindex`), fino al 100 %; `IndexWaiting { waiting: true }` la mette in pausa (gialla); con più account vale il minimo.
- **ADR-0004 v2** (`docs/adr/ADR-0004-v2-promemoria-riunioni.md`, che sostituisce la v1, più l'indice di `docs/adr/README.md`): RA-001 superato in parte, il chip resta per i promemoria e le notifiche di sistema coprono gli altri tre fatti.

**Files:**
- Create: `desktop/src-tauri/src/notifications.rs`; `notify_rules.rs` (puro); `desktop/src-tauri/src/taskbar.rs`; `taskbar_rules.rs` (puro); `desktop/src/lib/notification-open.ts`; `desktop/src/components/NotificationSettings.tsx`
- Modify: `app/src/application/events.rs` (`AppEvent`, `:9`); `app/src/application/mail.rs`; `app/src/application/scheduler.rs`; `app/src/application/sync.rs`; `app/src/application/workspace.rs` (`inbox_unread`, `:39-64`); `app/src/infrastructure/oauth.rs`, `app/src/infrastructure/mail.rs` (`:285`, `:1055`), `app/src/infrastructure/gmail_drafts.rs` (`:70`): il codice `sign_in_expired`
- Modify: `desktop/src-tauri/src/runtime.rs` (`TauriEventSink::emit`, `:29-36`); `desktop/src-tauri/src/commands/assistant.rs` (`search_reindex`, `:70`: una riga `taskbar::reset_index()`); `main.rs` (plugin e comandi, vedi tabella); `Cargo.toml`, `Cargo.lock`
- Modify: `desktop/src/App.tsx` (una riga, vedi tabella); `desktop/src/components/DesktopSettings.tsx` (segnaposto Task 3); `desktop/src/lib/preview.ts`; `desktop/src-tauri/windows/installer-hooks.nsh` (la chiave dell'AUMID, nel blocco `${If} $UpdateMode <> 1`, prima del blocco dei dati)
- Modify: `desktop/tests/native-feel.mjs` (segnaposto Task 3); `docs/aspetto-nativo.md` (righe 28 e 30); `docs/adr/ADR-0004-v2-promemoria-riunioni.md` (nuovo), `docs/adr/README.md`; `app/MECHANICS.md:110` (il refresh token sta nella cassaforte)

**Interfaces:**
- Consumes: `background::restore`, `background::installed`, `background::set_tray_tooltip` (Task 1).
- Produces: le righe «Da 3» della tabella; in più `notify_rules.rs`: `struct WindowState { visible: bool, focused: bool }`, `fn should_notify(kind: Kind, enabled: &Prefs, window: WindowState) -> bool`, `fn new_mail_text(items: &[(String, String)], account: &str, italian: bool) -> (String, String)`; `taskbar_rules.rs`: `fn badge_rgba(count: u32, size: u32) -> Option<Vec<u8>>`, `struct IndexBars`, `fn IndexBars::apply(&mut self, account: u32, percent: Option<u8>, waiting: Option<bool>) -> Option<(Bar, u8)>`; comandi `notifications_get() -> NotificationPrefs`, `notifications_set(prefs: NotificationPrefs) -> AppResult<()>`; `lib/notification-open.ts`: `installNotificationOpen({ openMail(id: number), openDrafts(), openAccounts() }): () => void`.

- [ ] **Step 1: test che falliscono.**
  - `notify_rules.rs`: `notify_rules_new_mail_is_quiet_while_the_window_has_focus()`; `notify_rules_three_messages_become_one_notice()`; `notify_rules_a_switched_off_kind_never_notifies()`; `notify_rules_texts_name_sender_and_subject_never_the_body()`, in italiano e in inglese.
  - `taskbar_rules.rs`: `badge_is_none_for_zero_and_nine_plus_above_nine()`; `index_bar_shows_only_a_full_indexing()` (90 % al primo annuncio → nessuna barra; 10 % → barra; 100 % → via; dopo il 100 % un calo per posta nuova → nessuna barra; `reset` → di nuovo); `index_bar_pauses_while_waiting()`; `index_bar_takes_the_lowest_account()`.
  - crate `app`: `mail_arrived_names_only_new_unread_inbox_messages()`; `mail_arrived_is_silent_on_the_first_sync_of_an_account()`; `scheduled_send_failed_is_announced_once()`; `sign_in_expired_comes_from_the_error_code()` (`sign_in_required`, `sign_in_expired`, `unauthorized` del server; un errore di rete no); `inbox_unread_matches_the_snapshot_count()`.
  - `native-feel.mjs`, controllo 24 «system notifications come from Rust only, through the registered app identity (rule 28)»: `main.json` senza `notification:`; il plugin è registrato in `main.rs`; `notifications.rs` scrive `AppUserModelId\` con l'identificatore dell'app e non chiama `SetCurrentProcessExplicitAppUserModelID`; `runtime.rs` non inoltra `MailArrived`, `ScheduledSendFailed`, `SignInExpired`. Controllo 25 «the taskbar shows unread mail and the full indexing (rule 30)»: `taskbar.rs` usa `set_overlay_icon` e `set_progress_bar` solo sulla finestra `main`.
- [ ] **Step 2:** i due `rustc --test`, `cargo test --manifest-path app/Cargo.toml --locked mail_arrived scheduled_send_failed sign_in_expired`, `node tests/native-feel.mjs`: FAIL.
- [ ] **Step 3: implementare,** prima la lettura del crate del plugin (vedi Decisioni), poi gli eventi del crate `app` e `inbox_unread`, le regole pure, `notifications.rs` e `taskbar.rs`, il sink, `NotificationSettings.tsx` (interruttore generale più i tre tipi, testo sul registro solo nella portatile), `installNotificationOpen` in `App.tsx`.
- [ ] **Step 4:** test PASS; `npm run check`; `npm run test:browser -- events smoke`; `node --test tests/events.mjs`; `cargo test --manifest-path app/Cargo.toml --locked` (tutto, perché `AppEvent` cambia).
- [ ] **Step 5:** sull'exe (scenari F e G del Task 9): una notifica per ognuno dei tre fatti, a finestra nascosta; il clic riporta la finestra; Non disturbare silenzia senza errori nel log.
- [ ] **Step 6:** `docs/aspetto-nativo.md`, righe 28 e 30 e la checklist «Nuova notifica di sistema» (un evento tipizzato in `AppEvent`, intercettato in `runtime.rs`, una regola in `notify_rules.rs`, un interruttore); ADR-0004 v2; `app/MECHANICS.md:110`.
- [ ] **Step 7: Commit** — `git commit -m "Announce new mail, failed scheduled sends and expired sign-ins as Windows notifications, and show unread mail and indexing on the taskbar"`

### Task 4: i link mailto aprono il composer di Qamille (Q10), poi Qamille come app di posta

**Decisioni:**
- **Un solo parser,** in TypeScript (`lib/mailto.ts`), secondo RFC 6068: destinatari nel percorso e in `to`, poi `cc`, `bcc`, `subject`, `body`, tutto decodificato; le altre intestazioni si ignorano; CR e LF si tolgono da destinatari e oggetto, restano solo nel corpo; ogni indirizzo passa dalla stessa convalida del composer; limite di 8 KB per l'indirizzo intero.
- **Nei messaggi** (`MessageHtml.tsx`, `openLink`): un `mailto:` chiama `requestCompose(parseMailto(…))`, mai più `open_link`. Anche `navigation_guard` e `new_window_guard` (`native_feel.rs:34`, `:52`) mandano un `mailto:` alla finestra principale come `compose-mailto`, non al browser. `open_link` (`workspace.rs:101-117`) accetta ancora `mailto:` per i link fuori dai messaggi.
- **Composer:** `composeMailto` in `App.tsx` crea la bozza come «Nuovo messaggio» (`create_draft` con l'account attivo), la salva con i campi del link (il corpo come testo, convertito come il composer converte il testo incollato) e la apre con `openComposer`.
- **Da un altro programma:** l'argomento `mailto:` di un avvio a freddo resta in attesa finché la pagina non lo chiede (`take_launch_mailto`, chiamato una volta da `installMailtoRouting`, dopo che l'ascoltatore di `compose-mailto` è attivo); quello di una seconda istanza arriva con `compose-mailto` dopo `background::restore`, oppure resta in attesa se la pagina non è ancora pronta. Un avvio con `mailto:` mostra sempre la finestra, anche con `--autostart`.
- **App di posta predefinita:** Windows non lascia impostarla a un programma. Qamille si registra (Capabilities `URLAssociations\mailto` → ProgID `Qamille.Url.mailto.<tag>`, comando `"<exe>" "%1"`, voce in `RegisteredApplications`) e apre `ms-settings:defaultapps?registeredAppUser=<nome>`: la scelta la fa la persona. Professional installato: l'hook `NSIS_HOOK_POSTINSTALL` scrive le chiavi col nome «Qamille», `POSTUNINSTALL` le toglie fuori da `/UPDATE`. Portatile: pulsante facoltativo in Impostazioni → Generali → Windows («Usa Qamille per i link mailto»), nome «Qamille (<cartella>)», e «Smetti» toglie tutto.

**Files:**
- Create: `desktop/src/lib/mailto.ts`; `desktop/tests/mailto.mjs` (node --test); `desktop/src-tauri/src/launch_args.rs` (con le regole pure in fondo e i loro test); `desktop/src-tauri/src/mail_handler.rs`; `desktop/src/components/MailtoSettings.tsx`
- Modify: `desktop/src/components/MessageHtml.tsx` (`openLink`, `:48-68`); `desktop/src/App.tsx` (vedi tabella); `desktop/src-tauri/src/native_feel.rs` (solo il ramo `mailto:`); `desktop/src-tauri/src/background.rs` (una riga in `second_instance`); `main.rs` (segnaposto `// phase 3, Task 4: launch arguments` e una riga di comandi dopo `open_link,`); `desktop/src-tauri/windows/installer-hooks.nsh`; `desktop/src/components/DesktopSettings.tsx` (segnaposto Task 4); `desktop/src/lib/preview.ts`
- Modify: `desktop/tests/native-feel.mjs` (segnaposto Task 4; il controllo 16 resta com'è, perché `externalAddress` accetta ancora `mailto:` e cambia solo chi lo apre); `desktop/tests/mail-html.mjs`; `docs/aspetto-nativo.md` (regole 23 e 25, riga 32)

**Interfaces:**
- Consumes: `background::second_instance`, `background::restore`, `background::installed`, `AUTOSTART_ARG` (Task 1).
- Produces: le righe «Da 4» della tabella; in più `launch_args.rs`: `pub fn mailto_arg(args: &[String]) -> Option<String>`, `pub fn route(app: &AppHandle, args: &[String])`; comandi `mail_handler_status() -> MailHandlerStatus { registered: bool, name: String }`, `mail_handler_register() -> AppResult<()>`, `mail_handler_unregister() -> AppResult<()>`.

- [ ] **Step 1: test che falliscono.**
  - `tests/mailto.mjs`: `test('recipients from the path and from to, cc and bcc', …)`; `test('subject and body are decoded, plus stays a plus', …)`; `test('a line break cannot add a header or a recipient', …)` (`mailto:a@b.it?subject=x%0D%0ABcc:c@d.it` → un solo destinatario, oggetto su una riga); `test('unknown headers are ignored and a non-mailto address is null', …)`; `test('an address over 8 KB is refused', …)`.
  - `launch_args.rs`: `mailto_arg_takes_only_a_whole_mailto_argument()` (`--autostart`, percorsi e `mailto:` con NUL → `None`).
  - `native-feel.mjs`, controllo 26 «a mailto link opens Qamille's composer, and the installer registers exactly the keys the app removes (rule 32, audit Q10)»: `MessageHtml.tsx` non passa un `mailto:` a `openExternal`; le guardie di navigazione emettono `compose-mailto`; le chiavi e il nome scritti in `NSIS_HOOK_POSTINSTALL` sono quelli di `mail_handler.rs` per l'installato, e `POSTUNINSTALL` li toglie solo fuori da `/UPDATE`.
  - Playwright: in `tests/mail-html.mjs` un caso nuovo, clic su `mailto:a@b.it?subject=x` dentro un messaggio dell'anteprima → composer aperto con `a@b.it` e oggetto `x`, e `window.openedLinks` (lo stub di `open_link` del test, `:46`) resta vuoto.
- [ ] **Step 2:** `node --test tests/mailto.mjs`, `cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked launch_args`, `node tests/native-feel.mjs`, `npm run test:browser -- mail-html`: FAIL.
- [ ] **Step 3:** implementare nell'ordine: parser e bus, `MessageHtml`, `composeMailto` e `installMailtoRouting` in `App.tsx`, guardie di `native_feel.rs`, `launch_args.rs` (avvio a freddo e seconda istanza), `mail_handler.rs` e il pulsante, l'hook NSIS.
- [ ] **Step 4:** test PASS; `npm run test:browser -- mail-html smoke`; `npm run check`.
- [ ] **Step 5:** sull'exe, con Qamille chiuso: `node tests/webview-recovery-native.mjs` (cambia `native_feel.rs`); poi lo scenario H del Task 9.
- [ ] **Step 6:** `docs/aspetto-nativo.md`: regola 23 (mailto va al composer), regola 25 (chiavi mailto), riga 32.
- [ ] **Step 7: Commit** — `git commit -m "Open mailto links in Qamille's composer and let Windows choose Qamille for them"`

### Task 5: «Apri» e «Mostra nella cartella» dopo un salvataggio

**Decisioni:**
- Niente `tauri-plugin-opener`: `open` 5 c'è già (`Cargo.toml:21`). «Apri» = `open::that_detached` sul file salvato, che col Mark-of-the-Web (regola 17) fa avvisare Windows prima di aprire un tipo pericoloso. «Mostra nella cartella» = `SHOpenFolderAndSelectItems` (Esplora risorse con il file selezionato), mai una riga di comando `explorer /select`.
- La pagina non passa percorsi: `attachment_save` ed `export_mail` danno un `token`, che vale solo per i file salvati in questa sessione (`saved_files::remember`). Un token sconosciuto → errore, nessun effetto.
- Il Toaster di `ds/` accetta fino a due azioni, pulsanti `ds` dentro la notifica: un clic esegue e chiude la notifica. Con azioni resta 10 s, e come oggi aspetta finché puntatore o focus sono sopra. Le azioni sono una comodità: il file resta raggiungibile da Esplora risorse.
- Fuori scopo (restano come sono): `meeting_export` e `contacts_save_vcf`.

**Files:**
- Create: `desktop/src-tauri/src/saved_files.rs`
- Modify: `desktop/src-tauri/src/commands/document_preview.rs` (`attachment_save`, `:26`); `desktop/src-tauri/src/commands/mail.rs` (`ExportSaved`, `:78`, `export_mail`, `:84`); `main.rs` (una riga di comandi dopo `attachment_save,` e `mod saved_files;`); `desktop/src-tauri/Cargo.toml` (solo una feature di `windows`, se serve, per esempio `Win32_UI_Shell_Common`)
- Modify: `desktop/src/ds/primitives/Toaster.tsx` (`notify`, `:27`); `desktop/src/components/AttachmentPreview.tsx` (`saveAttachment`, `:191-206`); `desktop/src/App.tsx` (`notify` `:349` passa le azioni; `exportEmails` `:371-395`); il `.eml` del lettore, se passa da `exportEmails`; `desktop/src/lib/preview.ts`
- Modify: `desktop/tests/toaster-layer.mjs`, `desktop/tests/attachment-preview.mjs`; `desktop/tests/native-feel.mjs` (segnaposto Task 5); `docs/aspetto-nativo.md` (riga 31 e la checklist «Salvare un file»)

**Interfaces:** produce `saved_files::remember`, `saved_file_open`, `saved_file_reveal`, `SavedFile`, `ExportSaved.token`, il nuovo `notify` (tabella).

- [ ] **Step 1: test che falliscono.**
  - `saved_files.rs`: `an_unknown_token_opens_nothing()`; `tokens_are_not_reused()`.
  - `tests/toaster-layer.mjs`: una notifica con due azioni mostra due pulsanti raggiungibili con Tab; il clic esegue l'azione e chiude la notifica; un `notify` successivo la sostituisce.
  - `tests/attachment-preview.mjs`: dopo «Salva», nell'anteprima (che risponde con un token finto) la notifica ha «Apri» e «Mostra nella cartella», e il clic registra `saved_file_reveal` col token.
  - `native-feel.mjs`, controllo 22 «a saved file is opened or shown only through its token (rule 31)»: `saved_file_open` e `saved_file_reveal` prendono un `u64`, non una stringa; `saved_files.rs` non lancia `explorer` con argomenti.
- [ ] **Step 2:** `cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked saved_files`, `npm run test:browser -- toaster-layer attachment-preview`, `node tests/native-feel.mjs`: FAIL.
- [ ] **Step 3:** implementare; i testi: «Apri» / «Open», «Mostra nella cartella» / «Show in folder».
- [ ] **Step 4:** test PASS; `npm run test:ds`; `npm run check`; `npm run test:browser -- attachment-panel mail-export`.
- [ ] **Step 5:** `docs/aspetto-nativo.md`, riga 31 e checklist «Salvare un file».
- [ ] **Step 6: Commit** — `git commit -m "Offer Open and Show in folder after saving an attachment or an export"`

### Task 6: menu contestuale nativo sulle righe della posta

**Decisioni:**
- Il menu lo costruisce Rust con `tauri::menu` (voci, separatori, voci con spunta) e lo apre con `popup_menu_at` sotto il cursore. Le coordinate seguono la convenzione di `show_system_menu` (`window_shell.rs:188`, chiamato da `lib/window-shell.ts:18-23`): la pagina manda `clientX * devicePixelRatio`, che comprende zoom dell'app e scala, e Rust usa una `PhysicalPosition` nell'area client. Niente `core:menu` in `main.json`. Il menu è un `Menu` nuovo a ogni apertura, rilasciato quando il comando finisce.
- Il comando restituisce la voce scelta, o `null` se il menu si chiude senza scelta: su Windows `popup_menu_at` aspetta la chiusura del menu, poi il comando attende l'evento del menu al massimo 500 ms. Se sul runtime vero `popup_menu_at` non aspettasse, si passa a un evento `row-menu-chosen` con un numero di richiesta, e lo si annota.
- Voci, decise in `lib/row-menu.ts` e mai in Rust: Rispondi, Rispondi a tutti, Inoltra | Archivia, Sposta nel cestino, Fatto, Posticipa a domani | Segna come letto / non letto | Fissa / Togli | Sposta in Posta in arrivo (fuori da Inbox). Le azioni sono quelle che la riga e il lettore già usano (`action`, `compose`), con le stesse regole di blocco (`sectionLocks`, `busyIds`). Su più righe spuntate restano solo le azioni di gruppo.
- Tasto destro su una riga non spuntata: agisce su quella riga (e sui messaggi della sua conversazione), senza aprirla. Su una riga spuntata: su tutte le spuntate.
- Tastiera: lo stesso `contextmenu` arriva da Maiusc+F10 e dal tasto Menu. Da tastiera (`pointerType` vuoto) il menu si apre sotto la riga a fuoco, a 16 px dal suo bordo sinistro. La riga chiama `preventDefault`, quindi il menu filtrato di WebView2 non compare sulle righe e resta altrove (regola 9). Anche l'ascoltatore di `installWindowShell` lascia stare un evento già fermato (`defaultPrevented`), quindi il menu di sistema non si apre insieme.
- Dopo la scelta il focus torna alla riga da cui è partito il menu, come con il menu «Altre azioni» del lettore.
- Nell'anteprima del browser non c'è un menu nativo: il comando finto registra le voci e risponde con la scelta preparata dal test.

**Files:**
- Create: `desktop/src-tauri/src/row_menu.rs`; `desktop/src/lib/row-menu.ts`; `desktop/tests/row-menu.mjs` (node --test, voci pure)
- Modify: `main.rs` (`mod row_menu;` sotto il segnaposto e una riga di comandi dopo `queue_mail_action,`); `desktop/src/components/MailList.tsx` (`MailListProps` `:28-56`, `RowProps` `:160`, la riga); `desktop/src/App.tsx` (prop `rowMenu` su `<MailList` `:1777`, `openRowMenu` dopo `action` `:1095`); `desktop/src/lib/preview.ts`; `desktop/tests/thread-rows.mjs` (Playwright: un caso nuovo)
- Modify: `desktop/tests/native-feel.mjs` (segnaposto Task 6); `docs/aspetto-nativo.md` (regola 9, riga 33, checklist)

**Interfaces:** produce `show_row_menu`, `RowMenuEntry`, `rowMenuEntries`, prop `rowMenu` (tabella).

- [ ] **Step 1: test che falliscono.**
  - `tests/row-menu.mjs`: `test('a single row offers reply, triage, read state, pin and inbox', …)`; `test('several checked rows offer only group actions', …)`; `test('a locked section disables what would change the message', …)`; `test('labels are Italian or English, never mixed', …)`.
  - Playwright (`thread-rows`): tasto destro su una riga → il comando finto riceve le voci, la scelta «Archivia» archivia quella riga e non apre il lettore; Maiusc+F10 sulla riga a fuoco apre il menu alla posizione della riga; dopo la scelta il focus è sulla riga.
  - `native-feel.mjs`, controllo 23 «the mail rows get a native menu built by an app command (rule 33)»: `row_menu.rs` usa `popup_menu_at`; `main.json` senza `core:menu`; la riga di `MailList.tsx` gestisce `onContextMenu` con `preventDefault`.
- [ ] **Step 2:** `node --test tests/row-menu.mjs`, `npm run test:browser -- thread-rows`, `node tests/native-feel.mjs`: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** test PASS; `npm run check`; `npm run test:browser -- smoke sidebar-threads glance-list`.
- [ ] **Step 5:** `docs/aspetto-nativo.md`: regola 9 (sulle righe il menu è dell'app), riga 33, checklist «Nuova voce del menu delle righe».
- [ ] **Step 6: Commit** — `git commit -m "Give mail rows a native context menu, also from Shift+F10 and the Menu key"`

### Task 7: nella Professional la chiave della cassaforte è protetta da Windows

**Decisioni:**
- **DPAPI, non Gestione credenziali.** La chiave resta accanto ai dati (`data\credentials.key.dpapi`, cifrata con `CryptProtectData` per l'utente, con un'entropia fissa dell'app e `CRYPTPROTECT_UI_FORBIDDEN`). Così la disinstallazione che cancella `data` la porta via senza altro codice, e ogni cartella dati ha la sua chiave. Una voce di Gestione credenziali andrebbe invece nominata per cartella e tolta a parte dal disinstallatore.
- Solo l'edizione Professional (`qamille::edition::edition()`). L'open source portatile mantiene `credentials.key` in chiaro, come dichiarano `secret.rs:1-3` e `docs/portable.md:21-23`.
- **Migrazione,** sotto `credentials.lock`, al primo `load` della Professional:
  1. scrive `credentials.key.dpapi.tmp`;
  2. lo rinomina in `credentials.key.dpapi`;
  3. lo rilegge e controlla che dia la stessa chiave;
  4. solo allora cancella `credentials.key`.
  In lettura vince `.dpapi`; se c'è anche la chiave in chiaro (migrazione interrotta fra 3 e 4) la si cancella dopo il controllo.
- Una `.dpapi` che non si apre (cartella copiata su un altro utente o computer): errore chiaro in due lingue («La chiave delle credenziali appartiene a un altro utente di Windows o a un altro computer: accedi di nuovo agli account in Impostazioni.»), nessun file cancellato o riscritto.
- MCP e worker sono lo stesso exe e lo stesso utente: leggono la chiave come l'app.

**Files:**
- Create: `app/src/infrastructure/dpapi.rs` (solo `cfg(windows)`)
- Modify: `app/src/infrastructure/secret.rs` (`Vault`, `:49`; `load`, `:92`; il salvataggio della chiave); `app/Cargo.toml` (feature `Win32_Security_Cryptography` di `windows-sys`, `:50`); `app/src/infrastructure/mod.rs`
- Modify: `docs/edizioni.md` («Disinstallare la Professional»: `credentials.key.dpapi`); `docs/portable.md` (nota sulla Professional)

**Interfaces:** produce `KeyStore`, `Vault::with_key_store`, `dpapi::protect(bytes: &[u8]) -> Result<Vec<u8>, String>`, `dpapi::unprotect(bytes: &[u8]) -> Result<Vec<u8>, String>`.

- [ ] **Step 1: test che falliscono** in `secret.rs` (cartelle temporanee, `KeyStore` esplicito, mai l'edizione di compilazione):
  - `vault_key_moves_to_dpapi_once_and_stays_readable()`;
  - `vault_key_interrupted_after_rename_is_finished_at_the_next_load()` (`.dpapi` e `credentials.key` insieme → si legge, poi resta solo `.dpapi`);
  - `vault_key_interrupted_before_rename_keeps_the_plain_key()` (solo `.tmp` → si legge dalla chiave in chiaro, il `.tmp` si rifà);
  - `vault_key_from_another_user_is_refused_and_nothing_is_deleted()` (`.dpapi` con byte non validi);
  - `vault_key_stays_plain_in_the_portable_store()`.
- [ ] **Step 2:** `cargo test --manifest-path app/Cargo.toml --locked vault_key`: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** test PASS; `cargo test --manifest-path app/Cargo.toml --locked secret` (i test esistenti della cassaforte).
- [ ] **Step 5:** documenti.
- [ ] **Step 6: Commit** — `git commit -m "Protect the Professional vault key with Windows DPAPI, migrating existing installs"`

### Task 8: updater e firma Authenticode (Professional), fino ai segreti

**Decisioni:**
- **Nell'app (questo repository):** `tauri-plugin-updater`, attivo solo se l'exe è compilato con `QAMILLE_UPDATER_PUBKEY` e `QAMILLE_UPDATER_ENDPOINT` (`option_env!`), l'edizione è Professional e Qamille è installato (`background::installed()`). Altrimenti il plugin non si registra e niente controlla aggiornamenti: nessun effetto sull'open source. La chiave pubblica e l'indirizzo passano al plugin da Rust (`Builder::pubkey`, `endpoints`), non da `tauri.conf.json`.
- Controllo un minuto dopo l'avvio, poi ogni 24 ore; lo scaricamento avviene subito, in background. A pacchetto pronto: evento `update-available` → avviso persistente nell'app (`UpdateNotice.tsx`, non un toast che sparisce) con «Riavvia e aggiorna». Il clic chiama `update_install`, che segna il pacchetto da installare e chiama `background::quit`: guardie e coda di Annulla invio come Esci. Se una guardia pone il veto non succede niente. Altrimenti, all'uscita, `updates::run_event` installa: `main.rs` passa da `.run(context)` a `.build(context)` + `.run(…)`, l'`Err` di `build` va ancora a `startup_failed(None, …)`, e il controllo 12 di `native-feel.mjs` segue la forma nuova. Mai un aggiornamento senza le guardie.
- `tauri.professional.conf.json` non chiede gli artefatti dell'updater: `build-release.bat` aggiunge `--config src-tauri/tauri.updater.conf.json` (con `bundle.createUpdaterArtifacts: true`) solo quando c'è `TAURI_SIGNING_PRIVATE_KEY`; senza, la build di oggi resta identica.
- **Firma Authenticode:** nel workflow di `acaprino/qamille-enterprise` (`.github/workflows/release.yml`), uno step dopo «Professional app» e prima di «Package» firma con `signtool sign /fd SHA256 /tr <timestamp> /td SHA256` l'exe dentro `build/` e, se c'è, l'installer. Il certificato arriva da `WINDOWS_CERTIFICATE` (PFX in base64) e `WINDOWS_CERTIFICATE_PASSWORD`. Senza segreti lo step scrive un avviso e non firma; la release esce come oggi.
- **Dove ci si ferma (proprietario):**
  1. il certificato di firma e la chiave dell'updater (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) nei segreti di GitHub di `qamille-enterprise`;
  2. un indirizzo **pubblico** per `latest.json` e per l'installer: gli asset di una release di un repository privato non si scaricano senza token;
  3. pubblicare di nuovo l'installer, che l'updater di Windows richiede: oggi la release pubblica solo lo zip, per scelta del proprietario.
- Il lavoro nel repository privato segue il suo `AGENTS.md` (da leggere con `gh api repos/acaprino/qamille-enterprise/contents/AGENTS.md`), su un ramo breve dal suo `develop`, con una pull request verso `develop`: mai `main`.

**Files:**
- Create: `desktop/src-tauri/src/updates.rs`; `desktop/src-tauri/tauri.updater.conf.json`; `desktop/src/components/UpdateNotice.tsx` (avviso persistente, con `Button` di `ds/`)
- Modify: `main.rs` (registrazione condizionale, una riga di comandi dopo `app_edition,`, `.build(context)` + `.run(…)` al posto di `.run(context)`, `:249-256`); `Cargo.toml`, `Cargo.lock`; `build-release.bat` (il `--config` condizionale, `:228`); `desktop/src/App.tsx` (una riga accanto a `<Toaster`); `docs/edizioni.md` (firma e aggiornamenti: che cosa serve e dove)
- Modify (repository `acaprino/qamille-enterprise`): `.github/workflows/release.yml` (step di firma; `latest.json` e installer solo quando le decisioni 2 e 3 sono prese, fino ad allora uno step commentato e spiegato); il suo README, sezione delle release
- Modify: `desktop/tests/native-feel.mjs` (segnaposto Task 8 e controllo 12)

**Interfaces:**
- Consumes: `background::quit`, `background::installed` (Task 1).
- Produces: `updates::plugin`, `updates::run_event`, `updates::spawn_checks(app: &AppHandle)`, comandi `update_status() -> Option<UpdateInfo { version: String, notes: String }>` e `update_install() -> AppResult<()>`.

- [ ] **Step 1: test che falliscono.** In `updates.rs` (regole pure in fondo): `updater_is_off_without_key_endpoint_professional_and_install()` (ogni combinazione); `a_vetoed_quit_installs_nothing()`. In `native-feel.mjs`, controllo 27 «the updater runs only in an installed Professional built with its key, and installs through the close guards»: `main.json` senza `updater:`; `update_install` passa da `background::quit` e l'installazione sta solo in `run_event`; `tauri.professional.conf.json` non contiene `createUpdaterArtifacts`. Il controllo 12 riconosce `.build(context)` con `startup_failed(None, …)` sul suo `Err`.
- [ ] **Step 2:** `cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked updates`, `node tests/native-feel.mjs`: FAIL.
- [ ] **Step 3:** implementare nell'app; poi, nel repository privato, lo step di firma. Prova del workflow solo con `workflow_dispatch` senza «publish» e senza segreti: l'avviso compare e la build passa.
- [ ] **Step 4:** test PASS; `build-release.bat --no-install` senza `TAURI_SIGNING_PRIVATE_KEY`: stesso risultato di prima.
- [ ] **Step 5:** `docs/edizioni.md`: il paragrafo «Firma e aggiornamenti», con l'elenco di ciò che manca al proprietario.
- [ ] **Step 6: Commit** — `git commit -m "Prepare the Professional updater and code signing, off until the owner provides the keys"`; nel repository privato la PR verso `develop`.

### Task 9: verifica sull'exe e documenti

**Files:**
- Modify: `docs/audit-tauri-nativo.md`: una tabella «Stato della fase 3», con commit e prove per ogni riga di «Funzioni native (fase 3): stato attuale», e Q8, Q10, Q22 segnati come risolti. Le frasi di «Che cosa non è più vero» che la fase 3 corregge: righe 114 e 160, `edizioni.md:58`, `app/MECHANICS.md:110`, ADR-0004 RA-001.
- Modify: `docs/aspetto-nativo.md`, sezione «Verifica nell'app vera»: gli scenari qui sotto.
- Modify: `desktop/scripts/test-browser.mjs` (`NODE_TEST`, `:17`): `undo-close`, `mailto`, `row-menu`.

**Regola dell'errore voluto:** ogni scenario di guasto si prova una volta, apposta. Ogni finestra la chiude lo script che l'ha aperta. Si usa solo l'exe di `release/` con cartelle dati di prova (`QAMILLE_DATA`), mai le copie installate in `D:\Software`. Ogni voce di registro scritta da una prova si toglie alla fine della prova.

- [ ] **Step 1:** `npm run build` in `desktop/`, poi la build di release delle Global Constraints; controllare `theme-boot.js` nell'exe; copiarlo in `release/Qamille.exe`. Con Qamille chiuso: `node tests/release-theme-probe.mjs` (URL `http://tauri.localhost/`), `node tests/webview-recovery-native.mjs`, `node tests/close-supervision-native.mjs`, `node tests/background-native.mjs`, `node tests/a11y-native.mjs`, `node tests/window-native.mjs`.
- [ ] **Step 2: scenari**, ciascuno annotato «osservato» oppure «da fare» con il motivo:
  - **A.** Prima X dopo l'aggiornamento: l'avviso compare una volta; «Esci, e d'ora in poi chiudi» esce e la X seguente chiude.
  - **B.** Q8: Annulla invio a 30 s, Invia, Esci dopo 2 s → la mail è in Inviati entro 40 s. Di nuovo con la rete staccata → compare la domanda e «Resta aperto» lascia Qamille nella tray.
  - **C.** Ripristino: dalla tray, da una seconda istanza a finestra nascosta, da ridotta a icona → finestra disegnata (mai bianca), davanti, con il focus. Tempo di CPU dei processi `msedgewebview2` in 10 s (`Get-Process`, differenza di `CPU`) a finestra nascosta più basso che a finestra visibile.
  - **D.** Avvio all'accesso acceso → il valore `Run` della copia di prova c'è, col nome e con `--autostart`; l'exe lanciato con quella riga resta nella tray, senza dialoghi. Spento come fa Gestione attività (il valore binario in `...\Explorer\StartupApproved\Run`, primo byte `03`) → l'interruttore risulta spento (Q22). Migrazione: un `Qamille-worker.vbs` che nomina l'exe di prova → al primo avvio il file sparisce e l'avvio all'accesso è acceso. Se nella cartella Esecuzione automatica c'è già un `Qamille-worker.vbs` di un'altra copia, lo scenario lo copia prima e lo rimette dopo, byte per byte. La disconnessione vera e il nuovo accesso li osserva il proprietario: «da fare» finché non lo dice.
  - **E.** Posizione: secondo monitor, chiudere, riaprire → stesso posto; `window-preferences.json` con `x` a 99999 scritto a mano → finestra al centro del monitor principale; ingrandita → riapre ingrandita senza un fotogramma di misura normale.
  - **F.** Notifiche, con account finti nella cartella di prova: un invio programmato con l'SMTP su `127.0.0.1:1` (fallisce), un account IMAP senza password (dà `sign_in_required`) → due notifiche col nome e l'icona di Qamille; il clic riporta la finestra sulla bozza o sulle Impostazioni. La posta nuova a finestra nascosta solo con un account di prova vero, altrimenti «da fare». Le chiavi AUMID scritte dalla prova si tolgono alla fine.
  - **G.** Nessuna valanga: un account aggiunto ora, lo storico di uno vecchio, l'avvio dopo una notte con posta arrivata → nessuna notifica; tre messaggi insieme → una (con l'account di prova vero, come in F).
  - **H.** mailto: clic in un messaggio → composer compilato. `start mailto:a@b.it` a Qamille chiuso e aperto → composer: si annota prima l'app di posta predefinita del proprietario e la si rimette alla fine; se non si può rimettere, lo scenario resta «da fare» per il proprietario.
  - **I.** Menu delle righe: tasto destro, Maiusc+F10, tasto Menu, al 150 % di scala e con lo zoom dell'app al 125 % → il menu è al cursore o sotto la riga.
  - **J.** File: «Mostra nella cartella» seleziona il file in Esplora risorse; «Apri» su un `.exe` salvato da un allegato → Windows avvisa (Mark-of-the-Web).
  - **K.** Professional (solo con `QAMILLE_SERVER`): migrazione della chiave su una copia di una cartella dati di prova; la stessa cartella aperta da un altro utente di Windows → il messaggio, nessun file cancellato. Installer: installare, accendere avvio all'accesso, notifiche e mailto, disinstallare → nessun valore `Run`, chiave AUMID o chiave mailto rimasti. L'installer è per utente e userebbe il nome fisso «Qamille»: si prova solo con un utente di Windows di prova, mai sul profilo del proprietario; senza, «da fare».
- [ ] **Step 3:** `node tests/native-feel.mjs`, `npm run test:ds`, `npm run check`, `node --test tests/session-end.mjs tests/close-guards.mjs tests/undo-close.mjs tests/mailto.mjs tests/row-menu.mjs`, `npm run test:browser` (tutti), `cargo test --manifest-path app/Cargo.toml --locked`, `cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked`: tutti verdi.
- [ ] **Step 4: Commit** — `git commit -m "Record the phase-3 native features and their checks"`

---

## Decisioni che il proprietario potrebbe voler cambiare

1. **Chiusura nella tray accesa anche per chi usa già Qamille.** L'avviso alla prima X offre «Esci, e d'ora in poi chiudi». L'alternativa è lasciarla spenta per chi aggiorna e accesa solo per le installazioni nuove.
2. **Notifiche nella portatile open source:** spente per default, accese dall'utente, che così scrive una chiave AUMID in `HKCU`. Nella Professional sono accese per default. L'alternativa è niente notifiche nella portatile.
3. **La Professional esce solo come zip:** l'updater di Windows ha bisogno dell'installer e di un indirizzo pubblico per `latest.json`. Finché non si decide, il Task 8 prepara tutto ma l'updater resta spento.
4. **DPAPI invece di Gestione credenziali** per la chiave della Professional: una cartella dati copiata su un altro computer non apre più le credenziali, ed è voluto.
5. **Il worker non parte più da solo all'accesso:** lo sostituisce Qamille nella tray. Chi usava solo il worker senza finestra trova Qamille nell'area di notifica.
6. **Notifiche della posta con mittente e oggetto** (mai il corpo); in alternativa solo «N messaggi nuovi».
