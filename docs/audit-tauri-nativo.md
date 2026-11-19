# Audit integrazione desktop nativa: Qamille (Tauri 2 + WebView2, Windows)

Skill applicata: `.claude/skills/tauri-native-desktop/SKILL.md`, passi 1–3 della procedura e consegna d'audit del passo 4. Nessun file del progetto è stato modificato.

**Difetto prioritario.** Nessuna delle sei finestre di sistema aperte con `rfd` (salva allegato, esporta email, allega file, importa ed esporta riunione, conferma "Registrazione in corso" alla chiusura) ha un owner. La principale resta attiva sotto il dialogo e può coprirlo. Durante la conferma di chiusura la X e Alt+F4 vengono ignorati in silenzio, perché il dialogo è ancora aperto (vedi F1).

## Stato delle correzioni (aggiornato il 2026-11-12, dopo l'audit)

Legenda delle verifiche: **[msvc]** compilato con `cargo xwin check --target x86_64-pc-windows-msvc --locked --features custom-protocol` (crate completa, in Linux con CRT e SDK Windows di cargo-xwin; nessun exe eseguito). **[unit]** test senza Windows: `rustc --edition 2024 --test src-tauri/src/shell_rules.rs` (6 test) e `node --test tests/session-end.mjs` (4 test). **[static]** `node tests/native-feel.mjs` (8 controlli). **[browser]** test Playwright sull'anteprima. **[Win]** resta da osservare con l'exe di release su Windows: niente di questa colonna è stato visto funzionare.

| # | Stato | Che cosa è cambiato | Verificato qui | Da osservare su Windows |
|---|---|---|---|---|
| F1 | Risolto (commit 4f5349e) | `set_parent(&window)` su tutti i dialoghi `rfd` | — | prove F1 |
| F2 | Risolto, da osservare | `src-tauri/src/window_shell.rs`: subclass di main **e** della finestra nascosta "Tao Thread Event Target", che riceve anch'essa `WM_ENDSESSION` ed esce con `process::exit(0)`. `WM_QUERYENDSESSION` → `ShutdownBlockReasonCreate` + `session-ending {last:false}` (salva bozze, Impostazioni, appunti; niente di irreversibile, perché un'altra app può annullare). `WM_ENDSESSION(TRUE)` → `{last:true}` (ferma e salva la registrazione, annulla i job), poi pompa i messaggi finché la pagina chiama `session_end_ready` o per al massimo 8 s, quindi passa a tao. `WM_ENDSESSION(FALSE)` toglie il motivo di blocco. JS: `sessionEnd` in `registerCloseGuard` (`close-guards.ts`, `window-close.ts`), usato da `ComposerDock.tsx` (flush di tutte le bozze), `App.tsx` (Impostazioni salvate senza domanda; meeting) | [msvc] [unit] [static] | `shutdown /l` con bozza digitata da < 1 s, Impostazioni modificate e registrazione attiva: al rientro bozza e Impostazioni salvate, registrazione chiusa e non "interrotta". Durante l'attesa compare "Qamille: Salvataggio di bozze, impostazioni e registrazione…". Annullare l'arresto (un'altra app con dati non salvati) lascia la registrazione attiva |
| F3 | Risolto | Comando `contacts_save_vcf` (`commands/contacts.rs`): "Salva con nome" `rfd` con owner, filtro vCard, risultato `empty`/`cancelled`/`saved`. `Contacts.tsx` lo usa sul desktop; l'anteprima browser resta sul Blob | [msvc] [browser] `contacts.mjs`, `compatto-contacts.mjs` | Esporta .vcf: "Salva con nome" sopra main, nessun riquadro download, la cartella Download non cambia, Annulla silenzioso, il toast indica numero e percorso |
| F4 | Risolto, da osservare | Alt+Spazio: `AcceleratorKeyPressed` sul controller di main (`SYSTEM_KEY_DOWN` + `VK_SPACE` → `Handled`). Tasto destro su un elemento con `data-tauri-drag-region`: `installWindowShell()` in `src/lib/window-shell.ts` → comando `show_system_menu(x, y)`. Il menu (`TrackPopupMenu` su `GetSystemMenu`, voci abilitate come in Windows) si apre dalla window procedure con un `WM_APP` postato, mai dentro un callback WebView2/IPC; la voce scelta va come `WM_SYSCOMMAND`, quindi "Chiudi" passa dai guard. Il filtro `ContextMenuRequested` non è cambiato | [msvc] [unit] [static] | Alt+Spazio con il focus in un campo; "Sposta" + frecce; "Chiudi" con Impostazioni modificate apre "Vuoi salvare le modifiche?"; tasto destro su "Posta / …" e sull'intestazione della sidebar; tasto destro in un campo mostra ancora Taglia/Copia/Incolla; il menu da tastiera risponde alle frecce |
| F5 | Non in ambito | Decisione di prodotto (layout sotto i 1000 px) | — | — |
| F6 | Risolto, da osservare | `"shadow": true` in `tauri.conf.json`. Il colore del bordo DWM non è forzato: Windows 11 usa il bordo di sistema (o il colore d'accento, se l'utente lo ha scelto) e il bordo scuro segue `set_theme` (vedi F12) | [static] | Windows 11: angoli arrotondati e ombra in chiaro e in scuro, ridimensionamento da fuori il bordo, la scrollbar del lettore al bordo destro non ridimensiona, Chiudi risponde fino all'angolo. Windows 10: bordo di 1 px (bianco in tema chiaro: annotare se stona in scuro) |
| F7 | Implementato, effetto da osservare | Finestra figlia `QAMILLE_SNAP_BUTTON` sopra il pulsante Ingrandisci, posizionata dai rettangoli che `WindowControls.tsx` riporta (`window_snap_button`, in pixel fisici, aggiornati su resize/zoom), sotto la striscia di ridimensionamento di Tauri. Risponde `HTMAXBUTTON`; gestisce da sé pressione e rilascio (ingrandisci/ripristina con `WM_SYSCOMMAND`, mai il pulsante classico di `DefWindowProc`), tasto destro = menu di sistema; la pagina disegna hover e pressione da `window-maximize-button`. Tastiera e Invio restano alla pagina. **Incertezza:** non ho trovato documentazione che garantisca il riquadro dei layout per `HTMAXBUTTON` restituito da una finestra figlia; il clic invece non dipende da questo | [msvc] [unit] [static] | Mezzo secondo su Ingrandisci: compaiono i layout? Clic singolo ingrandisce, nuovo clic ripristina, doppio clic non lascia la finestra in uno stato strano; hover e pressione visibili; il bordo superiore sopra il pulsante ridimensiona ancora; il tooltip del pulsante non compare più al passaggio (atteso). Se i layout non compaiono, il clic deve comunque funzionare: annotarlo |
| F8 | Risolto, da osservare | `print_mail` → `window_shell::print_with_system_dialog()`: `ICoreWebView2_16::ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM)`; se l'interfaccia manca (runtime vecchio) ricade su `window.print()` | [msvc] [static] | "Stampa / Salva PDF": dialogo Stampa di Windows sopra main, nessuna anteprima Edge; "Microsoft Print to PDF" produce le pagine di `print.css` |
| F9 | Risolto, da osservare | Ctrl+= / + / Ctrl+- / Ctrl+0 (`zoomKey()` in `shortcut-guard.ts`, gestiti in `keyboard()` prima di ogni altro controllo, anche nei campi) → comando `window_zoom` (`WebviewWindow::set_zoom`), passi 50–200 %, salvato in `window-preferences.json` e applicato prima di mostrare la finestra. Zoom effettivo = zoom dell'app × "Dimensione del testo" di Windows (`HKCU\Software\Microsoft\Accessibility\TextScaleFactor`), limitato a 50–200 %, riletto quando la finestra riprende il focus. Ctrl+rotella non aggiunto | [msvc] [unit] [static] | Ctrl+=/-/0 con il focus in lista e in un campo; zoom conservato al riavvio. "Dimensione del testo" 150 %: l'app parte al 150 % (se risulta 225 %, WebView2 applica già la scala da sé e il prodotto va tolto da `effective_zoom`). A 1000×640 e 150 % le azioni dei dialoghi restano raggiungibili (vedi F17, non risolto) |
| F10 | Risolto per gli stati elencati | `src/ds/styles/forced-colors.css` (importato per ultimo da `main.tsx`), solo colori di sistema: anello di focus `Highlight`; riga selezionata, voce attiva di sidebar e Impostazioni, scheda, elemento scelto di Select/DatePicker con contorno `Highlight`; elemento evidenziato delle liste tratteggiato; chip, tag e conteggi con bordo; bordo priorità (era un'ombra); Switch con bordo, riempimento `Highlight` e pollice `ButtonText`/`HighlightText`; Checkbox selezionata; bordo per dialoghi, popover, tooltip e toast; disattivati in `GrayText` | [browser] nuovo `tests/forced-colors.mjs` (12 controlli, scuro e chiaro; senza il foglio la riga selezionata non ha alcun segno) | Temi "Deserto" e "Acquatico": stesse verifiche più menu di sistema leggibile |
| F11 | Risolto, da osservare | `window_shell::make_tool_window()` dopo `build()` in `reminders.rs`: `WS_EX_TOOLWINDOW` senza `WS_EX_APPWINDOW`. **Owner non impostato**: con un owner il chip sparirebbe con main ridotta a icona, contro ADR-0004. `Reminder.tsx`: Esc = chiudi (come la ×, "più tardi"), annuncio `role="alert"` alla comparsa di ogni nuovo chip. Annuncio anche in main non fatto | [msvc] [static] [browser] `compatto-notices.mjs` | Alt+Tab e Win+Tab: una sola voce Qamille; mentre si scrive nel Blocco note il chip compare e il cursore resta lì; Esc dopo un clic sul chip lo chiude; Assistente vocale legge "Riunione … tra N min". La sezione Confirmation di ADR-0004 va aggiornata con queste prove |
| F12 | Risolto, da osservare | Comando `window_appearance` dall'effetto del tema (`App.tsx`): `set_theme` della finestra (barra e, tramite `ThemeChanged` di tauri-runtime-wry, `PreferredColorScheme` di WebView2) e salvataggio in `window-preferences.json`. All'avvio `show_main_window` applica lo stesso aspetto prima di mostrare | [msvc] [unit] | Windows chiaro con Aspetto = Scuro (e viceversa): nessun lampo all'avvio; tasto destro in un campo: menu scuro; tornare a "Sistema" segue di nuovo Windows |
| F13 | Risolto | Fondo della finestra = `--ground` (`tauri.conf.json` `#f6f7f9`, `DARK_GROUND` `#12161e`, `paintWindowBackground()`), test 4 di `native-feel.mjs` aggiornato | [static] | Ridimensionamento veloce dal bordo destro in scuro: nessuna striscia più chiara |
| F14 | Risolto (2026-11-16) | `MessageHtml.tsx`: il `keydown` del documento del messaggio passa una copia all'elemento iframe, che risale la pagina fino a `keyboard()` di `App.tsx` con i suoi controlli sul bersaglio; frecce, Pag su/giù, Home, Fine, Spazio, Tab, Ctrl+A e Ctrl+C restano al messaggio e lo scorrono | [browser] `mail-html.mjs` (freccia giù non cambia messaggio, Ctrl+K va alla ricerca, E archivia) | Ctrl+= / Ctrl+- con il focus nel corpo HTML: lo zoom dell'app risponde |
| F15 | Non fatto (scelta di design: migrare i menu a Ark `Menu`) | — | — | — |
| F16 | Parziale | `Modal.tsx`: se l'opener non è più nel DOM, il focus torna al trigger (`aria-controls`) del popup che lo conteneva; altrimenti resta come prima | [browser] `primitives-overlays.mjs`, `a11y-keyboard.mjs` | Caso per caso con la tastiera: anteprima di stampa ed eliminazione contatto aperte da un menu |
| F17 | Risolto (2026-11-16) | `ds/styles/ark/dialog.css`: l'ultima riga di azioni di ogni `Modal` resta attaccata al fondo mentre il resto scorre; una linea sottile, guidata dallo scorrimento, compare solo se c'è altro contenuto sotto | [browser] a 500×320 px CSS (1000×640 al 200 %) Salva di "Nuovo contatto" visibile prima e dopo lo scorrimento; a 1440×900 posizione dei pulsanti invariata | A 1000×640 con zoom 200 %: "Unisci contatti" e "Accedi con Google" |
| F18 | Non fatto (politica della coda dei toast: scelta di prodotto) | — | — | — |
| F19 | Parziale | La conferma "Registrazione in corso" ha già owner e titolo localizzato (`meetings.rs`); pulsanti `OkCancelCustom`, titoli e filtri di importa/esporta riunione non fatti | — | — |
| F20 | Risolto | `connect_google`: a fine accesso, se Qamille non ha il focus, `request_user_attention(Informational)` | [msvc] | Il pulsante sulla barra lampeggia una volta, il focus resta al browser |
| F21 | Non fatto (scelta di prodotto) | — | — | — |
| F22 | Risolto | (a) `capabilities/main.json` senza `core:default`: solo `event:allow-listen/unlisten`, `window:allow-is-maximized`, `allow-internal-toggle-maximize` (doppio clic della drag region) più quelli che c'erano; zoom, tema e menu di sistema sono comandi dell'app. Elenco ricavato dagli import di `@tauri-apps/api` (core, event, window) e dalle chiamate di `drag.js` 2.12.5. (b) `background-notice` va solo a main (`emit_to("main", …)`, 14 punti in `main.rs`); `app-event` resta a tutte le finestre | [msvc] (le capability sono validate in build) [static] | Tutti i `*-native.mjs` senza errori ACL in console; doppio clic sulla barra ingrandisce ancora |
| F23–F25 | Non fatti | F23 cambia l'ordine di Tab e le zone di trascinamento (design); F24 va prima osservato; F25 è una funzione nuova | — | — |

## Seconda applicazione della skill (2026-11-14)

Skill aggiornata lo stesso giorno: ora ha i riferimenti (`references/guide.md`, `frame.md`, `web-layer.md`, `recovery.md`, `keyboard-accessibility.md`, `acceptance.md`) che al primo audit mancavano. Versioni dal lockfile: tauri 2.12.5, tauri-runtime-wry 2.12.3, wry 0.57.1, webview2-com 0.39.1, rfd 0.15.4; tutte le API usate sono sotto la matrice delle versioni minime della skill. Questa volta su Windows 11 (26200), con l'exe di release compilato e osservato.

Legenda: **[static]** `node tests/native-feel.mjs` (11 controlli). **[unit]** `rustc --edition 2024 --test src-tauri/src/shell_rules.rs` (10 test). **[native]** `node tests/webview-recovery-native.mjs` sull'exe compilato con `--features custom-protocol` (7 controlli, osservati). **[regr]** `release-theme-probe.mjs`, `a11y-native.mjs`, `glance-scale.mjs` sullo stesso exe, verdi.

| # | Lacuna rispetto alla skill aggiornata | Che cosa è cambiato | Verifica |
|---|---|---|---|
| N1 | **Prioritaria.** Un crash del processo della pagina (o del processo browser di WebView2) lasciava la finestra morta o la pagina d'errore del motore, senza via d'uscita: F5 e Ctrl+R sono spenti (regola 8) e wry 0.57 non registra `ProcessFailed` | `recovery.rs`: ricarica automatica con limite (3 in 2 minuti), poi finestra di messaggio con owner main; "non risponde" chiede; morte del processo browser → riavvio (il nuovo processo aspetta il vecchio: Tauri lo avvia prima che il vecchio esca, e `ui.lock` è preso con `try_lock`; il plugin single-instance 2.5.5 rilascia invece il suo blocco da solo in `cleanup_before_exit`); pagina dell'app non caricata → messaggio con il motivo (build di sviluppo) e Riprova. Pagina d'errore di WebView2 spenta. "Ricarica la finestra" nel menu di sistema, con salvataggio prima (`reload-requested` → `reload_ready`). Regola 21 | [static] [unit] [native] |
| N2 | Navigazioni verso altri indirizzi permesse in entrambe le finestre | `on_navigation(native_feel::navigation_guard)`: solo l'origine dell'app; link web e mailto al browser di sistema dalla principale. Regola 23 | [static] [unit] [native] (controllo 3) |
| N3 | Cursore a mano su pulsanti, schede, interruttori, voci di elenco (16 regole CSS e 2 classi Tailwind), contro la regola Microsoft che la skill riporta | Freccia ovunque nell'interfaccia; la mano resta sui link nei messaggi. Regola 3 cambiata | [static] |
| N4 | Ritirata: non era una lacuna. `IsPasswordAutosaveEnabled` è `FALSE` per default in WebView2 (documentazione Microsoft) e wry 0.57.1 non lo tocca; la riga aggiunta è stata tolta | Nessun codice. Regola 10 | — |
| N5 | Immagini e link dell'interfaccia trascinabili con la fantasma del browser | `img, a { -webkit-user-drag: none }`. Regola 22 | [static]; da osservare a mano |
| N6 | Tasti Indietro/Avanti del mouse, clic centrale e Ctrl+clic sui link: comportamenti da browser non coperti dagli acceleratori | `installBrowserGuards()` in entrambe le finestre. Regola 23 | [static]; da osservare con un mouse a 5 tasti |
| N7 | Invio della ricerca e del campo data senza guardia IME: con un IME giapponese o cinese la conferma del candidato avviava la ricerca | `imeComposing()` (`isComposing` o `keyCode` 229) nei tre gestori di Invio, controllato dal test 11. Regola 24 | [static]; da osservare con un IME |
| N8 | All'avvio scuro la WebView nasceva con il `--ground` chiaro e passava al scuro solo dopo | `background_color` del config = `DARK_GROUND` quando `starts_dark()`: wry lo passa a WebView2 alla creazione (`ICoreWebView2ControllerOptions3`). Non uso `WEBVIEW2_DEFAULT_BACKGROUND_COLOR`: vale per ogni WebView del processo e prevale sull'opzione di creazione, quindi anche il promemoria trasparente nascerebbe con un primo fotogramma opaco (dedotto dalla documentazione Microsoft e dal sorgente di wry, non osservato). Regola 7 | [static] [regr] |

**Non fatto, scelte di prodotto o di design (in ordine di utilità):** F5 larghezza minima sotto i 500 epx per gli snap; per un client di posta, icona overlay dei non letti e `request_user_attention` all'arrivo di posta con l'app in secondo piano, barra di progresso nella taskbar per le sincronizzazioni lunghe; `scroll_bar_style(FluentOverlay)` (scrollbar di Windows 11, stessa scelta per tutte le WebView della cartella dati); F24 rimbalzo da decidere per superficie, dopo averlo osservato con un touchpad di precisione; registro unico dei comandi (scorciatoie, menu e elenco da una sola fonte); F14, F15, F17, F18, F23, F25 come sopra. Il colore d'accento di sistema non serve: l'app non lo segue.

**Test non aggiornato dopo il restyle, sistemato:** `tests/thread-rows-native.mjs` falliva su "every reply sits on the same edge ()" (anche con l'exe di stamattina): misurava `.reply-avatar`, che il restyle Compatto (059bd4a) ha tolto. Ora misura il nome del mittente contro quello della riga, come `compatto-list.mjs`; 12 su 12 sull'exe installato. `tests/window-native.mjs` usa l'exe di debug e il server di sviluppo: non eseguito.

## Terza applicazione della skill (2026-11-18)

Skill `tauri-native-desktop` 1.2.0 (plugin `tauri-app-builder`), che rispetto alla seconda passata aggiunge `context-menus.md`, `dialogs.md`, `boundaries.md`, `os-integration.md`, `notifications.md`, `packaging-updates.md` e `native-plugins.md`, e chiede: menu contestuale per lista di voci ammesse o sostituito; impostazioni WebView2 valide solo dalla navigazione successiva; handler installati prima di spegnere la pagina d'errore; "salva prima di ricaricare" senza mai scartare per un timeout; supervisione nativa quando la chiusura la decide JS; avvio nascosto con un limite; eventi in broadcast senza dati sensibili. Stesse versioni del lockfile (tauri 2.12.5, wry 0.57.1, webview2-com 0.39.1, rfd 0.15.4), Windows 11 26200, sul ramo `develop`.

Legenda: **[native]** test sull'exe di release con `--features custom-protocol`. **[browser]** test Playwright dell'anteprima (`npm run test:browser -- …`); dove indicato, il test nuovo fallisce senza la correzione e passa con. **[unit]** `cargo test` del crate desktop (20) e `node --test`. **[static]** `node tests/native-feel.mjs` (11 controlli). **Da osservare**: niente lo ha visto funzionare.

| # | Lacuna | Correzione (commit) | Verifica |
|---|---|---|---|
| T1 | "Ricarica la finestra" ricaricava dopo 3 s anche senza salvataggio, e anche se il salvataggio era fallito | La pagina risponde `saved`; salvataggio fallito → "Non ricaricare / Ricarica comunque"; nessuna risposta entro 10 s → "Attendi / Ricarica adesso" (era 3 s: appena avviata la pagina ha impiegato fino a 4 s a salvare, osservato). Le finestre di messaggio di `recovery.rs` sono task dialog con verbi (rfd `common-controls-v6`); Esc e la X scelgono l'opzione innocua (58936ed) | [native] `webview-recovery-native.mjs` 7/7 in 3 esecuzioni di fila sull'exe finale; [unit] `session-end.mjs`. Limite noto: se la pagina risponde dopo i 10 s, il dialogo resta aperto sopra la pagina già ricaricata e i suoi pulsanti non fanno più niente |
| T2 | Con un guard bloccato o il renderer fermo la X e Alt+F4 non facevano niente, per sempre | `close_seen` / `close_settled`; senza risposta entro 3 s, o alla seconda richiesta dopo 10 s, "Attendi / Chiudi Qamille" con owner main (a06b02f, 7495ed2) | [native] nuovo `close-supervision-native.mjs` 3/3 sull'exe finale |
| T3 | E, N, R, frecce, Ctrl+K e Ctrl+Z agivano sul messaggio dietro un `Modal` o da dentro un popover | `shortcutCoveredByLayer` in `keyboard()` (f1b3505) | [browser] `shortcut-guard.mjs` |
| T4 | **Ritirato: falso positivo.** `browser_url` (`app/src/application/server_account.rs`) accetta già solo `https://` (o `http://` verso questo computer) e `checkout_status` valida l'id, con test | — | — |
| T5 | F5, Ctrl+P e la pagina d'errore potevano funzionare sul primo documento: `harden_webview` gira dopo il `Navigate` di `build()` | `harden_webview(&window, pagina)` ricarica la pagina dell'app dopo le impostazioni (53c007c) | [native] `release-theme-probe.mjs` 9/9 (URL `http://tauri.localhost/`), `a11y-native.mjs` verde, tutti i test native avviano bene; F5/Ctrl+P nel primo secondo: da osservare |
| T6 | Probabile doppia scala con "Dimensione del testo" (zoom app × `TextScaleFactor`, mentre la `RasterizationScale` di WebView2 la include già) | Non cambiato: serve l'osservazione | Da osservare con Dimensione del testo 150 %: se l'app parte al 225 % (o al limite del 200 % con zoom 100 %), togliere il fattore da `effective_zoom` |
| T7 | Menu contestuale a lista di esclusione: passavano "Copia link a evidenziazione" (indirizzo `tauri.localhost`), "Copia indirizzo immagine" e qualsiasi voce nuova del runtime | Lista di voci ammesse `MENU_ITEMS_KEPT` (2407d72) | [unit] `cargo test native_feel` 5/5; da osservare su vuoto, selezione, link, immagine, campo, Maiusc+F10 |
| T8 | Una pagina che si carica ma non disegna lasciava la finestra vuota senza messaggio | `frontend_ready` da `main.tsx`; senza entro 15 s, "Ricarica la finestra / Chiudi Qamille" (44b19b5) | Da osservare con un bundle rotto |
| T9 | La pagina d'errore si spegneva prima che esistessero gli handler di recupero; errori solo su stderr, invisibili in release | Prima gli handler, poi gli interruttori; `native_feel::report` scrive anche `<dati>/native.log` (57ff5e6) | [static] |
| T10 | Il promemoria che non si carica veniva ricostruito ogni 15 s senza fine; bloccato restava in primo piano | Limite di 3 in 2 minuti anche per il chip; chip bloccato chiuso (444d38a). Non fatto: `no_redirection_bitmap` (lampo non osservato: prima una cattura a 60 fps) e lo stile tool window prima di mostrare (un `show()` dopo la costruzione potrebbe prendere il focus, INV-004) | [static] |
| T11 | Mano sui link delle guide (Chromium la dà a ogni `a[href]`), divieto sullo Switch disattivato | `button, a { cursor: default }`; il test 2 aveva due byte backspace al posto di `\b` e non vedeva le classi Tailwind (62e7cfb) | [static] |
| T12 | Configurazione MCP, scope OAuth, percorsi, errori, URL e utente del server non selezionabili | `pre, code, [role='alert'], .compute-account strong, [data-subscription]` nella regola 2 (62e7cfb) | [static] |
| T13 | Temi Contrasto: nessun anello di focus su ricerca, campo assistente e titolo del calendario | Anello `Highlight` sui contenitori (60c92a7) | [browser] `forced-colors.mjs`, fallisce senza |
| T14 | I toast lanciati da un `Modal` non venivano annunciati: Ark nasconde il resto della pagina con `aria-hidden` (non `inert`, come scritto nell'analisi); errore di eliminazione account sotto il velo | Toaster in un Portal su `<body>` (regione live, che Ark lascia stare); errore dentro il dialogo (22fc4e8) | [browser] nuovo `toaster-layer.mjs`, fallisce senza |
| T15 | Focus su `<body>` quando un passo sostituisce il pulsante; focus iniziale sull'azione distruttiva | `useFocusAfterStep` (collegamento al server, abbonamento); `Modal` con `initialFocus`, "Libera spazio" ed "Elimina account" partono da Annulla (cde80e6, 22fc4e8) | [browser] `server-connect.mjs`, fallisce senza |
| T16 | Zoom 200 % alla finestra minima: il layout non regge | Non fatto: dipende dalla decisione su F5 (larghezza minima) | — |
| T17 | Le guardie del browser non arrivavano nell'iframe del messaggio (Indietro del mouse, clic centrale, file trascinato); un link trascinato da un'altra app non era bloccato | `installBrowserGuards` copre anche i drop e qualsiasi realm; `MessageHtml` la installa nella finestra dell'iframe (116c603) | [static] test 11; da osservare con un mouse a 5 tasti |
| T18 | `app-event` (oggetti delle email, avvisi) andava anche al promemoria | `emit_to("main", …)`; il test 8 rifiuta un broadcast (b9311f4) | [static] |
| T19 | Nessun lampeggio della barra dopo l'accesso al server o l'eliminazione account dal browser | `back_from_browser` condiviso con l'accesso Google (b9311f4) | Da osservare |
| T20 | Dialoghi con OK/Annulla e filtri non localizzati | Riunione: "Termina e chiudi / Continua a registrare"; titoli e filtri di importa/esporta riunione ed esporta email (b9311f4) | Da osservare |
| T21 | Un avvio fallito (cartella già aperta, anche dopo un riavvio lento) usciva senza messaggio | Finestra di messaggio con il motivo (b9311f4). Non fatto: `legacy_local.startup()` su un thread (creerebbe una corsa con l'avviso; il ritardo capita solo con un vecchio server vivo che non risponde) | Da osservare |
| T22 | Ctrl+Ins (copia) col tastierino senza BlocNum azzerava lo zoom | `zoomKey` non guarda più `code === 'Numpad0'` (f1b3505) | [unit] `session-end.mjs` |
| P2 | Esc che annulla un candidato IME chiudeva composer, ricerca o pannelli; linea sopra le azioni dei `Modal` spenta da "riduci animazioni"; rubrica delle etichette che trascina la sidebar; testo del lettore che si sposta all'arrivo della scrollbar; destinatario e data non validi solo a colore; avviso della vecchia modalità locale sopra la barra del titolo | 7157ba9 (anche: `lib/dismiss.ts` inutilizzato tolto), 5a4d185, c8c9b94 | [static] test 11 esteso a Esc e ai `.ts`; [browser] nuovo `modal-actions-edge.mjs`, fallisce senza |

**Restano decisioni di prodotto:** larghezza minima sotto i 1000 px (F5, con T16); notifiche di sistema per l'edizione Professional, ora installata con NSIS (il plugin ufficiale funziona solo per app installate, clic gestito solo con il toast a schermo); firma Authenticode e aggiornamento automatico (assenti); credenziali nella cassaforte portatile invece che in Gestione credenziali di Windows (scelta dichiarata in `secret.rs`); menu con semantica `role=menu` (F15); `app-region: drag` per tocco e penna; icona overlay dei non letti e barra di avanzamento nella taskbar.

## 0. Metodo, fonti e limiti

| Livello | Che cosa |
|---|---|
| **Codice ispezionato** | `AGENTS.md`, `docs/aspetto-nativo.md`, ADR-0004, ADR-0008 v2, tutto `desktop/src-tauri/` (Cargo.toml/lock, tauri.conf.json, capabilities, permissions generate, build.rs, main.rs, native_feel.rs, reminders.rs, runtime.rs, commands/*.rs), le superfici React richieste (App.tsx, WindowControls, PrintPreview, Settings, GoogleSignInDialog, UnsavedDialog, ComposerDock/Composer, Contacts, MeetingMenu, Toaster, Modal/SidePanel/Popover, MessageHtml, reminder/Reminder.tsx, lib/window-close.ts, close-guards.ts, native-window.ts, shortcut-guard.ts). Ho anche letto i sorgenti delle crate alle **versioni esatte del lockfile**, scaricati da crates.io in una cartella di lavoro: tauri 2.12.5, tauri-runtime-wry 2.12.3, wry 0.57.1, tao 0.37.2, rfd 0.15.4, tauri-plugin-single-instance 2.5.5. |
| **Compilato** | Niente. Non ho eseguito `cargo` per non scrivere in `src-tauri/target` e per non far rigenerare `permissions/main-window.toml` da `build.rs` mentre un altro agente lavora. |
| **Test automatici eseguiti** (Linux, Node 22.23.4) | `node tests/native-feel.mjs`: 7/7 ok, due volte. Tra le due esecuzioni l'altro agente ha cambiato `DARK_SURFACE` e `theme.css` (14:41 UTC): il test è rimasto verde. `node --test tests/close-guards.mjs tests/shortcut-guard.mjs`: 3 ok, 1 saltato (serve `QAMILLE_PREVIEW_URL`). Non ho eseguito i test browser né i `*-native`: i secondi richiedono l'exe Windows. |
| **Osservato nell'app reale** | Niente. Questo container è Linux, senza exe e senza WebView2. Ogni comportamento "su Windows" qui sotto è dedotto dal codice o dalla documentazione e va confermato con l'exe di release. |

**Fonti.** `references/guide.md` e `references/acceptance.md`, citati dalla skill, **non esistono** nel repository: la cartella della skill contiene solo SKILL.md. Le conclusioni contrassegnate **[R]** si basano quindi sulla mia ricerca, cioè sui sorgenti delle crate alle versioni bloccate e sulla documentazione primaria Microsoft e Tauri, non su quelle guide. Lo stesso vale per tutte le prove di accettazione: le ho definite io.
- WebView2 `AreBrowserAcceleratorKeysEnabled`: https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2settings.arebrowseracceleratorkeysenabled
- WebView2 `ICoreWebView2Controller3` (RasterizationScale e scala del testo): https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller3
- Snap layouts per app con barra personalizzata: https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-snap-layout-menu
- Alt+Space che non funziona con il focus nella WebView2: https://github.com/MicrosoftEdge/WebView2Feedback/issues/1703 (e https://weblog.west-wind.com/posts/2021/Apr/15/WebView2-Forwarding-Alt-Keys-to-a-Host-WPF-Window)
- Doc comment di Tauri sui metodi del builder (`shadow`, `owner`, `zoom_hotkeys_enabled`), letti nel sorgente 2.12.5.

Le righe citate si riferiscono ai file letti tra le 14:20 e le 14:47 UTC del 2026-11-12. `desktop/src` e `native_feel.rs` sono in modifica: le righe potrebbero spostarsi.

---

## 1. Contesto rilevato

- **Piattaforma distribuita:** solo Windows. L'app è portatile: `release/Qamille.exe`, `bundle.active: false` (`tauri.conf.json:24`), nessun installer e quindi nessun AppUserModelID registrato. Non c'è un percorso macOS o Linux distribuito. Il frontend gira anche nel browser (anteprima con `lib/preview.ts`), che resta il percorso dei test browser.
- **Versioni (Cargo.lock / package.json):** tauri 2.12.5, tauri-build 2.7.4, tauri-runtime-wry 2.12.3, wry 0.57.1, tao 0.37.2, webview2-com 0.39.1 (diretta, per `native_feel.rs`), windows-core 0.62, tauri-plugin-single-instance 2.5.5, rfd 0.15.4 (dialoghi), open 5.4.5. JS: @tauri-apps/api 2.12.4 (installata), @tauri-apps/cli ^2.12.4, React 19.3.1, @ark-ui/react 5.39.3. **Nessun** tauri-plugin-dialog, -notification, -window-state, -opener o -shell. La versione del runtime WebView2 è quella installata sul PC dell'utente (Evergreen) e non è nota da qui.
- **Finestre:**
  - `main`: creata in `setup` con `create = false` e `visible = false` (`main.rs:56-60, 74-80`). 1440×900, minimo 1000×640, `decorations: false`, `shadow: false`, `backgroundColor #ffffff`, `dragDropEnabled: false` (`tauri.conf.json:13`).
  - `meeting-reminder` (chip): `reminders.rs:152-168`. Senza bordi, trasparente, `always_on_top`, `skip_taskbar`, `focused(false)`, non ridimensionabile. In alto al centro dell'area di lavoro del monitor **primario**, senza owner.
- **Barra del titolo:** propria, disegnata nella pagina.
  - Trascinamento: `data-tauri-drag-region` su `header.topbar` e sui suoi span (`App.tsx:1492-1505`), su `sidebar-heading` e `brand` (`Sidebar.tsx:81-82`) e sulla barra d'avvio (`App.tsx:1458-1459`).
  - Pulsanti: `WindowControls.tsx` (riduci, ingrandisci/ripristina, chiudi) con larghezza 46 px come Windows.
  - **[R]** Lo script `drag.js` di Tauri 2.12.5 trascina con `mousedown` e ingrandisce con il doppio clic, solo su un elemento con l'attributo nudo come bersaglio diretto. tao trascina con `WM_NCLBUTTONDOWN(HTCAPTION)`, cioè il ciclo di spostamento di sistema, quindi Aero Snap e Shake funzionano.
  - **[R]** La finestra senza decorazioni di tao mantiene `WS_CAPTION | WS_SYSMENU | WS_SIZEBOX | WS_MAXIMIZEBOX | WS_MINIMIZEBOX` e toglie l'area non client con `WM_NCCALCSIZE` (`tao window_state.rs:239-298`, `event_loop.rs:2074`). Win+frecce e ingrandimento senza coprire la barra delle applicazioni funzionano.
  - **[R]** Bordi di ridimensionamento: con `shadow: false` tauri-runtime-wry crea una finestra figlia `TAURI_DRAG_RESIZE_BORDERS` che copre una cornice di `SM_CXFRAME`/`SM_CYFRAME` px **dentro** l'area client, su tutti e quattro i lati. La toglie quando la finestra è ingrandita (`undecorated_resizing.rs:208-245, 418-446`).
- **ACL e capability:**
  - `main` (`capabilities/main.json`): `core:default` più destroy, close, minimize, toggle-maximize, start-dragging, set-background-color, più il set generato `main-window-commands`. Il set contiene tutti i comandi tranne `reminder_*` e lo genera `build.rs` da `generate_handler!`.
  - `meeting-reminder` (`capabilities/reminder.json`): solo `core:event:allow-listen/unlisten` e i tre comandi `reminder_view`, `reminder_join` e `reminder_dismiss`.
  - Difesa in profondità: `open_link` ed `event_join_record` rifiutano le finestre diverse da `main` (`commands/workspace.rs:99-103`, `reminders.rs:251-253`).
- **Istanza singola:** plugin con callback `unminimize` e `set_focus` (`main.rs:62-67`). L'identificatore include l'hash della cartella dati: un'istanza per copia portatile (`main.rs:48-55`). **[R]** Su Windows `set_focus` di tao usa `SetForegroundWindow`, con il trucco del tasto Alt simulato se non basta (`tao window.rs:139-150, 1514-1539`).
- **Chiusura:** un solo `onCloseRequested` (`lib/window-close.ts:15-25`) annulla l'evento ed esegue i guard in ordine. `composers` (flush delle bozze, `ComposerDock.tsx:138-172`) e `settings` (apre `UnsavedDialog`, `App.tsx:1379-1385`) possono porre il veto. `meetings` è registrato `last` (`App.tsx:1351-1374`): conferma OS, flush degli appunti, stop della registrazione e cancellazione dei job. Se nessuno pone il veto parte `destroy()`. Durante il flush lo shell è `inert` (`App.tsx:1475`). La distruzione di `main` chiude il chip (`main.rs:374-379`, `reminders.rs:36-41`).
- **Tema all'avvio:**
  - `public/theme-boot.js` imposta `data-theme` prima del CSS, per main e per il chip.
  - Rust colora la finestra nascosta con `DARK_SURFACE` se il **sistema** è scuro, poi la mostra (`native_feel.rs:17-22`).
  - Durante l'uso `paintWindowBackground()` segue `--surface` (`native-window.ts`, chiamata da `App.tsx:1280`).
  - `native_feel::harden_webview` disattiva gli acceleratori del browser e filtra il menu contestuale (`native_feel.rs:27-44, 48-66, 109-153`).

## 2. Inventario delle superfici e classificazione

Legenda: **OS** = controllo nativo del sistema, **TW** = finestra Tauri con contenuto web, **INT** = superficie interna alla WebView principale.

| Superficie | Implementazione attuale | Classe giusta | Motivazione | Fallback |
|---|---|---|---|---|
| Barra del titolo e pulsanti | HTML + `drag.js` + `WindowControls` | INT, con funzioni OS da ripristinare | Il look unlumen è una scelta di prodotto, ma la barra sostituisce menu di sistema, snap layouts e tasto destro (F4–F7) | Cornice OS (`decorations: true`) se la natività diventa prioritaria |
| Salva allegato, esporta email, allega file, importa ed esporta riunione | `rfd` (IFileDialog) **senza owner** | **OS, con owner = main** | Il file picker è un controllo OS. Manca solo la proprietà della finestra (F1) | Nessuno: va corretto |
| Importa .vcf | `<input type=file>` (`Contacts.tsx:371-379`) | OS (dialogo Chromium nella WebView) | **[R]** Il file chooser di WebView2 è il dialogo comune di Windows aperto dal processo host. Va bene così; è solo incoerente con `rfd` | Passarlo a `rfd` con owner per avere titolo e filtro uniformi |
| Esporta .vcf | `<a download>` su Blob (`Contacts.tsx:315-320`) | **OS (dialogo Salva)** | **[R]** Nessun gestore di download in wry/Tauri, quindi parte il download predefinito di WebView2, cioè l'interfaccia download del browser (F3) | Comando backend con `rfd` e owner, come `export_mail` |
| Conferma "Registrazione in corso" alla chiusura | `rfd::MessageDialog` senza owner, OK/Annulla (`meetings.rs:99-109`) | OS, con owner | Decisione bloccante sul ciclo di vita, accettabile come message box | `Modal` interno; ma il guard gira durante la chiusura e la message box OS è più robusta |
| Errore cartella dati all'avvio | `rfd::MessageDialog` (`main.rs:34-38`) | OS senza owner | La finestra non esiste ancora: corretto | — |
| Modifiche non salvate (Settings), Rinomina ed Elimina riunione, Accedi con Google, Unisci contatti, Elimina contatto | `Modal` (Ark Dialog modale, `ds/primitives/Modal.tsx`) | **INT** | Sono decisioni legate al contesto della principale, brevi e senza bisogno di stare fuori dal frame. ADR-0008 le vuole modali interne. Restare interne è la scelta conservativa | TW modale con owner solo se un giorno escono dal frame minimo (non è il caso a 1000×640) |
| Anteprima di stampa | `Modal` lg + `window.print()` | INT (anteprima) + **OS (stampa)** | L'anteprima è contenuto dell'app. La stampa dovrebbe usare il dialogo di sistema, non l'anteprima del browser (F8) | — |
| Settings | Vista della principale (`s.view === 'Settings'`) | INT, non modale | Pagina dell'app con stato sporco protetto dal guard. Non serve una finestra | TW non modale e riusabile solo se richiesto dal prodotto |
| Pannello allegati, dettaglio evento | `SidePanel` (Ark Dialog `modal={false}`) | INT | ADR-0008: si guarda o si modifica in contesto | — |
| Composer | `ComposerDock`, schede ancorate in basso a destra | INT | Stato condiviso con lista e bozze. Un pop-out sarebbe un costo di prodotto (secondo contesto JS, ACL) | Finestra Tauri non modale per bozza, solo su richiesta |
| Menu (riunione, ordinamento e altri) | `Popover` con `<button>` semplici | INT (**non** finestre) | Tooltip e menu a discesa restano interni, come vuole la skill. Manca solo la semantica da menu (F15) | — |
| Select, DatePicker, Tooltip | Ark, in Portal | INT | Come sopra | — |
| Notifiche (toast) | Ark Toast, `max: 1` | INT | Esito di azioni in app; la cronologia è nel centro attività | — |
| Promemoria riunione (chip) | TW `meeting-reminder` | **TW** (ADR-0004) | Deve comparire sopra le altre app e con la principale ridotta a icona. Il toast di Windows è stato scartato (RA-001: app portatile, nessun pulsante dal plugin) | — |
| Menu contestuale | Menu WebView2 filtrato (`BROWSER_MENU_ITEMS`) | OS (menu WebView2) | Copia/Incolla/ortografia nativi; niente voci del browser | — |
| Scorciatoie | `keyboard()` in App.tsx; acceleratori del browser spenti | INT | — | — |
| Tray, avvio automatico, scorciatoie globali, updater | Assenti | Scelte di prodotto | La skill non li impone. ADR-0004 accetta "niente chip con l'app chiusa" | — |

## 3. Contratto desktop (passo 3)

| Superficie | Owner | Modalità | Dimensioni | Focus | Chiusura | Risultato e stato autorevole |
|---|---|---|---|---|---|---|
| Dialoghi file e message box `rfd` | **deve essere** `main` (`set_parent`) | modale applicativa: owner disabilitato finché è aperto | gestite dall'OS | all'apertura sul dialogo; alla chiusura torna a main, sull'elemento che l'ha aperto | OK, Annulla, X, Esc, Alt+F4 chiudono **solo il dialogo** | Il path torna al comando Rust, che scrive il file. Annullare restituisce `None` e la UI non mostra errori |
| `Modal` interni | la WebView main | modale interna: focus intrappolato, `aria-modal`, il clic sul velo non chiude | `max-height: calc(100vh - 48px)`, scorre tutto il contenuto | ritorno all'opener se ancora nel DOM (`Modal.tsx:19-29`) | Esc → `onClose`, pulsanti espliciti | Stato in React; il guard decide la chiusura della finestra |
| Chip promemoria | nessuno (deve restare visibile con main ridotta a icona; un owner lo nasconderebbe: doc `owner` di Tauri, "An owned window is hidden when its owner is minimized") | non modale, sempre in primo piano | 600 × (52n + 8(n−1)) logici, in alto al centro del primario | non lo prende alla comparsa (INV-004). Lo prende se cliccato | ×, Entra, timeout del motore, distruzione di main | Lo stato autorevole è il motore Rust (`MeetingReminderService`). La pagina riceve `reminder-changed` più una lettura iniziale `reminder_view`: corretto, nessun secondo backend |
| Finestra main | — | — | min 1000×640 | — | X, Alt+F4, chiusura dalla barra delle applicazioni (WM_CLOSE): passano dai guard. **Fine sessione di Windows: no** (F2) | Bozze: backend (autosave dopo 1,2 s). Settings: stato React finché non si salva |

`parent`, `owner` e `transient_for` non sono sinonimi. **[R]** In Tauri 2.12.5 su Windows `.parent()` crea un owned window (`WS_POPUP` + owner), non una child. Per i dialoghi `rfd`, `set_parent` passa l'HWND come owner a `IFileDialog::Show`, `MessageBoxW` o `TaskDialogIndirect` (`rfd win_cid/file_dialog/dialog_ffi.rs:115-117`, `message_dialog.rs:137, 209`). Oggi riceve `null` perché non è mai chiamato.

---

## 4. Findings

Formato: **superficie → stato attuale → soluzione → prova di accettazione**. Dove la prova si può fare: [Linux] in questo container, [Win] solo con l'exe Windows.

### P0: difetti di comportamento

**F1. Dialoghi `rfd` senza owner**
- **Stato attuale.** `rfd::AsyncFileDialog::new()` e `FileDialog::new()` senza `set_parent` in `commands/document_preview.rs:35`, `drafts.rs:126`, `mail.rs:97`, `meetings.rs:43` e `:83`. `MessageDialog::new()` in `meetings.rs:102`. **[R]** Con parent `None`, rfd chiama `IFileDialog::Show(NULL)` e `MessageBoxW(NULL)`. Conseguenze:
  - la principale resta abilitata, quindi si possono avviare un secondo export o una seconda importazione;
  - un clic su main copre il dialogo;
  - durante la conferma di chiusura `closing = true` (`window-close.ts:18-19`): ogni altra X o Alt+F4 viene ignorata in silenzio e l'utente vede "la X non fa niente";
  - main può essere chiusa con un Salva aperto.
- **Soluzione.** Aggiungere `window: tauri::WebviewWindow` ai sei comandi (Tauri lo inietta, il JS non cambia) e chiamare `.set_parent(&window)` sul builder. **[R]** `WebviewWindow` implementa `HasWindowHandle + HasDisplayHandle` e raw-window-handle è 0.6.2 per entrambe le crate. Per `meeting_confirm_close`, usare anche `MessageButtons::OkCancelCustom("Termina e chiudi", "Continua a registrare")` (vedi F19). In alternativa, `tauri-plugin-dialog` imposta da sé il parent, ma aggiunge una dipendenza e una capability: non serve.
- **Prova di accettazione.** [Win] Con Salva allegato aperto, un clic sulla principale non la attiva e non porta niente in primo piano, il dialogo resta sopra. Alt+Tab mostra una sola voce Qamille. Annulla ridà il focus al pulsante che lo ha aperto. Con una registrazione attiva, la X apre la message box sopra main; un secondo clic su main non la nasconde; Annulla lascia la registrazione attiva e main aperta; OK chiude. [Win, automatizzabile] Estendere `tests/a11y-native.mjs` (UIA): mentre il dialogo è aperto, `IsEnabled` della finestra main deve essere `false`. [Linux] Solo la revisione del codice e, se presenti le dipendenze GTK e WebKit, `cargo check`.

**F2. Disconnessione, arresto o riavvio di Windows**
- **Stato attuale.** **[R]** tao 0.37.2 non gestisce `WM_QUERYENDSESSION` e, su `WM_ENDSESSION`, chiama `std::process::exit(0)` (`tao event_loop.rs:2338-2351`). `CloseRequested` non arriva e i guard non partono. Si perdono:
  - le modifiche non salvate di Settings;
  - gli appunti riunione non ancora salvati (`flushMeetingDrafts`);
  - fino a 1,2 s di testo del composer (autosave in `Composer.tsx:223`).
  
  La registrazione si interrompe senza `meeting_stop` e viene recuperata solo al riavvio (`recover_interrupted`). Anche Restart Manager, per esempio Windows Update, segue questa strada.
- **Soluzione.** Fare il subclass dell'HWND di main in `native_feel.rs` (stesso schema `SetWindowSubclass` usato da tauri-runtime-wry). Su `WM_QUERYENDSESSION`:
  1. `ShutdownBlockReasonCreate(hwnd, "Salvataggio di bozze e registrazione…")`;
  2. emettere `session-ending` a main;
  3. restituire `TRUE`.
  
  In JS, un handler esegue i flush **non interattivi** (bozze, appunti, `meeting_stop`) e chiama un comando `session_end_ready` che rilascia il blocco. Settings sporco va salvato in una bozza o perso esplicitamente: niente dialoghi durante lo shutdown. Va accettato che Windows conceda al massimo circa 5 s.
- **Prova di accettazione.** [Win] Bozza digitata da meno di 1 s, Settings modificato, registrazione attiva; poi `shutdown /l`. Al riavvio, la bozza contiene l'ultimo carattere e la registrazione è chiusa (non "interrotta"); per Settings vale la regola scelta. Durante l'attesa, Windows mostra "Qamille: Salvataggio…" nella schermata "Questa app impedisce l'arresto". [Linux] Test unitario del flush non interattivo in `close-guards`, con guard sintetici.

### P1: lacune rispetto alle convenzioni native

**F3. Esporta .vcf passa dall'interfaccia download di WebView2**
- **Stato attuale.** `Contacts.tsx:315-320` crea un `<a download>` su Blob. **[R]** wry registra `DownloadStarting` solo se Tauri ha un `on_download` (`wry webview2/mod.rs:871-946`); Qamille non lo imposta. Vale quindi il download predefinito di WebView2: file in Download, senza scegliere dove, con l'interfaccia download del browser.
- **Soluzione.** `contacts_export_vcf` salva con `rfd` e owner (come `export_mail`) e restituisce il path. La UI mostra "Esportati N contatti in …". Il percorso con il Blob resta per l'anteprima browser.
- **Prova.** [Win] Esporta .vcf apre "Salva con nome" sopra main e non compare nessun riquadro download. La cartella Download non cambia. Annulla non mostra errori. [Linux] Il test browser `contacts.mjs` continua a passare sul percorso di anteprima.

**F4. Menu di sistema: Alt+Space e tasto destro sulla barra**
- **Stato attuale.** `WS_SYSMENU` c'è, ma:
  - (a) **[R]** con il focus nella WebView2 Alt+Space non arriva all'host (WebView2Feedback #1703; Alt+F4 invece funziona);
  - (b) il tasto destro sulla zona di trascinamento non apre il menu di sistema: `drag.js` gestisce solo il tasto sinistro, e il filtro `ContextMenuRequested` toglie tutte le voci da un'etichetta e restituisce `Handled` (`native_feel.rs:147-150`), quindi non compare nessun menu.
  
  Senza il menu di sistema mancano Sposta e Ridimensiona da tastiera, una funzione di accessibilità di Windows.
- **Soluzione.** In `native_feel::harden` aggiungere `AcceleratorKeyPressed` per Alt+Space (`WM_SYSKEYDOWN`, `VK_SPACE`) → `Handled` + `TrackPopupMenu(GetSystemMenu(hwnd))` + `PostMessage(WM_SYSCOMMAND, cmd)`. Aggiungere un comando `show_system_menu(x, y)` chiamato da `contextmenu` sugli elementi `[data-tauri-drag-region]` (App.tsx, Sidebar.tsx). Il comando entra da solo nella capability di main tramite `build.rs`. La voce "Chiudi" del menu manda `SC_CLOSE`, cioè `WM_CLOSE`, quindi i guard partono.
- **Prova.** [Win] Alt+Space con il focus in un campo apre il menu Ripristina/Sposta/Ridimensiona/Riduci a icona/Ingrandisci/Chiudi. "Sposta" + frecce sposta la finestra. "Chiudi" con Settings sporco apre "Vuoi salvare le modifiche?". Il tasto destro sul titolo "Posta / …" apre lo stesso menu; il tasto destro su un campo mostra ancora Taglia/Copia/Incolla. [Linux] Un nuovo controllo statico in `native-feel.mjs` (harden registra `AcceleratorKeyPressed`) e `cargo test native_feel` per l'eventuale logica pura.

**F5. Snap e larghezza minima**
- **Stato attuale.** `minWidth: 1000` (`tauri.conf.json:13`). Microsoft raccomanda una larghezza minima di **al massimo 500 epx**, meglio ≤ 330, perché gli snap layouts funzionino. Esempi: su 1920×1080 al 100% mezza area vale 960 epx; al 150% vale 640 epx. In entrambi i casi Win+← o "affianca" non riesce a dare mezzo schermo e la finestra sborda dalla zona. **[R]** doc snap layouts.
- **Soluzione.** È un lavoro di layout: comprimere sotto i 1000 px, per esempio con sidebar compressa, lista sopra il lettore o lettore a tutta larghezza, e scendere almeno a `minWidth` 640, idealmente 500. Costo alto e da pianificare. Va deciso con il prodotto.
- **Prova.** [Linux] Test browser a 640, 800 e 960 px di larghezza, senza scorrimento orizzontale e con le azioni raggiungibili (stesso stile di `glance-scale.mjs`). [Win] Win+← su 1920×1080 al 100% e al 150%: la finestra occupa esattamente metà schermo.

**F6. `shadow: false`: niente ombra e angoli arrotondati, bordi di ridimensionamento dentro il contenuto**
- **Stato attuale.** `tauri.conf.json:13`. **[R]** Con `shadow: false` la finestra figlia di ridimensionamento ritaglia una cornice di circa 4 px (× DPI) **dentro** l'area client su tutti i lati (`undecorated_resizing.rs:418-446`). Gli ultimi pixel della scrollbar del lettore sul bordo destro, i primi 4 px della barra e il lato destro del pulsante Chiudi (finestra non ingrandita) diventano maniglie di ridimensionamento. Su Windows 11 mancano l'ombra DWM e gli angoli arrotondati: la finestra non si distingue dallo sfondo, soprattutto col tema scuro.
- **Soluzione.** `shadow: true`. **[R]** Doc Tauri: "true will make undecorated window have a 1px white border, and on Windows 11, it will have a rounded corners". In quel caso il ridimensionamento avviene nell'ombra DWM, fuori dalla finestra; dentro resta solo il bordo superiore. Su Windows 11, colorare il bordo con `DwmSetWindowAttribute(DWMWA_BORDER_COLOR)` sul `--border` del tema; su Windows 10 resta il bordo di 1 px. Aggiornare test 4 e 5 di `native-feel.mjs` e la tabella di `docs/aspetto-nativo.md`.
- **Prova.** [Win] Windows 11: angoli arrotondati e ombra in chiaro e in scuro. Il trascinamento della scrollbar del lettore al bordo destro non ridimensiona. Si ridimensiona da 1–8 px fuori dal bordo visibile. Il pulsante Chiudi risponde fino all'angolo. Windows 10: bordo di 1 px accettabile.

**F7. Snap layouts al passaggio sul pulsante Ingrandisci (Windows 11)**
- **Stato attuale.** Il pulsante è HTML: `WM_NCHITTEST` non restituisce mai `HTMAXBUTTON` e il riquadro dei layout non compare. Win+Z dovrebbe funzionare comunque perché la finestra ha `WS_MAXIMIZEBOX` (da verificare).
- **Soluzione, costo esplicito.** Una finestra figlia trasparente sopra il pulsante, con subclass della top-level che restituisce `HTMAXBUTTON` e inoltra hover e pressione alla pagina per lo stato visivo. Lo schema è quello di plugin di terze parti come tauri-plugin-decorum. È lavoro Win32 non banale, da fare dopo F4 e F6. In alternativa, riconsiderare la cornice OS.
- **Prova.** [Win, Windows 11] Il passaggio di 0,5 s su Ingrandisci mostra i layout e un clic su una zona aggancia la finestra (richiede anche F5). Win+Z mostra i layout già oggi: annotare il risultato.

**F8. Stampa: doppia anteprima, la seconda è del browser**
- **Stato attuale.** `PrintPreview.tsx:20-31` chiama `print_mail`, che chiama `window.print()` (`commands/mail.rs:118-122`). **[R]** wry implementa `print()` con `eval("window.print()")` (`wry webview2/mod.rs:1886-1891`): si apre l'anteprima di stampa di Edge dopo quella dell'app.
- **Soluzione.** In `print_mail` usare `ICoreWebView2_16::ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM)` tramite webview2-com, già dipendenza: apre il dialogo Stampa di Windows, che offre anche "Microsoft Print to PDF". In alternativa `PrintToPdf` + `rfd` per "Salva PDF".
- **Prova.** [Win] "Stampa / Salva PDF" apre il dialogo Stampa di sistema, con owner main e senza anteprima Edge. Stampare su PDF produce le stesse pagine di `print.css`.

**F9. Ingrandimento accessibile e scala del testo**
- **Stato attuale.**
  - `SetAreBrowserAcceleratorKeysEnabled(false)` (`native_feel.rs:113`). **[R]** Spegne anche Ctrl+Più e Ctrl+Meno (doc Microsoft).
  - wry imposta `IsZoomControlEnabled` e `IsPinchZoomEnabled` da `zoom_hotkeys_enabled`, falso per default (`wry webview2/mod.rs:649, 670`).
  - Non c'è un ingrandimento nell'app: nessun controllo né scorciatoia.
  - I testi sono in px (`theme.css:7-10`, 11,5–14 px).
  - **[R]** La `RasterizationScale` di WebView2 *può* includere la scala del testo di Windows, ma l'esempio Microsoft la imposta a mano da `UISettings.TextScaleFactor`. Probabilmente "Dimensione del testo" (Accessibilità) non ha quindi effetto su Qamille: da verificare.
  
  La skill chiede di conservare l'ingrandimento accessibile e di sostituire esplicitamente le scorciatoie intercettate.
- **Soluzione.** Ctrl+= / Ctrl+- / Ctrl+0 (e Ctrl+rotella, se si vuole) in `keyboard()`, che chiamano `getCurrentWebview().setZoom()`; aggiungere `core:webview:allow-set-webview-zoom` a `main.json`. Persistere il valore; valori 80–200%. All'avvio partire da `TextScaleFactor` (Rust, `windows` con `UI_ViewManagement`, costo moderato) e seguire `TextScaleFactorChanged`. Va documentato in `aspetto-nativo.md`, regola 8.
- **Prova.** [Linux] Test browser: Ctrl+= chiama `setZoom(1.1)` (mock) e non arriva alla lista. [Win] Con "Dimensione del testo" al 150%: Qamille parte ingrandito; Ctrl+0 torna al 100%. A 1000×640 con zoom 150% le azioni dei dialoghi restano raggiungibili scorrendo.

**F10. Contrasto elevato (forced-colors)**
- **Stato attuale.** Nessuna regola `@media (forced-colors: active)` in `src` (grep). Selezione delle righe, Switch, Checkbox, velo e stato attivo dei pulsanti icona si basano su background e box-shadow, che in forced colors vengono rimossi o sostituiti. Il risultato non è verificato.
- **Soluzione.** Prima verificare, poi intervenire con regole mirate: `outline` o `border` con `Highlight` per la selezione, `forced-color-adjust: none` solo dove serve (avatar, colori dei calendari), `SelectedItem`/`ButtonText` per gli stati.
- **Prova.** [Linux] Playwright `page.emulateMedia({ forcedColors: 'active' })` sull'anteprima: riga selezionata, focus, Switch on/off, pulsante disattivato e dialogo distinguibili, con screenshot di confronto (serve un Chromium di Playwright nel container: non trovato in `~/.cache/ms-playwright`). [Win] Tema Contrasto "Deserto" e "Acquatico": stesse verifiche e menu di sistema leggibile.

**F11. Il chip promemoria compare probabilmente in Alt+Tab e non si raggiunge da tastiera**
- **Stato attuale.** **[R]** Senza owner tao imposta `ON_TASKBAR`, cioè `WS_EX_APPWINDOW` (`tao window.rs:1281-1283`, `window_state.rs:255-257`). `skip_taskbar` toglie solo il pulsante dalla barra con `ITaskbarList::DeleteTab` (`window.rs:1543-1548`). Una finestra visibile, senza owner e con `WS_EX_APPWINDOW` compare in Alt+Tab e Win+Tab: quindi due voci "Qamille", e passare al chip mette il focus in una finestra senza bordo e senza indicatore. INV-004 copre la barra delle applicazioni, non Alt+Tab. Il chip non ha Esc né una live region, e l'Assistente vocale non lo annuncia (`Reminder.tsx:129-192`).
- **Soluzione.**
  - In `reminders.rs`, dopo `build()`: `WS_EX_TOOLWINDOW` (e togliere `WS_EX_APPWINDOW`) via `SetWindowLongPtrW` sull'`hwnd()`. In alternativa `.focusable(false)` (`WS_EX_NOACTIVATE`): il clic non toglie il focus all'app in uso, ma va provato con WebView2, che potrebbe non ricevere i clic.
  - In `Reminder.tsx`: `role="alert"` o `aria-live` sul primo chip ed Esc = chiudi.
  - Opzionale: in main, un annuncio "Riunione X tra N min" con Invio = Entra, per chi usa solo la tastiera o l'Assistente vocale.
- **Prova.** [Win] Con un chip visibile, Alt+Tab e Win+Tab mostrano una sola voce Qamille. Il chip non è nella barra delle applicazioni. Mentre si scrive in Blocco note, il chip compare e il cursore resta in Blocco note. Con l'Assistente vocale attivo, il chip viene annunciato. [Linux] Test `cargo test` della funzione pura che calcola gli stili, se estratta.

### P2: rifiniture

**F12. Avvio con un tema esplicito diverso da quello di sistema**
- **Stato attuale.** `show_main_window` sceglie il colore dal **tema di sistema** (`native_feel.rs:18`), mentre `theme-boot.js` legge `localStorage` (scelta esplicita). Con Aspetto = Scuro e Windows chiaro, la finestra si mostra bianca finché la pagina non disegna (al contrario con Chiaro su Windows scuro). Inoltre il `PreferredColorScheme` di WebView2 non segue mai la scelta dell'app (`setTheme` non è chiamato): menu contestuale e interfacce Chromium restano del tema di sistema.
- **Soluzione.** Salvare l'aspetto anche nelle impostazioni lette da Rust, oppure mostrare la finestra da JS dopo il primo paint (`allow-show`). Chiamare `getCurrentWindow().setTheme(null | 'dark' | 'light')` dall'effetto del tema (`App.tsx:1277-1281`, permesso `allow-set-theme`). Con il tema forzato, `prefers-color-scheme` della pagina cambia: va letto solo quando Aspetto = Sistema, che è già il caso.
- **Prova.** [Win] Windows chiaro con Aspetto = Scuro: nessun lampo bianco all'avvio (estendere `release-theme-probe.mjs`). Tasto destro su un campo: il menu è scuro.

**F13. Colore di fondo nativo contro il colore esterno della pagina**
- **Stato attuale.** La finestra usa `--surface` (`#ffffff` / `#1a202c`), ma `html`, `body` e `.app-shell` disegnano `--ground` (`#f6f7f9` / `#12161e`) (`system.css:27-37`, `styles.css:14-20`). Durante il ridimensionamento la striscia scoperta ha un colore diverso dalla cornice, visibile in scuro.
- **Soluzione.** Usare `--ground` come colore della finestra: è una modifica della regola 6 di `aspetto-nativo.md` e dei test 4 e 7, da concordare.
- **Prova.** [Win] Ridimensionamento veloce dal bordo destro in tema scuro: nessuna striscia più chiara.

**F14. Le scorciatoie non funzionano con il focus nel corpo HTML del messaggio**
- **Stato attuale.** iframe `sandbox="allow-same-origin"` (`MessageHtml.tsx:60-67`). `loaded()` aggancia solo `click` (`:39-48`); il `keydown` globale (`App.tsx:1327`) non vede i tasti premuti dentro l'iframe. Dopo un clic nel testo per selezionarlo, Ctrl+K, N, R, E e le frecce non funzionano.
- **Soluzione.** In `loaded()`, `doc.addEventListener('keydown', e => window.dispatchEvent(new KeyboardEvent('keydown', e)))` per i tasti non di editing; Ctrl+C e Ctrl+A restano all'iframe.
- **Prova.** [Linux] Test browser: clic nel corpo HTML, poi Ctrl+K mette il focus nella ricerca e E archivia.

**F15. Menu fatti con Popover**
- **Stato attuale.** `Popover` con `<button>` (`MeetingMenu.tsx:106-124`): non c'è `role="menu"` né `menuitem`, le frecce non spostano tra le voci e non c'è ricerca per iniziale. Esc e il ritorno del focus funzionano.
- **Soluzione.** Usare Ark `Menu` per i menu di comandi; Popover resta per i filtri.
- **Prova.** [Linux] Test browser: le frecce spostano tra le voci, Home/End, Esc torna al pulsante "Altre azioni", UIA "menu item".

**F16. Ritorno del focus dai Modal**
- **Stato attuale.** `Modal.tsx:19-29` torna all'opener solo se è ancora nel DOM. MeetingMenu lo gestisce esplicitamente (`MeetingMenu.tsx:52-56`). Altri dialoghi aperti da un menu che si smonta (anteprima di stampa da un menu della lista, eliminazione di un contatto) potrebbero lasciare il focus su `body`. Non l'ho verificato caso per caso.
- **Soluzione.** Passare un `returnFocus` esplicito (il trigger del menu) o usare `finalFocusEl` di Ark.
- **Prova.** [Linux] Esistono già test browser a11y: per ogni dialogo, apertura da tastiera, Esc, poi `document.activeElement` è il trigger.

**F17. Contenuto lungo nei Modal**
- **Stato attuale.** Scorre tutto il dialogo, azioni comprese (`dialog.css:29`). Con un testo ingrandito (F9) i pulsanti finiscono fuori vista.
- **Soluzione.** Struttura fissa: testa, corpo scorrevole, azioni in fondo.
- **Prova.** [Linux] Viewport 1000×640 a zoom 200%: i pulsanti di "Unisci contatti" e "Accedi con Google" restano visibili.

**F18. Notifiche: un errore può essere sostituito**
- **Stato attuale.** `max: 1` (`Toaster.tsx:15`): un esito ("Recupero all'avvio completato") può rimpiazzare un errore di 12 s appena arrivato, per esempio quello della migrazione (`main.rs:109-121` contro `App.tsx:1400-1402`).
- **Soluzione.** Un errore non viene rimpiazzato da uno stato: si accoda, oppure resta finché lo si chiude.
- **Prova.** [Linux] Unit test della coda.

**F19. Message box e dialoghi file**
- **Stato attuale.** I pulsanti OK/Annulla rispondono a una domanda (`meetings.rs:108`): le linee guida Windows chiedono verbi espliciti. `meeting_import` e `meeting_export` non hanno titolo localizzato e filtrano "Audio" e "Meeting" (`meetings.rs:43, 83`).
- **Soluzione.** `OkCancelCustom`, `set_title` localizzato, filtri "File audio" e "Markdown".
- **Prova.** [Win] Revisione visiva.

**F20. Accesso Google completato nel browser**
- **Stato attuale.** Qamille non torna in primo piano e non segnala niente sulla barra delle applicazioni: nessuna chiamata `set_focus` né `request_user_attention` dopo `connect_google`.
- **Soluzione.** `request_user_attention(Informational)` (lampeggio della barra), non `set_focus`, che ruberebbe il focus al browser.
- **Prova.** [Win] Al termine dell'accesso il pulsante di Qamille sulla barra lampeggia una volta.

**F21. Chip e presentazioni o condivisione schermo**
- **Stato attuale.** `always_on_top` compare anche sopra PowerPoint a schermo intero e nelle condivisioni dello schermo. ADR-0004 accetta di ignorare "Non disturbare".
- **Soluzione (scelte di prodotto).** `SHQueryUserNotificationState` (`QUNS_PRESENTATION_MODE`, `QUNS_RUNNING_D3D_FULL_SCREEN`) per rimandare il chip, e/o `content_protected(true)` per nasconderlo dalle catture.
- **Prova.** [Win] Durante una presentazione a schermo intero il chip non compare o compare all'uscita; in Teams o Meet il chip non appare nello schermo condiviso.

**F22. ACL a privilegio minimo**
- **Stato attuale.**
  - (a) `main` ha `core:default`, che **[R]** in 2.12.5 include `core:menu:default` e `core:tray:default` (creare menu, menu dell'app e tray da JS), `core:event:allow-emit/emit_to` e `core:webview:allow-internal-toggle-devtools`. Il frontend importa solo `api/core`, `api/event` e `api/window` (grep).
  - (b) Il backend emette quasi tutto con `emit` (a tutte le WebView: `app-event` in `runtime.rs:32`; `background-notice` in `main.rs:103, 134, 145, 178, 215, 258, 261, 348, 363, 367`). Il chip, che ha `core:event:allow-listen`, può quindi ascoltare notifiche con testo di errori e contenuti. Il codice del chip ascolta solo `reminder-changed`, quindi il rischio è basso.
  - Per il resto la capability del chip è già minima.
- **Soluzione.** (a) Sostituire `core:default` con l'elenco esplicito: `core:event:allow-listen`, `core:event:allow-unlisten`, i getter della finestra (`allow-is-maximized`), `allow-internal-toggle-maximize`, `core:app:default` se serve, più quelli già presenti. (b) `emit_to("main", …)` o `emit_filter` per gli eventi destinati a main.
- **Prova.** [Win] Tutti i `*-native.mjs` passano senza errori ACL in console. [Linux] Un nuovo controllo statico: `main.json` non contiene `core:default`, il chip non ha permessi `emit` e `main.rs`/`runtime.rs` non usano `emit("background-notice"…)` senza destinatario.

**F23. Barra del titolo: dettagli**
- **Stato attuale.** Le zone senza attributo (`.global-search` fuori dal campo, `.topbar-end`, `.topbar-actions`) non trascinano: `drag.js` accetta solo il bersaglio diretto (`App.tsx:1508-1561`). I tre pulsanti di finestra sono tre fermate di Tab, mentre in Windows i pulsanti della barra non ricevono il focus da tastiera.
- **Soluzione.** `data-tauri-drag-region="deep"` sul contenitore, con `"false"` sui controlli. `tabIndex={-1}` sui pulsanti di finestra, che restano raggiungibili da Alt+Space (F4) e Win+frecce.
- **Prova.** [Win] Doppio clic in qualunque spazio vuoto della barra ingrandisce. Tab dalla ricerca va al primo comando dell'app, non a "Riduci a icona".

**F24. Overscroll ed effetto rimbalzo**
- **Stato attuale.** Non c'è `overscroll-behavior` (grep). `aspetto-nativo.md` dice che il rimbalzo vale solo su macOS. **[R]** Chromium su Windows supporta però l'overscroll elastico con touchpad di precisione e touch (dipende dalla versione del runtime, da verificare).
- **Soluzione.** Se lo si osserva, `overscroll-behavior: none` su `html, body` e `contain` sui pannelli scorrevoli, senza toccare lo scorrimento né le animazioni a molla (`motion`, che rispettano già `prefers-reduced-motion`).
- **Prova.** [Win] Touchpad di precisione: due dita oltre la fine della lista non producono rimbalzo, e lo scorrimento resta fluido.

**F25. F6 per passare da un pannello all'altro**
- **Stato attuale.** Non esiste. È una convenzione di Esplora file e Outlook: sidebar → lista → lettore → barra.
- **Soluzione.** In `keyboard()`, F6 e Maiusc+F6 scorrono gli `[data-pane]`.
- **Prova.** [Linux] Test browser.

### Cose già a posto (da non rompere)

- Hardening WebView2 e filtro del menu contestuale coperti da test (`native-feel.mjs`, `cargo test native_feel`).
- Chiusura con veto: ordine dei guard testato (`close-guards.mjs`), `inert` durante il flush, `UnsavedDialog` con Esc = "Continua a modificare". La X, Alt+F4 e la chiusura dalla barra delle applicazioni passano tutte da `WM_CLOSE` e quindi dai guard (Alt+F4 **[R]**).
- Ingrandimento e trascinamento di sistema (HTCAPTION, Aero Snap), Win+frecce, ridimensionamento della finestra senza bordi, icona Ingrandisci/Ripristina aggiornata da `onResized`.
- File HTML5 trascinati fuori bersaglio bloccati (`App.tsx:1336-1349`). Nuove finestre negate per default (**[R]** `wry webview2/mod.rs:782-863`). Link solo verso il browser di sistema, solo da main.
- Chip: privilegi minimi, link scelto dal motore (INV-006), non prende il focus alla comparsa, si chiude con main, la prima lettura più gli eventi evitano un secondo backend, e un clic subito dopo la comparsa viene ignorato (`ARMED_AFTER_MS`).
- `Modal`: focus intrappolato, Esc, clic sul velo che non chiude. `SidePanel` non modale (ADR-0008 INV-002). `prefers-reduced-motion` diffuso.
- Selezione del testo limitata ai contenuti, con IME ed editing invariati (input, textarea, contenteditable selezionabili).

---

## 5. Modifiche che farei, in ordine

| # | Modifica | File | Verifica |
|---|---|---|---|
| 1 | Owner `main` per tutti i dialoghi `rfd`, pulsanti espliciti nella conferma di chiusura (F1, F19) | `src-tauri/src/commands/document_preview.rs`, `drafts.rs`, `mail.rs`, `meetings.rs` | [Linux] Revisione; `cargo check` solo se sono installate le dipendenze di sistema. [Win] Prove F1, più un controllo UIA `IsEnabled=false` in `a11y-native.mjs` |
| 2 | Esporta .vcf dal backend con Salva con nome (F3) | `src-tauri/src/commands/contacts.rs`, `src/components/Contacts.tsx` | [Linux] `tests/contacts.mjs` (percorso di anteprima). [Win] Nessuna interfaccia download, file dove scelto |
| 3 | Fine sessione: `WM_QUERYENDSESSION` + `ShutdownBlockReasonCreate` + flush non interattivo (F2) | `src-tauri/src/native_feel.rs` (o un nuovo `session.rs`), `main.rs`, `src/lib/window-close.ts`, `close-guards.ts` | [Linux] Unit test del flush non interattivo con `node --test`. [Win] `shutdown /l` con bozza, Settings e registrazione |
| 4 | Menu di sistema con Alt+Space e tasto destro sulla barra (F4); drag region `deep`, pulsanti fuori dal Tab (F23) | `native_feel.rs`, `main.rs` (nuovo comando, permesso generato da `build.rs`), `App.tsx`, `Sidebar.tsx`, `WindowControls.tsx` | [Linux] `native-feel.mjs` esteso. [Win] Prove F4 e F23; `window-native.mjs` esteso con "Chiudi dal menu di sistema → dialogo non salvato" |
| 5 | `shadow: true` + colore del bordo DWM (F6) | `tauri.conf.json`, `native_feel.rs`, `tests/native-feel.mjs`, `docs/aspetto-nativo.md` | [Linux] `native-feel.mjs`. [Win] Windows 11 e 10: angoli, ombra, maniglie fuori dal contenuto |
| 6 | Zoom dell'app con Ctrl+= / Ctrl+- / Ctrl+0 e scala del testo di Windows all'avvio (F9) | `App.tsx`, `capabilities/main.json`, `native_feel.rs` (TextScaleFactor), `lib/preferences.ts`, `docs/aspetto-nativo.md` | [Linux] Test browser delle scorciatoie. [Win] "Dimensione del testo" 150%, `glance-scale.mjs` a zoom 1,25 e 1,5 |
| 7 | Audit e regole forced-colors (F10) | `src/ds/styles/system.css`, `styles.css`, `ds/styles/ark/*.css`, nuovo test browser | [Linux] Playwright `forcedColors: 'active'` (serve installare il Chromium di Playwright). [Win] Temi Contrasto |
| 8 | Stampa con il dialogo di sistema (F8) | `src-tauri/src/commands/mail.rs` (webview2-com `ICoreWebView2_16`) | [Win] Solo prova manuale |
| 9 | Chip fuori da Alt+Tab, Esc e annuncio all'Assistente vocale (F11) | `src-tauri/src/reminders.rs`, `src/reminder/Reminder.tsx` | [Win] Alt+Tab, Blocco note mantiene il focus, Assistente vocale; aggiornare la sezione Confirmation di ADR-0004 |
| 10 | Tema esplicito: avvio senza lampo e `setTheme` di WebView2 (F12), colore di fondo `--ground` (F13) | `native_feel.rs`, `main.rs`, `App.tsx`, `capabilities/main.json`, `public/theme-boot.js`, `tests/native-feel.mjs` | [Linux] `native-feel.mjs`. [Win] `release-theme-probe.mjs` esteso al caso tema app ≠ tema di sistema |
| 11 | ACL minime (F22) | `capabilities/main.json`, `main.rs`, `runtime.rs`, `reminders.rs` | [Linux] Controllo statico nuovo. [Win] Tutti i `*-native.mjs` senza errori ACL |
| 12 | Rifiniture di tastiera e dialoghi: F14, F15, F16, F17, F18, F25, F20 | `MessageHtml.tsx`, `ds/primitives/Modal.tsx`, `Popover` o nuovo `Menu`, `Toaster.tsx`, `App.tsx`, `Settings.tsx` | [Linux] Test browser per ognuno. [Win] F20 |
| 13 | Layout sotto i 1000 px per lo snap (F5), poi eventuale hit test `HTMAXBUTTON` per gli snap layouts (F7) | `tauri.conf.json`, `styles.css` e componenti di layout; Win32 in `native_feel.rs` | [Linux] Test browser a 640–960 px. [Win] Win+← al 100% e al 150%, hover su Ingrandisci |

Dopo ogni passo che tocca `src-tauri/src/`, CSS globali o finestre: `node tests/native-feel.mjs`, come chiede `AGENTS.md`. Per gli exe: build con `build-release.bat --no-install` (o `--features custom-protocol`), poi controllo della stringa `theme-boot.js` o `node tests/release-theme-probe.mjs`.

**Cosa si può verificare in questo container Linux:** test statici Node (`native-feel.mjs`, `close-guards.mjs`), test browser dell'anteprima (con il Chromium di Playwright installato), emulazione forced-colors e zoom nella pagina, unit test Rust delle funzioni pure (`cargo test native_feel`, compilazione pesante e non eseguita qui).

**Solo su Windows con l'exe di release:** proprietà e modalità dei dialoghi, Alt+Space e menu di sistema, snap e snap layouts, ombra e bordi DWM, fine sessione, stampa di sistema, Alt+Tab e focus del chip, Assistente vocale e UIA, temi Contrasto reali, scala del testo di Windows, overscroll con touchpad, lampo del tema all'avvio.

**Azione per l'utente:** approvare o scartare il punto 13 (larghezza minima sotto i 1000 px e snap layouts), perché è l'unico che cambia il layout del prodotto. I punti 1–12 si possono fare con incrementi verificabili senza decisioni di design.
