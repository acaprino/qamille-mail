// Qamille in the background, on the release exe (docs/aspetto-nativo.md, rules 26 and 27): the X
// puts the window in the tray with its page paused, a second start and a restore bring it back in
// front, a minimized window pauses its page too, «Esci» in the system menu quits, and a start with
// --autostart stays in the tray, ready, without asking anything.
// Run with Qamille closed: node tests/background-native.mjs   (from desktop/)
// Sample workspace only, in a folder under checks/ (support/native-app.mjs). Nothing is written to the
// registry: --autostart is passed by hand, the start-at-sign-in setting is never touched.
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { delay, executable, launchNative, root } from './support/native-app.mjs';

const run = promisify(execFile);
const WM_SYSCOMMAND = 0x0112;
const SC_CLOSE = 0xf060;
const SC_MINIMIZE = 0xf020;
const SC_RESTORE = 0xf120;
const SC_APP_QUIT = 0x5c20; // background_rules.rs
const script = path.join(import.meta.dirname, 'support/win-windows.ps1');

async function win(action, target, message = 0, wparam = 0) {
  const { stdout } = await run('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script,
    action, String(target), String(message), String(wparam),
  ], { windowsHide: true });
  if (action === 'post') return stdout.trim();
  if (action === 'foreground') return Number(stdout.trim());
  return [JSON.parse(stdout.trim() || '[]')].flat();
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

/** The visible main window of the process, if any (dialogs are #32770). */
const mainWindowOf = async (pid) => (await win('windows', pid)).find((w) => w.cls !== '#32770' && w.title.startsWith('Qamille'));
const visibility = (session) => session.page.evaluate(() => document.visibilityState);
const exited = (child) => new Promise((resolve) => (child.exitCode !== null ? resolve() : child.once('exit', resolve)));

let failed = false;
const step = async (name, body) => {
  try {
    await body();
    console.log(`ok   ${name}`);
  } catch (error) {
    failed = true;
    console.error(`FAIL ${name}\n     ${String(error?.stack || error).split('\n').join('\n     ')}`);
    throw error;
  }
};

// Part 1: the close button, the way back, minimize and «Esci».
{
  const session = await launchNative('background', { background: { close_to_tray: true, tray_notice_shown: true } });
  const { child, data } = session;
  try {
    const main = await until('the main window', () => mainWindowOf(child.pid));
    await until('the page drawn', async () => (await visibility(session)) === 'visible');

    await step('the X puts the window in the tray: Qamille keeps running and its page is hidden', async () => {
      await win('post', main.hwnd, WM_SYSCOMMAND, SC_CLOSE);
      await until('the window hidden', async () => !(await mainWindowOf(child.pid)));
      await until('the page hidden', async () => (await visibility(session)) === 'hidden');
      await delay(2000);
      assert.equal(child.exitCode, null, 'Qamille must still run');
      assert.equal((await win('windows', child.pid)).length, 0, 'no window and no dialog of Qamille is visible');
    });

    await step('a second start brings the window back, drawn and in front', async () => {
      const second = spawn(executable(), [], {
        cwd: root, windowsHide: true, stdio: 'ignore',
        env: { ...process.env, QAMILLE_DATA: data, QAMILLE_DEMO: '1', WEBVIEW2_USER_DATA_FOLDER: path.join(data, 'webview') },
      });
      await until('the second start to hand over and exit', async () => second.exitCode !== null, 15000);
      const back = await until('the window visible again', () => mainWindowOf(child.pid));
      assert.equal(back.hwnd, main.hwnd, 'the same window comes back');
      await until('the page visible', async () => (await visibility(session)) === 'visible');
      await until('the window in front', async () => (await win('foreground', 0)) === main.hwnd, 5000);
    });

    await step('a minimized window pauses its page, and a restored one resumes it', async () => {
      await win('post', main.hwnd, WM_SYSCOMMAND, SC_MINIMIZE);
      await until('the page hidden while minimized', async () => (await visibility(session)) === 'hidden');
      await win('post', main.hwnd, WM_SYSCOMMAND, SC_RESTORE);
      await until('the page visible again', async () => (await visibility(session)) === 'visible');
    });

    await step('«Esci» in the system menu quits Qamille, also from the tray', async () => {
      const entry = (await win('menu', main.hwnd)).find((e) => e.id === SC_APP_QUIT);
      assert.equal(entry?.text, 'Quit Qamille', 'the system menu offers «Quit Qamille»');
      await win('post', main.hwnd, WM_SYSCOMMAND, SC_CLOSE);
      await until('the window hidden', async () => !(await mainWindowOf(child.pid)));
      // What Alt+Space and a click on the entry send.
      await win('post', main.hwnd, WM_SYSCOMMAND, SC_APP_QUIT);
      await Promise.race([exited(child), delay(10000)]);
      assert.notEqual(child.exitCode, null, 'Qamille must end within 10 s of «Esci»');
    });
  } finally {
    await session.browser?.close().catch(() => {});
    if (child.exitCode === null) child.kill();
  }
  await delay(2000);
  await fs.rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 }).catch(() => {});
}

// Part 2: a start at sign-in stays in the tray, ready, and asks nothing.
{
  const session = await launchNative('background-autostart', { background: { close_to_tray: true, tray_notice_shown: true }, args: ['--autostart'] });
  const { child, data } = session;
  try {
    await step('a start with --autostart stays in the tray, draws its page, and opens no dialog', async () => {
      // Past the 15 s watch of recovery.rs: a hidden window never asks «Qamille did not open».
      await delay(20000);
      assert.equal(child.exitCode, null, 'Qamille must run');
      assert.deepEqual(await win('windows', child.pid), [], 'no visible window or dialog');
      // App's first effect calls frontend_ready after its first render (App.tsx).
      assert.ok(await session.page.evaluate(() => !!document.querySelector('.compose-main')), 'the page rendered while hidden');
      const log = await fs.readFile(path.join(data, 'native.log'), 'utf8').catch(() => '');
      assert.ok(!/did not open|failed/i.test(log), `native.log: ${log}`);
    });
    await step('the first restore shows the window already drawn, still without a dialog', async () => {
      await session.invoke('restore_main_window');
      const main = await until('the window visible', () => mainWindowOf(child.pid));
      await until('the page visible', async () => (await visibility(session)) === 'visible');
      await delay(1500);
      assert.ok(!(await win('windows', child.pid)).some((w) => w.cls === '#32770'), 'no dialog after the restore');
      await win('post', main.hwnd, WM_SYSCOMMAND, SC_APP_QUIT);
      await Promise.race([exited(child), delay(10000)]);
      assert.notEqual(child.exitCode, null, 'Qamille must end within 10 s of «Esci»');
    });
  } finally {
    await session.browser?.close().catch(() => {});
    if (child.exitCode === null) child.kill();
  }
  await delay(2000);
  await fs.rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 }).catch(() => {});
}

if (failed) process.exit(1);
console.log('\nbackground: all checks pass');
