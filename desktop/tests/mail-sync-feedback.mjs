// Sync feedback after ADR-0007 decision 7 / INV-009: the phase and progress of a mail sync live
// only in the single activity indicator, fed by the typed `sync-progress` app-events. The legacy
// empty `mail-sync-changed` channel and its toast are gone; errors stay separate alerts.
// Synthetic desktop commands only; no real Google account.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const base = process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:5187';
const sample = JSON.parse(await fs.readFile(new URL('../src/lib/sample.json', import.meta.url), 'utf8'));
const browser = await chromium.launch({ channel: 'msedge', headless: true });
const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1280, height: 900 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(({ settings, account }) => {
  const callbacks = new Map();
  const listeners = new Map();
  let callbackId = 0;
  const workspace = { accounts: [{ ...account, id: 1, name: 'New account', email: 'new@example.test', kind: 'Gmail', last_sync: 2000000000 }],
    counts: {}, labels: [], signatures: [], drafts: [], undo_actions: [],
    has_ai_key: false, has_google_secret: false,
    settings: { ...settings, locale: 'en-US', dark: false } };
  let syncStatus = { revision: 0, accounts: [] };
  let holdStatus = false;
  let releaseStatus;
  if (/running/.test(window.location.search))
    syncStatus = { revision: 7, accounts: [{ account_id: 1, phase: 'syncing', added: 0, errors: [] }] };
  if (/complete/.test(window.location.search))
    syncStatus = { revision: 7, accounts: [{ account_id: 1, phase: 'complete', added: 3, errors: [] }] };
  if (/racing/.test(window.location.search)) holdStatus = true;
  // The backend writes its status first and emits the typed event second.
  window.setSyncPhase = (phase, errors = []) => {
    syncStatus = { revision: syncStatus.revision + 1, accounts: [{ account_id: 1, phase, added: phase === 'complete' ? 3 : 0, errors }] };
    window.emitDesktop('app-event', { kind: 'sync-progress', account_id: 1, phase, added: phase === 'complete' ? 3 : 0 });
  };
  window.releaseStatus = () => releaseStatus?.();
  window.calls = [];
  window.emitDesktop = (event, payload) => {
    for (const handler of listeners.get(event) || []) callbacks.get(handler)({ event, payload });
  };
  window.isTauri = true;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    transformCallback(callback) { const id = ++callbackId; callbacks.set(id, callback); return id; },
    async invoke(command, args = {}) {
      window.calls.push(command);
      if (command === 'plugin:event|listen') {
        listeners.set(args.event, [...(listeners.get(args.event) || []), args.handler]);
        return args.handler;
      }
      if (command === 'plugin:event|unlisten') return null;
      if (command === 'plugin:window|is_maximized') return false;
      if (command === 'workspace_snapshot') return structuredClone(workspace);
      if (command === 'mail_sync_status') {
        const snapshot = structuredClone(syncStatus);
        if (holdStatus) {
          holdStatus = false;
          return new Promise(resolve => { releaseStatus = () => resolve(snapshot); });
        }
        return snapshot;
      }
      if (command === 'list_mail') return { items: [], total: 0, next_offset: null };
      if (command === 'meeting_capture_status') return { state: 'idle' };
      if (command === 'meeting_list_page') return { items: [], total: 0, next_offset: null };
      if (command === 'sync_mail') return new Promise((resolve, reject) => { window.finishManualSync = resolve; window.failManualSync = reject; });
      throw new Error(`Unexpected command: ${command}`);
    },
  };
}, { settings: sample.settings, account: sample.accounts[0] });

const count = (command) => page.evaluate(name => window.calls.filter(call => call === name).length, command);
const indicator = page.locator('.activity-indicator');
const line = indicator.locator('.activity-line');
const toast = page.locator('.toast');
const sync = page.getByRole('button', { name: 'Sync mail', exact: true });
// Waits until the app has not called the backend for 300 ms (at most 5 s).
const quiet = () => page.evaluate(async () => {
  let last = window.calls.length;
  let since = Date.now();
  while (Date.now() - since < 5000) {
    await new Promise(resolve => setTimeout(resolve, 50));
    if (window.calls.length !== last) [last, since] = [window.calls.length, Date.now()];
    else if (Date.now() - since >= 300) return;
  }
});

try {
  await page.goto(base);
  await expect(page.locator('.mail-list')).toBeVisible();
  await quiet();
  await expect(indicator).toHaveCount(0);
  await expect(page.locator('.sync-feedback')).toHaveCount(0);

  // A typed sync-progress shows the phase in the indicator and nowhere else.
  const listsBefore = await count('list_mail');
  await page.evaluate(() => window.setSyncPhase('syncing'));
  await expect(line).toBeVisible({ timeout: 1500 });
  await expect(line).toContainText('Syncing');
  await expect(sync).toBeEnabled();
  await expect(sync.locator('.spinning')).toHaveCount(0);
  await expect(page.locator('.sync-feedback')).toHaveCount(0);
  await expect(page.locator('.empty-list')).not.toContainText('Sync in progress');
  await expect(toast).toHaveCount(0);
  await page.evaluate(() => window.setSyncPhase('complete'));
  await expect(indicator).toHaveCount(0, { timeout: 1500 });
  // No completion notice, and no list reload per phase (INV-004): the typed events carry what
  // changed; a sync with no new mail leaves the list alone.
  await expect(toast).toHaveCount(0);
  await quiet();
  assert.equal(await count('list_mail'), listsBefore, 'a sync phase must not reload the list');
  console.log('PASS sync phases appear only in the activity indicator, with no toast and no list reload');

  // The retired channel is not listened to any more.
  const before = await page.evaluate(() => window.calls.length);
  await page.evaluate(() => window.emitDesktop('mail-sync-changed', null));
  await quiet();
  assert.equal(await page.evaluate(count => window.calls.length - count, before), 0, 'mail-sync-changed triggers nothing');
  await expect(indicator).toHaveCount(0);
  console.log('PASS the legacy mail-sync-changed event is ignored');

  // Errors stay a separate alert; the toolbar keeps the retry hint; the indicator never shows them.
  await page.evaluate(() => window.setSyncPhase('syncing'));
  await expect(line).toBeVisible({ timeout: 1500 });
  await page.evaluate(() => window.setSyncPhase('error', ['Connection interrupted. Try again.']));
  await expect(toast).toHaveAttribute('role', 'alert');
  await expect(toast).toContainText('Connection interrupted');
  await expect(indicator).toHaveCount(0, { timeout: 1500 });
  await expect(sync).toBeEnabled();
  await expect(sync).toHaveAttribute('title', /Connection interrupted/);
  await expect(sync).toHaveClass(/sync-failed/);
  await expect(page.locator('.empty-list')).not.toContainText('Mail is not up to date yet');
  console.log('PASS a failed sync raises one separate alert and marks the toolbar button');

  // Retry from the button: the request starts one sync; the indicator follows the events.
  await sync.click();
  await expect.poll(() => count('sync_mail')).toBe(1);
  await page.evaluate(() => window.setSyncPhase('syncing'));
  await expect(line).toBeVisible({ timeout: 1500 });
  await page.evaluate(() => {
    window.setSyncPhase('complete');
    window.finishManualSync({ added: 3, errors: [] });
  });
  await expect(indicator).toHaveCount(0, { timeout: 1500 });
  await expect(sync).toHaveAttribute('title', 'Sync mail');
  await expect(sync).not.toHaveClass(/sync-failed/);
  await expect(page.locator('.empty-list')).toContainText('all caught up');
  console.log('PASS a manual retry follows the typed events and clears the error mark');

  // A manual request that fails outright is an alert too.
  await sync.click();
  await expect.poll(() => count('sync_mail')).toBe(2);
  await page.evaluate(() => window.failManualSync(new Error('Synthetic storage failure')));
  await expect(toast).toContainText('Synthetic storage failure');
  await expect(sync).toHaveAttribute('title', /Synthetic storage failure/);
  console.log('PASS a failed manual request is reported as an alert');

  await page.evaluate(async () => {
    const { setLanguageChoice } = await import('/src/lib/i18n.ts');
    setLanguageChoice('it');
    window.setSyncPhase('syncing');
  });
  await expect(line).toContainText('Sincronizzazione');
  await page.evaluate(() => window.setSyncPhase('complete'));
  await expect(indicator).toHaveCount(0, { timeout: 1500 });
  console.log('PASS the indicator label follows the interface language');

  // Opening the UI during a sync that started before the listener existed restores its activity.
  await page.goto(`${base}/?running`);
  await expect(line).toBeVisible({ timeout: 1500 });
  await expect(line).toContainText(/Syncing|Sincronizzazione/);
  await expect(page.locator('.topbar-actions .spinning')).toHaveCount(0);
  await page.evaluate(() => window.setSyncPhase('complete'));
  await expect(indicator).toHaveCount(0, { timeout: 1500 });
  console.log('PASS opening the UI during an existing background sync restores it in the indicator');

  // A completed retained snapshot never replays anything.
  await page.goto(`${base}/?complete`);
  await expect(page.locator('.mail-list')).toBeVisible();
  await quiet();
  await expect(indicator).toHaveCount(0);
  await expect(toast).toHaveCount(0);
  console.log('PASS completed snapshots never replay notifications');

  // A typed event that overtakes the startup snapshot wins: no stuck activity.
  await page.goto(`${base}/?running&racing`);
  await expect(page.locator('.mail-list')).toBeVisible();
  await expect.poll(() => count('mail_sync_status')).toBe(1);
  await page.evaluate(() => window.setSyncPhase('complete'));
  await page.evaluate(() => window.releaseStatus());
  await quiet();
  await page.waitForTimeout(1200);
  await expect(indicator).toHaveCount(0);
  console.log('PASS a stale startup snapshot never resurrects a finished sync');

  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
