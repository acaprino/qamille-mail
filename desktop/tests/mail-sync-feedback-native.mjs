// Native sync events and retained status, using an isolated account with no credentials.
// Missing credentials stop before any provider connection; no Google login or mail is used.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import { spawn } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '../..');
const data = path.join(root, 'checks/mail-sync-native', `${Date.now()}-${process.pid}`);
const executable = process.env.QAMILLE_SYNC_EXE || path.join(root, 'app/target/release/qamille-desktop.exe');
const store = JSON.parse(await fs.readFile(path.join(root, 'desktop/src/lib/sample.json'), 'utf8'));
store.accounts = [{ ...store.accounts[0], id: 1, kind: 'Gmail', provider_id: 'gmail', name: 'Sync test',
  email: `sync-feedback-${process.pid}@example.test`, username: `sync-feedback-${process.pid}@example.test`,
  password: '', oauth_client_id: '', last_sync: null }];
for (const field of ['emails', 'drafts', 'events', 'meetings', 'signatures', 'undo_actions']) store[field] = [];
store.settings.locale = 'en-US';
store.settings.time_zone = 'UTC';
await fs.mkdir(data, { recursive: true });
await fs.writeFile(path.join(data, 'store.json'), JSON.stringify(store));
const server = net.createServer();
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
await new Promise(resolve => server.close(resolve));
const env = { ...process.env, QAMILLE_DATA: data,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` };
delete env.QAMILLE_DEMO;
const child = spawn(executable, [], { cwd: root, env, windowsHide: true, stdio: 'pipe' });
let output = '';
child.stdout.on('data', bytes => output += bytes);
child.stderr.on('data', bytes => output += bytes);
let spawnError;
child.on('error', error => spawnError = error);
let browser;
let page;
try {
  await expect.poll(async () => {
    if (spawnError) throw spawnError;
    assert.equal(child.exitCode, null, `Native test exited: ${output}`);
    try { return (await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) })).ok; }
    catch { return false; }
  }, { timeout: 30000 }).toBe(true);
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = browser.contexts()[0];
  page = context.pages()[0] || await context.waitForEvent('page');
  await page.waitForFunction(() => !!window.__TAURI_INTERNALS__);
  await page.evaluate(() => localStorage.setItem('qamille:ui-language', 'en'));
  await page.clock.install();
  await page.reload();
  const status = () => page.evaluate(() => window.__TAURI_INTERNALS__.invoke('mail_sync_status'));
  await expect.poll(async () => (await status()).accounts[0]?.phase).toBe('error');
  // Typed `sync-progress` events feed the activity indicator; the failure is a separate alert
  // (ADR-0007 decision 7, INV-009). The old `.sync-feedback` overlay no longer exists.
  const alert = page.locator('.toast.error');
  const indicator = page.locator('.activity-indicator');
  await expect(alert).toContainText('Sync incomplete');
  await expect(alert).toContainText('Reconnect this mail account in Settings');
  await expect(page.locator('.sync-feedback')).toHaveCount(0);
  await expect(page.locator('.empty-list')).toContainText('You’re all caught up');
  const before = (await status()).revision;
  await page.getByRole('button', { name: 'Sync mail', exact: true }).click();
  await expect.poll(async () => (await status()).revision).toBeGreaterThan(before);
  await expect(page.getByRole('button', { name: 'Sync mail', exact: true })).toBeEnabled();
  await page.reload();
  await expect(alert).toContainText('Sync incomplete');
  await expect(page.getByRole('button', { name: 'Sync mail', exact: true })).toBeEnabled();
  await page.clock.fastForward(12100);
  await expect(alert).toHaveCount(0);
  await expect(indicator).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Sync mail', exact: true })).toHaveAttribute('title', /Reconnect this mail account/);
  console.log('PASS native background sync, retry, reload and automatic dismissal with retained toolbar error');
} finally {
  if (page) await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('plugin:window|close', { label: 'main' })).catch(() => {});
  if (browser) await browser.close();
  if (child.exitCode === null) await new Promise(resolve => {
    const timer = setTimeout(() => { child.kill(); resolve(); }, 5000);
    child.once('exit', () => { clearTimeout(timer); resolve(); });
  });
}
