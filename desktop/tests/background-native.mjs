// Qamille in the background, on the release exe (docs/aspetto-nativo.md, rules 26 and 27): the X
// puts the window in the tray with its page paused, a second start and a restore bring it back in
// front, a minimized window pauses its page too, «Esci» in the system menu quits, and a start with
// --autostart stays in the tray, ready, without asking anything.
// Run with Qamille closed: node tests/background-native.mjs   (from desktop/)
// Sample workspace only, in a folder under checks/ (support/native-app.mjs). Nothing is written to the
// registry: --autostart is passed by hand, the start-at-sign-in setting is never touched.
// Whether the page is hidden is read from the page's own log of `visibilitychange`, written while
// DevTools is detached: a page with Playwright attached reports `visible` even in a hidden window
// (attaching turns a hidden page visible again), so a live `document.visibilityState` proves nothing.
import { chromium } from '@playwright/test';
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
/** What the person can see of the process: its windows and dialogs. Not the 16×16 message windows of
 * tao («Tao Thread Event Target») and of the single-instance plugin, which Windows counts as visible
 * but never draws. */
const shown = async (pid) => (await win('windows', pid)).filter((w) => w.cls === '#32770' || w.title.startsWith('Qamille'));
const exited = (child) => new Promise((resolve) => (child.exitCode !== null ? resolve() : child.once('exit', resolve)));

/** Starts the page's log of visibility changes (kept across a detach). */
const watchVisibility = (session) => session.page.evaluate(() => {
  if (window.__visibility) return;
  window.__visibility = [[Date.now(), document.visibilityState]];
  document.addEventListener('visibilitychange', () => window.__visibility.push([Date.now(), document.visibilityState]));
});
/** DevTools off the page, so that it can be hidden. */
const detach = async (session) => { await session.browser.close().catch(() => {}); session.page = undefined; };
/** DevTools back on the page; returns when, since attaching makes the page visible again. */
async function attach(session) {
  const at = Date.now();
  session.browser = await chromium.connectOverCDP(`http://127.0.0.1:${session.port}`);
  session.page = session.browser.contexts()[0].pages().find((p) => p.url().startsWith('http://tauri.localhost/'));
  assert.ok(session.page, 'the app page is gone');
  return at;
}
/** The page's visibility at `time`, from its own log (read with DevTools attached again). */
async function visibilityAt(session, time) {
  const changes = await session.page.evaluate(() => window.__visibility);
  return changes.filter(([at]) => at <= time).at(-1)?.[1];
}
/** Acts with DevTools detached, waits, and returns the page's visibility just before reattaching. */
async function visibilityAfter(session, act, wait = 3000) {
  await detach(session);
  await act();
  await delay(wait);
  const at = await attach(session);
  return visibilityAt(session, at - 1);
}
const invoke = (session, command, args = {}) =>
  session.page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });

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
    await until('the page drawn', async () => (await session.page.evaluate(() => document.visibilityState)) === 'visible');
    await watchVisibility(session);

    await step('the X puts the window in the tray: Qamille keeps running and its page is hidden', async () => {
      const state = await visibilityAfter(session, async () => {
        await win('post', main.hwnd, WM_SYSCOMMAND, SC_CLOSE);
        await until('the window hidden', async () => !(await mainWindowOf(child.pid)));
      });
      assert.equal(state, 'hidden', 'the page of a window in the tray must be hidden');
      assert.equal(child.exitCode, null, 'Qamille must still run');
      assert.deepEqual(await shown(child.pid), [], 'no window and no dialog of Qamille is visible');
    });

    await step('a second start brings the window back, drawn and in front', async () => {
      const state = await visibilityAfter(session, async () => {
        const second = spawn(executable(), [], {
          cwd: root, windowsHide: true, stdio: 'ignore',
          env: { ...process.env, QAMILLE_DATA: data, QAMILLE_DEMO: '1', WEBVIEW2_USER_DATA_FOLDER: path.join(data, 'webview') },
        });
        await until('the second start to hand over and exit', async () => second.exitCode !== null, 15000);
        const back = await until('the window visible again', () => mainWindowOf(child.pid));
        assert.equal(back.hwnd, main.hwnd, 'the same window comes back');
        await until('the window in front', async () => (await win('foreground', 0)) === main.hwnd, 5000);
      }, 1500);
      assert.equal(state, 'visible', 'the page of the window that came back must be visible');
    });

    await step('a minimized window pauses its page, and a restored one resumes it', async () => {
      const minimized = await visibilityAfter(session, () => win('post', main.hwnd, WM_SYSCOMMAND, SC_MINIMIZE));
      assert.equal(minimized, 'hidden', 'the page of a minimized window must be hidden');
      const restored = await visibilityAfter(session, () => win('post', main.hwnd, WM_SYSCOMMAND, SC_RESTORE), 1500);
      assert.equal(restored, 'visible', 'the page of a restored window must be visible');
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
      // DevTools off at once: attached, it would make the hidden page visible (see the top).
      await watchVisibility(session);
      await detach(session);
      // Past the 15 s watch of recovery.rs: a hidden window never asks «Qamille did not open».
      await delay(20000);
      assert.equal(child.exitCode, null, 'Qamille must run');
      assert.deepEqual(await shown(child.pid), [], 'no visible window or dialog');
      const at = await attach(session);
      assert.equal(await visibilityAt(session, at - 1), 'hidden', 'the page of a start in the tray must be hidden');
      // App's first effect calls frontend_ready after its first render (App.tsx).
      assert.ok(await session.page.evaluate(() => !!document.querySelector('.compose-main')), 'the page rendered while hidden');
      const log = await fs.readFile(path.join(data, 'native.log'), 'utf8').catch(() => '');
      assert.ok(!/did not open|failed/i.test(log), `native.log: ${log}`);
    });
    await step('the first restore shows the window already drawn, still without a dialog', async () => {
      await invoke(session, 'restore_main_window');
      const main = await until('the window visible', () => mainWindowOf(child.pid));
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
