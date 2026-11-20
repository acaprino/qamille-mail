# Aspetto nativo dell'app desktop

Qamille gira dentro WebView2: senza accorgimenti si comporta come una pagina web
(testo selezionabile ovunque, F5 ricarica, tasto destro con "Indietro" e "Stampa",
finestra bianca all'avvio col tema scuro). Questa guida elenca le regole che lo
fanno sembrare un'app Windows e come non romperle.

Origine: [8 tips for creating a native look and feel in Tauri applications](https://dev.to/akr/8-tips-for-creating-a-native-look-and-feel-in-tauri-applications-3loe),
adattato a Tauri 2 su Windows. Il consiglio solo macOS sul beep dei tasti non si applica;
lo scorrimento oltre il bordo sì: un touchpad di precisione o un touchscreen lo fanno anche
su Windows (regola 4).

**Controllo automatico:** `node tests/native-feel.mjs` (da `desktop/`, o
`npm run test:native-feel`). Gira in meno di un secondo, senza build. Va lanciato
dopo ogni modifica a CSS globali, tema, finestre o `src-tauri/src/`.

## Le regole

| # | Regola | Dove vive | Controllo |
|---|---|---|---|
| 1 | Il testo dell'interfaccia non si seleziona e mostra la freccia, non la barra a I | `body` in `desktop/src/ds/styles/system.css` e `desktop/src/reminder/reminder.css` | test 1 |
| 2 | Campi, editor e contenuti da leggere restano selezionabili, con cursore di testo; così anche ciò che si copia altrove (configurazioni e percorsi in `pre`/`code`, errori con `role="alert"`, URL e utente del server, stato dell'abbonamento) | regola `input, textarea, [contenteditable], .selectable, .reader-scroll, pre, code, [role='alert']…` subito sotto `body` in `ds/styles/system.css` | test 2 |
| 3 | Pulsanti, link e controlli con la freccia, mai la mano né il divieto: Microsoft chiede la freccia su ogni elemento cliccabile e l'effetto hover al posto della mano. Chromium mette la mano su ogni `a[href]`, quindi anche i link delle guide la tolgono. La mano resta solo sui link dentro i messaggi, che sono documento (nell'iframe del messaggio, con il suo stile) | `button, a { cursor: default }` in `ds/styles/system.css`; nessun `cursor: pointer`/`not-allowed` né `cursor-pointer`/`cursor-not-allowed` in `src/` | test 2 |
| 4 | La radice di ogni WebView non scorre e non scivola oltre il bordo: un touchpad di precisione o un touchscreen non possono spostare tutta la vista e scoprire lo sfondo della finestra. Scorrono solo i pannelli, ciascuno nel suo contenitore con `overscroll-behavior: contain`, così la rotella a fine corsa non passa alla radice. Una schermata più alta della finestra (accesso, avvio, tutto a 200 %) è essa stessa il contenitore che scorre, e il suo centro sicuro (`safe center`) non taglia mai la parte alta. L'iframe del messaggio resta senza `overscroll-behavior`: non scorre in verticale e passa la rotella al lettore | `html, body { height: 100%; overflow: hidden; overscroll-behavior: none }` in `ds/styles/system.css` e (senza `height`) in `reminder/reminder.css`; `.app-shell { overflow: hidden }`; `.onboarding` (`components/Onboarding.css`) e `.startup-state` (`styles.css`): `height: 100vh; overflow: auto; overscroll-behavior: contain`; `contain` anche su liste e corpi che scorrono (`ds/styles/ark/select.css`, `recipients.css`, `dialog.css`) | test 14 + `tests/root-lock.mjs` |
| 5 | Il tema all'avvio è già quello giusto, anche con "Sistema" | `desktop/public/theme-boot.js` (stessa logica di `resolvedAppearance()` in `src/lib/appearance.ts`) | test 3 |
| 6 | Lo sfondo della finestra nativa è il `--ground` del tema (quello di `html`, `body` e `.app-shell`), mai bianco su tema scuro: ridimensionando non compare una striscia di un altro colore | chiaro: `backgroundColor` in `tauri.conf.json`; scuro: `DARK_GROUND` in `src-tauri/src/native_feel.rs`; durante l'uso: `paintWindowBackground()` in `src/lib/native-window.ts`, chiamata dall'effetto del tema in `App.tsx` | test 4, 6, 7 |
| 7 | La finestra principale nasce nascosta e si mostra già nell'aspetto scelto in Impostazioni (Chiaro, Scuro o Sistema) e con lo zoom salvato. Se parte scura, la WebView nasce già con `DARK_GROUND` (WebView2 prende il colore alla creazione, prima di qualsiasi pagina). Barra del titolo, menu contestuale e controlli di WebView2 seguono lo stesso aspetto | `window_config.visible = false`; `window_shell::starts_dark()` (Impostazioni, oppure `AppsUseLightTheme` di Windows) → `background_color` in `main.rs`; `native_feel::show_main_window()` → `window_shell::apply_startup_preferences()`, che legge `window-preferences.json` nella cartella dati; lo scrive il comando `window_appearance`, chiamato dall'effetto del tema in `App.tsx` | test 5, 10 |
| 8 | Niente scorciatoie da browser: F5/Ctrl+R, Ctrl+P, Ctrl+F, F12. Lo zoom però resta, ed è dell'app: Ctrl+= (o +), Ctrl+- e Ctrl+0, anche con il focus in un campo, dall'80 % al 200 % (passi da 50 % a 200 %), salvato. Parte dalla "Dimensione del testo" di Windows (Accessibilità), riletta quando la finestra riprende il focus; Ctrl+0 torna a quella | `SetAreBrowserAcceleratorKeysEnabled(false)` in `native_feel::harden_webview()`; zoom: `zoomKey()` in `src/lib/shortcut-guard.ts`, `keyboard()` in `App.tsx`, comando `window_zoom` (`WebviewWindow::set_zoom`) in `window_shell.rs`, passi in `shell_rules.rs` | test 5, 8 + `tests/session-end.mjs` + `rustc --test src/shell_rules.rs` |
| 9 | Tasto destro senza voci da browser: restano solo Annulla, Ripeti, Taglia, Copia, Incolla (anche come testo semplice), Seleziona tutto, emoji, suggerimenti ortografici, Copia indirizzo del link e Copia immagine; ogni altra voce (Indietro, Ricarica, Stampa, Condividi, Cerca, Traduci, voci nuove del runtime) non compare, e dove non resta niente non compare il menu | filtro `ContextMenuRequested` in `native_feel.rs`, lista di voci ammesse `MENU_ITEMS_KEPT`; vale anche dentro l'iframe del messaggio | test 5 + `cargo test native_feel` |
| 10 | Niente "Suggerimenti" automatici del browser sui campi (coprirebbero quelli dei destinatari) e niente "Salvare la password?": in WebView2 il salvataggio delle password è spento per default e wry non lo accende, quindi non serve codice | `.general_autofill_enabled(false)` su ogni `WebviewWindowBuilder` | test 5 |
| 11 | Scorciatoie da tastiera proprie dell'app | `keyboard()` in `App.tsx` | test di flusso esistenti |
| 12 | Menu di sistema (Ripristina, Sposta, Ridimensiona, Riduci a icona, Ingrandisci, Chiudi) con Alt+Spazio anche con il focus nella pagina, e con il tasto destro sulla barra del titolo o sull'intestazione della sidebar. "Chiudi" passa dai guard di chiusura | Alt+Spazio: `AcceleratorKeyPressed` in `window_shell::attach_main()`; tasto destro: `installWindowShell()` in `src/lib/window-shell.ts` sugli elementi con `data-tauri-drag-region`, comando `show_system_menu`. Il menu si apre dalla window procedure (`WM_APP`), mai dentro un callback di WebView2 | test 8 |
| 13 | Ombra DWM, angoli arrotondati (Windows 11) e bordi di ridimensionamento fuori dal contenuto | `"shadow": true` in `tauri.conf.json`; su Windows 10 resta un bordo di 1 px | test 8 |
| 14 | Snap layouts al passaggio sul pulsante Ingrandisci (Windows 11) | una finestra figlia sopra il pulsante risponde `HTMAXBUTTON` (`window_shell.rs`, sotto la striscia di ridimensionamento di Tauri). Lì il mouse lo gestisce Windows: il clic ingrandisce o ripristina da Rust, la pagina riceve `window-maximize-button` e disegna hover e pressione (`WindowControls.tsx`, `data-caption`). Il tooltip del pulsante non compare più al passaggio; tastiera e Invio restano alla pagina | test 8 |
| 15 | Disconnessione, arresto, riavvio e Restart Manager salvano prima di uscire: bozze, Impostazioni non salvate (salvate, nessuna domanda), appunti delle riunioni; la registrazione si ferma e si salva solo quando la sessione finisce davvero. Windows mostra "Salvataggio di bozze, impostazioni e registrazione…" mentre aspetta (al massimo 8 s) | `WM_QUERYENDSESSION`/`WM_ENDSESSION` in `window_shell.rs` (anche sulla finestra nascosta di tao, che altrimenti esce subito), evento `session-ending`, `sessionEnd` di ogni guard registrato con `registerCloseGuard`, comando `session_end_ready` | test 8 + `tests/session-end.mjs` |
| 16 | La stampa apre il dialogo Stampa di Windows dopo l'anteprima dell'app, non l'anteprima di Edge | `ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM)` in `window_shell::print_with_system_dialog()`, chiamata da `print_mail`; con un runtime WebView2 vecchio ricade su `window.print()` | test 8 |
| 17 | Salvataggi su file sempre con il dialogo "Salva con nome" di Windows, con owner la finestra; mai un download di WebView2 | `rfd` con `set_parent(&window)` in `src-tauri/src/commands/*` (es. `contacts_save_vcf`); `<a download>` solo nell'anteprima browser | revisione |
| 18 | Temi Contrasto di Windows: selezione, pagina attiva, focus, Switch, Checkbox, chip e bordi che significano qualcosa restano visibili | `src/ds/styles/forced-colors.css` (solo colori di sistema: `Highlight`, `HighlightText`, `CanvasText`, `ButtonText`, `GrayText`), importato per ultimo da `src/main.tsx` | `tests/forced-colors.mjs` |
| 19 | Il promemoria riunione non compare in Alt+Tab né nella barra delle applicazioni, si chiude con Esc ("più tardi") e l'Assistente vocale lo annuncia | `window_shell::make_tool_window()` in `reminders.rs` (`WS_EX_TOOLWINDOW`, senza owner: con la principale ridotta a icona deve restare visibile, ADR-0004); `role="alert"` ed Esc in `src/reminder/Reminder.tsx` | test 8 |
| 20 | Privilegi minimi della finestra principale: solo i permessi core che la pagina usa, niente `core:default` (menu, tray, emit) | `capabilities/main.json` | test 8 |
| 21 | Mai la pagina d'errore di WebView2 né una finestra morta. Se il processo della pagina si chiude (crash, memoria) la pagina si ricarica da sola, al massimo 3 volte in 2 minuti; poi una finestra di messaggio di Windows, con owner la principale, chiede se ricaricare. Se la pagina non risponde chiede se ricaricarla. Se si chiude il processo browser di WebView2 chiede di riavviare Qamille (Tauri avvia il nuovo processo prima che il vecchio esca: il nuovo aspetta che il vecchio abbia lasciato `ui.lock`). Se la pagina dell'app non si carica (build di sviluppo senza `npm run dev`) lo dice e propone di riprovare; se si carica ma non disegna entro 15 s (script rotto, primo render di App che lancia), propone "Ricarica la finestra" / "Chiudi Qamille": `frontend_ready` arriva dal primo effetto di `App.tsx`, quindi una pagina bianca non passa per pronta. Il controllo dei 15 s riparte con ogni pagina che la finestra carica (`on_page_load`), anche una ricarica chiesta dalla pagina stessa, e su quel dialogo Esc e la X ricaricano: la finestra non resta mai bianca. Un errore di render o di un effetto senza error boundary (React toglie tutto l'albero) apre subito lo stesso dialogo, anche a metà sessione ("La finestra di Qamille è vuota"): `onUncaughtError` in `src/main.tsx` → `frontend_failed`. Un avvio che fallisce (cartella già aperta in un altro Qamille, anche dopo un riavvio lento; archivio illeggibile; WebView2 Runtime mancante, con l'indirizzo per installarlo; un panic) mostra il motivo in una finestra di messaggio di Windows, lo scrive in `native.log`, ferma come ogni uscita i lavori delle riunioni ed esce con 1; se la finestra principale è già comparsa, si nasconde prima del messaggio, e sopra quel messaggio non si apre nessun dialogo di ripristino. Tauri 2.12 trasformerebbe l'`Err` di `.setup` in un panic, e un panic lì chiude il processo senza una parola. "Ricarica la finestra" nel menu di sistema (Alt+Spazio, o Maiusc+clic destro sul pulsante nella barra delle applicazioni) funziona anche a pagina bianca: prima la pagina salva come alla disconnessione, senza fermare la registrazione, e ricarica solo se ha salvato tutto: un passo di salvataggio (bozze, Impostazioni, appunti delle riunioni) che lancia o risponde `false` conta come non salvato (`CloseGuards.endSession`). Se qualcosa non si salva chiede ("Non ricaricare" / "Ricarica comunque"); se la pagina non risponde entro 10 s chiede ("Attendi" / "Ricarica adesso"): non scarta mai da sola. La chiusura la decide la pagina (i guard): se non conferma di averla ricevuta entro 3 s, o se la si chiede di nuovo mentre una chiusura dura da più di 10 s, Qamille chiede "Attendi" / "Chiudi Qamille", così la X e Alt+F4 non restano mai senza effetto. Ogni uscita senza la pagina ("Chiudi Qamille", "Riavvia Qamille") ferma e salva prima la registrazione in corso e ferma i lavori delle riunioni, come il guard di chiusura della pagina (`finish_meetings_before_exit`, al massimo 20 s): la riunione risulta fermata, non "interrotta". Le finestre di messaggio sono task dialog con pulsanti che dicono cosa fanno; Esc e la X scelgono l'opzione innocua, mai "Chiudi Qamille" | `src-tauri/src/recovery.rs` (`ask`, rfd con `common-controls-v6`; `watch_startup`, `page_load` come `on_page_load` della principale, `offer_reload`, `frontend_failed`); `SetIsBuiltInErrorPageEnabled(false)`, `ProcessFailed` e `NavigationCompleted` in `native_feel::harden_webview()`; limite in `shell_rules::may_reload_by_itself`; voce `SC_APP_RELOAD` in `window_shell.rs`; `reload-requested` → `reload_ready` in `src/lib/window-close.ts`; avvio fallito: `setup_app` e `startup_failed` in `main.rs`, testo in `shell_rules::startup_failure_text` | test 9, 12 + `rustc --test src/shell_rules.rs` + `tests/webview-recovery-native.mjs` + `tests/close-supervision-native.mjs` + `tests/session-end.mjs` |
| 22 | Icone, avatar e link dell'interfaccia non si trascinano fuori dalla finestra con la "fantasma" del browser | `img, a { -webkit-user-drag: none }` in `ds/styles/system.css`; le righe della lista si trascinano con il loro gestore | test 2 |
| 23 | Ogni finestra mostra solo la pagina dell'app: una navigazione verso un altro indirizzo viene rifiutata e un link web o mailto si apre nel browser di sistema (solo dalla principale). I tasti Indietro/Avanti del mouse non navigano; clic centrale e Ctrl+clic su un link non aprono finestre | `.on_navigation(native_feel::navigation_guard(app, …))` su ogni `WebviewWindowBuilder` (`shell_rules::is_app_page`, `opens_outside`); `installBrowserGuards()` (`src/lib/browser-guards.ts`) in `src/main.tsx` e `src/reminder/main.tsx` | test 5, 11 + `rustc --test src/shell_rules.rs` |
| 24 | L'Invio che conferma un candidato dell'IME (giapponese, cinese, coreano) non invia né cerca, e l'Esc che lo annulla non chiude il composer, la ricerca o un pannello | `imeComposing()` (`src/ds/ime.ts`: `isComposing` o `keyCode` 229) in ogni gestore di Invio e di Esc (ricerca in `App.tsx`, `DateField`, `RecipientsInput`, `Composer`, `SearchBar`, `ActivityCenter`) | test 11 |

Già a posto grazie a Tauri/wry, da non riattivare: barra di stato dei link
(`SetIsStatusBarEnabled(false)`), zoom del browser con Ctrl+rotella e pizzico (`zoomHotkeysEnabled`
falso di default: lo zoom è quello dell'app, regola 8), gesti indietro/avanti del touchpad.

## Checklist per chi modifica

**Nuovo pannello con testo da copiare** (dettaglio, risposta, trascrizione, errore):
aggiungi la classe `selectable` al contenitore, oppure il suo selettore alla regola 2
in `ds/styles/system.css`. Non mettere `user-select: text` su tutta la pagina.

**Nuovo elemento cliccabile che non è un `<button>`**: preferisci un `<button>`. In ogni caso
niente `cursor: pointer` (regola 3): lo stato hover basta.

**Cambi `--ground` in `theme.css`** (es. restyle Ark): aggiorna anche
`backgroundColor` in `tauri.conf.json` (chiaro) e `DARK_GROUND` in `native_feel.rs`
(scuro). Il test 4 fallisce finché non coincidono. `--surface` non è più legato alla finestra.

**Nuovo editor con dati da salvare alla chiusura**: registra il guard con
`registerCloseGuard(id, guard, { sessionEnd })`. `sessionEnd(last)` salva senza chiedere
niente (a fine sessione nessuno risponde a un dialogo): con `last = false` Windows può
ancora annullare la fine sessione, quindi niente di irreversibile; con `last = true` sì.

**Nuova zona di trascinamento** (`data-tauri-drag-region`): il tasto destro su di essa apre
il menu di sistema da sé (regola 12). Se contiene un controllo, il controllo non deve avere
l'attributo.

**Nuovo stato mostrato solo con un colore di sfondo o un'ombra** (riga selezionata, voce
attiva, interruttore): aggiungi il selettore a `ds/styles/forced-colors.css` con un bordo o un
contorno in un colore di sistema, e un caso a `tests/forced-colors.mjs`.

**Nuova finestra Tauri**: blocca la radice del suo foglio di stile (regola 4): `html, body { overflow: hidden;
overscroll-behavior: none }`, e `overscroll-behavior: contain` su ogni suo contenitore che scorre; poi aggiungi
il foglio al controllo 14 di `tests/native-feel.mjs`. Subito dopo `.build()?` chiama
`crate::native_feel::harden_webview(&window, "pagina.html")`, con la pagina del builder: la ricarica dopo le impostazioni, che WebView2 applica dalla navigazione successiva (scorciatoie, menu, pagina d'errore e
ripristino, regola 21) e aggiungi al builder `.general_autofill_enabled(false)` e
`.on_navigation(crate::native_feel::navigation_guard(app, false))` (`true` solo se la finestra
può aprire link nel browser). Nel suo `main.tsx` chiama `installBrowserGuards()`. Se la finestra
è opaca, costruiscila nascosta e mostrala con lo sfondo del tema, come `show_main_window()`. Il
test 5 conta builder e chiamate in ogni file `.rs`.

**Nuovo campo che reagisce a Invio o a Esc**: prima di agire controlla `imeComposing(event.nativeEvent)`
(`src/ds/ime.ts`); il test 11 cerca ogni `key === 'Enter'` o `'Escape'` nei `.ts` e `.tsx`.

**Nuova voce da tenere nel tasto destro**: aggiungi il suo nome (`Name` di
`ICoreWebView2ContextMenuItem`) a `MENU_ITEMS_KEPT` e un caso ai test in fondo a
`native_feel.rs`. Il filtro è una lista di voci ammesse apposta: quelle che non
conosce (Cerca, Traduci, Leggi ad alta voce, voci nuove di un runtime aggiornato) non compaiono.

**Ti serve ricaricare o aprire DevTools**: nelle build debug il tasto destro mantiene
"Ispeziona"; da lì si apre DevTools e si ricarica. In release non c'è.

**Stampa**: passa solo da `PrintPreview.tsx`, che chiama `print_mail`: dialogo Stampa di
Windows (regola 16). Ctrl+P del browser è spento di proposito.

**Salvare un file**: un comando Rust con `rfd::AsyncFileDialog::new().set_parent(&window)`
(regola 17), come `export_mail` o `contacts_save_vcf`; il percorso con il Blob resta solo per
l'anteprima browser (`desktop` falso).

## Verifica nell'app vera

1. `npm run build` in `desktop/`, poi la build come in `AGENTS.md`.
2. Con Windows in tema scuro e Aspetto = Sistema: l'avvio non mostra mai un lampo bianco.
3. Trascina il mouse su sidebar e lista: non si evidenzia niente. Sul corpo di un messaggio sì.
4. Tasto destro su un'etichetta: nessun menu. Su testo selezionato: Copia. In un campo: Taglia/Copia/Incolla.
5. F5, Ctrl+R, Ctrl+P, Ctrl+F: non succede niente. Ctrl+K apre la ricerca dell'app.
6. Ctrl+=, Ctrl+-, Ctrl+0 ingrandiscono, rimpiccioliscono e ripristinano la pagina, anche con il
   focus in un campo; lo zoom resta dopo il riavvio. Con "Dimensione del testo" di Windows al 150 %
   l'app parte al 150 % (non al 225 %: se succede, WebView2 applica già la scala da sé e va tolta).
7. Alt+Spazio con il focus in un campo, e tasto destro sul titolo "Posta / …": menu di sistema.
   "Sposta" + frecce sposta la finestra; "Chiudi" con Impostazioni modificate chiede se salvare.
   Tasto destro in un campo: ancora Taglia/Copia/Incolla.
8. Windows 11: angoli arrotondati e ombra in chiaro e in scuro; si ridimensiona da appena fuori
   dal bordo; la scrollbar del lettore sul bordo destro scorre, non ridimensiona.
9. Windows 11: mezzo secondo sul pulsante Ingrandisci mostra gli snap layouts; un clic sul
   pulsante ingrandisce e ripristina; hover e pressione si vedono.
10. Con una bozza appena digitata, Impostazioni modificate e una registrazione attiva:
    `shutdown /l`. Al rientro bozza e Impostazioni ci sono, la registrazione è chiusa e non
    "interrotta".
11. Stampa / Salva PDF: si apre il dialogo Stampa di Windows, senza l'anteprima di Edge.
12. Tema Contrasto (Impostazioni → Accessibilità → Temi a contrasto): riga selezionata, voce
    attiva della sidebar, focus e interruttori distinguibili.
13. Con un promemoria visibile: Alt+Tab mostra una sola voce Qamille; Esc sul promemoria lo
    chiude; con l'Assistente vocale il promemoria viene letto.
14. `node tests/webview-recovery-native.mjs` (Qamille chiuso; `QAMILLE_NATIVE_EXE` per un exe
    diverso da `release/Qamille.exe`): termina il renderer e poi il processo browser di
    WebView2 e controlla ricarica, limite, finestra di messaggio modale, riavvio e la voce
    "Ricarica la finestra".
15. Passando su pulsanti, schede, interruttori e voci di menu il cursore resta la freccia; sui
    link dentro un messaggio diventa la mano.
16. Con una cartella dati usa e getta: archivio rovinato, `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` su
    una cartella vuota, `ui.lock` tenuto da un altro processo. Ogni volta compare un messaggio con
    il motivo (per WebView2, l'indirizzo da cui installarlo), una riga va in `native.log` e il
    processo esce con 1.
17. Touchpad di precisione e touchscreen: trascina con due dita (o il dito) oltre ogni bordo, in Posta,
    nel lettore con un messaggio lungo, in Impostazioni, nella schermata d'accesso e d'avvio (anche a
    200 % in una finestra piccola) e sul promemoria: niente scivola e non compare lo sfondo della finestra.
