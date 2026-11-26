// Exercise the packaged app with disposable normal and explicit demo workspaces.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { execFile, spawn } from 'node:child_process';
import { promisify } from 'node:util';

const root = path.resolve(import.meta.dirname, '../..');
const artifacts = path.join(root, 'checks/workspace-startup', `native-${Date.now()}-${process.pid}`);
const executable = path.resolve(process.env.QAMILLE_STARTUP_EXE || path.join(root, 'release/Qamille.exe'));
const sessions = [];
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

// The app's single-instance handler focuses the existing window. Never run this
// test while a user's normal or preview window might receive that activation.
if (process.platform === 'win32') {
  const { stdout } = await promisify(execFile)('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    "ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(Qamille|qamille-desktop)(\\.exe)?$' -and $_.CommandLine -notmatch '(?:^|\\s)--(?:mcp|worker|embedding-worker)(?:\\s|$)' } | Select-Object -ExpandProperty ProcessId)",
  ], { windowsHide: true });
  const running = JSON.parse(stdout);
  assert.deepEqual(running, [], 'Close existing Qamille app windows before this isolated native test.');
}
await fs.access(executable);
await fs.mkdir(artifacts, { recursive: true });

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address();
  await new Promise(resolve => server.close(resolve));
  return port;
}

async function launch(name, data, demo = false, options = {}) {
  await fs.mkdir(data, { recursive: true });
  // `stop` closes with the X, which closes Qamille here, as before the tray (src-tauri/src/background.rs).
  await fs.writeFile(path.join(data, 'background.json'), JSON.stringify({ close_to_tray: false, tray_notice_shown: true }));
  const port = await freePort();
  const env = { ...process.env, ...options.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` };
  if (options.portable) {
    delete env.QAMILLE_DATA;
    delete env.WEBVIEW2_USER_DATA_FOLDER;
  } else {
    env.QAMILLE_DATA = options.workspaceOverride ?? data;
    if (options.workspaceOverride) delete env.WEBVIEW2_USER_DATA_FOLDER;
    else env.WEBVIEW2_USER_DATA_FOLDER = path.join(data, 'webview');
  }
  delete env.QAMILLE_DEMO;
  if (demo) env.QAMILLE_DEMO = '1';
  const child = spawn(options.executable || executable, [], {
    cwd: options.cwd || root, env, windowsHide: true, stdio: 'pipe',
  });
  const session = { name, child, data, demo, output: '', pageErrors: [] };
  sessions.push(session);
  child.stdout.on('data', bytes => { session.output += bytes; });
  child.stderr.on('data', bytes => { session.output += bytes; });
  child.on('error', error => { session.spawnError = error; });
  let ready = false;
  for (let attempt = 0; attempt < 120; attempt++) {
    if (session.spawnError) throw session.spawnError;
    assert.equal(child.exitCode, null, `${name} exited before its own window opened: ${session.output}`);
    try {
      ready = (await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) })).ok;
      if (ready) break;
    } catch { /* WebView is starting. */ }
    await delay(250);
  }
  assert.ok(ready, `${name} did not expose its test window: ${session.output}`);
  session.browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = session.browser.contexts()[0];
  session.page = context.pages()[0] || await context.waitForEvent('page');
  const { page } = session;
  page.setDefaultTimeout(15000);
  page.on('pageerror', error => session.pageErrors.push(error.message));
  await page.addInitScript(() => {
    window.__startupAlerts = [];
    new MutationObserver(() => {
      for (const alert of document.querySelectorAll('[data-scope="toast"]:has([data-slot="toast-error"])')) {
        const message = alert.textContent.trim();
        if (!window.__startupAlerts.includes(message)) window.__startupAlerts.push(message);
      }
    }).observe(document, { childList: true, subtree: true });
  });
  await page.waitForFunction(() => !!window.__TAURI_INTERNALS__);
  assert.equal(new URL(page.url()).origin, 'http://tauri.localhost', `${name}: the UI must be served by the packaged executable`);
  await page.evaluate(() => localStorage.setItem('qamille:ui-language', 'en'));
  await page.reload();
  await page.locator('.compose-main').waitFor();
  session.invoke = (command, args = {}) => page.evaluate(({ command, args }) =>
    window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  return session;
}

async function assertNoErrors(session) {
  assert.deepEqual(session.pageErrors, [], `${session.name}: JavaScript errors`);
  assert.deepEqual(await session.page.evaluate(() => window.__startupAlerts), [], `${session.name}: app errors`);
  await expect(session.page.locator('[data-scope="toast"][data-part="root"]:has([data-slot="toast-error"])')).toHaveCount(0);
}

async function assertEmpty(session) {
  const store = JSON.parse(await fs.readFile(path.join(session.data, 'store.json'), 'utf8'));
  for (const field of ['accounts', 'emails', 'drafts', 'events', 'meetings', 'signatures']) {
    assert.equal(store[field].length, 0, `${session.name}: persisted ${field} should be empty`);
  }
  const workspace = await session.invoke('workspace_snapshot');
  assert.deepEqual(workspace.accounts, [], `${session.name}: normal startup created accounts`);
  assert.deepEqual(workspace.drafts, []);
  assert.deepEqual(workspace.signatures, []);
  assert.ok(Object.values(workspace.counts).every(count => count === 0));
  const mail = await session.invoke('list_mail', { query: { search: 'in:anywhere', offset: 0, limit: 100, threads: false } });
  assert.deepEqual(mail.items, [], `${session.name}: normal startup created mail`);
  await expect(session.page.locator('.mail-open')).toHaveCount(0);
  await expect(session.page.locator('.empty-list').getByRole('heading', { name: 'You’re all caught up' })).toBeVisible();
  await assertNoErrors(session);
}

async function assertAccountsAccessible(session) {
  const { page } = session;
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Your name', exact: true })).toHaveValue('You');
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Accounts', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Your email accounts', exact: true })).toBeVisible();
  await expect(page.locator('.connected-account')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Sign in with Google', exact: true })).toBeEnabled();
  await assertNoErrors(session);
}

async function stop(session) {
  if (!session || session.stopped) return;
  session.stopped = true;
  if (session.child.exitCode === null) {
    if (session.page && !session.page.isClosed()) {
      await session.page.getByRole('group', { name: 'Window controls' })
        .getByRole('button', { name: 'Close window', exact: true }).click({ timeout: 3000 }).catch(() => {});
    }
    for (let attempt = 0; attempt < 40 && session.child.exitCode === null; attempt++) await delay(100);
    if (session.child.exitCode === null) session.child.kill();
  }
  await session.browser?.close().catch(() => {});
}

async function copyPortableApp(folder) {
  await fs.mkdir(folder, { recursive: true });
  const copied = path.join(folder, path.basename(executable));
  await fs.copyFile(executable, copied);
  for (const entry of await fs.readdir(path.dirname(executable), { withFileTypes: true })) {
    if (entry.isFile() && /\.dll$/i.test(entry.name)) {
      await fs.copyFile(path.join(path.dirname(executable), entry.name), path.join(folder, entry.name));
    }
  }
  return copied;
}

async function assertPortableProfile(folder) {
  const profile = path.join(folder, 'data', 'webview');
  assert.ok((await fs.stat(profile)).isDirectory(), 'WebView profile belongs beside the executable');
  assert.ok((await fs.readdir(profile)).length > 0, 'The portable WebView profile contains browser data');
}

async function movePortableFolder(source, destination) {
  // WebView2 can retain cache handles briefly after the host process exits.
  // Require a real rename, waiting only while Windows still reports it busy.
  const deadline = Date.now() + 15000;
  for (;;) {
    try {
      await fs.rename(source, destination);
      return;
    } catch (error) {
      if (!['EPERM', 'EACCES', 'EBUSY'].includes(error.code) || Date.now() >= deadline) throw error;
      await delay(250);
    }
  }
}

async function assertRelativeWorkspace() {
  const appFolder = path.join(artifacts, 'relative-override-app');
  const workingDirectory = path.join(artifacts, 'relative-working-directory');
  const data = path.join(workingDirectory, 'relative-data');
  await fs.mkdir(workingDirectory, { recursive: true });
  const options = {
    executable: await copyPortableApp(appFolder),
    cwd: workingDirectory,
    workspaceOverride: 'relative-data',
  };
  const relative = await launch('relative-workspace-first-start', data, false, options);
  await assertEmpty(relative);
  await relative.page.evaluate(() => localStorage.setItem('qamille:relative-workspace-test', 'preserved'));
  const preferences = path.join(data, 'webview', 'EBWebView', 'Default', 'Preferences');
  // WebView2 can defer writing Preferences until its profile closes.
  await stop(relative);
  await expect.poll(async () => (await fs.stat(preferences).catch(() => null))?.isFile() ?? false, {
    message: 'A relative workspace override must keep the browser profile under the selected working-directory workspace',
    timeout: 10000,
  }).toBe(true);
  await assert.rejects(fs.stat(path.join(appFolder, 'relative-data')), { code: 'ENOENT' },
    'A relative workspace override must not create a second workspace beside the executable');
  const restarted = await launch('relative-workspace-restart', data, false, options);
  assert.equal(await restarted.page.evaluate(() => localStorage.getItem('qamille:relative-workspace-test')), 'preserved');
  await assertEmpty(restarted);
  await stop(restarted);
  console.log('PASS relative workspace overrides keep mail and persistent browser preferences in the same selected folder');
}

// Task 3.4 regression guard: the window must appear quickly because the
// interrupted-state recovery (undo + meetings) now runs on the `qamille-startup`
// thread instead of blocking setup(). Opt-in because it needs a built executable and
// a machine warm enough to hit the brief's 1.5 s budget, so the controller runs it
// post-merge. Enable with QAMILLE_STARTUP_TIMING=1 (budget overridable via
// QAMILLE_STARTUP_MAX_MS). Measures process start -> `.compose-main` visible, the
// same "window is interactive" marker launch() waits for.
async function measureDemoStartupLatency() {
  const budgetMs = Number(process.env.QAMILLE_STARTUP_MAX_MS || 1500);
  const data = path.join(artifacts, 'startup-timing');
  await fs.mkdir(data, { recursive: true });
  const port = await freePort();
  const env = { ...process.env,
    QAMILLE_DATA: data,
    QAMILLE_DEMO: '1',
    WEBVIEW2_USER_DATA_FOLDER: path.join(data, 'webview'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` };
  const startedAt = Date.now();
  const child = spawn(executable, [], { cwd: root, env, windowsHide: true, stdio: 'pipe' });
  const session = { name: 'startup-timing', child, data, demo: true, output: '', pageErrors: [] };
  sessions.push(session);
  child.stdout.on('data', bytes => { session.output += bytes; });
  child.stderr.on('data', bytes => { session.output += bytes; });
  child.on('error', error => { session.spawnError = error; });
  let ready = false;
  for (let attempt = 0; attempt < 240; attempt++) {
    if (session.spawnError) throw session.spawnError;
    assert.equal(child.exitCode, null, `startup timing: app exited before its window opened: ${session.output}`);
    try {
      ready = (await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) })).ok;
      if (ready) break;
    } catch { /* WebView is starting. */ }
    await delay(25);
  }
  assert.ok(ready, `startup timing: WebView never exposed its debug port: ${session.output}`);
  session.browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = session.browser.contexts()[0];
  session.page = context.pages()[0] || await context.waitForEvent('page');
  const { page } = session;
  page.setDefaultTimeout(15000);
  await page.locator('.compose-main').waitFor();
  const elapsedMs = Date.now() - startedAt;
  console.log(`Startup latency (process start -> .compose-main visible): ${elapsedMs} ms (budget ${budgetMs} ms)`);
  await stop(session);
  assert.ok(elapsedMs <= budgetMs,
    `Window must appear within ${budgetMs} ms of process start; took ${elapsedMs} ms`);
  return elapsedMs;
}

try {
  await assertRelativeWorkspace();
  if (!process.argv.includes('--relative-only')) {
  const normalData = path.join(artifacts, 'normal');
  const normal = await launch('normal-first-start', normalData);
  await assertEmpty(normal);
  await normal.page.screenshot({ path: path.join(artifacts, 'normal-empty.png') });
  await assertAccountsAccessible(normal);
  await normal.page.screenshot({ path: path.join(artifacts, 'normal-accounts.png') });
  console.log('PASS packaged normal startup: zero accounts/mail and working Settings → Accounts');
  await stop(normal);

  const restarted = await launch('normal-restart', normalData);
  await assertEmpty(restarted);
  console.log('PASS normal workspace stays empty after restart');
  await stop(restarted);

  const preview = await launch('explicit-preview', path.join(artifacts, 'preview'), true);
  const sample = await preview.invoke('workspace_snapshot');
  assert.equal(sample.accounts.length, 2);
  assert.ok(sample.accounts.every(account => account.kind === 'Sample'));
  const sampleMail = await preview.invoke('list_mail', { query: { search: 'in:anywhere', offset: 0, limit: 100, threads: false } });
  assert.ok(sampleMail.items.length > 0, 'Explicit preview should contain sample mail');
  await expect(preview.page.locator('.mail-open').first()).toBeVisible();
  await assertNoErrors(preview);
  await preview.page.screenshot({ path: path.join(artifacts, 'explicit-preview.png') });
  console.log('PASS QAMILLE_DEMO=1 explicitly starts a separate sample workspace');

  const concurrentNormal = await launch('normal-while-preview-open', normalData);
  await assertEmpty(concurrentNormal);
  assert.equal(preview.child.exitCode, null, 'Preview should stay open alongside normal production');
  assert.equal((await preview.invoke('workspace_snapshot')).accounts.length, 2);
  await assertNoErrors(preview);
  await concurrentNormal.page.screenshot({ path: path.join(artifacts, 'normal-alongside-preview.png') });
  console.log('PASS production opens its own empty window while the sample preview remains open');
  await stop(concurrentNormal);
  await stop(preview);

  const portableA = path.join(artifacts, 'portable-a');
  const portableB = path.join(artifacts, 'portable-b');
  const decoyRoaming = path.join(artifacts, 'unrelated-appdata');
  const decoyLocal = path.join(artifacts, 'unrelated-localappdata');
  const unrelatedCwd = path.join(artifacts, 'unrelated-working-directory');
  await Promise.all([decoyRoaming, decoyLocal, unrelatedCwd].map(folder => fs.mkdir(folder, { recursive: true })));
  const portableOptions = {
    portable: true,
    cwd: unrelatedCwd,
    env: { APPDATA: decoyRoaming, LOCALAPPDATA: decoyLocal },
  };
  const firstPortable = await launch('portable-first-start', path.join(portableA, 'data'), false, {
    ...portableOptions, executable: await copyPortableApp(portableA),
  });
  await assertEmpty(firstPortable);
  await assertPortableProfile(portableA);
  const portableWorkspace = await firstPortable.invoke('workspace_snapshot');
  assert.equal(portableWorkspace.settings.ai_provider, 'Offline');
  assert.equal(portableWorkspace.has_ai_key, false);
  const syntheticSecret = 'portable-native-test-secret';
  await firstPortable.invoke('save_settings', {
    update: {
      settings: { ...portableWorkspace.settings, user_name: 'Portable workspace owner' },
      ai_key: syntheticSecret,
    },
  });
  assert.equal((await firstPortable.invoke('workspace_snapshot')).has_ai_key, true);
  // Phase 3, Task 7: the key is wrapped by Windows for this user, never written in clear.
  const keyPath = path.join(portableA, 'data', 'credentials.key.dpapi');
  const credentialsPath = path.join(portableA, 'data', 'credentials.json');
  assert.ok((await fs.stat(keyPath)).size > 0, 'Portable credential key is stored beside the executable');
  await assert.rejects(fs.stat(path.join(portableA, 'data', 'credentials.key')), 'No plain credential key beside it');
  const encryptedCredentials = await fs.readFile(credentialsPath, 'utf8');
  assert.equal(encryptedCredentials.includes(syntheticSecret), false, 'Credential file must not contain plaintext secrets');
  const ciphertext = Buffer.from(JSON.parse(encryptedCredentials).ciphertext);
  assert.ok(ciphertext.length > 0, 'Portable credential file contains encrypted data');
  assert.equal(ciphertext.includes(Buffer.from(syntheticSecret)), false, 'Ciphertext must not expose the saved secret');
  assert.equal((await fs.readFile(path.join(portableA, 'data', 'store.json'), 'utf8')).includes(syntheticSecret), false);
  await firstPortable.page.evaluate(() => localStorage.setItem('qamille:sidebar-collapsed', 'true'));
  await firstPortable.page.screenshot({ path: path.join(artifacts, 'portable-first-start.png') });
  await stop(firstPortable);
  console.log('PASS portable default startup stores workspace, encrypted credentials and browser profile beside the executable');

  await movePortableFolder(portableA, portableB);
  const movedPortable = await launch('portable-after-move', path.join(portableB, 'data'), false, {
    ...portableOptions, executable: path.join(portableB, path.basename(executable)),
  });
  const movedWorkspace = await movedPortable.invoke('workspace_snapshot');
  assert.equal(movedWorkspace.settings.user_name, 'Portable workspace owner');
  assert.equal(movedWorkspace.has_ai_key, true, 'Portable credentials survive moving the executable directory');
  assert.equal(await movedPortable.page.evaluate(() => localStorage.getItem('qamille:sidebar-collapsed')), 'true');
  await assertPortableProfile(portableB);
  await assertNoErrors(movedPortable);
  console.log('PASS moving the portable directory preserves saved settings, credentials and interface preferences');

  const independentPortable = await launch('independent-portable-copy', path.join(portableA, 'data'), false, {
    ...portableOptions, executable: await copyPortableApp(portableA),
  });
  await assertEmpty(independentPortable);
  const independentWorkspace = await independentPortable.invoke('workspace_snapshot');
  assert.equal(independentWorkspace.settings.user_name, 'You');
  assert.equal(independentWorkspace.has_ai_key, false, 'Independent portable copies do not share credentials');
  assert.equal(await independentPortable.page.evaluate(() => localStorage.getItem('qamille:sidebar-collapsed')), null);
  assert.equal(movedPortable.child.exitCode, null, 'The moved portable copy stays open independently');
  assert.equal((await movedPortable.invoke('workspace_snapshot')).settings.user_name, 'Portable workspace owner');
  await assertNoErrors(movedPortable);
  await stop(independentPortable);
  await stop(movedPortable);
  for (const folder of [decoyRoaming, decoyLocal, unrelatedCwd]) {
    assert.deepEqual(await fs.readdir(folder), [], `Portable startup must leave ${folder} untouched`);
  }
  console.log('PASS portable copies run independently and do not write into AppData or the working directory');
  }
  if (process.env.QAMILLE_STARTUP_TIMING === '1') {
    const elapsedMs = await measureDemoStartupLatency();
    console.log(`PASS demo window appeared in ${elapsedMs} ms — startup recovery runs off the main thread`);
  }
  console.log(`Artifacts: ${artifacts}`);
} catch (error) {
  for (const session of sessions) {
    await session.page?.screenshot({ path: path.join(artifacts, `${session.name}-failure.png`) }).catch(() => {});
  }
  throw error;
} finally {
  for (const session of sessions.toReversed()) await stop(session);
}
