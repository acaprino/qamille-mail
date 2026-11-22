// Starts the built Qamille.exe on a disposable workspace and attaches Playwright to its WebView2
// window through the remote debugging port, the way the other *-native tests do. Shared by the
// native tests of the 2026-11-07 restyle (thread-rows-native, a11y-native, glance-scale).
//
// Safety: only Sample (demo) accounts, only a folder under checks/, and every outgoing HTTP(S)
// request of the Rust side is pointed at a dead local proxy, so nothing can reach a mail server
// or a model host by accident.
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { promisify } from 'node:util';

export const root = path.resolve(import.meta.dirname, '../../..');
export const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** QAMILLE_NATIVE_EXE overrides; by default the Release folder of this checkout. */
export const executable = () =>
  path.resolve(process.env.QAMILLE_NATIVE_EXE || path.join(root, 'release/Qamille.exe'));

/** The single-instance handler would hand focus to a window the user has open: never test then. */
export async function assertNoQamilleWindow() {
  if (process.platform !== 'win32') return;
  const { stdout } = await promisify(execFile)('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    "@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(Qamille|qamille-desktop)(\\.exe)?$' -and $_.CommandLine -notmatch '(?:^|\\s)--(?:mcp|worker|embedding-worker)(?:\\s|$)' } | Select-Object -ExpandProperty ProcessId) | ConvertTo-Json -Compress",
  ], { windowsHide: true });
  const running = stdout.trim() ? JSON.parse(stdout) : [];
  assert.equal([running].flat().length, 0, 'A Qamille window is open: close it before this isolated native test.');
}

export async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

/**
 * @param name     folder name under checks/
 * @param store    optional object written as the workspace's store.json (the sample workspace reads it)
 * @param browserArgs extra WebView2 arguments, e.g. '--force-device-scale-factor=1.5'
 * @param background the workspace's background.json (src-tauri/src/background.rs): by default the X
 *                   closes Qamille, as these tests expect, and no first-hide message waits
 * @param args     command-line arguments of Qamille, e.g. ['--autostart']
 */
export async function launchNative(name, { store, browserArgs = '', language = 'en', background = { close_to_tray: false, tray_notice_shown: true }, args = [] } = {}) {
  const exe = executable();
  await fs.access(exe);
  await assertNoQamilleWindow();
  const data = path.join(root, 'checks', name, `native-${Date.now()}-${process.pid}`);
  await fs.mkdir(path.join(data, 'webview'), { recursive: true });
  await fs.writeFile(path.join(data, 'background.json'), JSON.stringify(background));
  // The interface language comes from the workspace's regional setting, so no page reload is needed
  // (a reload would make the app open and mark read a second message: it opens the first row at every start).
  const seed = structuredClone(store ?? JSON.parse(await fs.readFile(path.join(root, 'desktop/src/lib/sample.json'), 'utf8')));
  seed.settings = { ...seed.settings, locale: language === 'en' ? 'en-US' : 'it-IT' };
  await fs.writeFile(path.join(data, 'store.json'), JSON.stringify(seed));
  const port = await freePort();
  const dead = 'http://127.0.0.1:9';
  const child = spawn(exe, args, {
    cwd: root, windowsHide: true, stdio: 'pipe',
    env: {
      ...process.env, QAMILLE_DATA: data, QAMILLE_DEMO: '1',
      HTTP_PROXY: dead, HTTPS_PROXY: dead, ALL_PROXY: dead, NO_PROXY: '',
      WEBVIEW2_USER_DATA_FOLDER: path.join(data, 'webview'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --lang=${language === 'en' ? 'en-US' : 'it-IT'} ${browserArgs}`.trim(),
    },
  });
  const session = { child, data, port, output: '', pageErrors: [], consoleProblems: [], fontResponses: [], violations: [] };
  child.stdout.on('data', (bytes) => { session.output += bytes; });
  child.stderr.on('data', (bytes) => { session.output += bytes; });
  child.on('error', (error) => { session.spawnError = error; });
  let ready = false;
  for (let attempt = 0; attempt < 120 && !ready; attempt++) {
    if (session.spawnError) throw session.spawnError;
    assert.equal(child.exitCode, null, `The app exited before its window opened: ${session.output}`);
    try { ready = (await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) })).ok; } catch { /* starting */ }
    if (!ready) await delay(250);
  }
  assert.ok(ready, `The app did not expose its window: ${session.output}`);
  session.browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = session.browser.contexts()[0];
  session.context = context;
  session.page = context.pages()[0] || await context.waitForEvent('page');
  const { page } = session;
  page.setDefaultTimeout(20000);
  page.on('pageerror', (error) => session.pageErrors.push(error.message));
  page.on('console', (message) => {
    if (['error', 'warning'].includes(message.type())) session.consoleProblems.push(`${message.type()}: ${message.text()}`);
  });
  page.on('response', (response) => {
    if (/\.(woff2?|ttf|otf)(\?|$)/i.test(response.url())) session.fontResponses.push({ url: response.url(), status: response.status() });
  });
  await page.waitForFunction(() => !!window.__TAURI_INTERNALS__);
  await page.addInitScript(() => {
    window.__cspViolations = [];
    document.addEventListener('securitypolicyviolation', (event) =>
      window.__cspViolations.push(`${event.violatedDirective} ${event.blockedURI}`));
  });
  session.invoke = (command, args = {}) =>
    page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  /** The font files the page loaded, from the resource timing entries (they survive without a reload). */
  session.fontEntries = () => page.evaluate(() => performance.getEntriesByType('resource')
    .filter((e) => /\.(woff2?|ttf|otf)(\?|$)/i.test(e.name)).map((e) => ({ url: e.name, status: e.responseStatus })));
  session.stop = async () => {
    await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' })).catch(() => {});
    await session.browser.close().catch(() => {});
    if (child.exitCode === null) {
      await new Promise((resolve) => {
        const timer = setTimeout(() => { child.kill(); resolve(); }, 5000);
        child.once('exit', () => { clearTimeout(timer); resolve(); });
      });
    }
  };
  // The workspace must be a Sample one: this helper never works on anything else.
  const workspace = await session.invoke('workspace_snapshot');
  assert.ok(workspace.accounts.length && workspace.accounts.every((a) => a.kind === 'Sample'), 'Sample accounts only');
  return session;
}

/** A token resolved to the colour string the browser writes in computed styles. */
export const colorOf = (page, name) =>
  page.evaluate((value) => {
    const probe = document.createElement('i');
    probe.style.color = `var(${value})`;
    document.body.append(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    return resolved;
  }, name);
