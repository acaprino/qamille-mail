// Guards the native look and feel described in docs/aspetto-nativo.md.
// Static checks plus the startup theme script run against every case.
// Run: node tests/native-feel.mjs   (from desktop/)
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { pathToFileURL } from 'node:url';

const desktop = path.resolve(import.meta.dirname, '..');
const read = (file) => fs.readFileSync(path.join(desktop, file), 'utf8').replace(/\r\n/g, '\n');
const stripComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, '');
const failures = [];
const check = (name, fn) => {
  try {
    fn();
    console.log(`ok   ${name}`);
  } catch (error) {
    failures.push(name);
    console.error(`FAIL ${name}\n     ${error.message.split('\n').join('\n     ')}`);
  }
};

// The declarations of the first rule whose selector list is exactly `selector`.
function rule(css, selector) {
  const plain = stripComments(css);
  const re = /([^{}]+)\{([^{}]*)\}/g;
  for (const [, selectors, body] of plain.matchAll(re)) {
    if (selectors.trim() === selector) return body;
  }
  throw new Error(`no "${selector}" rule`);
}
function ruleContaining(css, selector) {
  const plain = stripComments(css);
  for (const [, selectors, body] of plain.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (selectors.split(',').map((s) => s.trim()).includes(selector)) return { selectors, body };
  }
  throw new Error(`no rule lists "${selector}"`);
}
const hex = (value) => {
  const h = value.trim().replace(/^#/, '').toLowerCase();
  return h.length === 3 ? [...h].map((c) => c + c).join('') : h.slice(0, 6);
};
// A Rust item from its `head` to the closing brace at the indentation of its first line, without
// `//` comments: an assertion on one function is met neither by the next one nor by a comment.
function rustItem(text, head) {
  const start = text.indexOf(head);
  assert.ok(start >= 0, `no ${head}`);
  const indent = text.slice(text.lastIndexOf('\n', start) + 1, start).match(/^\s*/)[0];
  const end = text.indexOf(`\n${indent}}`, start);
  assert.ok(end >= 0, `${head} has no closing brace`);
  return text.slice(start, end + indent.length + 2).replace(/(^|\s)\/\/.*$/gm, '$1');
}
// The handler a registration in `native_feel::webview2::harden` adds: the code between the
// registration before it and this one, which must come before the page is loaded again.
function hardenHandler(event) {
  const harden = rustItem(read('src-tauri/src/native_feel.rs'), 'pub unsafe fn harden(');
  const reload = harden.indexOf('core.Navigate(');
  assert.ok(reload >= 0, 'harden must load the page again once the handlers are in');
  const at = harden.indexOf(`.add_${event}(`);
  assert.ok(at >= 0 && at < reload, `harden must register ${event} before it loads the page again`);
  return harden.slice(harden.lastIndexOf('.add_', at - 1) + 1, at);
}

check('1. interface text is not selectable and keeps the arrow cursor', () => {
  const body = rule(read('src/ds/styles/system.css'), 'body');
  assert.match(body, /user-select:\s*none/, 'body needs user-select: none');
  assert.match(body, /cursor:\s*default/, 'body needs cursor: default');
  const reminder = rule(read('src/reminder/reminder.css'), 'body');
  assert.match(reminder, /user-select:\s*none/, 'reminder body needs user-select: none');
  assert.match(reminder, /cursor:\s*default/, 'reminder body needs cursor: default');
});

check('2. fields, editors and reading content stay selectable', () => {
  const css = read('src/ds/styles/system.css');
  // pre/code: configurations and paths to paste elsewhere; alerts: errors to quote (audit T12).
  for (const selector of ['input', 'textarea', "[contenteditable]:not([contenteditable='false'])", '.selectable', '.reader-scroll', 'pre', 'code', "[role='alert']"]) {
    const { body } = ruleContaining(css, selector);
    assert.match(body, /user-select:\s*text/, `${selector} must opt back into user-select: text`);
    assert.match(body, /cursor:\s*auto/, `${selector} must show the text cursor (cursor: auto)`);
  }
  const arrow = (selector) =>
    [...stripComments(css).matchAll(/([^{}]+)\{([^{}]*)\}/g)].some(
      ([, selectors, body]) => selectors.split(',').map((s) => s.trim()).includes(selector) && /cursor:\s*default/.test(body),
    );
  assert.ok(arrow('button'), 'buttons keep the arrow (cursor: default)');
  // Chromium gives every a[href] the hand; the interface's links (setup guides) keep the arrow (T11).
  assert.ok(arrow('a'), 'links of the interface keep the arrow (cursor: default)');
  assert.match(rule(css, 'img,\na'), /-webkit-user-drag:\s*none/, 'interface images and links must not drag a browser ghost');
  // Rule 3: no hand cursor on the app's controls, in any stylesheet or Tailwind class.
  const dir = path.join(desktop, 'src');
  for (const file of fs.readdirSync(dir, { recursive: true }).filter((f) => /\.(css|tsx)$/.test(f))) {
    const text = stripComments(fs.readFileSync(path.join(dir, file), 'utf8'));
    assert.ok(!/cursor:\s*(pointer|not-allowed)|\bcursor-(pointer|not-allowed)\b/.test(text), `${file}: a hand or "not allowed" cursor on a control (docs/aspetto-nativo.md rule 3)`);
  }
});

check('3. the startup script picks the same theme as the app', () => {
  const script = read('public/theme-boot.js');
  const cases = [
    { saved: null, systemDark: true, want: 'dark' },
    { saved: null, systemDark: false, want: 'light' },
    { saved: 'system', systemDark: true, want: 'dark' },
    { saved: 'system', systemDark: false, want: 'light' },
    { saved: 'dark', systemDark: false, want: 'dark' },
    { saved: 'light', systemDark: true, want: 'light' },
    { saved: 'nonsense', systemDark: true, want: 'dark' },
    { saved: 'throws', systemDark: true, want: 'dark' },
  ];
  for (const { saved, systemDark, want } of cases) {
    const dataset = {};
    const context = {
      localStorage: {
        getItem: (key) => {
          if (saved === 'throws') throw new Error('storage disabled');
          return key === 'qamille:appearance' ? saved : null;
        },
      },
      document: { documentElement: { dataset } },
    };
    context.window = { matchMedia: (q) => ({ matches: q === '(prefers-color-scheme: dark)' && systemDark }), ...context };
    vm.runInNewContext(script, context);
    assert.equal(dataset.theme, want, `saved=${saved} systemDark=${systemDark}`);
  }
});

check('4. native window backgrounds match --ground in theme.css (what html, body and .app-shell paint)', () => {
  const theme = stripComments(read('src/ds/tokens/theme.css'));
  const light = theme.match(/:root\s*\{[^}]*?--ground:\s*([^;]+);/);
  const dark = theme.match(/:root\[data-theme='dark'\]\s*\{[^}]*?--ground:\s*([^;]+);/);
  assert.ok(light && dark, 'theme.css must define --ground for :root and dark');
  const conf = JSON.parse(read('src-tauri/tauri.conf.json'));
  assert.equal(hex(conf.app.windows[0].backgroundColor), hex(light[1]), 'tauri.conf.json backgroundColor ≠ light --ground');
  const rust = read('src-tauri/src/native_feel.rs').match(/DARK_GROUND: Color = Color\(0x(\w\w), 0x(\w\w), 0x(\w\w), 0xff\)/);
  assert.ok(rust, 'native_feel.rs must define DARK_GROUND');
  assert.equal(rust.slice(1, 4).join('').toLowerCase(), hex(dark[1]), 'DARK_GROUND ≠ dark --ground');
  assert.match(read('src/lib/native-window.ts'), /getPropertyValue\('--ground'\)/, 'paintWindowBackground must follow --ground');
});

check('5. every window gets the WebView2 hardening', () => {
  const dir = path.join(desktop, 'src-tauri/src');
  const sources = fs.readdirSync(dir, { recursive: true }).filter((f) => f.endsWith('.rs'));
  for (const file of sources) {
    const text = fs.readFileSync(path.join(dir, file), 'utf8');
    const builders = (text.match(/WebviewWindowBuilder::(?:new|from_config)\(/g) ?? []).length;
    const hardened = (text.match(/native_feel::harden_webview\(/g) ?? []).length;
    const autofill = (text.match(/\.general_autofill_enabled\(false\)/g) ?? []).length;
    const guarded = (text.match(/\.on_navigation\((?:crate::)?native_feel::navigation_guard\(/g) ?? []).length;
    assert.ok(hardened >= builders, `${file}: ${builders} window builder(s), ${hardened} harden_webview call(s)`);
    assert.ok(autofill >= builders, `${file}: ${builders} window builder(s), ${autofill} general_autofill_enabled(false)`);
    assert.ok(guarded >= builders, `${file}: ${builders} window builder(s), ${guarded} on_navigation(native_feel::navigation_guard(..))`);
  }
  const main = read('src-tauri/src/main.rs');
  assert.match(main, /window_config\.visible = false;/, 'main window must be built hidden');
  assert.match(main, /native_feel::show_main_window\(&main_window\)/, 'main window must be shown by show_main_window');
  const rust = read('src-tauri/src/native_feel.rs');
  assert.match(rust, /SetAreBrowserAcceleratorKeysEnabled\(false\)/, 'browser shortcuts must stay off');
  assert.match(rust, /add_ContextMenuRequested/, 'the right-click menu must stay filtered');
  // A filter that fails shows no menu, never the browser's whole one, and says why (rule 9).
  assert.match(
    hardenHandler('ContextMenuRequested'),
    /if let Err\(error\) = trim_menu\(&args\) \{\s*report\([\s\S]*?\);\s*args\.SetHandled\(true\)\?;\s*\}/,
    'a COM error in the menu filter must hide the menu (SetHandled(true)) and go to report',
  );
  // Microsoft does not document the entry names: the ones the filter takes out go to native.log.
  const trim = rustItem(rust, 'unsafe fn trim_menu(');
  assert.match(trim, /names_to_log\(&entries, &removals,[\s\S]*?report\(/, 'trim_menu must write the names it takes out to native.log');
});

check('6. the page keeps the native window background in step', () => {
  assert.match(read('src/App.tsx'), /void paintWindowBackground\(\);/, 'App.tsx must call paintWindowBackground');
  const capability = JSON.parse(read('src-tauri/capabilities/main.json'));
  assert.ok(capability.permissions.includes('core:window:allow-set-background-color'), 'main capability lacks set-background-color');
});

// Node strips the type annotations of this dependency-free module.
const { windowColor } = await import(pathToFileURL(path.join(desktop, 'src/lib/native-window.ts')).href);
check('7. --ground values become window colors', () => {
  assert.deepEqual(windowColor(' #1d2431'), [0x1d, 0x24, 0x31]);
  assert.deepEqual(windowColor('#fff'), [255, 255, 255]);
  assert.equal(windowColor('var(--x)'), null);
});

check('8. the custom title bar keeps what a Windows title bar does (audit F2 F4 F6 F7 F8 F9 F11 F22)', () => {
  const conf = JSON.parse(read('src-tauri/tauri.conf.json'));
  assert.equal(conf.app.windows[0].shadow, true, 'main window needs shadow: true (DWM shadow, rounded corners, resize outside the content)');
  assert.match(read('src-tauri/src/main.rs'), /window_shell::attach_main\(&main_window\)/, 'main window must get window_shell::attach_main');
  const shell = read('src-tauri/src/window_shell.rs');
  assert.match(shell, /add_AcceleratorKeyPressed/, 'Alt+Space must reach the system menu');
  assert.match(shell, /TrackPopupMenu\(/, 'right-click on the bar opens the system menu');
  assert.match(shell, /WM_QUERYENDSESSION/, 'session end must save before tao exits');
  assert.match(shell, /ShutdownBlockReasonCreate/, 'session end must tell Windows why it waits');
  assert.match(shell, /HTMAXBUTTON/, 'maximize button must answer HTMAXBUTTON for snap layouts');
  assert.match(shell, /COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM/, 'printing uses the Windows print dialog');
  assert.match(read('src-tauri/src/reminders.rs'), /window_shell::make_tool_window\(&window\)/, 'the reminder chip stays out of Alt+Tab');
  assert.match(read('src/lib/window-shell.ts'), /'contextmenu'/, 'the page sends right-clicks on drag regions to the system menu');
  assert.match(read('src/App.tsx'), /zoomKey\(e\)/, 'Ctrl+= / Ctrl+- / Ctrl+0 are the app zoom');
  const capability = JSON.parse(read('src-tauri/capabilities/main.json'));
  assert.ok(!capability.permissions.includes('core:default'), 'main capability must list its core permissions, not core:default');
  assert.ok(!capability.permissions.some((p) => /emit|menu|tray/.test(p)), 'main capability must not allow emit, menus or tray');
  // Events with mail subjects, notices and errors go to the main window only, never to the
  // reminder chip, which may listen (audit F22b, T18).
  for (const file of fs.readdirSync(path.join(desktop, 'src-tauri/src'), { recursive: true }).filter((f) => f.endsWith('.rs'))) {
    const text = stripComments(read(path.join('src-tauri/src', file)));
    assert.ok(!/\.emit\(\s*"(app-event|background-notice)"/.test(text), `${file}: app-event and background-notice need emit_to("main", …)`);
  }
});

// Node strips the type annotations of these dependency-free modules.
const { externalAddress, fragmentOf, isHistoryButton, isStrayDrop, opensLinkElsewhere } = await import(pathToFileURL(path.join(desktop, 'src/lib/browser-guards.ts')).href);
const { imeComposing } = await import(pathToFileURL(path.join(desktop, 'src/ds/ime.ts')).href);
check("9. a failed page is the app's to answer, never WebView2's error page (rule 21)", () => {
  const rust = read('src-tauri/src/native_feel.rs');
  assert.match(rust, /SetIsBuiltInErrorPageEnabled\(false\)/, 'the WebView2 error page must stay off');
  assert.match(rust, /add_ProcessFailed/, 'a crashed or hung page must reach recovery.rs');
  assert.match(rust, /add_NavigationCompleted/, 'a page that did not load must reach recovery.rs');
  assert.match(rust, /OPERATION_CANCELED/, 'a cancelled navigation is not a failed page');
  const recovery = read('src-tauri/src/recovery.rs');
  assert.match(recovery, /may_reload_by_itself/, 'automatic reloads need a limit');
  assert.match(recovery, /set_parent\(window\)/, 'recovery dialogs are owned by the window');
  const main = read('src-tauri/src/main.rs');
  assert.match(main, /recovery::wait_for_previous_instance\(\);/, 'a restart waits for the old process');
  assert.match(main, /recovery::reload_ready/, 'reload_ready must be a command');
  assert.match(read('src-tauri/src/window_shell.rs'), /AppendMenuW\(menu, MF_STRING, shell_rules::SC_APP_RELOAD/, 'the system menu offers the native reload');
  assert.match(read('src/lib/window-close.ts'), /'reload-requested'/, 'the page saves before the native reload');
});

check("10. the first frame of the main window is the starting theme's ground (rule 7)", () => {
  const main = read('src-tauri/src/main.rs');
  assert.match(main, /window_shell::starts_dark\(\)[\s\S]{0,80}background_color = Some\(native_feel::DARK_GROUND\)/, 'a dark start creates the webview on DARK_GROUND');
});

check('11. browser mouse gestures and the IME Enter stay away from the app (rules 23, 24)', () => {
  for (const entry of ['src/main.tsx', 'src/reminder/main.tsx']) {
    assert.match(read(entry), /installBrowserGuards\(\);/, `${entry} must install the browser guards`);
  }
  assert.ok(isHistoryButton(3) && isHistoryButton(4) && !isHistoryButton(0) && !isHistoryButton(1));
  assert.ok(opensLinkElsewhere({ button: 1, ctrlKey: false, shiftKey: false, metaKey: false }, true));
  assert.ok(opensLinkElsewhere({ button: 0, ctrlKey: true, shiftKey: false, metaKey: false }, true));
  assert.ok(!opensLinkElsewhere({ button: 0, ctrlKey: false, shiftKey: false, metaKey: false }, true));
  assert.ok(!opensLinkElsewhere({ button: 1, ctrlKey: false, shiftKey: false, metaKey: false }, false));
  // A file anywhere, or a link from another app outside a field, would open in the page (T17).
  assert.ok(isStrayDrop(['Files'], false) && isStrayDrop(['Files'], true) && isStrayDrop(['text/uri-list', 'text/plain'], false));
  assert.ok(!isStrayDrop(['text/uri-list', 'text/plain'], true) && !isStrayDrop(['text/plain'], false));
  assert.match(read('src/components/MessageHtml.tsx'), /installBrowserGuards\(view\)/, 'the message frame gets the guards too');
  assert.ok(imeComposing({ isComposing: true, keyCode: 13 }) && imeComposing({ isComposing: false, keyCode: 229 }));
  assert.ok(!imeComposing({ isComposing: false, keyCode: 13 }));
  // Every plain Enter or Escape handler checks the IME first: confirming or dropping a candidate is
  // the IME's (audit N7). The reminder chip has no text field.
  const dir = path.join(desktop, 'src');
  const sources = fs
    .readdirSync(dir, { recursive: true })
    .filter((f) => /\.tsx?$/.test(f) && !/vendor|reminder[\\/]Reminder\.tsx$/.test(f));
  for (const file of sources) {
    const text = fs.readFileSync(path.join(dir, file), 'utf8');
    for (const line of text.split('\n').filter((l) => /key\s*[!=]==\s*'(Enter|Escape)'/.test(l))) {
      assert.match(line, /imeComposing\(/, `${file}: an Enter or Escape handler without the IME check: ${line.trim()}`);
    }
  }
});

check('12. a failed start and a blank page reach a message (rule 21, audit Q2 Q15)', () => {
  // A top-level Rust item from its `head` to its closing brace at column 0: an assertion on one
  // function can never be met by the function that follows it.
  const item = (text, head) => {
    const start = text.indexOf(head);
    assert.ok(start >= 0, `no ${head}`);
    return text.slice(start, text.indexOf('\n}', start) + 2);
  };
  const main = read('src-tauri/src/main.rs');
  // Tauri 2.12 turns an Err returned from `.setup` into a panic, and a panic there ends the process
  // (101): a release build (no console) would end without a word. The closure says both itself.
  const opened = main.indexOf('.setup(move |app| {');
  assert.ok(opened >= 0, 'main.rs must call .setup(move |app| { … })');
  const closure = main.slice(opened, main.indexOf('\n        })', opened));
  for (const [part, why] of [
    ['std::panic::catch_unwind(std::panic::AssertUnwindSafe(', 'catches a panic of setup_app'],
    ['setup_app(app, ', 'runs setup_app'],
    ['Ok(Err(error)) => error', 'hands an Err to startup_failed'],
    ['shell_rules::panic_message(&*panic)', 'hands a panic to startup_failed with its message'],
    ['startup_failed(Some(app.handle()), &*error)', 'says the failure, with the app at hand'],
  ]) {
    assert.ok(closure.includes(part), `the .setup closure ${why}: no ${part}`);
  }
  const failed = item(main, 'fn startup_failed(');
  const at = (part) => {
    const index = failed.indexOf(part);
    assert.ok(index >= 0, `startup_failed: no ${part}`);
    return index;
  };
  for (const part of ['shell_rules::startup_failure_text(', 'rfd::MessageLevel::Error']) at(part);
  const dialog = at('rfd::MessageDialog::new()');
  assert.ok(at('native_feel::report(') < dialog, 'startup_failed writes native.log before the message');
  // A failure after build(): the main window is on screen, and would stay usable behind an ownerless message.
  assert.ok(at('window.hide()') < dialog, 'startup_failed hides a main window already shown before the message');
  assert.ok(at('recovery::stop_asking()') < dialog, 'no recovery dialog opens over the message of a failed start');
  assert.ok(dialog < at('recovery::finish_meetings_before_exit(app)'), 'startup_failed stops the meetings as every exit does');
  assert.ok(at('recovery::finish_meetings_before_exit(app)') < at('std::process::exit(1)'), 'startup_failed exits with 1, last');
  assert.match(main, /\.run\(context\);[\s\S]*?if let Err\(error\) = started \{\s*startup_failed\(None, &error\)/, 'a failed Builder::build says it too');
  const setup = item(main, 'fn setup_app(');
  const lookup = setup.indexOf('tauri::webview_version()');
  const build = setup.indexOf('.build()?');
  assert.ok(lookup >= 0 && lookup < build, 'a missing WebView2 Runtime is found before the window is built');
  // Every page the main webview starts loading is watched, a reload the page starts itself too; the
  // first load also from the moment the window shows, in case it never starts.
  assert.ok(
    setup.slice(setup.indexOf('WebviewWindowBuilder::from_config('), build).includes('.on_page_load(recovery::page_load)'),
    'the main window must arm the 15 s watch at every page load (on_page_load)',
  );
  assert.ok(setup.indexOf('recovery::watch_startup(&main_window)') > build, 'the first load is watched from the moment the window shows');
  assert.match(main, /generate_handler!\[[^\]]*recovery::frontend_failed/, 'frontend_failed must be a command');
  assert.match(read('src-tauri/permissions/main-window.toml'), /"allow-frontend-failed"/, 'the main window may call frontend_failed');
  assert.ok(!JSON.parse(read('src-tauri/capabilities/reminder.json')).permissions.includes('allow-frontend-failed'), 'the reminder chip may not');
  // A render error with no error boundary empties the window: React reports it, the native side asks.
  const entry = read('src/main.tsx');
  assert.match(entry, /createRoot\(document\.getElementById\('app'\)!,\s*\{[^}]*onUncaughtError/, 'main.tsx must pass onUncaughtError to createRoot');
  assert.match(entry, /call\('frontend_failed'/, 'onUncaughtError must reach frontend_failed');
  // `frontend_ready` after render() fires whatever App does; from App's first effect, a render that
  // throws never reports ready.
  assert.ok(!/call\(\s*'frontend_ready'/.test(entry), 'frontend_ready must not be called from main.tsx');
  assert.match(read('src/App.tsx'), /useEffect\(\(\) => \{\s*if \(desktop\) void call\('frontend_ready'\)[^\n]*\n\s*\}, \[\]\);/, 'an effect of App.tsx reports the first frame');
  const recovery = read('src-tauri/src/recovery.rs');
  assert.match(recovery, /#\[tauri::command\]\s*pub fn frontend_failed\(window: WebviewWindow, message: String\)/, 'recovery.rs must define the frontend_failed command');
  assert.match(item(recovery, 'pub fn page_load('), /PageLoadEvent::Started[\s\S]*?watch_startup\(&window\)/, 'a page that starts loading arms the 15 s watch');
  assert.match(item(recovery, 'fn reload('), /watch_startup\(window\)/, 'a reload Qamille asks for is watched even if it never starts');
  // Esc and the X reload: only "Chiudi Qamille" leaves, and nothing leaves the window blank.
  assert.match(
    item(recovery, 'fn offer_reload('),
    /== Some\(false\);\s*if close \{\s*exit_app\(window\.app_handle\(\)\)\s*\} else \{\s*reload\(window\)\s*\}/,
    'Esc on the dialog of an empty page reloads',
  );
  assert.match(item(recovery, 'fn ask_once('), /STOPPED_ASKING\.load\(/, 'ask_once opens nothing once a failed start stopped asking');
  assert.match(recovery, /Failure::EngineExited if main => \{\s*if STOPPED_ASKING\.load\(/, 'nor does the restart question');
  assert.match(item(recovery, 'pub fn watch_startup('), /offer_reload\(/, 'the 15 s watch asks with offer_reload');
  assert.match(item(recovery, 'pub fn frontend_failed('), /offer_reload\(/, 'frontend_failed asks with offer_reload');
});

check('13. the first window fits the work area (audit Q4)', () => {
  // Tauri applies `center` and `preventOverflow` from the window config: on a small or scaled screen
  // the first window would otherwise open with its title-bar buttons off the work area.
  const conf = JSON.parse(read('src-tauri/tauri.conf.json'));
  const main = conf.app.windows.find((w) => w.label === 'main');
  assert.ok(main, 'tauri.conf.json must declare the main window');
  assert.equal(main.center, true, 'main window needs center: true (it opens in the middle of the work area)');
  assert.equal(main.preventOverflow, true, 'main window needs preventOverflow: true (it never opens past the work area)');
});

check('14. every WebView locks its root against overscroll (rule 4, audit Q5)', () => {
  // The declarations of every rule whose selector list names `selector`: the lock may sit on `html`
  // alone or on `html, body`.
  const bodiesOf = (css, selector) =>
    [...stripComments(css).matchAll(/([^{}]+)\{([^{}]*)\}/g)]
      .filter(([, selectors]) => selectors.split(',').map((s) => s.trim()).includes(selector))
      .map(([, , body]) => body);
  const locked = (css, selector, declaration, why) =>
    assert.ok(bodiesOf(css, selector).some((body) => declaration.test(body)), why);
  const system = read('src/ds/styles/system.css');
  const reminder = read('src/reminder/reminder.css');
  // A precision touchpad or a touchscreen must never slide the whole view and uncover the window
  // background: on html (the scroller of the viewport) and on the document, in the main window and the chip.
  locked(system, 'html', /overscroll-behavior:\s*none/, 'system.css: html needs overscroll-behavior: none');
  locked(reminder, 'html', /overscroll-behavior:\s*none/, 'reminder.css: html needs overscroll-behavior: none');
  for (const [name, css] of [['system.css', system], ['reminder.css', reminder]]) {
    for (const selector of ['html', 'body']) locked(css, selector, /overflow:\s*hidden/, `${name}: ${selector} must not scroll (overflow: hidden)`);
  }
  // A screen taller than the window scrolls inside its own container, which hands nothing to the root.
  const scrollers = [
    ['src/components/Onboarding.css', '.onboarding'],
    ['src/styles.css', '.startup-state'],
  ];
  for (const [file, selector] of scrollers) {
    const css = read(file);
    locked(css, selector, /(?<![-\w])overflow:\s*auto/, `${file}: ${selector} must scroll inside itself (overflow: auto)`);
    locked(css, selector, /overscroll-behavior:\s*contain/, `${file}: ${selector} must contain its overscroll`);
    locked(css, selector, /(?<![-\w])height:\s*100vh/, `${file}: ${selector} must be exactly one window tall (height: 100vh)`);
  }
  // A long unbreakable error is wider than the window at 200 %: centred without `safe` its left side is cut off.
  locked(read('src/styles.css'), '.startup-state', /align-items:\s*safe center/, 'styles.css: .startup-state must centre safely (align-items: safe center)');
  // The message frame stays free of the lock: it never scrolls vertically and hands the wheel to the reader.
  assert.ok(!/overscroll-behavior/.test(read('src/lib/message-html.ts')), 'the message frame must not set overscroll-behavior');
});

check('15. the reminder takes no file drops and every webview refuses frame, history and permission requests', () => {
  // wry's own drop handler would take a file dragged onto the chip, with the copy cursor, for a drop
  // nothing reads; without it the page refuses the drop (rule 23). The main window leaves drops to its page.
  const show = rustItem(read('src-tauri/src/reminders.rs'), 'fn show(');
  const opened = show.indexOf('WebviewWindowBuilder::new(');
  const built = show.indexOf('.build()?', opened);
  assert.ok(opened >= 0 && built > opened, 'reminders.rs: show() must build the chip with WebviewWindowBuilder::new(…)…build()?');
  assert.ok(show.slice(opened, built).includes('.disable_drag_drop_handler()'), 'reminders.rs: the chip must be built with .disable_drag_drop_handler()');
  const main = JSON.parse(read('src-tauri/tauri.conf.json')).app.windows.find((w) => w.label === 'main');
  assert.equal(main?.dragDropEnabled, false, 'tauri.conf.json: the main window needs dragDropEnabled: false');
  // Each guard is in before harden loads the page again, so it holds for the first document shown.
  assert.match(
    hardenHandler('FrameNavigationStarting'),
    /let refused = [^;]*\|\| !crate::shell_rules::frame_may_navigate\(&take_pwstr\(uri\)\);\s*if refused \|\| steps_through_history\(&args\) \{\s*args\.SetCancel\(true\)\?;/,
    'FrameNavigationStarting must refuse what shell_rules::frame_may_navigate does not allow, and Back and Forward',
  );
  assert.match(hardenHandler('NavigationStarting'), /if steps_through_history\(&args\) \{\s*args\.SetCancel\(true\)\?;/, 'NavigationStarting must refuse Back and Forward');
  const history = rustItem(read('src-tauri/src/native_feel.rs'), 'unsafe fn steps_through_history(');
  assert.match(history, /\.NavigationKind\(&mut kind\)/, 'steps_through_history must read NavigationKind');
  assert.match(history, /Ok\(\(\)\) => kind == COREWEBVIEW2_NAVIGATION_KIND_BACK_OR_FORWARD,/, 'steps_through_history must compare with BACK_OR_FORWARD');
  // A link or script that asks for a window of its own goes to the browser; the main window never gets one.
  assert.match(read('src-tauri/src/main.rs'), /\.on_new_window\(native_feel::new_window_guard\(app\.handle\(\)\)\)/, 'main.rs: the main window must be built with .on_new_window(native_feel::new_window_guard(app.handle()))');
  const newWindow = rustItem(read('src-tauri/src/native_feel.rs'), 'pub fn new_window_guard(');
  assert.ok(newWindow.includes('shell_rules::opens_from_new_window(') && newWindow.includes('NewWindowResponse::Deny'), 'new_window_guard must open only what opens_from_new_window accepts, and deny');
  assert.ok(!/NewWindowResponse::(Allow|Create)/.test(newWindow), 'new_window_guard must never allow a window');
  assert.match(
    hardenHandler('PermissionRequested'),
    /state != COREWEBVIEW2_PERMISSION_STATE_DEFAULT \{\s*return Ok\(\(\)\);\s*\}[\s\S]*\bargs\.SetState\(COREWEBVIEW2_PERMISSION_STATE_DENY\)\s*\}\)\);\s*core$/,
    'a permission no handler answered must be denied, as the last thing its handler does',
  );
});

check('16. a link in a message opens only an address written with http://, https:// or mailto:, or scrolls to a part of the message (rule 23)', () => {
  assert.equal(externalAddress('https://example.com/x'), 'https://example.com/x');
  // What the URL parser drops around an address, and nothing more.
  assert.equal(externalAddress(' \t https://example.com/x\n '), 'https://example.com/x');
  assert.equal(externalAddress('HTTP://Example.com/a b'), 'http://example.com/a%20b');
  assert.equal(externalAddress('mailto:someone@example.com?subject=Hi'), 'mailto:someone@example.com?subject=Hi');
  // A no-break space, a BOM or an ideographic space is not dropped: the address is relative, and
  // resolved against the app's own it would reach the browser as http://tauri.localhost/%C2%A0https://…
  // A scheme without "//" resolves against the app's address too (http:foo is http://tauri.localhost/foo).
  const refused = [
    '\u00a0https://example.com/x', '\ufeffhttps://example.com/x', '\u3000https://example.com/x', '\u2003https://example.com/x',
    'http:foo', 'HTTP:/foo', 'https:/example.com', 'https:\\\\example.com', 'https:\\example.com', 'mailto', '//example.com/x', '/x', 'x.html', '',
    'https://', 'javascript:alert(1)', 'data:text/html,x', 'file:///C:/x', 'tel:123', '#x',
  ];
  for (const written of refused) assert.equal(externalAddress(written), null, JSON.stringify(written));
  assert.equal(fragmentOf('#part2'), 'part2');
  assert.equal(fragmentOf(' #part2 '), 'part2');
  assert.equal(fragmentOf('#'), '');
  assert.equal(fragmentOf('#a%20b'), 'a%20b');
  for (const written of ['', 'part2', 'https://example.com/#x', '\u00a0#x', '/#x']) assert.equal(fragmentOf(written), null, JSON.stringify(written));
});

// The body of `!macro <name>` in an NSIS hooks file, without `;` comments: an assertion on one macro
// is met neither by the other macro nor by a comment.
function nshMacro(text, name) {
  const plain = text.replace(/^[ \t]*;.*$/gm, '');
  const found = plain.match(new RegExp(`^!macro ${name}\\b([\\s\\S]*?)^!macroend`, 'm'));
  assert.ok(found, `the hooks file must define !macro ${name}`);
  return found[1];
}

check("17. the installer removes the worker's startup and, when asked, the data (audit Q23)", () => {
  const nsis = JSON.parse(read('src-tauri/tauri.professional.conf.json')).bundle?.windows?.nsis;
  const hooksPath = nsis?.installerHooks?.replace(/^\.\//, '');
  assert.equal(hooksPath, 'windows/installer-hooks.nsh', 'the Professional installer must take its hooks from windows/installer-hooks.nsh');
  const hooks = read(`src-tauri/${hooksPath}`);

  // What the app writes, read from the code that writes it: the startup file and the data folder.
  const launcher = rustItem(read('../app/src/infrastructure/mcp_config.rs'), 'fn startup_launcher()');
  assert.match(launcher, /env::var_os\("APPDATA"\)/, 'the startup file is still placed under %APPDATA%');
  const written = launcher.match(/\.join\("(Microsoft\/Windows\/[^"]*\.vbs)"\)/);
  assert.ok(written, 'startup_launcher no longer joins a .vbs path: change this check and the hook together');
  const startupFile = written[1].replaceAll('/', '\\');
  const dataDir = rustItem(read('../app/src/infrastructure/legacy_store.rs'), 'pub fn data_dir()');
  assert.match(dataDir, /\.parent\(\)[\s\S]*\.join\("data"\)/, 'the data folder is still <executable folder>\\data');

  const before = nshMacro(hooks, 'NSIS_HOOK_PREUNINSTALL');
  // Only the workers started from this install folder: matched on the executable's full path and on
  // --worker as a whole argument. Killing by image name (taskkill /IM, nsis_tauri_utils::KillProcess)
  // would end every Qamille of the user too: a portable copy, or the window itself.
  assert.ok(!/taskkill|KillProcess|\/IM\b/i.test(before), 'workers must not be ended by image name');
  assert.match(before, /\$INSTDIR\\\$\{MAINBINARYNAME\}\.exe/, 'the hook must name the executable of this install folder');
  assert.match(before, /\$\$_\.ExecutablePath\s+-eq\s+\$\$env:\w+/, 'a process must be matched on its full executable path');
  assert.match(before, /\$\$_\.CommandLine\s+-match\s+'\(\^\|\\s\)--worker\(\\s\|\$\$\)'/, 'a process must be matched on --worker as a whole argument');
  assert.match(before, /Stop-Process\s+-Id\s+\$\$_\.ProcessId/, 'the matching processes must be stopped');
  // Nothing is deleted before the uninstaller's "Qamille is running" step: cancelling there leaves
  // Qamille installed, and it must keep its startup file and its data.
  assert.ok(!/\b(Delete|RMDir)\b/i.test(before) && !before.includes(startupFile), 'the hook that runs first must not delete anything');

  const after = nshMacro(hooks, 'NSIS_HOOK_POSTUNINSTALL');
  // The startup file goes only once the uninstall went through, and not on an /UPDATE. The Delete must
  // sit inside the If $UpdateMode <> 1 block: no ${EndIf} between the two.
  assert.match(after, /ReadEnvStr\s+\$\d\s+APPDATA/, 'the startup folder is found from %APPDATA%, as mcp_config.rs does');
  assert.match(
    after,
    new RegExp(`\\$\\{If\\}\\s+\\$UpdateMode\\s*<>\\s*1(?:(?!\\$\\{EndIf\\})[\\s\\S])*?Delete\\s+"\\$\\d\\\\${startupFile.replaceAll('\\', '\\\\')}"`),
    `${startupFile} must be deleted after the uninstall, and not on an /UPDATE`,
  );
  // The template's "Delete app data" only empties %APPDATA%\<identifier> and %LOCALAPPDATA%\<identifier>,
  // where Qamille keeps nothing: its data and credentials.key sit beside the program.
  assert.match(
    after,
    /\$\{If\}\s+\$DeleteAppDataCheckboxState\s*=\s*1\s+\$\{AndIf\}\s+\$UpdateMode\s*<>\s*1\s+\$\{AndIf\}\s+\$INSTDIR\s*!=\s*""\s+RMDir\s+\/r\s+"\$INSTDIR\\data"/,
    'the data folder must go only when the box is ticked, never on an /UPDATE, and never from an empty $INSTDIR',
  );
  assert.ok(!/RMDir\s+\/r\s+"\$INSTDIR"/.test(after), 'only the data folder is removed recursively, never the install folder');

  // A manual upgrade runs the old uninstaller without /UPDATE, so it removes the startup file too: the
  // app writes it again when it starts, with automatic startup on, never while it is off.
  const refresh = rustItem(read('../app/src/infrastructure/mcp_config.rs'), 'pub fn refresh_autostart()');
  assert.match(refresh, /refresh_autostart_at\(/, 'refresh_autostart must do the work of refresh_autostart_at');
  const decision = rustItem(read('../app/src/infrastructure/mcp_config.rs'), 'fn launcher_refresh(');
  assert.match(decision, /if !autostart \{\s*return None;\s*\}/, 'a launcher must never be written while automatic startup is off');
  assert.match(
    rustItem(read('src-tauri/src/main.rs'), 'fn setup_app('),
    /if !demo \{(?:(?!\n    \})[\s\S])*?mcp_config::refresh_autostart\(\)/,
    'setup_app must refresh the worker startup file when the app starts (not in the demo)',
  );
});

check("18. the calendar sheet's end-of-session step never calls the provider (rule 15, audit Q6)", () => {
  // Saving an event at sign-out or reload would create it, or send invitations to its guests, without
  // anyone confirming: the guard answers `false` and "Ricarica la finestra" asks instead.
  const calendar = stripComments(read('src/components/Calendar.tsx')).replace(/(^|\s)\/\/.*$/gm, '$1');
  const start = calendar.search(/useDirtyCloseGuard\(\s*'calendar-event'/);
  assert.ok(start >= 0, "Calendar.tsx no longer registers useDirtyCloseGuard('calendar-event', …): change this check with it");
  let depth = 0;
  let end = -1;
  for (let at = calendar.indexOf('(', start); at < calendar.length && end < 0; at++) {
    if (calendar[at] === '(') depth++;
    if (calendar[at] === ')' && --depth === 0) end = at;
  }
  assert.ok(end > start, 'the useDirtyCloseGuard call of the calendar sheet is not closed');
  const guard = calendar.slice(start, end + 1);
  assert.match(guard, /,\s*async \(\) => false,?\s*\)$/, "the calendar sheet's saveNow must be exactly `async () => false`");
  assert.ok(!/\b(save|calendarCall)\b/.test(guard), "the calendar sheet's guard must not reach save() or calendarCall");
});

if (failures.length) {
  console.error(`\n${failures.length} native-feel check(s) failed. See docs/aspetto-nativo.md.`);
  process.exit(1);
}
console.log('\nnative feel: all checks pass');
