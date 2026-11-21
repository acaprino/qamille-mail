# Fase 2 nativa (riparazioni e modifiche standard): piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** chiudere i difetti della quarta passata nativa che non chiedono decisioni di prodotto: perdite di dati, avvio, modifiche standard mancanti della skill, tastiera e accessibilità senza cambi di design, Mark-of-the-Web, disinstallazione.

**Architecture:** ogni task ripara un comportamento già deciso, usando come requisito il comportamento stesso: nessuna funzione nuova visibile. Le correzioni restano negli adattatori che già possiedono quel comportamento: `close-guards.ts` per la chiusura, `recovery.rs` per il ripristino, `native_feel.rs` per l'avvio delle WebView, le primitive di `ds/` per focus e menu. Le funzioni pure vanno in `shell_rules.rs` (Rust) o in `lib/*.ts` senza Tauri (JS), così si provano senza l'app.

**Tech Stack:** Tauri 2.12.5, wry 0.57.1, webview2-com 0.39.1, windows 0.62.2, rfd 0.15.4; React 19.3.1, @ark-ui/react 5.39.3; test con `node --test`, Playwright (anteprima, `npm run test:browser -- <nome>`), `node tests/native-feel.mjs`, `cargo test`, test `*-native.mjs` sull'exe di release.

**Spec:** `docs/audit-tauri-nativo.md`, sezione «Quarta applicazione della skill (2026-11-20)»: tabelle «Modifiche standard», «Difetti, dal più grave» (Q1–Q24 e P2), «Proposta per le fasi 2 e 3». La skill è `tauri-native-desktop` 2.1.0 (plugin `tauri-app-builder`).

## Global Constraints

- Si lavora su `develop`, oppure su un ramo breve che parte da `develop`. Mai `main`, mai cambiare la versione, mai creare tag (`AGENTS.md`).
- Dopo ogni modifica a CSS globali, `theme.css`, `theme-boot.js`, finestre Tauri o `desktop/src-tauri/src/`: `node tests/native-feel.mjs` in `desktop/`, tutto `ok`.
- Dopo una modifica a `recovery.rs` o `native_feel.rs`: `node tests/webview-recovery-native.mjs` in `desktop/`, con Qamille chiuso, sull'exe di release.
- L'exe di release si costruisce solo così:
  - prima `npm run build` in `desktop/`;
  - poi `cargo build --manifest-path desktop/src-tauri/Cargo.toml --release --features custom-protocol --locked --jobs 1` con `CARGO_TARGET_DIR=D:\Projects\qamille-mail\app\target`, che è la cache di `build-release.bat`.
  - Mai un exe senza `custom-protocol` in `release/`.
- Nessuna dipendenza nuova in npm o Cargo. Ammesse solo feature nuove della crate `windows` 0.62.2 già presente.
- Modulo `ds/` (ADR-0011 v3):
  - i componenti dell'app non importano `@ark-ui/*`: il comportamento nuovo va in una primitiva di `desktop/src/ds/primitives/`;
  - colori, raggi, ombre e testi solo da token (`tests/style-tokens.mjs`);
  - `npm run test:ds` verde.
- Testi visibili sempre in due lingue: `tr('italiano', 'English')` nel frontend, `window_shell::italian()` in Rust.
- Ogni nuovo gestore di Invio o di Esc controlla prima `imeComposing(event)` (`desktop/src/ds/ime.ts`). Lo verifica il test 11 di `native-feel.mjs`.
- Nuova regola o regola cambiata del comportamento nativo: aggiornare `docs/aspetto-nativo.md` nello stesso commit.
- Commenti nel codice in inglese, come il resto del progetto.

## Review Focus

1. Un passo di fine sessione che restituisce `undefined` o un valore diverso da un booleano conta come salvato; solo `false` o un'eccezione contano come non salvato. Va nel test del Task 1.
2. Fine sessione con un modulo non valido (evento con la fine prima dell'inizio, contatto senza rubrica): il passo non lancia, restituisce `false`, e la disconnessione prosegue. Va nei test del Task 2.
3. Il blocco dell'overscroll non deve togliere lo scorrimento ai pannelli. Lo scorrimento deve restare per:
   - l'onboarding e la schermata d'avvio, a 1000×640 e al 200 %;
   - il lettore, quando la rotella è sopra una mail HTML.
   Va nel test del Task 7.
4. Fermare la registrazione all'uscita di ripristino quando nessuna registrazione è attiva non fa niente e non mostra errori. Va nel test del Task 3.
5. Salvare un allegato su un volume che non accetta i flussi alternativi (FAT32, alcune condivisioni di rete) riesce comunque: solo il Mark-of-the-Web manca, e lo si scrive nel log. Va nel test del Task 14.

---

### Task 1: «Ricarica la finestra» non scarta più un salvataggio fallito (Q1)

**Files:**
- Modify: `desktop/src/lib/close-guards.ts` (tipo `SessionEndStep`, `endSession`)
- Modify: `desktop/src/components/ComposerDock.tsx:171-173` (`sessionEnd` delle bozze)
- Modify: `desktop/src/App.tsx:1447-1453` (`sessionEnd` delle riunioni) e `:1471-1473` (`sessionEnd` delle Impostazioni)
- Test: `desktop/tests/session-end.mjs`

**Interfaces:**
- Produces: `type SessionEndStep = (last: boolean) => Promise<boolean | void>`. `endSession` restituisce `false` se un passo lancia **o** restituisce esattamente `false`. Il Task 2 si basa su questo.

- [ ] **Step 1: test che falliscono** in `session-end.mjs`:
  - `test('a step that answers false counts as not saved', …)`: un passo `async () => false` più uno `async () => true` → `assert.equal(await guards.endSession(false), false)`; entrambi i passi sono stati eseguiti (contatore = 2).
  - `test('a step that answers nothing or true counts as saved', …)`: passi `async () => {}` e `async () => true` → `true`.
- [ ] **Step 2:** `node --test tests/session-end.mjs` (da `desktop/`): il primo test nuovo FALLISCE (oggi `true`).
- [ ] **Step 3:** in `endSession`, `if ((await sessionEnd?.(last)) === false) saved = false;` dentro lo stesso `try`. Poi i tre passi:
  - bozze: `(await Promise.all(...flush())).every((ok) => ok !== false)`;
  - riunioni: restituisce il risultato di `flushMeetingDrafts()`, e con `last` anche l'esito di stop e annullamento dei job (un'eccezione resta un'eccezione);
  - Impostazioni: `true` quando non c'è niente da salvare (non in Impostazioni o non sporche), altrimenti `(await settingsPanel.current?.save()) !== false`.
- [ ] **Step 4:** `node --test tests/session-end.mjs`: 6/6 PASS. `npm run check` in `desktop/`: nessun errore di tipo.
- [ ] **Step 5:** in `docs/aspetto-nativo.md`, regola 21, «ricarica solo se ha salvato tutto» ora è vera. In `docs/audit-tauri-nativo.md` segnare Q1 come risolto.
- [ ] **Step 6: Commit** — `git commit -m "Reload the window only when every save step reports success"`

### Task 2: scheda evento e modulo contatto proteggono la chiusura (Q6)

**Files:**
- Create: `desktop/src/lib/dirty-guard.ts` (senza Tauri)
- Modify: `desktop/src/components/Calendar.tsx` (stato `derive().dirty`, `save()` a `:718`)
- Modify: `desktop/src/components/contacts/ContactForm.tsx` (aggiungere lo stato sporco: `clean()` confrontato con i valori iniziali)
- Test: `desktop/tests/close-guards.mjs` (node --test)

**Interfaces:**
- Consumes: `registerCloseGuard(id, guard, { sessionEnd })` da `lib/window-close.ts`; la semantica di `false` del Task 1.
- Produces:
  - `useDirtyCloseGuard(id: string, dirty: boolean, onVeto: () => void, saveNow: () => Promise<boolean>): void` in `desktop/src/lib/dirty-guard.ts`.
  - Comportamento: registra la guardia solo mentre `dirty` è vero e la toglie quando torna falso o il componente si smonta.
  - La guardia chiama `onVeto()` e restituisce `false`. Il passo di fine sessione restituisce `saveNow()`.
  - La logica pura `createDirtyGuard(register)` sta nello stesso file, così si prova senza React.

- [ ] **Step 1: test che falliscono** in `close-guards.mjs`:
  - `test('a dirty editor vetoes the close and asks', …)`: con `dirty` vero, `run()` → `false` e `onVeto` chiamato una volta.
  - `test('a clean editor leaves the close alone', …)`: con `dirty` falso, `run()` → `true`.
  - `test('at session end a dirty editor saves, and an invalid form answers false', …)`: `saveNow` che restituisce `false` → `endSession(true)` è `false` e nessuna eccezione.
- [ ] **Step 2:** `node --test tests/close-guards.mjs`: FAIL (il modulo non esiste).
- [ ] **Step 3:** implementare `dirty-guard.ts`, poi collegarlo:
  - **Calendar:** id `'calendar-event'`, `onVeto` = `busyEditing()` più il focus sul primo campo della scheda, `saveNow` = `save()`. `save()` deve restituire `Promise<boolean>`: `false` se ritorna presto (`timeError`, `readOnly`, `busy`) o se fallisce.
  - **ContactForm:** id `'contact-form'`, `onVeto` mostra l'errore del modulo «Salva o annulla prima le modifiche al contatto.» / «Save or discard the contact first.», `saveNow` = `submit` senza evento, che restituisce `boolean`.
- [ ] **Step 4:** test nuovi PASS. Poi `npm run test:browser -- calendar-flow contacts`: PASS (nessuna regressione).
- [ ] **Step 5: Commit** — `git commit -m "Ask before closing the window over an unsaved event or contact"`

### Task 3: il ripristino ferma e salva la registrazione prima di uscire (Q7)

**Files:**
- Modify: `desktop/src-tauri/src/recovery.rs`. Uscite a `:134`, `:182` (anche `restart_app`), `:255`, `:341`, `:381`.
- Modify: `desktop/src-tauri/src/runtime.rs`, solo se `DesktopState` non espone già `meetings`.
- Test: modulo di test in fondo a `recovery.rs`, oppure funzione pura in `shell_rules.rs`.

**Interfaces:**
- Produces: `fn finish_meetings_before_exit(app: &AppHandle)` in `recovery.rs`.
  - Se `app.try_state::<DesktopState>()` c'è e `state.meetings.capture_status()` dice registrazione o pausa: chiama `state.meetings.stop()`, poi l'equivalente Rust di `meeting_cancel_all_jobs` (vedi `commands/meetings.rs`).
  - Ogni errore va a `native_feel::report`, mai un panic.
  - Va chiamata subito prima di ogni `exit(0)` e di `restart_app` nei percorsi elencati.

- [ ] **Step 1: test che fallisce.** Estrarre la decisione in `shell_rules::must_stop_capture(status: CaptureState) -> bool`, o in un enum locale se `CaptureSnapshot` non si può usare in `shell_rules.rs`, che deve restare senza dipendenze. Test: `#[test] fn only_an_active_or_paused_capture_is_stopped_before_exit()` → idle `false`, recording `true`, paused `true`.
- [ ] **Step 2:** `cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked shell_rules` con `CARGO_TARGET_DIR=D:\Projects\qamille-mail\app\target`: FAIL.
- [ ] **Step 3:** implementare `must_stop_capture` e `finish_meetings_before_exit`, e chiamarla nei cinque percorsi.
- [ ] **Step 4:** test PASS; `node tests/native-feel.mjs`: tutto ok.
- [ ] **Step 5:** nel task finale (16), sull'exe: registrazione attiva, terminare il processo browser di WebView2 (procedura in `webview-recovery-native.mjs`), Riavvia. La riunione risulta fermata, non «interrotta».
- [ ] **Step 6: Commit** — `git commit -m "Stop and save a recording before a recovery restart or close"`

### Task 4: un avvio fallito lo dice, e una pagina bianca non passa per pronta (Q2, Q15)

**Files:**
- Modify: `desktop/src-tauri/src/main.rs:84-...`: corpo di `.setup` spostato in `fn setup_app(app: &mut tauri::App, …) -> Result<(), Box<dyn std::error::Error>>`.
- Modify: `desktop/src-tauri/src/shell_rules.rs`: testo dell'errore, funzione pura.
- Modify: `desktop/src-tauri/src/recovery.rs` (`watch_startup`, `frontend_ready`, nuovo comando `frontend_failed`); `desktop/src/main.tsx`; `desktop/src/App.tsx` (primo effetto).
- Test: test in `shell_rules.rs`; `desktop/tests/native-feel.mjs` (controllo nuovo).

**Interfaces:**
- Produces:
  - `pub fn startup_failure_text(error: &str, italian: bool, webview_missing: bool) -> String` in `shell_rules.rs`.
  - Comando `#[tauri::command] fn frontend_failed(window: WebviewWindow, message: String)` in `recovery.rs`. Apre lo stesso dialogo «Ricarica la finestra / Chiudi Qamille» del controllo dei 15 s e va registrato in `generate_handler!`; `build.rs` lo aggiunge alla capability.

- [ ] **Step 1: test che falliscono:**
  - in `shell_rules.rs`, `#[test] fn startup_failure_names_the_reason_and_the_runtime()`:
    - con `webview_missing` il testo contiene «WebView2» e l'indirizzo `https://go.microsoft.com/fwlink/p/?LinkId=2124703`;
    - senza, contiene l'errore;
    - in italiano comincia con «Qamille non è riuscito ad avviarsi.».
  - in `native-feel.mjs`, controllo 12 «a failed start and a blank page reach a message»:
    - `main.rs` chiama `setup_app` dentro `.setup` e gestisce l'`Err` con `startup_failed`;
    - `main.tsx` passa `onUncaughtError` a `createRoot`;
    - `frontend_ready` non è più chiamato da `main.tsx` ma da un effetto di `App.tsx`.
- [ ] **Step 2:** `rustc --edition 2024 --test desktop/src-tauri/src/shell_rules.rs -o %TEMP%\shell_rules.exe` e `node tests/native-feel.mjs`: FAIL.
- [ ] **Step 3:** implementare.
  - In `.setup`: `if let Err(error) = setup_app(app, …) { startup_failed(&error) }`. `startup_failed` scrive in `native.log` (`native_feel::report`), mostra `rfd::MessageDialog` di livello Error con `startup_failure_text(…)` ed esce con `std::process::exit(1)`.
  - `webview_missing` = `tauri::webview_version().is_err()`, controllato prima di `WebviewWindowBuilder::build()`.
  - Il controllo dei 15 s riparte a ogni ricarica dell'app (`reload`, `reload_ready`); Esc sul dialogo d'avvio equivale a «Ricarica la finestra», non lascia la finestra bianca.
  - Il commento a `main.rs:588-590` va corretto.
- [ ] **Step 4:** test PASS; `node tests/native-feel.mjs`: 12/12 ok.
- [ ] **Step 5:** nel Task 16, sull'exe con una cartella dati usa e getta, ogni volta deve comparire un messaggio e il processo deve uscire con 1:
  - archivio rovinato;
  - `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` su una cartella vuota;
  - `ui.lock` tenuto da un altro processo.
- [ ] **Step 6: Commit** — `git commit -m "Show why Qamille could not start, and treat a blank page as a failed start"`

### Task 5: il dialogo della posta vecchia parte da «Lascia così» (Q3)

**Files:**
- Modify: `desktop/src/components/HistoryUnreadDialog.tsx` (`initialFocus` del `Modal`)
- Test: `desktop/tests/primitives-overlays.mjs`, oppure un file nuovo `desktop/tests/history-unread.mjs` con un caso nell'anteprima.

**Interfaces:** `Modal` accetta già `initialFocus?: () => HTMLElement | null` (`ds/primitives/Modal.tsx:16`).

- [ ] **Step 1: test che fallisce:** aperto il dialogo dall'anteprima (evento `history-unread` simulato come fanno i test esistenti), `document.activeElement` ha il testo «Leave as is», e uno Spazio non chiama `mark_history_read` (o il comando che usa `markRead`).
- [ ] **Step 2:** `npm run test:browser -- history-unread`: FAIL.
- [ ] **Step 3:** `ref` sul pulsante «Lascia così» e `initialFocus={() => leaveRef.current}`.
- [ ] **Step 4:** PASS.
- [ ] **Step 5: Commit** — `git commit -m "Start the old-mail dialog on Leave as is"`

### Task 6: la prima finestra sta nell'area di lavoro (Q4)

**Files:**
- Modify: `desktop/src-tauri/tauri.conf.json:13` (`"center": true, "preventOverflow": true`)
- Test: `desktop/tests/native-feel.mjs`, controllo 13 «the first window fits the work area»: legge `tauri.conf.json`, la finestra `main` ha `center === true` e `preventOverflow === true`.

- [ ] **Step 1:** scrivere il controllo 13 → FAIL.
- [ ] **Step 2:** cambiare la configurazione → `node tests/native-feel.mjs`: 13/13 ok.
- [ ] **Step 3:** nel Task 16, primo avvio con la scala al 150 % su 1920×1080: finestra tutta nell'area di lavoro.
- [ ] **Step 4: Commit** — `git commit -m "Keep the first window inside the work area"`

### Task 7: la radice di ogni WebView non scorre oltre il bordo (Q5, modifica standard 4)

**Files:**
- Modify: `desktop/src/ds/styles/system.css:25-40` (`html, body { height: 100%; overflow: hidden; overscroll-behavior: none; }`)
- Modify: `desktop/src/components/Onboarding.css:4-9` e `desktop/src/styles.css:2437-2446` (`.onboarding` e `.startup-state` diventano contenitori che scorrono: `height: 100vh; overflow: auto; overscroll-behavior: contain`)
- Modify: `desktop/src/reminder/reminder.css:28-33` (`overscroll-behavior: none`)
- Modify: `desktop/src/ds/styles/ark/select.css:59`, `ark/recipients.css:115`, `ark/dialog.css:29,182` (`overscroll-behavior: contain` su liste e corpi che scorrono)
- Modify: `docs/aspetto-nativo.md`: regola 4 riscritta, frase sul rimbalzo «solo macOS» tolta, checklist «Nuova finestra Tauri» con il blocco della radice.
- Test: `desktop/tests/native-feel.mjs`, controllo 14; `desktop/tests/glance-frame.mjs` o un file nuovo `desktop/tests/root-lock.mjs`.

L'iframe del messaggio resta senza `overscroll-behavior`: non scorre in verticale e deve passare la rotella al lettore.

- [ ] **Step 1: test che falliscono:**
  - controllo 14 «every WebView locks its root»: `system.css` e `reminder.css` hanno `overscroll-behavior: none` su `html` (o `html, body`), e `system.css` ha `overflow: hidden` su `html, body`.
  - Test browser `root-lock.mjs`:
    - a 1000×640 con zoom 2, `document.scrollingElement.scrollHeight === clientHeight` in Posta, in Impostazioni e nella schermata d'avvio;
    - `.onboarding` scorre fino al pulsante finale;
    - con la rotella sopra l'iframe di una mail HTML lunga, `.reader-scroll.scrollTop` aumenta.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare le modifiche CSS elencate.
- [ ] **Step 4:** `node tests/native-feel.mjs` tutto ok; `npm run test:browser -- root-lock compatto-settings compatto-reader mail-html onboarding-professional` PASS; `npm run test:ds` PASS.
- [ ] **Step 5:** nel Task 16, sull'exe: touchpad di precisione e touch oltre ogni bordo, niente scivola. Se non c'è un dispositivo, annotare la prova come da fare.
- [ ] **Step 6: Commit** — `git commit -m "Lock the root of every webview against overscroll"`

### Task 8: guardie native complete in ogni WebView (modifiche standard 6, 7, 10)

**Files:**
- Modify: `desktop/src-tauri/src/reminders.rs:154-171` (`.disable_drag_drop_handler()`)
- Modify: `desktop/src-tauri/src/native_feel.rs`:
  - `harden_webview`: `add_FrameNavigationStarting`, che rifiuta ogni URL di frame tranne `about:srcdoc` e `about:blank`;
  - `NavigationStarting`, che rifiuta le navigazioni indietro e avanti con `ICoreWebView2NavigationStartingEventArgs3::NavigationKind() == COREWEBVIEW2_NAVIGATION_KIND_BACK_OR_FORWARD`;
  - `PermissionRequested`, che nega per default;
  - il filtro del menu, che su errore COM chiama `SetHandled(true)` senza mostrare il menu e scrive l'errore con `report`;
  - il registro dei nomi delle voci di menu che il filtro toglie, scritto una volta per nome in `native.log`.
- Modify: `desktop/src-tauri/src/shell_rules.rs`: `pub fn frame_may_navigate(uri: &str) -> bool`.
- Modify: `docs/aspetto-nativo.md`, regole 9 e 23.
- Test: test in `shell_rules.rs`; `cargo test native_feel`; `desktop/tests/native-feel.mjs`, controllo 15.

**Interfaces:**
- Produces: `frame_may_navigate("about:srcdoc") == true`, `frame_may_navigate("about:blank") == true`, `frame_may_navigate("https://x.it/") == false`, `frame_may_navigate("http://tauri.localhost/index.html") == false`.

- [ ] **Step 1: test che falliscono:**
  - `#[test] fn only_the_message_document_may_load_in_a_frame()` con i quattro casi sopra;
  - controllo 15 «the reminder takes no file drops and every webview refuses frame, history and permission requests»: `reminders.rs` contiene `disable_drag_drop_handler`; `native_feel.rs` registra `add_FrameNavigationStarting` e `add_PermissionRequested`, e confronta con `COREWEBVIEW2_NAVIGATION_KIND_BACK_OR_FORWARD`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare. Ogni registrazione COM fallita va a `report`, mai `unwrap`.
- [ ] **Step 4:**
  - `cargo test --locked native_feel shell_rules` (con il `CARGO_TARGET_DIR` della cache) PASS;
  - `node tests/native-feel.mjs` tutto ok;
  - nel Task 16, `webview-recovery-native.mjs` 7/7.
- [ ] **Step 5:** nel Task 16, sull'exe, tasto destro su vuoto, testo, link, immagine e campo, con Maiusc+F10. Poi leggere `native.log`: i nomi registrati servono a correggere `MENU_ITEMS_KEPT`, per esempio se il nome vero è `copyLink` e non `copyLinkLocation`.
- [ ] **Step 6: Commit** — `git commit -m "Refuse frame, history and permission requests natively, and fail the menu filter closed"`

### Task 9: i link portano sempre al browser di sistema (Q11, Q12)

**Files:**
- Create: `desktop/src/lib/external-link.ts`.
- Modify: `desktop/src/components/MailboxSync.tsx:185`.
- Modify: `desktop/src/components/GoogleSetupGuide.tsx:13-21`, che usa l'helper invece della copia locale.
- Modify: `desktop/src/components/MessageHtml.tsx:34` e `desktop/src/lib/browser-guards.ts:37`.
- Modify: `desktop/src-tauri/src/main.rs:94-99`: `.on_new_window(...)` sul builder di main.
- Test: `desktop/tests/mailbox-sync-ui.mjs`, `desktop/tests/mail-html.mjs`.

**Interfaces:**
- Produces: `openExternal(event: { preventDefault(): void; currentTarget: { href: string } }, onError?: (message: string) => void): Promise<void>` in `lib/external-link.ts`. Chiama `call('open_link', { url })` sul desktop; nell'anteprima mantiene il comportamento attuale.
- Produces: `linkTarget(element: Element): string | null` in `browser-guards.ts`.
  - Risolve, contro `ownerDocument.baseURI`, l'indirizzo di `a[href]`, `area[href]` e `svg a`, sia con `href` sia con `xlink:href`.
  - Lo usano sia `MessageHtml` sia `installBrowserGuards`.
- Produces: `on_new_window` in main. Per un URL che `shell_rules::opens_outside` accetta chiama `open` del browser di sistema, come `open_link`; poi restituisce sempre `NewWindowResponse::Deny`.

- [ ] **Step 1: test che falliscono:**
  - `mailbox-sync-ui.mjs`: dopo un ritiro dei permessi fallito, clic sul link «Permissions of your Google account»; un ascoltatore in cattura su `document` registra `defaultPrevented === true`, e l'anteprima registra una chiamata `open_link`;
  - `mail-html.mjs`: in una mail con `<map><area href="https://example.com/a">` e un `<svg><a xlink:href="https://example.com/b">`, il clic sulla zona apre il browser (anteprima: chiamata `open_link` con l'URL assoluto) e l'iframe mantiene il suo documento (`frame.url()` invariato).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `npm run test:browser -- mailbox-sync-ui mail-html google-sign-in` PASS; `node tests/native-feel.mjs` ok.
- [ ] **Step 5: Commit** — `git commit -m "Open every external link, image map and SVG link in the system browser"`

### Task 10: scorciatoie che agiscono solo dove devono (Q13, AltGr, Esc nella ricerca)

**Files:**
- Modify: `desktop/src/lib/shortcut-guard.ts`: funzioni pure nuove.
- Modify: `desktop/src/App.tsx:1283-1354` (`keyboard()`), `:1618-1624` (Esc della ricerca).
- Modify: `desktop/src/components/Sidebar.tsx:107-113` e `desktop/src/components/Reader.tsx` (`aria-keyshortcuts` accanto ai suggerimenti esistenti).
- Test: `desktop/tests/shortcut-guard.mjs` (node --test).

**Interfaces:**
- Produces in `shortcut-guard.ts`:
  - `arrowsMoveMail(target: EventTarget | null): boolean`: vero solo se il bersaglio è `body`, oppure è dentro `.mail-list` o `.reader`;
  - `isAppChord(e: { ctrlKey: boolean; metaKey: boolean; altKey: boolean }): boolean`: `(ctrlKey || metaKey) && !altKey`, così AltGr, che su Windows è Ctrl+Alt, non conta.

- [ ] **Step 1: test che falliscono** in `shortcut-guard.mjs`, con elementi finti `{ closest(sel) }`:
  - `arrowsMoveMail`: vero per un bersaglio in `.mail-list` e per `body`; falso per un bersaglio in `.sidebar`, `.topbar` e `[role=region]` del centro attività;
  - `isAppChord({ctrlKey:true, altKey:true})` è falso.
- [ ] **Step 2:** `node --test tests/shortcut-guard.mjs`: FAIL.
- [ ] **Step 3:** usarle in `keyboard()`:
  - Ctrl+K e Ctrl+Z passano da `isAppChord`;
  - Su e Giù da `arrowsMoveMail(e.target)`;
  - Esc nella ricerca (con `imeComposing`) svuota il campo se ha del testo, altrimenti fa quello che fa oggi.
  - `aria-keyshortcuts` sui pulsanti che mostrano una scorciatoia: `"N"`, `"R"`, `"E"`, `"Control+K"`, e lo zoom sul controllo dello zoom se c'è.
- [ ] **Step 4:** PASS; `npm run test:browser -- a11y-keyboard search shortcut-guard` PASS.
- [ ] **Step 5: Commit** — `git commit -m "Keep the mail arrows and app shortcuts where they belong"`

### Task 11: il focus torna dove era (Q14, Q17, anelli di focus)

**Files:**
- Modify: `desktop/src/ds/primitives/SidePanel.tsx:28-60`: ricorda l'elemento attivo all'apertura e lo rimette a fuoco alla chiusura, se è ancora nel DOM, con lo stesso ripiego di `Modal.tsx:32-45`.
- Modify: `desktop/src/components/Composer.tsx:180-191, 270, 316` e `desktop/src/App.tsx:1250-1253`:
  - una risposta parte dal corpo;
  - Invia, Salva e chiudi e Scarta riportano il focus alla riga selezionata della lista, o all'elemento che aveva aperto il composer.
- Modify: `desktop/src/components/RichEditor.tsx:83-98`: tabindex mobile nella barra (`role=toolbar`), con frecce sinistra e destra, Home e Fine.
- Modify: `desktop/src/styles.css:2735` e `:1884-1890`: anello di focus visibile sull'`h1` di ripiego e sul corpo dell'editor, con i token esistenti dell'anello.
- Test: `desktop/tests/a11y-keyboard.mjs` (casi nuovi), `desktop/tests/composer-tabs.mjs`, `desktop/tests/calendar-flow.mjs`.

- [ ] **Step 1: test che falliscono:**
  - calendario: Tab fino a un evento, Invio, Esc → `activeElement` è l'evento;
  - composer: R, poi si scrive «ciao» → il testo sta nel corpo; Invia (anteprima) → `activeElement` è la riga selezionata;
  - barra dell'editor: un solo elemento con `tabindex=0`; la freccia destra sposta il focus sul pulsante seguente.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `npm run test:browser -- a11y-keyboard composer-tabs calendar-flow primitives-overlays` PASS; `npm run test:ds` PASS.
- [ ] **Step 5: Commit** — `git commit -m "Return the focus after side panels and the composer, and start replies in the body"`

### Task 12: i menu dei comandi sono menu veri (Q16, F15)

**Files:**
- Create: `desktop/src/ds/primitives/Menu.tsx` (Ark `Menu`) e `desktop/src/ds/styles/ark/menu.css`, con le stesse misure e gli stessi token del Popover usato oggi per i menu, quindi nessun cambio visibile.
- Modify: `desktop/src/ds/styles/ark.css` (import), `desktop/src/ds/README.md` (sezione «Menu»).
- Modify: `desktop/src/components/Reader.tsx:292`, `Contacts.tsx:388`, `meetings/MeetingMenu.tsx:106` (percorso da verificare), `RecordNow.tsx:87`.
- Test: `desktop/tests/primitives-overlays.mjs`; `desktop/tests/ds-usage.mjs`, regola nuova: niente `Popover` con figli tutti `<button>` che fanno da menu.

**Interfaces:**
- Produces: `<Menu trigger={ReactElement} label={string} items={MenuItem[]} />` con `type MenuItem = { value: string; label: string; onSelect: () => void; destructive?: boolean; disabled?: boolean; shortcut?: string }`.
  - Le voci distruttive vanno in fondo, dopo un separatore.
  - Esc e la scelta di una voce riportano il focus al trigger, come fa Ark.

- [ ] **Step 1: test che falliscono** in `primitives-overlays.mjs`, sul menu «Altre azioni» del lettore:
  - `role=menu`, voci `role=menuitem`;
  - Giù sposta tra le voci e Home/Fine vanno agli estremi;
  - una lettera porta alla voce che comincia così;
  - Esc riporta il focus al trigger.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare `Menu` e migrare i quattro menu.
- [ ] **Step 4:** `npm run test:browser -- primitives-overlays compatto-reader compatto-contacts compatto-meetings meetings` PASS; `npm run test:ds` PASS; `visual-compare` dei quattro menu, se esiste lo snapshot, senza differenze.
- [ ] **Step 5: Commit** — `git commit -m "Make the command menus real menus with Ark Menu"`

### Task 13: i temi Contrasto mostrano lo stato dei calendari e la barra della sidebar (Q18)

**Files:**
- Modify: `desktop/src/ds/styles/forced-colors.css`: regole per `.cal-row` mostrato e nascosto (`Calendar.css:73-89`), e per la barra di scorrimento della sidebar (`styles.css:95-116`); solo colori di sistema.
- Test: `desktop/tests/forced-colors.mjs` (casi nuovi).

- [ ] **Step 1: test che falliscono:** con `forcedColors: 'active'`, un calendario mostrato e uno nascosto hanno contorni o sfondi di sistema diversi; il cursore della barra della sidebar ha un colore di sistema diverso dal fondo.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `npm run test:browser -- forced-colors` PASS.
- [ ] **Step 5: Commit** — `git commit -m "Show calendar visibility and the sidebar scrollbar in Contrast themes"`

### Task 14: gli allegati salvati portano il Mark-of-the-Web (Q9)

**Files:**
- Create: `desktop/src-tauri/src/zone.rs`.
- Modify: `desktop/src-tauri/src/main.rs` (`mod zone;`), `desktop/src-tauri/src/commands/document_preview.rs:46-53`.
- Modify: `desktop/src-tauri/Cargo.toml`: feature `Win32_System_Com` della crate `windows`, se `CoCreateInstance` la richiede.
- Test: test in `zone.rs`.

**Interfaces:**
- Produces: `pub fn save_downloaded(path: &Path, bytes: &[u8], source_url: Option<&str>) -> std::io::Result<()>` in `zone.rs`.
  - Scrive il file.
  - Poi applica la zona Internet con `IAttachmentExecute` (`CLSID AttachmentServices`, `SetClientGuid` con un GUID fisso di Qamille, `SetLocalPath`, `SetSource`, `Save`).
  - Se COM non è disponibile, ripiega su `pub fn write_zone_identifier(path: &Path) -> std::io::Result<()>`, che scrive `"[ZoneTransfer]\r\nZoneId=3\r\n"` nel flusso `Zone.Identifier`.
  - Un errore della sola zona non fa fallire il salvataggio: va a `native_feel::report`.

- [ ] **Step 1: test che falliscono:**
  - `#[test] fn a_saved_file_carries_zone_3()`: file temporaneo, `write_zone_identifier`, poi la lettura di `<path>:Zone.Identifier` contiene `ZoneId=3`;
  - `#[test] fn a_volume_without_streams_still_keeps_the_file()`: simulato con un percorso il cui flusso non si può aprire; il file principale c'è e l'errore torna come `Err` solo dalla funzione della zona.
- [ ] **Step 2:** `cargo test --locked zone`: FAIL.
- [ ] **Step 3:** implementare e usarla in `document_preview.rs` al posto di `std::fs::write`.
- [ ] **Step 4:** PASS. Nel Task 16, sull'exe: salvare un `.docx` allegato, poi `Get-Content <file> -Stream Zone.Identifier` → `ZoneId=3`, e Word lo apre in Visualizzazione protetta.
- [ ] **Step 5: Commit** — `git commit -m "Mark saved attachments as coming from the Internet"`

### Task 15: la disinstallazione toglie avvio del worker e, se chiesto, i dati (Q23)

**Files:**
- Create: `desktop/src-tauri/windows/installer-hooks.nsh`, con le macro `NSIS_HOOK_PREUNINSTALL` e `NSIS_HOOK_POSTUNINSTALL`.
- Modify: `desktop/src-tauri/tauri.professional.conf.json`: `bundle.windows.nsis.installerHooks`.
- Test: `desktop/tests/native-feel.mjs`, controllo 16.

**Interfaces:**
- `NSIS_HOOK_PREUNINSTALL`:
  - termina i processi `Qamille.exe` avviati con `--worker` dalla cartella `$INSTDIR`;
  - cancella il file d'avvio del worker scritto da `mcp_config.rs:203-213`. Va calcolato con lo stesso nome usato lì; il Task del piano 2 lo sostituirà con un valore `Run`.
- `NSIS_HOOK_POSTUNINSTALL`: se `$DeleteAppDataCheckboxState == 1`, fa `RMDir /r "$INSTDIR\data"` (cartella dati di `legacy_store.rs:30-35`).

- [ ] **Step 1: test che fallisce:** controllo 16 «the installer removes the worker's startup and, when asked, the data»:
  - `tauri.professional.conf.json` punta a `windows/installer-hooks.nsh`;
  - il file definisce le due macro;
  - contiene `$DeleteAppDataCheckboxState` e il nome del file d'avvio preso da `mcp_config.rs`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `node tests/native-feel.mjs` ok.
- [ ] **Step 5:** nel Task 16, con `QAMILLE_SERVER` impostata:
  - `build-release.bat --no-install` costruisce l'installer;
  - installazione, avvio del worker, poi disinstallazione con la casella spuntata;
  - `%LOCALAPPDATA%\Qamille` non c'è più e all'accesso successivo non compare nessun errore di Windows Script Host.
  - Se `QAMILLE_SERVER` non c'è, annotare la prova come da fare.
- [ ] **Step 6: Commit** — `git commit -m "Clean the worker's startup entry and, when asked, the data on uninstall"`

### Task 16: verifica sull'exe e documenti

**Files:**
- Modify: `docs/audit-tauri-nativo.md`, quarta passata: una tabella «Stato delle correzioni della fase 2» con commit e prove per Q1–Q7, Q9, Q11–Q18, Q23 e per le modifiche standard 4, 6, 7, 10.
- Modify: `docs/aspetto-nativo.md`, sezione «Verifica nell'app vera»: i controlli nuovi.

- [ ] **Step 1:** `npm run build` in `desktop/`, poi la build di release nelle Global Constraints. Controllare che l'exe contenga `theme-boot.js`; copiarlo in `release/Qamille.exe`.
- [ ] **Step 2:** con Qamille chiuso, in `desktop/`:
  - `node tests/webview-recovery-native.mjs` 7/7;
  - `node tests/close-supervision-native.mjs` 3/3;
  - `node tests/release-theme-probe.mjs` 9/9, con URL `http://tauri.localhost/`;
  - `node tests/a11y-native.mjs` verde.
- [ ] **Step 3:** le prove manuali dei Task 3, 4, 6, 7, 8, 14 e 15. Per ognuna annotare «osservato», oppure «da fare» con il motivo, per esempio nessun touchpad o nessuna `QAMILLE_SERVER`.
- [ ] **Step 4:** `node tests/native-feel.mjs`, `npm run test:ds`, `node --test tests/session-end.mjs tests/close-guards.mjs tests/shortcut-guard.mjs`, e i test browser toccati dai task: tutti verdi.
- [ ] **Step 5: Commit** — `git commit -m "Record the phase-2 native fixes and their checks"`

---

## Dopo questo piano: fase 3 (piani separati, in quest'ordine)

Ogni voce è approvata dal proprietario in Auto («Fai tu», 2026-11-20). Ognuna avrà il suo piano, scritto quando ci si arriva, e prima di implementarla si dice che cosa cambia nell'app e nel progetto, come chiede `background-presence.md`.

1. **Presenza in background.**
   - Chiusura nella tray con icona, Mostra ed Esci; Esci passa dalle guardie e invia la coda di annulla invio (Q8). Avviso alla prima chiusura.
   - Avvio all'accesso spento per default, nella tray con il plugin autostart: sostituisce il file VBS del worker (Q22).
   - Una sola via di ripristino per tray, seconda istanza e clic sulle notifiche: mostra, ripristina, focus.
   - Webview nascosta quando la finestra è ridotta a icona.
2. **Dimensione e posizione della finestra**, salvate in `window-preferences.json` e sempre su un monitor collegato.
3. **Notifiche di sistema** (prima nell'edizione Professional: AUMID dell'installer) per posta nuova, invii programmati falliti e accesso scaduto. **Stato sulla barra delle applicazioni**: icona sovrapposta con i non letti, barra di avanzamento per l'indicizzazione completa. ADR-0004 va aggiornato.
4. **mailto**: i link nei messaggi aprono il composer di Qamille (Q10); poi la registrazione come app di posta predefinita (hook NSIS; facoltativa nella portatile) e gli argomenti della seconda istanza.
5. **File dall'OS**: «Apri» e «Mostra nella cartella» dopo un salvataggio.
6. **Menu contestuale delle righe**: un menu nativo costruito da un comando dell'app, senza `core:menu` in `main.json`.
7. **Credenziali**: nell'edizione Professional, la chiave della cassaforte in Gestione credenziali o con DPAPI; la portatile mantiene la scelta dichiarata in `secret.rs`.
8. **Aggiornamenti e firma** (Professional): updater e Authenticode. Servono dal proprietario un certificato di firma e la chiave dell'updater nei segreti di GitHub: il piano prepara il codice e il workflow e si ferma lì.

Restano fuori, come decisioni del proprietario: Q19 (dopo l'osservazione), Q20, Q21, F5 e T16, e i P2 non elencati nei task sopra.
