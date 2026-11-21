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

check("17. the installer removes the start at sign-in, the old worker's startup file and, when asked, the data (audit Q22 Q23)", () => {
  const nsis = JSON.parse(read('src-tauri/tauri.professional.conf.json')).bundle?.windows?.nsis;
  const hooksPath = nsis?.installerHooks?.replace(/^\.\//, '');
  assert.equal(hooksPath, 'windows/installer-hooks.nsh', 'the Professional installer must take its hooks from windows/installer-hooks.nsh');
  const hooks = read(`src-tauri/${hooksPath}`);

  // What the app writes, read from the code that writes it: the Run entry of the installed
  // Professional, the old worker's startup file (versions before phase 3) and the data folder.
  const entryName = rustItem(read('src-tauri/src/background_rules.rs'), 'pub fn autostart_entry_name(').match(/if installed \{ "([^"]+)"\.to_owned\(\) \}/);
  assert.ok(entryName, 'autostart_entry_name no longer gives the installed Professional a fixed name: change this check and the hook together');
  const launcher = rustItem(read('../app/src/infrastructure/mcp_config.rs'), 'fn startup_launcher()');
  assert.match(launcher, /env::var_os\("APPDATA"\)/, 'the old startup file is still looked for under %APPDATA%');
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
  // Qamille installed, and it must keep its start at sign-in and its data.
  assert.ok(!/\b(Delete|DeleteRegValue|DeleteRegKey|RMDir)\b/i.test(before) && !before.includes(startupFile), 'the hook that runs first must not delete anything');

  const after = nshMacro(hooks, 'NSIS_HOOK_POSTUNINSTALL');
  // Each removal sits inside the If $UpdateMode <> 1 block (no ${EndIf} between the two): an /UPDATE keeps them.
  const inUpdateBlock = (what) => new RegExp(`\\$\\{If\\}\\s+\\$UpdateMode\\s*<>\\s*1(?:(?!\\$\\{EndIf\\})[\\s\\S])*?${what}`);
  const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  for (const key of ['Software\\Microsoft\\Windows\\CurrentVersion\\Run', 'Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run']) {
    assert.match(
      after,
      inUpdateBlock(`DeleteRegValue\\s+HKCU\\s+"${escape(key)}"\\s+"${escape(entryName[1])}"`),
      `the uninstall must remove "${entryName[1]}" from HKCU\\${key}, and not on an /UPDATE`,
    );
  }
  assert.ok(!/HKLM/.test(after), 'the app writes its start at sign-in only for the current user');
  assert.match(after, /ReadEnvStr\s+\$\d\s+APPDATA/, 'the old startup folder is found from %APPDATA%, as mcp_config.rs does');
  assert.match(
    after,
    inUpdateBlock(`Delete\\s+"\\$\\d\\\\${escape(startupFile)}"`),
    `${startupFile} of an earlier version must be deleted after the uninstall, and not on an /UPDATE`,
  );
  // The template's "Delete app data" only empties %APPDATA%\<identifier> and %LOCALAPPDATA%\<identifier>,
  // where Qamille keeps nothing: its data and credentials.key sit beside the program.
  assert.match(
    after,
    /\$\{If\}\s+\$DeleteAppDataCheckboxState\s*=\s*1\s+\$\{AndIf\}\s+\$UpdateMode\s*<>\s*1\s+\$\{AndIf\}\s+\$INSTDIR\s*!=\s*""\s+RMDir\s+\/r\s+"\$INSTDIR\\data"/,
    'the data folder must go only when the box is ticked, never on an /UPDATE, and never from an empty $INSTDIR',
  );
  assert.ok(!/RMDir\s+\/r\s+"\$INSTDIR"/.test(after), 'only the data folder is removed recursively, never the install folder');
  // The worker's startup file is never written again: nothing in the app puts it back.
  assert.ok(!/refresh_autostart/.test(hooks), 'the hooks no longer point to refresh_autostart, which is gone');
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

// phase 3, Task 1: checks 19-20
check('19. the close button hides, and one restore path brings the window back (rule 26)', () => {
  const main = read('src-tauri/src/main.rs');
  const background = read('src-tauri/src/background.rs');
  // A second start, the tray, a notification and a mail link all come back through background::restore.
  assert.match(main, /\.plugin\(tauri_plugin_single_instance::init\(background::second_instance\)\)/, 'the single-instance callback must be background::second_instance');
  assert.match(rustItem(background, 'pub fn second_instance('), /\brestore\(app\)/, 'a second start restores the window');
  // The close button hides only while hides_on_close() says so; otherwise the page's guards decide.
  const events = main.slice(main.indexOf('.on_window_event('), main.indexOf('.invoke_handler('));
  assert.match(
    events,
    /tauri::WindowEvent::CloseRequested \{ api, \.\. \} => \{\s*if background::hides_on_close\(\) \{\s*api\.prevent_close\(\);\s*background::hide\(window\.app_handle\(\)\);?\s*\} else \{\s*recovery::close_requested\(window\.app_handle\(\)\);?\s*\}/,
    'CloseRequested must prevent the close and hide only when background::hides_on_close(), and else go to recovery::close_requested',
  );
  assert.equal((main.match(/prevent_close\(\)/g) ?? []).length, 1, 'main.rs prevents a close in one place only');
  assert.match(events, /tauri::WindowEvent::Resized\(_\) => background::window_resized\(window\)/, 'a minimized window pauses its webview (background::window_resized)');
  assert.match(rustItem(read('src-tauri/src/recovery.rs'), 'pub fn close_seen('), /!crate::background::hides_on_close\(\)/, 'close_seen tells the page not to close when Rust only hid the window');
  assert.match(read('src/lib/window-close.ts'), /await call<boolean>\('close_seen'\)/, 'the page waits for close_seen before running its guards');
  // The webview is visible before the window shows, then the window comes to the front, on screen.
  const restore = rustItem(background, 'pub fn restore(');
  const at = (part) => {
    const index = restore.indexOf(part);
    assert.ok(index >= 0, `restore: no ${part}`);
    return index;
  };
  assert.ok(at('set_webview_visible(') < at('.show()') && restore.slice(at('set_webview_visible('), at('.show()')).includes('true'), 'restore makes the webview visible before show');
  assert.ok(at('.show()') < at('.unminimize()') && at('.unminimize()') < at('.set_focus()'), 'restore: show, then unminimize, then set_focus');
  assert.match(restore, /recovery::offer_reload_if_not_ready\(/, 'the first restore offers the reload the hidden window did not ask for');
  // Nothing else shows the main window: a reload or a dialog never pulls it out of the tray.
  const allowed = { 'native_feel.rs': 'pub fn show_main_window(', 'background.rs': 'pub fn restore(' };
  const shows = (text) => (text.match(/\b[A-Za-z_]\w*\.show\(\)/g) ?? []).length;
  for (const file of fs.readdirSync(path.join(desktop, 'src-tauri/src'), { recursive: true }).filter((f) => f.endsWith('.rs'))) {
    const text = read(path.join('src-tauri/src', file)).replace(/(^|\s)\/\/.*$/gm, '$1');
    if (!shows(text)) continue;
    const owner = allowed[path.basename(file)];
    assert.ok(owner, `${file} shows a window (.show()): only native_feel::show_main_window and background::restore may`);
    assert.equal(shows(rustItem(text, owner)), shows(text), `${file}: .show() outside ${owner}`);
  }
  // The tray: Show and Quit, with ids of its own (every menu event reaches every handler, the row menu's
  // too); Quit closes the window through the page's guards, never app.exit.
  assert.match(background, /MenuItem::with_id\(\s*app,\s*"tray:show",/, 'the tray menu has "tray:show"');
  assert.match(background, /MenuItem::with_id\(\s*app,\s*"tray:quit",/, 'the tray menu has "tray:quit"');
  assert.match(background, /"tray:show" => restore\(app\),\s*"tray:quit" => quit\(app\),\s*_ => \{\}/, 'the tray menu calls restore and quit, and ignores ids it does not own');
  const quit = rustItem(background, 'pub fn quit(');
  assert.ok(quit.includes('QUITTING.store(true') && /\.close\(\)/.test(quit) && !/\.exit\(/.test(quit), 'quit raises QUITTING and calls close(), never exit');
  const recovery = read('src-tauri/src/recovery.rs');
  // «Attendi» on «Qamille non si chiude», or Esc: «Esci» is over, so the next X hides again.
  assert.match(
    rustItem(recovery, 'pub fn close_requested('),
    /== Some\(false\);\s*if close \{[^}]*\} else \{\s*crate::background::close_settled\(\);?\s*\}/,
    'waiting on «Qamille non si chiude» lowers QUITTING (background::close_settled)',
  );
  // The recording question during «Esci» from the tray: the window comes back with it.
  const confirm = rustItem(read('src-tauri/src/commands/meetings.rs'), 'pub async fn meeting_confirm_close(');
  assert.ok(/background::restore\(/.test(confirm) && confirm.indexOf('background::restore(') < confirm.indexOf('MessageDialog'), 'meeting_confirm_close restores the window before asking');
  // Session end: every step first (the recording stops without waiting for the queue), then the queue
  // gets the rest of the time, then Windows may go on.
  assert.match(
    read('src/lib/window-close.ts'),
    /if \(last\) void saving\.then\(\(\) => undoQueue\?\.settle\(\)\)\.finally\(\(\) => call\('session_end_ready'\)/,
    'at session end the queue is waited for after the other steps, before session_end_ready',
  );
  const shell = read('src-tauri/src/window_shell.rs');
  assert.match(
    rustItem(shell, 'fn add_reload_entry('),
    /AppendMenuW\(menu, MF_STRING, crate::background_rules::SC_APP_QUIT as usize, quit_label\(italian\)\);\s*let _ = AppendMenuW\(menu, MF_SEPARATOR/,
    'the system menu offers «Esci da Qamille» right under «Chiudi»',
  );
  assert.match(
    shell,
    /WM_SYSCOMMAND if \(wparam\.0 as u32 & 0xFFF0\) == crate::background_rules::SC_APP_QUIT => \{[^}]*crate::background::quit\(&?app\)/,
    'Esci in the system menu calls background::quit',
  );
  // A hidden window never asks "Qamille non si è aperto" by itself: the 15 s watch and a page that did
  // not load read is_visible() first (the load's watch then asks at the first restore).
  for (const head of ['pub fn watch_startup(', 'pub fn page_not_loaded(']) {
    const body = rustItem(recovery, head);
    assert.ok(body.indexOf('is_visible()') >= 0 && body.indexOf('is_visible()') < body.indexOf('ask_once('), `${head.slice(7, -1)} asks only after reading is_visible()`);
  }
});

check('20. start at sign-in is Qamille in the tray, an HKCU Run value Qamille writes quoted, never a VBS (rule 27, audit Q22)', () => {
  const cargo = read('src-tauri/Cargo.toml');
  assert.match(cargo, /^tauri = \{ version = "2", features = \[[^\]]*"tray-icon"/m, 'tauri needs the tray-icon feature');
  // auto-launch (the autostart plugin's backend) writes the path unquoted, and HKLM when it may.
  assert.ok(!/tauri-plugin-autostart|auto-launch/.test(cargo + read('src-tauri/Cargo.lock')), 'no autostart plugin: Qamille writes its own entry');
  const rules = read('src-tauri/src/background_rules.rs');
  assert.match(rustItem(rules, 'pub fn run_command('), /format!\("\\"\{\w+\}\\" \{AUTOSTART_ARG\}"\)/, 'the Run value is "<exe>" --autostart, the executable in quotes');
  const background = read('src-tauri/src/background.rs');
  assert.match(background, /background_rules::autostart_entry_name\(installed\(\),/, 'the entry name comes from autostart_entry_name');
  assert.match(background, /background_rules::workspace_tag\(/, 'the entry name carries the stable tag of the data folder');
  const tagCode = ['pub fn workspace_tag(', 'pub fn fnv1a64('].map((head) => rustItem(rules, head)).join('') + rustItem(background, 'fn autostart_entry(');
  assert.ok(/fnv1a64\(/.test(tagCode) && !/DefaultHasher|Hasher/.test(tagCode), 'the tag is FNV-1a, not DefaultHasher, which changes between Rust releases');
  const startup = rustItem(background, 'mod startup {');
  assert.match(startup, /const RUN: &str = r"Software\\Microsoft\\Windows\\CurrentVersion\\Run";/, 'the entry lives in the Run key');
  assert.match(startup, /const APPROVED: &str = r"Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";/, 'Task Manager\'s switch is its twin under StartupApproved');
  assert.ok(/HKEY_CURRENT_USER/.test(startup) && !/HKEY_LOCAL_MACHINE/.test(background), 'only for the current user, never machine-wide');
  assert.match(rustItem(background, 'fn turn_on('), /background_rules::run_command\(/, 'turning it on writes run_command');
  assert.match(rustItem(background, 'pub async fn autostart_set('), /\bturn_on\(\)/, 'the switch in Settings turns it on through turn_on');
  const main = read('src-tauri/src/main.rs');
  const setup = rustItem(main, 'fn setup_app(');
  assert.match(
    setup,
    /if background_rules::stays_hidden\(&args, background::restarted_hidden\(\)\) \{\s*native_feel::prepare_main_window\(&main_window\);\s*native_feel::set_webview_visible\(&main_window, false\);?\s*\} else \{\s*native_feel::show_main_window\(&main_window\);?\s*\}/,
    'a start with AUTOSTART_ARG prepares the window and leaves it in the tray; a restart comes back as it was',
  );
  // A restart keeps the arguments of the first start: it says whether the window was on screen.
  assert.match(main, /recovery::wait_for_previous_instance\(\);\s*background::take_restart_state\(\);/, 'main reads the restart state before any thread starts');
  assert.match(rustItem(read('src-tauri/src/recovery.rs'), 'fn restart_app('), /crate::background::before_restart\(app\);[\s\S]*request_restart\(\)/, 'a restart says first whether the window was hidden');
  // Windows has the last word, also when Settings shows the switch: off in Task Manager stays off after an upgrade.
  assert.match(rustItem(background, 'fn status('), /follow_windows\(\)/, 'reading the state for Settings follows Windows');
  assert.match(rustItem(background, 'fn follow_windows('), /set_start_at_sign_in\(enabled\)/, 'what Windows has is what Qamille remembers');
  assert.match(setup, /if !demo \{(?:(?!\n    \})[\s\S])*?background::migrate_legacy_launcher\(/, 'setup_app migrates the old worker startup file, never in the demo');
  // A manual upgrade runs the old uninstaller without /UPDATE, which removes the entry: the installed
  // Professional puts it back at its next start, only if the person had turned it on in Qamille.
  assert.match(rustItem(background, 'pub fn migrate_legacy_launcher('), /put_back_after_upgrade\(/, 'the startup pass puts the entry back after a manual upgrade');
  assert.match(rustItem(background, 'fn put_back_after_upgrade('), /if !installed\(\) \|\| !preferences\(\)\.start_at_sign_in/, 'only for the installed Professional, and only when it was turned on in Qamille');
  // The app writes no startup file any more, and the page reaches start at sign-in only through app commands.
  const mcp = read('../app/src/infrastructure/mcp_config.rs');
  assert.ok(!/launcher_content|refresh_autostart|set_autostart/.test(mcp), 'mcp_config.rs must not write a startup file (launcher_content, refresh_autostart, set_autostart)');
  for (const file of ['capabilities/main.json', 'capabilities/reminder.json']) {
    assert.ok(!JSON.parse(read(`src-tauri/${file}`)).permissions.some((p) => /^(autostart|core:tray|core:menu):/.test(p)), `${file}: no autostart, tray or menu permission for the page`);
  }
  assert.ok(!/@tauri-apps\/plugin-autostart/.test(read('package.json')), 'no autostart package for the page');
});

// phase 3, Task 2: check 21

check('21. the saved placement is applied before the first show and only on a connected monitor (rule 29)', () => {
  const shell = read('src-tauri/src/window_shell.rs');
  const placement = read('src-tauri/src/placement.rs');
  // The decisions are pure (std only, tested with `rustc --test`), and the module is declared once.
  assert.ok(!/^\s*use\s+(tauri|serde|crate)\b/m.test(placement), 'placement.rs must stay free of Tauri, serde and the app');
  assert.match(read('src-tauri/src/main.rs'), /^mod placement;$/m, 'main.rs declares `mod placement;`');
  assert.ok(!/\bplacement::/.test(read('src-tauri/src/main.rs')), 'main.rs only declares the module: window_shell.rs does the rest');
  // The preferences file keeps it, and an older file without it still reads.
  const preferences = rustItem(shell, 'struct WindowPreferences {');
  assert.match(preferences, /#\[serde\([^\]]*\bdefault\b[^\]]*\)\]\s*placement:/, 'WindowPreferences.placement needs #[serde(default ...)]');
  // Applied by the startup preferences, which never show the window themselves.
  const startup = rustItem(shell, 'pub fn apply_startup_preferences(');
  assert.match(startup, /\bapply_placement\(window\b/, 'apply_startup_preferences must apply the saved placement');
  assert.ok(!/\.show\(/.test(startup), 'apply_startup_preferences must not show the window: the caller does, once it is painted');
  const feel = read('src-tauri/src/native_feel.rs');
  const preparing = ['prepare_main_window(', 'show_main_window('].find((name) => feel.includes(`pub fn ${name}`));
  assert.ok(preparing, 'native_feel.rs has no function that prepares the main window');
  const prepare = rustItem(feel, `pub fn ${preparing}`);
  assert.match(prepare, /\bapply_startup_preferences\(/, `${preparing.slice(0, -1)} must apply the startup preferences`);
  if (/\.show\(/.test(prepare)) {
    assert.ok(prepare.search(/\.show\(/) > prepare.search(/\bapply_startup_preferences\(/), 'the placement must be applied before show()');
  }
  // Only on a monitor that is connected now, with the work area (not the taskbar) as the limit.
  const apply = rustItem(shell, 'fn apply_placement(');
  assert.match(apply, /\.available_monitors\(\)/, 'apply_placement must ask for the connected monitors');
  assert.match(apply, /\.work_area\(\)/, 'apply_placement must use the work area of each monitor');
  assert.match(apply, /\bplacement::restore\(/, 'apply_placement must decide with placement::restore');
  assert.match(apply, /\bmin_logical_size\(/, 'apply_placement must pass the minimum size of the window config');
  assert.match(apply, /\.set_position\([\s\S]*\.set_size\(/, 'position first (a move to another scale settles), then size');
  assert.ok(!/\.show\(|\.maximize\(/.test(apply), 'apply_placement only places: it neither shows nor maximizes');
  // Maximized: the normal rectangle is applied and remembered, then the window is maximized, still hidden.
  assert.match(startup, /\bapply_placement\([\s\S]*\bremember_start_rectangle\([\s\S]*\.maximize\(\)/, 'the rectangle is applied and remembered before maximize()');
  // The saving is the window's own listener, registered from attach_main (no line in main.rs).
  assert.match(rustItem(shell, 'pub fn attach_main('), /\bwatch_placement\(/, 'attach_main must register the placement saver');
  const watch = rustItem(shell, 'fn watch_placement(');
  assert.match(watch, /\.on_window_event\(/, 'watch_placement registers a window event listener');
  for (const event of ['Moved', 'Resized', 'ScaleFactorChanged', 'CloseRequested', 'Destroyed']) {
    assert.ok(new RegExp(`WindowEvent::${event}\\b`).test(watch), `the placement saver must listen to ${event}`);
  }
  assert.match(watch, /Moved\([^)]*\)\s*\|[\s\S]*?ScaleFactorChanged\b[\s\S]*?=>\s*\{[\s\S]*?save_placement_soon\(\)/, 'a move, a resize and a scale change are saved after a delay');
  assert.match(watch, /CloseRequested\b[\s\S]*?=>\s*\{[\s\S]*?track_placement\([\s\S]*?\bsave_placement\(\)/, 'a close request saves at once, after taking in where the window is');
  assert.match(watch, /Destroyed\s*=>\s*save_placement\(\)/, 'a destroyed window saves at once');
  assert.match(shell, /PLACEMENT_DELAY: Duration = Duration::from_millis\(500\)/, 'moves and resizes are saved 500 ms after the last one');
  // Never from a minimized or hidden window.
  const track = rustItem(shell, 'fn track_placement(');
  assert.match(track, /\.is_minimized\(\)/, 'a minimized window is never saved');
  assert.match(track, /\.is_visible\(\)/, 'a hidden window is never saved');
  // A maximize moves the window before tao's flag changes: Windows is asked as well, or the screen-sized rectangle is saved.
  assert.match(rustItem(shell, 'fn is_maximized('), /\bis_zoomed\b/, 'is_maximized must ask Windows (IsZoomed) as well as tao');
  // The window config keeps centring it and keeping it in the work area where nothing is applied.
  const main = JSON.parse(read('src-tauri/tauri.conf.json')).app.windows.find((w) => w.label === 'main');
  assert.equal(main.center, true, 'main window needs center: true (no saved placement, or a monitor that is gone)');
  assert.equal(main.preventOverflow, true, 'main window needs preventOverflow: true');
  assert.ok(main.minWidth > 0 && main.minHeight > 0, 'minWidth and minHeight are the least a saved size may have');
});

// phase 3, Task 5: check 22
check('22. a saved file is opened or shown only through its token (rule 31)', () => {
  const rust = read('src-tauri/src/saved_files.rs');
  // The code without its tests and comments: an assertion on what it does is met by neither.
  const code = rust.slice(0, rust.indexOf('#[cfg(test)]')).replace(/(^|\s)\/\/.*$/gm, '$1');
  assert.ok(code.length > 0 && rust.includes('#[cfg(test)]'), 'saved_files.rs keeps its tests in a test module');
  // The page never names a path: each command takes the token, an unsigned number, and nothing else.
  for (const command of ['saved_file_open', 'saved_file_reveal']) {
    assert.match(code, new RegExp(String.raw`#\[tauri::command\]\s*pub async fn ${command}\(token: u64\) -> AppResult<\(\)>`), `${command} must take only token: u64`);
  }
  const main = read('src-tauri/src/main.rs');
  assert.match(main, /attachment_save,\s*saved_files::saved_file_open, saved_files::saved_file_reveal,/, 'both commands are registered after attachment_save');
  // Open goes through the shell, which asks about a file from the Internet zone; Show in folder through
  // the shell call that selects the file. No process is started with a command line that carries the
  // file: a name such as /select,… must never be read as an Explorer switch.
  assert.match(code, /open::that_detached\(path\)/, 'Open must use open::that_detached');
  assert.match(code, /SHOpenFolderAndSelectItems\(/, 'Show in folder must use SHOpenFolderAndSelectItems');
  assert.ok(!/explorer/i.test(code), 'saved_files.rs must not start explorer');
  assert.ok(!/Command::new|\.args?\(|std::process|\/select/.test(code), 'saved_files.rs must not build a command line');
  // What a save hands out is the token, and a save is the only way to get one.
  assert.match(code, /pub fn remember\(path: PathBuf\) -> u64/, 'remember(path) -> u64 is the one way in');
  assert.match(read('src-tauri/src/commands/document_preview.rs'), /pub async fn attachment_save\([^)]*\) -> AppResult<Option<SavedFile>>/, 'attachment_save returns the SavedFile with its token');
  assert.match(read('src-tauri/src/commands/mail.rs'), /pub struct ExportSaved \{[^}]*token: u64/, 'export_mail returns the token');
  // The page asks for the commands with the token alone, and from one place.
  const asking = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(path.join(desktop, dir), { withFileTypes: true })) {
      const file = `${dir}/${entry.name}`;
      if (entry.isDirectory()) walk(file);
      else if (/\.(ts|tsx)$/.test(entry.name) && /saved_file_(open|reveal)/.test(read(file))) asking.push(file);
    }
  };
  walk('src');
  assert.deepEqual(asking.sort(), ['src/lib/preview.ts', 'src/lib/saved-file.ts'], 'only the helper and the browser preview name the commands');
  assert.match(read('src/lib/saved-file.ts'), /call\(command, \{ token \}\)/, 'the helper calls the commands with { token } only');
});

// phase 3, Task 6: check 23
check('23. the mail rows get a native menu built by an app command (rule 33)', () => {
  // Rust builds and shows the menu; the page only says which entries (lib/row-menu.ts) and gets
  // back the chosen one. The page has no menu permission of its own.
  const menu = read('src-tauri/src/row_menu.rs');
  assert.match(rustItem(menu, 'pub async fn show_row_menu('), /\.popup_menu_at\(/, 'show_row_menu opens the menu with popup_menu_at, under the pointer');
  assert.match(read('src-tauri/src/main.rs'), /row_menu::show_row_menu/, 'show_row_menu must be an app command');
  const capability = JSON.parse(read('src-tauri/capabilities/main.json'));
  assert.ok(!capability.permissions.some((p) => /menu/.test(p)), 'no core:menu in main.json: the menu is built in Rust');
  // The row stops the event: no WebView2 menu on the rows, and the system menu stays shut.
  const list = read('src/components/MailList.tsx');
  assert.match(list, /onContextMenu: \(event: MouseEvent<HTMLElement>\) => openRowMenu\(event, email\)/, 'the mail row must handle contextmenu');
  assert.match(rustItem(list, 'function openRowMenu('), /event\.preventDefault\(\)/, 'the row menu handler must call preventDefault');
  assert.match(read('src/lib/window-shell.ts'), /if \(event\.defaultPrevented/, 'installWindowShell must leave a stopped contextmenu alone');
});

// phase 3, Task 3: checks 24-25

// phase 3, Task 4: check 26
const { isMailto, parseMailto } = await import(pathToFileURL(path.join(desktop, 'src/lib/mailto.ts')).href);
check("26. a mailto link opens Qamille's composer, and the installer registers exactly the keys the app removes (rule 32, audit Q10)", () => {
  // A message's mail link goes to the composer, never to open_link (another mail program). The web
  // links of a message still leave through open_link, so check 16 is unchanged.
  const html = read('src/components/MessageHtml.tsx');
  const openLink = rustItem(html, 'function openLink(');
  assert.match(
    openLink,
    /if \(isMailto\(address\)\) \{\s*const fields = parseMailto\(address\);\s*if \(fields\) requestCompose\(fields\);[\s\S]*?return;\s*\}\s*void openExternal\(/,
    'MessageHtml: a mailto: address goes to requestCompose(parseMailto(…)) and returns before openExternal',
  );
  assert.equal((html.match(/openExternal\(/g) ?? []).length, 1, 'MessageHtml calls openExternal in one place only');
  assert.ok(isMailto(externalAddress('MAILTO:a@b.it')) && !isMailto(externalAddress('https://example.com/mailto:a@b.it')), 'isMailto reads the scheme');
  assert.deepEqual(parseMailto(externalAddress('mailto:a@b.it?subject=x%0D%0ABcc:c@d.it')).bcc, [], 'a line break adds no recipient');
  // The navigation hooks of the main window send a mail link to the page as compose-mailto, a web link
  // to the browser; the reminder chip (open_outside false) opens neither. A mail address that is not a
  // whole link (over 8 KB) is dropped there, never handed to the browser.
  const feel = read('src-tauri/src/native_feel.rs');
  for (const head of ['pub fn navigation_guard(', 'pub fn new_window_guard(']) {
    assert.match(
      rustItem(feel, head),
      /if crate::launch_args::has_mail_scheme\((?:[^()]|\(\))*\) \{\s*crate::launch_args::from_navigation\(&app, [^;]+\);\s*\} else \{\s*open_in_browser\([^;]+\);\s*\}/,
      `${head.slice(7, -1)}: every mailto: address goes to launch_args::from_navigation, only a web one to open_in_browser`,
    );
  }
  const launch = read('src-tauri/src/launch_args.rs');
  assert.match(
    rustItem(launch, 'pub fn from_navigation('),
    /if !is_mail_link\(&link\) \{\s*return;\s*\}[\s\S]*\bdeliver\(/,
    'from_navigation drops a mail address that is not a whole link, and hands the others to deliver',
  );
  assert.match(rustItem(launch, 'fn deliver('), /\.emit_to\("main", "compose-mailto", \w+\)/, 'deliver emits compose-mailto to the main window');
  // A page that loads again (a reload, a crash) has no listener until it asks: the links wait for it.
  assert.match(
    rustItem(read('src-tauri/src/recovery.rs'), 'pub fn page_load('),
    /if payload\.event\(\) == PageLoadEvent::Started \{[^}]*crate::launch_args::page_started\(\);/,
    'a page that starts loading in the main window makes the links wait for its take_launch_mailto',
  );
  // Another program's link: at a cold start it waits for the page (not after a restart, which keeps the
  // first start's arguments); a second start's comes after background::restore.
  const mainRs = read('src-tauri/src/main.rs');
  const setup = rustItem(mainRs, 'fn setup_app(');
  assert.match(
    setup,
    /if background::restarted_hidden\(\)\.is_none\(\) \{\s*launch_args::route\(app\.handle\(\), &args\);\s*\}/,
    'setup_app routes the mailto: argument of a start that is not a restart',
  );
  assert.match(setup, /let args: Vec<String> = launch_args::effective\(std::env::args\(\)\.collect\(\)\);/, 'setup_app reads the arguments as launch_args::effective gives them');
  // A link Windows passed first takes no option after it: a caller that did not quote it
  // (`mailto:x" --worker "`) cannot start a worker or an MCP server instead of the composer.
  const start = rustItem(mainRs, 'fn main(');
  const effective = start.indexOf('let args = launch_args::effective(std::env::args().collect());');
  assert.ok(effective >= 0 && effective < start.indexOf('"--skill"'), 'main reads launch_args::effective before the --skill, --mcp and --worker branches');
  assert.ok(!/std::env::args(?:_os)?\(\)\.(?:nth|any)\(/.test(start), 'main picks its mode from launch_args::effective, never from the raw arguments');
  const second = rustItem(read('src-tauri/src/background.rs'), 'pub fn second_instance(');
  assert.ok(/\brestore\(app\);\s*crate::launch_args::route\(app, &args\);/.test(second), 'a second start restores the window, then routes its mailto: argument');
  assert.match(rustItem(launch, 'pub fn take_launch_mailto('), /\.take\(\)/, 'the page takes the waiting link');

  // Qamille never makes itself the default (Windows lets only the person choose): it registers what it
  // opens, for the current user only, and never writes the mailto key itself.
  const handler = read('src-tauri/src/mail_handler.rs');
  assert.ok(!/HKEY_LOCAL_MACHINE|HKLM/.test(handler), 'mail_handler.rs writes only under HKEY_CURRENT_USER');
  assert.match(handler, /HKEY_CURRENT_USER/, 'mail_handler.rs writes under HKEY_CURRENT_USER');
  assert.match(handler, /ms-settings:defaultapps\?registeredAppUser=/, "the choice is the person's, in Settings > Default apps");
  const constant = (name) => {
    const found = handler.match(new RegExp(`pub const ${name}: &str = r?"([^"]+)";`));
    assert.ok(found, `mail_handler.rs: no pub const ${name}`);
    return found[1];
  };
  const [classes, registered, apps] = ['CLASSES', 'REGISTERED', 'APPS'].map(constant);
  const [name, progId, key] = ['INSTALLED_NAME', 'INSTALLED_PROG_ID', 'INSTALLED_KEY'].map(constant);
  assert.equal(classes, 'Software\\Classes');
  assert.equal(registered, 'Software\\RegisteredApplications');
  // What the app writes for a copy (`entries`), and therefore what the installer must write for the
  // installed Professional, with the fixed names of `handler_keys(true, …)`.
  const entries = rustItem(handler, 'pub fn entries(');
  const shapes = [
    /\(prog\.clone\(\), String::new\(\), keys\.display\.clone\(\)\)/,
    /\(format!\(r"\{prog\}\\DefaultIcon"\), String::new\(\), icon\.clone\(\)\)/,
    /\(format!\(r"\{prog\}\\shell\\open\\command"\), String::new\(\), command\)/,
    /\(capabilities\.clone\(\), "ApplicationName"\.into\(\), keys\.display\.clone\(\)\)/,
    /\(capabilities\.clone\(\), "ApplicationDescription"\.into\(\), keys\.display\.clone\(\)\)/,
    /\(capabilities\.clone\(\), "ApplicationIcon"\.into\(\), icon\)/,
    /\(format!\(r"\{capabilities\}\\URLAssociations"\), "mailto"\.into\(\), keys\.prog_id\.clone\(\)\)/,
    /\(hive\.registered\.to_owned\(\), keys\.name\.clone\(\), capabilities\)/,
  ];
  for (const shape of shapes) assert.match(entries, shape, `entries no longer writes ${shape}: change this check and the hook together`);
  assert.equal((entries.match(/^\s*\(/gm) ?? []).length, shapes.length, 'entries writes exactly these values');
  assert.match(entries, /let command = format!\("\\"\{exe\}\\" \\"%1\\""\);/, 'the command is "<exe>" "%1", both in quotes');
  assert.match(entries, /let icon = format!\("\\"\{exe\}\\",0"\);/, 'the icon is "<exe>",0');
  assert.match(entries, /let prog = format!\(r"\{\}\\\{\}", hive\.classes, keys\.prog_id\);/, 'the ProgID lives under Software\\Classes');
  assert.match(rustItem(handler, 'fn capabilities_key('), /format!\(r"\{\}\\\{\}\\Capabilities", hive\.apps, keys\.key\)/, 'the Capabilities key is <APPS>\\<key>\\Capabilities');
  assert.match(
    rustItem(handler, 'pub fn handler_keys('),
    /if installed \{\s*HandlerKeys \{\s*name: INSTALLED_NAME\.into\(\),\s*display: INSTALLED_NAME\.into\(\),\s*prog_id: INSTALLED_PROG_ID\.into\(\),\s*key: INSTALLED_KEY\.into\(\),?\s*\}/,
    'handler_keys(true, …) gives the fixed names',
  );
  const exe = '<exe>';
  const prog = `${classes}\\${progId}`;
  const capabilities = `${apps}\\${key}\\Capabilities`;
  const expected = [
    [prog, '', name],
    [`${prog}\\DefaultIcon`, '', `"${exe}",0`],
    [`${prog}\\shell\\open\\command`, '', `"${exe}" "%1"`],
    [capabilities, 'ApplicationName', name],
    [capabilities, 'ApplicationDescription', name],
    [capabilities, 'ApplicationIcon', `"${exe}",0`],
    [`${capabilities}\\URLAssociations`, 'mailto', progId],
    [registered, name, capabilities],
  ];
  const hooks = read('src-tauri/windows/installer-hooks.nsh');
  const quoted = '"((?:\\$\\\\"|[^"])*)"';
  const unquote = (text) => text.replaceAll('$\\"', '"').replaceAll('$INSTDIR\\${MAINBINARYNAME}.exe', exe);
  const install = nshMacro(hooks, 'NSIS_HOOK_POSTINSTALL');
  const writes = [...install.matchAll(new RegExp(`^\\s*WriteRegStr\\s+(\\w+)\\s+${quoted}\\s+${quoted}\\s+${quoted}\\s*$`, 'gm'))];
  assert.ok(writes.every((write) => write[1] === 'HKCU'), 'the installer writes the mailto keys under HKCU only');
  assert.deepEqual(
    writes.map((write) => write.slice(2).map(unquote)),
    expected,
    'NSIS_HOOK_POSTINSTALL writes exactly what mail_handler.rs writes for the installed Professional, in its order',
  );
  assert.ok(!/\bWriteReg(?!Str\b)\w*/.test(install) && !/\bDeleteReg/.test(install), 'the install hook writes only those strings');
  const announce = /System::Call "shell32::SHChangeNotify\(i,i,i,i\) \(0x08000000, 0x1000, 0, 0\)"/;
  assert.match(install, announce, 'the install hook tells Windows the associations changed (SHCNE_ASSOCCHANGED)');
  // The uninstaller removes what the app's «Smetti» removes, and not on an /UPDATE.
  const uninstall = nshMacro(hooks, 'NSIS_HOOK_POSTUNINSTALL');
  const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const inUpdateBlock = (what) => new RegExp(`\\$\\{If\\}\\s+\\$UpdateMode\\s*<>\\s*1(?:(?!\\$\\{EndIf\\})[\\s\\S])*?${what}`);
  for (const [what, removal] of [
    ['the RegisteredApplications value', `DeleteRegValue\\s+HKCU\\s+"${escape(registered)}"\\s+"${escape(name)}"`],
    ['the ProgID', `DeleteRegKey\\s+HKCU\\s+"${escape(prog)}"`],
    ["the copy's key with its Capabilities", `DeleteRegKey\\s+HKCU\\s+"${escape(`${apps}\\${key}`)}"`],
  ]) {
    assert.match(uninstall, inUpdateBlock(removal), `the uninstall must remove ${what}, and not on an /UPDATE`);
  }
  const removal = rustItem(handler, 'pub fn unregister(');
  assert.ok(
    /delete_value\(hive\.registered, &keys\.name\)/.test(removal) &&
      /delete_tree\(&format!\(r"\{\}\\\{\}", hive\.classes, keys\.prog_id\)\)/.test(removal) &&
      /delete_tree\(&format!\(r"\{\}\\\{\}", hive\.apps, keys\.key\)\)/.test(removal),
    "unregister removes the RegisteredApplications value, the ProgID and the copy's key, as the uninstaller does",
  );
  assert.ok(!/HKLM/.test(uninstall) && !/Classes\\mailto\b/i.test(hooks + handler), 'neither the app nor the installer writes or removes the mailto key itself');
});

// phase 3, Task 8: check 27

if (failures.length) {
  console.error(`\n${failures.length} native-feel check(s) failed. See docs/aspetto-nativo.md.`);
  process.exit(1);
}
console.log('\nnative feel: all checks pass');
