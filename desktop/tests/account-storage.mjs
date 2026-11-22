import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { spawn } from 'node:child_process';

const fixture = JSON.parse(await fs.readFile(new URL('../src/lib/sample.json', import.meta.url), 'utf8'));
const first = fixture.accounts[0];
const second = fixture.accounts[1];
first.kind = 'Gmail';
delete first.storage_policy;
delete second.storage_policy;
const template = fixture.emails[0];
fixture.emails = [
  { ...template, id: 9001, account_id: first.id, message_id: '<old@test>', date: 1_700_000_000, subject: 'Oldest budget message', body: 'o'.repeat(500_000), attachments: [], pinned: false, due: null, snoozed_until: null, done: false, folder: 'Inbox' },
  { ...template, id: 9002, account_id: first.id, message_id: '<new@test>', date: 1_700_000_100, subject: 'Newest budget message', body: 'n'.repeat(500_000), attachments: [], pinned: false, due: null, snoozed_until: null, done: false, folder: 'Inbox' },
  { ...template, id: 9003, account_id: second.id, message_id: '<other@test>', date: 1_600_000_000, subject: 'Other account message', body: 'Other account content stays intact.', attachments: [], pinned: false, due: null, snoozed_until: null, done: false, folder: 'Inbox' },
];
fixture.drafts = [];
fixture.undo_actions = [];
fixture.next_id = 10000;

const port = 1423;
const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: new URL('../', import.meta.url), stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
});
let serverOutput = '';
server.stdout.on('data', (data) => serverOutput += data);
server.stderr.on('data', (data) => serverOutput += data);
let browser;
try {
  let ready = false;
  for (let attempt = 0; attempt < 60; attempt++) {
    if (server.exitCode !== null) throw new Error(`Preview server stopped: ${serverOutput}`);
    try { ready = (await fetch(`http://127.0.0.1:${port}`)).ok; } catch { /* Starting. */ }
    if (ready) break;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  assert.ok(ready, `Preview server did not start: ${serverOutput}`);
  browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
  const page = await browser.newPage({ locale: 'en-US', viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.route(/\/src\/lib\/sample\.json/, (route) => route.fulfill({
    contentType: 'application/javascript', body: `export default ${JSON.stringify(fixture)};`,
  }));
  await page.route(/\/src\/lib\/api\.ts(?:\?t=\d+)?$/, (route) => route.fulfill({
    contentType: 'application/javascript',
    body: `
      export * from './api.ts?storage-test-original';
      import { call as originalCall } from './api.ts?storage-test-original';
      export async function call(command, args = {}) {
        const result = await originalCall(command, args);
        if (command === 'get_account_storage' && globalThis.__holdNextStorageRead) {
          globalThis.__holdNextStorageRead = false;
          await new Promise((resolve) => { globalThis.__releaseStorageRead = resolve; });
        }
        return result;
      }
    `,
  }));
  const snapshot = () => page.evaluate(async () => (await import('/src/lib/api.ts')).call('workspace_snapshot'));
  // The two Selects of the section: open the button, choose the item by its label.
  const mode = () => page.getByRole('combobox', { name: 'Local storage', exact: true });
  const choose = async (select, label) => {
    await select.click();
    await page.getByRole('option', { name: label, exact: typeof label === 'string' }).click();
  };
  async function openStorage() {
    await page.getByRole('complementary', { name: 'Sidebar' }).getByRole('button', { name: 'Alex Personal workspace' }).click();
    await page.getByRole('button', { name: 'Local storage', exact: true }).click();
    // ADR-0005: the account storage section is the only offline-mail control.
    assert.equal(await page.getByRole('button', { name: 'Offline mail', exact: true }).count(), 0);
    await expect(mode()).toBeVisible();
  }
  await page.goto(`http://127.0.0.1:${port}`);
  await page.waitForTimeout(1000);
  if (errors.length) throw new Error(`Preview errors: ${errors.join('; ')}`);
  await openStorage();
  await expect(mode()).toContainText('Unlimited');
  await page.evaluate(() => { window.__holdNextStorageRead = true; });
  await page.getByRole('button', { name: 'Refresh storage status', exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.__releaseStorageRead)).toBe('function');
  await choose(mode(), 'Limited');
  await page.getByLabel('Account storage limit in GB').fill('0.5');
  await page.getByRole('button', { name: 'Save account storage', exact: true }).click();
  await expect(page.getByLabel('Account storage usage')).toContainText('500.0 MB limit');
  await page.evaluate(async () => {
    window.__releaseStorageRead();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
  await expect(page.getByLabel('Account storage usage')).toContainText('500.0 MB limit');
  await choose(page.getByRole('combobox', { name: 'Storage account' }), new RegExp(second.email));
  await expect(mode()).toContainText('Unlimited');
  await choose(page.getByRole('combobox', { name: 'Storage account' }), new RegExp(first.email));
  await expect(page.getByLabel('Account storage limit in GB')).toHaveValue('0.5');
  await page.getByText('Keep all mail offline', { exact: true }).click();
  await expect(page.getByRole('switch', { name: 'Keep all mail offline' })).toBeChecked();
  await page.getByRole('button', { name: 'Save account storage', exact: true }).click();
  await expect(page.getByLabel('Account storage usage')).toContainText('Offline copy is incomplete');
  await page.getByLabel('Account storage limit in GB').fill('0.0007');
  await page.getByRole('button', { name: 'Save account storage', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('smaller than protected');
  assert.equal((await snapshot()).accounts.find((account) => account.id === first.id).storage_policy.limit.max_bytes, 500_000_000);
  await page.getByText('Keep all mail offline', { exact: true }).click();
  await expect(page.getByRole('switch', { name: 'Keep all mail offline' })).not.toBeChecked();
  await page.getByRole('button', { name: 'Save account storage', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Apply limit and clear local copies' })).toBeVisible();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  assert.equal((await snapshot()).emails.find((email) => email.id === 9001).body.length, 500_000);
  await page.getByRole('button', { name: 'Save account storage', exact: true }).click();
  await page.getByRole('button', { name: 'Apply limit and clear local copies' }).click();
  await expect(page.getByLabel('Account storage usage')).toContainText('700.0 KB limit');
  const cleaned = await snapshot();
  assert.equal(cleaned.emails.find((email) => email.id === 9001), undefined);
  assert.equal(cleaned.emails.find((email) => email.id === 9002).body.length, 500_000);
  assert.equal(cleaned.emails.find((email) => email.id === 9003).body, 'Other account content stays intact.');
  assert.equal(cleaned.accounts.find((account) => account.id === second.id).storage_policy.limit.mode, 'unlimited');
  await page.reload();
  await openStorage();
  await expect(page.getByLabel('Account storage limit in GB')).toHaveValue('0.0007');
  await choose(mode(), 'Unlimited');
  await page.getByRole('button', { name: 'Save account storage', exact: true }).click();
  await expect(page.getByLabel('Account storage usage')).toContainText('No limit');
  await expect(page.getByLabel('Account storage limit in GB')).toHaveCount(0);
  assert.deepEqual(errors, []);
  await page.screenshot({ path: '../checks/account-storage.png' });
  console.log('PASS per-account limits, stale refresh isolation, protected offline mail, explicit cleanup confirmation, oldest-only eviction, account isolation, reload persistence and unlimited mode');
} finally {
  await browser?.close();
  server.kill();
}
