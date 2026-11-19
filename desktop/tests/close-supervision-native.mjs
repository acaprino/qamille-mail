// Closing the main window when its page cannot answer (src-tauri/src/recovery.rs, close_requested;
// docs/aspetto-nativo.md rule 21). The page's close guards decide every close, so a stuck page would
// leave the X and Alt+F4 doing nothing. Here the page is frozen, the window gets SC_CLOSE, and after
// CLOSE_ANSWER (3 s) a task dialog owned by the main window offers "Wait" / "Close Qamille";
// "Close Qamille" ends the process. A normal close (live page) raises no dialog: that is step 7 of
// webview-recovery-native.mjs.
// Windows only, on the built exe (QAMILLE_NATIVE_EXE overrides), on a disposable Sample workspace
// under checks/. Close Qamille first. Run: node tests/close-supervision-native.mjs (from desktop/).
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { delay, launchNative } from './support/native-app.mjs';

const run = promisify(execFile);
const WM_SYSCOMMAND = 0x0112;
const SC_CLOSE = 0xf060;
const TDM_CLICK_BUTTON = 0x0466;
const SECOND_BUTTON = 1001; // rfd OkCancelCustom: the second button, "Close Qamille"
const script = path.join(import.meta.dirname, 'support/win-windows.ps1');

async function win(action, target, message = 0, wparam = 0) {
  const { stdout } = await run('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script,
    action, String(target), String(message), String(wparam),
  ], { windowsHide: true });
  return action === 'post' ? stdout.trim() : [JSON.parse(stdout.trim() || '[]')].flat();
}

async function until(what, test, timeout = 20000) {
  const end = Date.now() + timeout;
  for (;;) {
    const value = await test().catch(() => undefined);
    if (value) return value;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await delay(250);
  }
}

const mainWindowOf = async (pid) =>
  (await win('windows', pid)).find((w) => w.cls !== '#32770' && w.title.startsWith('Qamille'));
const dialogOf = async (pid, owner) =>
  (await win('windows', pid)).find((w) => w.cls === '#32770' && w.owner === owner);

const session = await launchNative('close-supervision');
const { child, data } = session;
let passed = false;
try {
  const main = await until('the main window', () => mainWindowOf(child.pid));
  // The page's thread is busy for 60 s: it cannot acknowledge the close request while the test
  // looks for the dialog (each window query compiles its PowerShell helper, seconds each).
  void session.page.evaluate(() => { const end = Date.now() + 60000; while (Date.now() < end); }).catch(() => {});
  await delay(500);
  const asked = Date.now();
  await win('post', main.hwnd, WM_SYSCOMMAND, SC_CLOSE);
  const dialog = await until('the close dialog', () => dialogOf(child.pid, main.hwnd), 15000);
  const after = Date.now() - asked;
  assert.match(dialog.title, /does not close/, `dialog title ${dialog.title}`);
  assert.ok(after >= 2500, `asked after ${after} ms, before the page had its time to answer`);
  const windows = await win('windows', child.pid);
  const owner = windows.find((w) => w.hwnd === main.hwnd);
  assert.ok(owner, `the main window is gone: ${JSON.stringify(windows)}`);
  assert.equal(owner.enabled, false, 'the main window stays usable under the dialog');
  await win('post', dialog.hwnd, TDM_CLICK_BUTTON, SECOND_BUTTON);
  await until('Qamille to exit', async () => child.exitCode !== null, 15000);
  passed = true;
  console.log('ok   a frozen page gets "Qamille does not close", and "Close Qamille" ends the app');
} finally {
  await session.browser?.close().catch(() => {});
  if (child.exitCode === null) child.kill();
}
await delay(2000);
await fs.rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 }).catch(() => {});
if (!passed) process.exit(1);
