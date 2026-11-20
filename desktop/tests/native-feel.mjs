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
const { isHistoryButton, isStrayDrop, opensLinkElsewhere } = await import(pathToFileURL(path.join(desktop, 'src/lib/browser-guards.ts')).href);
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

if (failures.length) {
  console.error(`\n${failures.length} native-feel check(s) failed. See docs/aspetto-nativo.md.`);
  process.exit(1);
}
console.log('\nnative feel: all checks pass');
