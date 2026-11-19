// WebView2 failures in the real app (docs/aspetto-nativo.md rule 21, src-tauri/src/recovery.rs):
// the app answers, never WebView2's error page, and never leaves a dead page.
//   1. the page is the embedded frontend (http://tauri.localhost), not the dev server;
//   2. the system menu has "Reload window", and it reloads the live page after it saved;
//   3. a navigation away from the app is refused without a dialog;
//   4. ending the page's renderer process reloads the page by itself, three times in two minutes;
//   5. the fourth time a Windows message box owned by the main window asks; while it is open the
//      main window is disabled, and "Reload the window" reloads and gives the main window back;
//   6. ending WebView2's browser process asks to restart; "Restart Qamille" starts a new Qamille that opens.
//   7. closing that Qamille normally exits without a recovery dialog.
// Windows only, on the built exe (QAMILLE_NATIVE_EXE overrides), on a disposable Sample workspace
// under checks/. Close Qamille first. Run: node tests/webview-recovery-native.mjs (from desktop/).
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { chromium } from '@playwright/test';
import { delay, executable, launchNative } from './support/native-app.mjs';

const run = promisify(execFile);
const WM_SYSCOMMAND = 0x0112;
const SC_APP_RELOAD = 0x5c10; // shell_rules.rs
// The recovery dialogs are task dialogs with buttons named by their action (rfd's
// OkCancelCustom): TDM_CLICK_BUTTON presses the first one, "Reload the window" / "Restart Qamille".
const TDM_CLICK_BUTTON = 0x0466;
const FIRST_BUTTON = 1000;
const APP = 'http://tauri.localhost/';
const script = path.join(import.meta.dirname, 'support/win-windows.ps1');

async function win(action, target, message = 0, wparam = 0) {
  const { stdout } = await run('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script,
    action, String(target), String(message), String(wparam),
  ], { windowsHide: true });
  return action === 'post' ? stdout.trim() : [JSON.parse(stdout.trim() || '[]')].flat();
}

async function processes(name) {
  const { stdout } = await run('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    `@(Get-CimInstance Win32_Process -Filter "Name='${name}'" | Select-Object ProcessId, CommandLine) | ConvertTo-Json -Compress`,
  ], { windowsHide: true, maxBuffer: 1 << 24 });
  return [JSON.parse(stdout.trim() || '[]')].flat();
}

/** The WebView2 processes of this workspace's profile (its folder is on their command line). */
async function webviewProcesses(data) {
  const profile = path.join(data, 'webview').toLowerCase();
  return (await processes('msedgewebview2.exe')).filter((p) => (p.CommandLine ?? '').toLowerCase().includes(profile));
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

/** A new DevTools connection to the page once the app has drawn it again. */
async function drawnPage(port) {
  return until('the app page to be drawn again', async () => {
    const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 3000 });
    const page = browser.contexts()[0]?.pages().find((p) => p.url().startsWith(APP));
    const drawn = page && await page.evaluate(() =>
      !!window.__TAURI_INTERNALS__ && !window.__beforeFailure && (document.getElementById('app')?.childElementCount ?? 0) > 0);
    if (drawn) return { browser, page };
    await browser.close().catch(() => {});
    return undefined;
  });
}

async function markPage(page) {
  await page.evaluate(() => { window.__beforeFailure = true; });
}

const mainWindowOf = async (pid) =>
  (await win('windows', pid)).find((w) => w.cls !== '#32770' && w.title.startsWith('Qamille'));
const dialogOf = async (pid, owner) =>
  (await win('windows', pid)).find((w) => w.cls === '#32770' && w.owner === owner);

const session = await launchNative('webview-recovery');
const { child, data, port } = session;
let current = { browser: session.browser, page: session.page };
let restartedPid = null;
const results = [];
const step = async (name, body) => {
  await body();
  results.push(name);
  console.log(`ok   ${name}`);
};

try {
  await step('1. the page is the embedded frontend, not the dev server', async () => {
    assert.ok(session.page.url().startsWith(APP), `page URL ${session.page.url()}`);
  });

  const main = await until('the main window', () => mainWindowOf(child.pid));

  await step('2. "Reload window" in the system menu reloads the live page after it saved', async () => {
    const entry = await until('the reload entry in the page language', async () =>
      (await win('menu', main.hwnd)).find((e) => e.id === SC_APP_RELOAD && e.text === 'Reload window'));
    assert.ok(entry);
    await markPage(current.page);
    const started = Date.now();
    await win('post', main.hwnd, WM_SYSCOMMAND, SC_APP_RELOAD);
    await current.page.waitForFunction(() => !window.__beforeFailure && (document.getElementById('app')?.childElementCount ?? 0) > 0, null, { timeout: 15000 });
    const took = Date.now() - started;
    // Without an answer Rust never reloads by itself (it asks after RELOAD_WAIT, 10 s): a reload
    // with no dialog means the page saved and answered reload_ready.
    assert.ok(took < 9500, `the reload took ${took} ms: the page did not answer reload-requested`);
    assert.equal(await dialogOf(child.pid, main.hwnd), undefined, 'the reload asked instead of reloading');
  });

  await step('3. a navigation away from the app is refused, without a dialog', async () => {
    await current.page.evaluate(() => { window.__beforeFailure = true; location.href = 'about:blank'; });
    await delay(1500);
    assert.ok(current.page.url().startsWith(APP), `navigated to ${current.page.url()}`);
    assert.equal(await current.page.evaluate(() => window.__beforeFailure), true, 'the page was replaced');
    assert.equal(await dialogOf(child.pid, main.hwnd), undefined, 'a cancelled navigation opened a dialog');
  });

  const killRenderers = async () => {
    const renderers = (await webviewProcesses(data)).filter((p) => /--type=renderer/.test(p.CommandLine));
    assert.ok(renderers.length > 0, 'no renderer process found for this profile');
    for (const { ProcessId } of renderers) {
      try { process.kill(ProcessId); } catch { /* already gone */ }
    }
  };

  await step('4. a killed page process reloads by itself, three times in two minutes', async () => {
    for (let time = 1; time <= 3; time++) {
      await markPage(current.page);
      await current.browser.close().catch(() => {});
      await killRenderers();
      current = await drawnPage(port);
      assert.equal(await dialogOf(child.pid, main.hwnd), undefined, `reload ${time} asked instead of reloading`);
    }
  });

  await step('5. the fourth time a modal message box asks, and "Reload the window" gives the window back', async () => {
    await markPage(current.page);
    await current.browser.close().catch(() => {});
    await killRenderers();
    const dialog = await until('the recovery message box', () => dialogOf(child.pid, main.hwnd));
    assert.match(dialog.title, /closed/, `dialog title ${dialog.title}`);
    assert.equal((await mainWindowOf(child.pid)).enabled, false, 'the main window stays usable under the dialog');
    await win('post', dialog.hwnd, TDM_CLICK_BUTTON, FIRST_BUTTON);
    current = await drawnPage(port);
    assert.equal((await mainWindowOf(child.pid)).enabled, true, 'the main window is still disabled');
  });

  await step('6. a killed WebView2 browser process asks to restart, and "Restart Qamille" opens a new Qamille', async () => {
    await current.browser.close().catch(() => {});
    const engine = (await webviewProcesses(data)).find((p) => !/--type=/.test(p.CommandLine));
    assert.ok(engine, 'no WebView2 browser process found for this profile');
    process.kill(engine.ProcessId);
    const dialog = await until('the restart message box', () => dialogOf(child.pid, main.hwnd));
    assert.match(dialog.title, /restart/, `dialog title ${dialog.title}`);
    await win('post', dialog.hwnd, TDM_CLICK_BUTTON, FIRST_BUTTON);
    await until('the old process to exit', async () => child.exitCode !== null, 20000);
    const name = path.basename(executable());
    const restarted = await until('the restarted process', async () =>
      (await processes(name)).find((p) => p.ProcessId !== child.pid && !/--(mcp|worker)/.test(p.CommandLine ?? '')));
    restartedPid = restarted.ProcessId;
    current = await drawnPage(port);
    assert.ok(await mainWindowOf(restartedPid), 'the restarted Qamille has no window');
  });

  await step('7. closing Qamille normally ends WebView2 without any recovery dialog', async () => {
    await current.page.evaluate(() => window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' })).catch(() => {});
    await current.browser.close().catch(() => {});
    const name = path.basename(executable());
    const end = Date.now() + 15000;
    for (;;) {
      const dialog = (await win('windows', restartedPid).catch(() => [])).find((w) => w.cls === '#32770');
      assert.equal(dialog, undefined, `a dialog opened while closing: ${dialog?.title}`);
      if (!(await processes(name)).some((p) => p.ProcessId === restartedPid)) break;
      assert.ok(Date.now() < end, 'Qamille did not exit after its window closed');
      await delay(200);
    }
    restartedPid = null;
  });
} finally {
  await current.page?.evaluate(() => window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' })).catch(() => {});
  await current.browser?.close().catch(() => {});
  await delay(3000);
  if (child.exitCode === null) child.kill();
  if (restartedPid) {
    try { process.kill(restartedPid); } catch { /* closed */ }
  }
  console.log(session.output.split('\n').filter((line) => /Qamille:/.test(line)).join('\n'));
}
await delay(2000);
await fs.rm(data, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 }).catch(() => {});
console.log(`\nwebview recovery: ${results.length}/7 checks pass`);
if (results.length !== 7) process.exit(1);
